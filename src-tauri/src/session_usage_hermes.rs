//! Hermes Agent 会话用量同步。
//!
//! Hermes 将会话与按模型累计用量保存在 `%LOCALAPPDATA%/hermes/state.db`
//!（也可以通过 `HERMES_HOME` 覆盖）。本模块只读取会话元数据和
//! `session_model_usage` 的计数，不读取 messages 正文或工具输出。
//!
//! `session_model_usage` 是累计表而不是逐请求日志，因此这里维护一份本地
//! 水位快照，每次只把计数增长部分写入 usage-pulse。这样重复同步不会重复
//! 计算历史 token，同时能把后续新增用量落在实际同步时间附近。

use crate::calculator::{CostCalculator, TokenUsage};
use crate::database::Database;
use crate::error::Result;
use crate::schema::{DATA_SOURCE_HERMES_SESSION, INPUT_TOKEN_SEMANTICS_FRESH};
use crate::session_usage::{
    find_model_pricing, get_sync_state, metadata_modified_nanos, modified_nanos_to_seconds,
    project_name_from_path, update_session_project, update_sync_state, SessionSyncResult,
};
use chrono::Utc;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Row};
use rust_decimal::Decimal;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const APP_TYPE: &str = "hermes";
const PROVIDER_ID: &str = "_hermes_session";
const PROVIDER_TYPE: &str = DATA_SOURCE_HERMES_SESSION;

#[derive(Debug, Clone)]
struct HermesUsageRow {
    session_id: String,
    model: String,
    billing_provider: String,
    billing_base_url: String,
    billing_mode: String,
    task: String,
    request_count: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_creation_tokens: i64,
    reasoning_tokens: i64,
    estimated_cost_usd: f64,
    actual_cost_usd: f64,
    first_seen: f64,
    last_seen: f64,
    project: String,
}

#[derive(Debug, Clone, Default)]
struct HermesSnapshot {
    request_count: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_creation_tokens: i64,
    reasoning_tokens: i64,
    estimated_cost_usd: f64,
    actual_cost_usd: f64,
}

#[derive(Debug, Clone)]
struct HermesDelta {
    request_id: String,
    usage_key: String,
    row: HermesUsageRow,
    created_at: i64,
    request_count: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_creation_tokens: i64,
    costs: (String, String, String, String, String),
}

