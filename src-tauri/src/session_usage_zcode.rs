//! ZCode 会话使用追踪。
//!
//! ZCode CLI 将模型调用记录保存在 `~/.zcode/cli/db/db.sqlite` 的
//! `model_usage` 表中。本模块只读该数据库，按 `model_usage.id` 幂等写入
//! usage-pulse 的统计库，不读取会话正文或工具输出。

use crate::calculator::{CostCalculator, TokenUsage};
use crate::database::Database;
use crate::error::Result;
use crate::schema::{DATA_SOURCE_ZCODE_SESSION, INPUT_TOKEN_SEMANTICS_TOTAL};
use crate::session_usage::{
    find_model_pricing, get_sync_state, metadata_modified_nanos, should_skip_session_insert,
    update_sync_state, DedupKey, SessionSyncResult,
};
use rusqlite::Connection;
use rust_decimal::Decimal;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const ZCODE_APP_TYPE: &str = "zcode";
const ZCODE_PROVIDER_ID: &str = "_zcode_session";
const ZCODE_PROVIDER_TYPE: &str = "zcode_session";

#[derive(Debug)]
struct ZcodeModelUsage {
    id: String,
    model_id: String,
    input_tokens: u32,
    output_tokens: u32,
    reasoning_tokens: u32,
    cache_creation_tokens: u32,
    cache_read_tokens: u32,
    started_at_ms: Option<i64>,
    completed_at_ms: Option<i64>,
    duration_ms: Option<i64>,
    time_to_first_token_ms: Option<i64>,
}

/// 同步 ZCode CLI SQLite 用量。
pub fn sync_zcode_usage(db: &Database) -> Result<SessionSyncResult> {
    let db_path = get_zcode_db_path();
    if !db_path.exists() {
        return Ok(SessionSyncResult::default());
    }

    let db_path_str = db_path.to_string_lossy().to_string();
    let file_modified = source_modified_nanos(&db_path)?;
    let (last_modified, _) = get_sync_state(db, &db_path_str)?;
    if file_modified <= last_modified {
        return Ok(SessionSyncResult {
            files_scanned: 1,
            ..Default::default()
        });
    }

    let source = Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(Duration::from_secs(2))?;

    let sessions = query_sessions(&source)?;
    let mut result = SessionSyncResult {
        files_scanned: 1,
        ..Default::default()
    };
    let mut has_error = false;

    for (session_id, watermark) in sessions.iter() {
        let sync_key = format!("{db_path_str}:{session_id}");
        let (session_last_modified, _) = get_sync_state(db, &sync_key)?;
        if *watermark <= session_last_modified {
            continue;
        }

        let (usages, has_incomplete_usage) = query_model_usages(&source, session_id)?;
        let mut session_has_error = false;

        for usage in usages {
            let request_id = format!("zcode_session:{}", usage.id);
            match insert_zcode_usage(db, &request_id, &usage, session_id) {
                Ok(true) => result.imported = result.imported.saturating_add(1),
                Ok(false) => result.skipped = result.skipped.saturating_add(1),
                Err(error) => {
                    result
                        .errors
                        .push(format!("ZCode 用量插入失败 {request_id}: {error}"));
                    result.skipped = result.skipped.saturating_add(1);
                    session_has_error = true;
                    has_error = true;
                }
            }
        }

        if !has_incomplete_usage && !session_has_error {
            update_sync_state(db, &sync_key, *watermark, 0)?;
        }
    }

    if !has_error {
        update_sync_state(db, &db_path_str, file_modified, 0)?;
    }

    if result.imported > 0 {
        log::info!(
            "[ZCODE-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 扫描 {} 个会话",
            result.imported,
            result.skipped,
            sessions.len()
        );
    }

    Ok(result)
}