/// 同步 Hermes state.db 的累计用量。
pub fn sync_hermes_usage(db: &Database) -> Result<SessionSyncResult> {
    let db_path = get_hermes_db_path();
    if !db_path.exists() {
        return Ok(SessionSyncResult::default());
    }

    let db_path_string = db_path.to_string_lossy().to_string();
    let file_modified = sqlite_modified_nanos(&db_path)?;
    let source = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(Duration::from_secs(2))?;
    let rows = query_usage_rows(&source)?;

    // 项目目录可能在首次导入后才被补齐；即使数据库没有变化，也修正空项目。
    for row in &rows {
        update_session_project(db, APP_TYPE, &row.session_id, &row.project)?;
    }

    let mut result = SessionSyncResult {
        files_scanned: 1,
        ..Default::default()
    };
    let (last_modified, _) = get_sync_state(db, &db_path_string)?;
    if file_modified <= last_modified {
        return Ok(result);
    }

    let fallback_created_at =
        modified_nanos_to_seconds(file_modified).unwrap_or_else(|| Utc::now().timestamp());
    let row_count = rows.len();
    let (imported, skipped) = db.with_conn(|conn| {
        let mut deltas = Vec::new();
        for row in rows {
            let usage_key = usage_key(&row);
            let previous = load_snapshot(conn, &usage_key)?.unwrap_or_default();
            let delta = build_delta(conn, row, usage_key, previous, fallback_created_at)?;
            if let Some(delta) = delta {
                deltas.push(delta);
            }
        }

        let transaction = conn.unchecked_transaction()?;
        let mut imported = 0u32;
        let mut skipped = 0u32;
        for delta in deltas {
            let inserted = transaction.execute(
                "INSERT OR IGNORE INTO proxy_request_logs (
                    request_id, provider_id, app_type, model, request_model,
                    request_count, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    input_token_semantics,
                    input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd,
                    latency_ms, first_token_ms, status_code, error_message, session_id,
                    provider_type, is_streaming, cost_multiplier, created_at, data_source, project
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                    ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21,
                    ?22, ?23, ?24, ?25, ?26, ?27
                )",
                rusqlite::params![
                    delta.request_id,
                    PROVIDER_ID,
                    APP_TYPE,
                    delta.row.model,
                    delta.row.model,
                    delta.request_count,
                    delta.input_tokens,
                    delta.output_tokens,
                    delta.cache_read_tokens,
                    delta.cache_creation_tokens,
                    INPUT_TOKEN_SEMANTICS_FRESH,
                    delta.costs.0,
                    delta.costs.1,
                    delta.costs.2,
                    delta.costs.3,
                    delta.costs.4,
                    0i64,
                    Option::<i64>::None,
                    200i64,
                    Option::<String>::None,
                    Some(delta.row.session_id.clone()),
                    PROVIDER_TYPE,
                    1i64,
                    "1.0",
                    delta.created_at,
                    DATA_SOURCE_HERMES_SESSION,
                    delta.row.project,
                ],
            )?;
            if inserted > 0 {
                imported = imported.saturating_add(1);
            } else {
                skipped = skipped.saturating_add(1);
            }

            transaction.execute(
                "INSERT OR REPLACE INTO hermes_usage_sync (
                    usage_key, session_id, model, billing_provider, billing_base_url,
                    billing_mode, task, request_count, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens, reasoning_tokens,
                    estimated_cost_usd, actual_cost_usd, last_seen, updated_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13, ?14, ?15, ?16, ?17
                )",
                rusqlite::params![
                    delta.usage_key,
                    delta.row.session_id,
                    delta.row.model,
                    delta.row.billing_provider,
                    delta.row.billing_base_url,
                    delta.row.billing_mode,
                    delta.row.task,
                    delta.row.request_count,
                    delta.row.input_tokens,
                    delta.row.output_tokens,
                    delta.row.cache_read_tokens,
                    delta.row.cache_creation_tokens,
                    delta.row.reasoning_tokens,
                    delta.row.estimated_cost_usd.to_string(),
                    delta.row.actual_cost_usd.to_string(),
                    delta.row.last_seen,
                    Utc::now().timestamp(),
                ],
            )?;
        }
        transaction.commit()?;
        Ok((imported, skipped))
    })?;

    result.imported = imported;
    result.skipped = skipped;
    update_sync_state(db, &db_path_string, file_modified, 0)?;

    if result.imported > 0 {
        log::info!(
            "[HERMES-SYNC] 同步完成: 导入 {} 条增量, 跳过 {} 条, 扫描 {} 个累计用量行",
            result.imported,
            result.skipped,
            row_count
        );
    }

    Ok(result)
}

fn get_hermes_db_path() -> PathBuf {
    get_hermes_dir().join("state.db")
}

fn get_hermes_dir() -> PathBuf {
    if let Some(value) = env::var_os("HERMES_HOME") {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            return path;
        }
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(path);
    }

    dirs::data_local_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hermes")
}

fn sqlite_modified_nanos(path: &Path) -> Result<i64> {
    let metadata = fs::metadata(path)?;
    let mut modified = metadata_modified_nanos(&metadata);
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        if let Ok(sidecar_metadata) = fs::metadata(PathBuf::from(sidecar)) {
            modified = modified.max(metadata_modified_nanos(&sidecar_metadata));
        }
    }
    Ok(modified)
}

fn query_usage_rows(conn: &Connection) -> Result<Vec<HermesUsageRow>> {
    let has_model_usage: bool = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'session_model_usage'
        )",
        [],
        |row| row.get(0),
    )?;

    if has_model_usage {
        let mut stmt = conn.prepare(
            "SELECT
                u.session_id,
                COALESCE(NULLIF(u.model, ''), 'unknown'),
                COALESCE(u.billing_provider, ''),
                COALESCE(u.billing_base_url, ''),
                COALESCE(u.billing_mode, ''),
                COALESCE(u.task, ''),
                COALESCE(u.api_call_count, 0),
                COALESCE(u.input_tokens, 0),
                COALESCE(u.output_tokens, 0),
                COALESCE(u.cache_read_tokens, 0),
                COALESCE(u.cache_write_tokens, 0),
                COALESCE(u.reasoning_tokens, 0),
                COALESCE(u.estimated_cost_usd, 0),
                COALESCE(u.actual_cost_usd, 0),
                COALESCE(u.first_seen, s.started_at, 0),
                COALESCE(u.last_seen, s.ended_at, s.started_at, 0),
                COALESCE(NULLIF(s.cwd, ''), NULLIF(s.git_repo_root, ''), '')
             FROM session_model_usage u
             LEFT JOIN sessions s ON s.id = u.session_id
             WHERE COALESCE(u.api_call_count, 0)
                 + COALESCE(u.input_tokens, 0)
                 + COALESCE(u.output_tokens, 0)
                 + COALESCE(u.cache_read_tokens, 0)
                 + COALESCE(u.cache_write_tokens, 0)
                 + COALESCE(u.reasoning_tokens, 0) > 0
             ORDER BY COALESCE(u.last_seen, u.first_seen, s.ended_at, s.started_at, 0), u.session_id, u.model",
        )?;
        let rows = stmt
            .query_map([], parse_usage_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if !rows.is_empty() {
            return Ok(rows);
        }
    }

    // 兼容旧版 Hermes：历史版本只有 sessions 的累计字段。
    let mut stmt = conn.prepare(
        "SELECT
            s.id,
            COALESCE(NULLIF(s.model, ''), 'unknown'),
            COALESCE(s.billing_provider, ''),
            COALESCE(s.billing_base_url, ''),
            COALESCE(s.billing_mode, ''),
            '',
            COALESCE(s.api_call_count, 0),
            COALESCE(s.input_tokens, 0),
            COALESCE(s.output_tokens, 0),
            COALESCE(s.cache_read_tokens, 0),
            COALESCE(s.cache_write_tokens, 0),
            COALESCE(s.reasoning_tokens, 0),
            COALESCE(s.estimated_cost_usd, 0),
            COALESCE(s.actual_cost_usd, 0),
            COALESCE(s.started_at, 0),
            COALESCE(s.ended_at, s.started_at, 0),
            COALESCE(NULLIF(s.cwd, ''), NULLIF(s.git_repo_root, ''), '')
         FROM sessions s
         WHERE COALESCE(s.api_call_count, 0)
             + COALESCE(s.input_tokens, 0)
             + COALESCE(s.output_tokens, 0)
             + COALESCE(s.cache_read_tokens, 0)
             + COALESCE(s.cache_write_tokens, 0)
             + COALESCE(s.reasoning_tokens, 0) > 0
         ORDER BY COALESCE(s.ended_at, s.started_at, 0), s.id",
    )?;
    let rows = stmt
        .query_map([], parse_usage_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn parse_usage_row(row: &Row<'_>) -> rusqlite::Result<HermesUsageRow> {
    let project_path: String = row.get(16)?;
    Ok(HermesUsageRow {
        session_id: row.get(0)?,
        model: row.get(1)?,
        billing_provider: row.get(2)?,
        billing_base_url: row.get(3)?,
        billing_mode: row.get(4)?,
        task: row.get(5)?,
        request_count: non_negative_i64(row.get(6)?),
        input_tokens: non_negative_i64(row.get(7)?),
        output_tokens: non_negative_i64(row.get(8)?),
        cache_read_tokens: non_negative_i64(row.get(9)?),
        cache_creation_tokens: non_negative_i64(row.get(10)?),
        reasoning_tokens: non_negative_i64(row.get(11)?),
        estimated_cost_usd: non_negative_f64(row.get(12)?),
        actual_cost_usd: non_negative_f64(row.get(13)?),
        first_seen: non_negative_f64(row.get(14)?),
        last_seen: non_negative_f64(row.get(15)?),
        project: project_name_from_path(Some(&project_path)),
    })
}

fn load_snapshot(conn: &Connection, usage_key: &str) -> Result<Option<HermesSnapshot>> {
    conn.query_row(
        "SELECT request_count, input_tokens, output_tokens, cache_read_tokens,
                cache_creation_tokens, reasoning_tokens, estimated_cost_usd, actual_cost_usd
         FROM hermes_usage_sync WHERE usage_key = ?1",
        [usage_key],
        |row| {
            Ok(HermesSnapshot {
                request_count: non_negative_i64(row.get(0)?),
                input_tokens: non_negative_i64(row.get(1)?),
                output_tokens: non_negative_i64(row.get(2)?),
                cache_read_tokens: non_negative_i64(row.get(3)?),
                cache_creation_tokens: non_negative_i64(row.get(4)?),
                reasoning_tokens: non_negative_i64(row.get(5)?),
                estimated_cost_usd: non_negative_f64(decimal_value(row, 6)?),
                actual_cost_usd: non_negative_f64(decimal_value(row, 7)?),
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn decimal_value(row: &Row<'_>, index: usize) -> rusqlite::Result<f64> {
    match row.get::<_, rusqlite::types::Value>(index)? {
        rusqlite::types::Value::Integer(value) => Ok(value as f64),
        rusqlite::types::Value::Real(value) => Ok(value),
        rusqlite::types::Value::Text(value) => Ok(value.parse::<f64>().unwrap_or(0.0)),
        rusqlite::types::Value::Null | rusqlite::types::Value::Blob(_) => Ok(0.0),
    }
}

fn build_delta(
    conn: &Connection,
    row: HermesUsageRow,
    usage_key: String,
    previous: HermesSnapshot,
    fallback_created_at: i64,
) -> Result<Option<HermesDelta>> {
    let request_count = delta_i64(row.request_count, previous.request_count);
    let input_tokens = delta_i64(row.input_tokens, previous.input_tokens);
    let output_tokens = delta_i64(row.output_tokens, previous.output_tokens);
    let cache_read_tokens = delta_i64(row.cache_read_tokens, previous.cache_read_tokens);
    let cache_creation_tokens =
        delta_i64(row.cache_creation_tokens, previous.cache_creation_tokens);
    // Hermes keeps reasoning_tokens as a detail of output usage. The shared
    // usage-pulse schema has no separate reasoning column, so keep the
    // counter in the snapshot without adding it a second time to output.
    let _reasoning_tokens = delta_i64(row.reasoning_tokens, previous.reasoning_tokens);
    let estimated_cost_usd = delta_f64(row.estimated_cost_usd, previous.estimated_cost_usd);
    let actual_cost_usd = delta_f64(row.actual_cost_usd, previous.actual_cost_usd);

    let has_token_delta =
        input_tokens > 0 || output_tokens > 0 || cache_read_tokens > 0 || cache_creation_tokens > 0;
    let request_count = if request_count > 0 {
        request_count
    } else if has_token_delta {
        1
    } else {
        0
    };
    if request_count == 0 && !has_token_delta {
        return Ok(None);
    }

    let usage = TokenUsage {
        input_tokens: clamp_token_count(input_tokens),
        output_tokens: clamp_token_count(output_tokens),
        cache_read_tokens: clamp_token_count(cache_read_tokens),
        cache_creation_tokens: clamp_token_count(cache_creation_tokens),
        model: Some(row.model.clone()),
        message_id: None,
    };
    let costs = if let Some(pricing) = find_model_pricing(conn, &row.model) {
        let cost = CostCalculator::calculate(&usage, &pricing, Decimal::ONE);
        (
            cost.input_cost.to_string(),
            cost.output_cost.to_string(),
            cost.cache_read_cost.to_string(),
            cost.cache_creation_cost.to_string(),
            cost.total_cost.to_string(),
        )
    } else {
        let reported_cost = if actual_cost_usd > 0.0 {
            actual_cost_usd
        } else {
            estimated_cost_usd
        };
        (
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
            reported_cost.to_string(),
        )
    };

    let snapshot_stamp = format!(
        "{}:{}:{}:{}:{}:{}:{}",
        row.request_count,
        row.input_tokens,
        row.output_tokens,
        row.cache_read_tokens,
        row.cache_creation_tokens,
        row.reasoning_tokens,
        row.last_seen
    );
    let request_id = format!(
        "{DATA_SOURCE_HERMES_SESSION}:{:016x}:{:016x}",
        stable_hash(&usage_key),
        stable_hash(&snapshot_stamp),
    );
    let created_at = timestamp_from_hermes(row.last_seen).unwrap_or(fallback_created_at);

    Ok(Some(HermesDelta {
        request_id,
        usage_key,
        row,
        created_at,
        request_count,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        costs,
    }))
}

fn usage_key(row: &HermesUsageRow) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        row.session_id,
        row.model,
        row.billing_provider,
        row.billing_base_url,
        row.billing_mode,
        row.task,
    )
}

fn stable_hash(value: &str) -> u64 {
    // FNV-1a：不依赖随机种子，保证同一 Hermes 累计行跨进程生成相同 ID。
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn delta_i64(current: i64, previous: i64) -> i64 {
    if current >= previous {
        current - previous
    } else {
        current
    }
}

fn delta_f64(current: f64, previous: f64) -> f64 {
    if current >= previous {
        (current - previous).max(0.0)
    } else {
        current.max(0.0)
    }
}

fn timestamp_from_hermes(value: f64) -> Option<i64> {
    if value <= 0.0 {
        return None;
    }
    Some(if value > 100_000_000_000.0 {
        (value / 1_000.0) as i64
    } else {
        value as i64
    })
}

fn clamp_token_count(value: i64) -> u32 {
    value.clamp(0, u32::MAX as i64) as u32
}

fn non_negative_i64(value: i64) -> i64 {
    value.max(0)
}

fn non_negative_f64(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cumulative_delta_handles_growth_and_reset() {
        assert_eq!(delta_i64(120, 100), 20);
        assert_eq!(delta_i64(20, 100), 20);
        assert!((delta_f64(1.25, 0.75) - 0.5).abs() < f64::EPSILON);
        assert!((delta_f64(0.2, 1.0) - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn normalizes_hermes_timestamps() {
        assert_eq!(timestamp_from_hermes(1_780_000_000.5), Some(1_780_000_000));
        assert_eq!(
            timestamp_from_hermes(1_780_000_000_123.0),
            Some(1_780_000_000)
        );
        assert_eq!(timestamp_from_hermes(0.0), None);
    }

    #[test]
    fn reads_usage_without_touching_message_content() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                model TEXT,
                billing_provider TEXT,
                billing_base_url TEXT,
                billing_mode TEXT,
                api_call_count INTEGER,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_read_tokens INTEGER,
                cache_write_tokens INTEGER,
                reasoning_tokens INTEGER,
                estimated_cost_usd REAL,
                actual_cost_usd REAL,
                started_at REAL,
                ended_at REAL,
                cwd TEXT,
                git_repo_root TEXT
            );
            CREATE TABLE session_model_usage (
                session_id TEXT,
                model TEXT,
                billing_provider TEXT,
                billing_base_url TEXT,
                billing_mode TEXT,
                task TEXT,
                api_call_count INTEGER,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_read_tokens INTEGER,
                cache_write_tokens INTEGER,
                reasoning_tokens INTEGER,
                estimated_cost_usd REAL,
                actual_cost_usd REAL,
                first_seen REAL,
                last_seen REAL
            );
            CREATE TABLE messages (id INTEGER, content TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions (id, cwd, started_at, ended_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                "session-1",
                r"D:\work\demo-project",
                1_780_000_000.0,
                1_780_000_010.0
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_model_usage (
                session_id, model, task, api_call_count, input_tokens, output_tokens,
                cache_read_tokens, cache_write_tokens, first_seen, last_seen
             ) VALUES (?1, ?2, '', 3, 100, 20, 40, 5, 1780000000, 1780000010)",
            rusqlite::params!["session-1", "demo-model"],
        )
        .unwrap();

        let rows = query_usage_rows(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].request_count, 3);
        assert_eq!(rows[0].input_tokens, 100);
        assert_eq!(rows[0].cache_creation_tokens, 5);
        assert_eq!(rows[0].project, "demo-project");
    }

    #[test]
    fn reads_decimal_values_from_text_snapshot_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE hermes_usage_sync (
                usage_key TEXT PRIMARY KEY,
                request_count INTEGER,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_read_tokens INTEGER,
                cache_creation_tokens INTEGER,
                reasoning_tokens INTEGER,
                estimated_cost_usd TEXT,
                actual_cost_usd TEXT
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO hermes_usage_sync VALUES (?1, 1, 2, 3, 4, 5, 6, ?2, ?3)",
            rusqlite::params!["key", "1.25", "0.5"],
        )
        .unwrap();

        let snapshot = load_snapshot(&conn, "key").unwrap().unwrap();
        assert!((snapshot.estimated_cost_usd - 1.25).abs() < f64::EPSILON);
        assert!((snapshot.actual_cost_usd - 0.5).abs() < f64::EPSILON);
    }
}