fn get_zcode_db_path() -> PathBuf {
    if let Some(custom_path) = env::var_os("ZCODE_DB") {
        let path = PathBuf::from(custom_path);
        if path.is_absolute() {
            return path;
        }
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(path);
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zcode")
        .join("cli")
        .join("db")
        .join("db.sqlite")
}

fn source_modified_nanos(db_path: &PathBuf) -> Result<i64> {
    let metadata = fs::metadata(db_path)?;
    let mut modified = metadata_modified_nanos(&metadata);
    for sidecar in [append_path_suffix(db_path, "-wal")] {
        if let Ok(sidecar_metadata) = fs::metadata(sidecar) {
            modified = modified.max(metadata_modified_nanos(&sidecar_metadata));
        }
    }
    Ok(modified)
}

fn append_path_suffix(path: &PathBuf, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn query_sessions(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT s.id,
                CASE
                    WHEN COALESCE(MAX(m.completed_at), 0) > COALESCE(s.time_updated, s.time_created, 0)
                    THEN COALESCE(MAX(m.completed_at), 0)
                    ELSE COALESCE(s.time_updated, s.time_created, 0)
                END
         FROM session s
         LEFT JOIN model_usage m ON m.session_id = s.id
         GROUP BY s.id
         ORDER BY 2",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row?);
    }
    Ok(sessions)
}

fn query_model_usages(conn: &Connection, session_id: &str) -> Result<(Vec<ZcodeModelUsage>, bool)> {
    let mut stmt = conn.prepare(
        "SELECT id, model_id, input_tokens, output_tokens, reasoning_tokens,
                cache_creation_input_tokens, cache_read_input_tokens,
                started_at, completed_at, duration_ms, time_to_first_token_ms
         FROM model_usage
         WHERE session_id = ?1
         ORDER BY COALESCE(started_at, completed_at, 0), id",
    )?;
    let rows = stmt.query_map([session_id], |row| {
        Ok(ZcodeModelUsage {
            id: row.get(0)?,
            model_id: row
                .get::<_, Option<String>>(1)?
                .filter(|model| !model.trim().is_empty())
                .unwrap_or_else(|| "unknown".to_string()),
            input_tokens: clamp_token_count(row.get::<_, Option<i64>>(2)?.unwrap_or(0)),
            output_tokens: clamp_token_count(row.get::<_, Option<i64>>(3)?.unwrap_or(0)),
            reasoning_tokens: clamp_token_count(row.get::<_, Option<i64>>(4)?.unwrap_or(0)),
            cache_creation_tokens: clamp_token_count(row.get::<_, Option<i64>>(5)?.unwrap_or(0)),
            cache_read_tokens: clamp_token_count(row.get::<_, Option<i64>>(6)?.unwrap_or(0)),
            started_at_ms: row.get(7)?,
            completed_at_ms: row.get(8)?,
            duration_ms: row.get(9)?,
            time_to_first_token_ms: row.get(10)?,
        })
    })?;

    let mut usages = Vec::new();
    let mut has_incomplete_usage = false;
    for row in rows {
        let usage = row?;
        if usage.completed_at_ms.is_none() {
            has_incomplete_usage = true;
            continue;
        }
        if usage.input_tokens == 0
            && usage.output_tokens == 0
            && usage.reasoning_tokens == 0
            && usage.cache_creation_tokens == 0
            && usage.cache_read_tokens == 0
        {
            continue;
        }
        usages.push(usage);
    }

    Ok((usages, has_incomplete_usage))
}

fn clamp_token_count(value: i64) -> u32 {
    value.clamp(0, u32::MAX as i64) as u32
}

fn timestamp_ms_to_unix_seconds(timestamp_ms: Option<i64>) -> i64 {
    timestamp_ms
        .filter(|timestamp| *timestamp > 0)
        .map(|timestamp| timestamp / 1000)
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0)
        })
}

fn insert_zcode_usage(
    db: &Database,
    request_id: &str,
    usage: &ZcodeModelUsage,
    session_id: &str,
) -> Result<bool> {
    db.with_conn(|conn| {
        let created_at = timestamp_ms_to_unix_seconds(usage.started_at_ms.or(usage.completed_at_ms));
        let output_tokens = usage.output_tokens.saturating_add(usage.reasoning_tokens);
        let dedup_key = DedupKey {
            app_type: ZCODE_APP_TYPE,
            model: &usage.model_id,
            input_tokens: usage.input_tokens,
            output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            created_at,
        };
        if should_skip_session_insert(conn, request_id, &dedup_key)? {
            return Ok(false);
        }

        let token_usage = TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            model: Some(usage.model_id.clone()),
            message_id: Some(usage.id.clone()),
        };
        let costs = find_model_pricing(conn, &usage.model_id)
            .map(|pricing| {
                let cost = CostCalculator::calculate_for_app(
                    ZCODE_APP_TYPE,
                    &token_usage,
                    &pricing,
                    Decimal::ONE,
                );
                (
                    cost.input_cost.to_string(),
                    cost.output_cost.to_string(),
                    cost.cache_read_cost.to_string(),
                    cost.cache_creation_cost.to_string(),
                    cost.total_cost.to_string(),
                )
            })
            .unwrap_or_else(|| ("0".into(), "0".into(), "0".into(), "0".into(), "0".into()));

        let inserted = conn.execute(
            "INSERT OR IGNORE INTO proxy_request_logs (
                request_id, provider_id, app_type, model, request_model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                input_token_semantics,
                input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd,
                latency_ms, first_token_ms, status_code, error_message, session_id,
                provider_type, is_streaming, cost_multiplier, created_at, data_source
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
            rusqlite::params![
                request_id,
                ZCODE_PROVIDER_ID,
                ZCODE_APP_TYPE,
                usage.model_id,
                usage.model_id,
                usage.input_tokens,
                output_tokens,
                usage.cache_read_tokens,
                usage.cache_creation_tokens,
                INPUT_TOKEN_SEMANTICS_TOTAL,
                costs.0,
                costs.1,
                costs.2,
                costs.3,
                costs.4,
                usage.duration_ms.unwrap_or(0).max(0),
                usage.time_to_first_token_ms.map(|value| value.max(0)),
                200i64,
                Option::<String>::None,
                Some(session_id.to_string()),
                Some(ZCODE_PROVIDER_TYPE),
                1i64,
                "1.0",
                created_at,
                DATA_SOURCE_ZCODE_SESSION,
            ],
        )?;

        Ok(inserted > 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_negative_and_large_token_counts() {
        assert_eq!(clamp_token_count(-1), 0);
        assert_eq!(clamp_token_count(i64::from(u32::MAX) + 1), u32::MAX);
    }

    #[test]
    fn converts_zcode_milliseconds_to_seconds() {
        assert_eq!(
            timestamp_ms_to_unix_seconds(Some(1_780_000_000_123)),
            1_780_000_000
        );
        assert!(timestamp_ms_to_unix_seconds(Some(0)) > 0);
    }

    #[test]
    fn reads_completed_usage_and_defers_incomplete_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                time_created INTEGER,
                time_updated INTEGER
            );
            CREATE TABLE model_usage (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                model_id TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                reasoning_tokens INTEGER,
                cache_creation_input_tokens INTEGER,
                cache_read_input_tokens INTEGER,
                started_at INTEGER,
                completed_at INTEGER,
                duration_ms INTEGER,
                time_to_first_token_ms INTEGER
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, time_created, time_updated) VALUES (?1, ?2, ?3)",
            rusqlite::params!["session-1", 1_000i64, 2_000i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO model_usage (
                id, session_id, model_id, input_tokens, output_tokens, reasoning_tokens,
                cache_creation_input_tokens, cache_read_input_tokens,
                started_at, completed_at, duration_ms, time_to_first_token_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                "usage-complete",
                "session-1",
                "GLM-5.2",
                100i64,
                20i64,
                5i64,
                4i64,
                30i64,
                1_780_000_000_000i64,
                1_780_000_001_000i64,
                1_000i64,
                200i64,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO model_usage (
                id, session_id, model_id, input_tokens, output_tokens, reasoning_tokens,
                cache_creation_input_tokens, cache_read_input_tokens, started_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                "usage-incomplete",
                "session-1",
                "GLM-5.2",
                50i64,
                0i64,
                0i64,
                0i64,
                0i64,
                1_780_000_002_000i64,
            ],
        )
        .unwrap();

        let sessions = query_sessions(&conn).unwrap();
        assert_eq!(sessions, vec![("session-1".to_string(), 1_780_000_001_000)]);

        let (usages, has_incomplete_usage) = query_model_usages(&conn, "session-1").unwrap();
        assert!(has_incomplete_usage);
        assert_eq!(usages.len(), 1);
        assert_eq!(usages[0].model_id, "GLM-5.2");
        assert_eq!(usages[0].input_tokens, 100);
        assert_eq!(usages[0].cache_read_tokens, 30);
        assert_eq!(usages[0].cache_creation_tokens, 4);
    }
}
