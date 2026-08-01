//! OpenCode 会话使用追踪。
//!
//! OpenCode 将消息保存在 `~/.local/share/opencode/opencode.db`。本模块只读
//! 该数据库，提取已完成的 assistant 消息并写入 usage-pulse 的统计库。

use crate::calculator::{CostCalculator, TokenUsage};
use crate::database::Database;
use crate::error::Result;
use crate::schema::INPUT_TOKEN_SEMANTICS_LEGACY;
use crate::session_usage::{
    find_model_pricing, get_sync_state, metadata_modified_nanos, should_skip_session_insert,
    update_sync_state, DedupKey, SessionSyncResult,
};
use rust_decimal::Decimal;
use serde_json::Value;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Debug)]
struct OpenCodeMessageData {
    input_tokens: u32,
    output_tokens: u32,
    reasoning_tokens: u32,
    cache_read_tokens: u32,
    cache_write_tokens: u32,
    cost: f64,
    model_id: String,
    timestamp_ms: i64,
}

/// 同步 OpenCode SQLite 会话数据。
pub fn sync_opencode_usage(db: &Database) -> Result<SessionSyncResult> {
    let db_path = get_opencode_db_path();
    if !db_path.exists() {
        return Ok(SessionSyncResult::default());
    }

    let db_path_str = db_path.to_string_lossy().to_string();
    let metadata = fs::metadata(&db_path)?;
    let mut file_modified = metadata_modified_nanos(&metadata);
    let wal_path = db_path.with_extension("db-wal");
    if let Ok(wal_metadata) = fs::metadata(wal_path) {
        file_modified = file_modified.max(metadata_modified_nanos(&wal_metadata));
    }

    let (last_modified, _) = get_sync_state(db, &db_path_str)?;
    if file_modified <= last_modified {
        return Ok(SessionSyncResult {
            files_scanned: 1,
            ..Default::default()
        });
    }

    let source = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
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

        let message_result = query_assistant_messages(&source, session_id);
        let (messages, has_incomplete_usage) = match message_result {
            Ok(result) => result,
            Err(error) => {
                result
                    .errors
                    .push(format!("OpenCode 会话消息查询失败 {session_id}: {error}"));
                has_error = true;
                continue;
            }
        };

        for (message_id, message) in messages {
            let request_id = format!("opencode_session:{session_id}:{message_id}");
            match insert_opencode_message(db, &request_id, &message, session_id) {
                Ok(true) => result.imported = result.imported.saturating_add(1),
                Ok(false) => result.skipped = result.skipped.saturating_add(1),
                Err(error) => {
                    result
                        .errors
                        .push(format!("OpenCode 消息插入失败 {request_id}: {error}"));
                    result.skipped = result.skipped.saturating_add(1);
                    has_error = true;
                }
            }
        }

        if !has_incomplete_usage && !has_error {
            update_sync_state(db, &sync_key, *watermark, 0)?;
        }
    }

    if !has_error {
        update_sync_state(db, &db_path_str, file_modified, 0)?;
    }

    if result.imported > 0 {
        log::info!(
            "[OPENCODE-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 扫描 {} 个会话",
            result.imported,
            result.skipped,
            sessions.len()
        );
    }

    Ok(result)
}

fn get_opencode_db_path() -> PathBuf {
    if let Ok(custom_path) = env::var("OPENCODE_DB") {
        if !custom_path.is_empty() {
            let path = PathBuf::from(custom_path);
            if path.is_absolute() {
                return path;
            }
            return get_opencode_data_dir().join(path);
        }
    }
    get_opencode_data_dir().join("opencode.db")
}

fn get_opencode_data_dir() -> PathBuf {
    if let Ok(xdg_data_home) = env::var("XDG_DATA_HOME") {
        if !xdg_data_home.is_empty() {
            return PathBuf::from(xdg_data_home).join("opencode");
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("share")
        .join("opencode")
}

fn query_sessions(conn: &rusqlite::Connection) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT s.id,
                MAX(s.time_updated, COALESCE(MAX(m.time_updated), s.time_updated))
         FROM session s
         LEFT JOIN message m ON m.session_id = s.id
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

fn query_assistant_messages(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> Result<(Vec<(String, OpenCodeMessageData)>, bool)> {
    let mut stmt =
        conn.prepare("SELECT id, data FROM message WHERE session_id = ?1 ORDER BY time_created")?;
    let rows = stmt.query_map([session_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut messages = Vec::new();
    let mut has_incomplete_usage = false;

    for row in rows {
        let (message_id, data) = row?;
        let value: Value = match serde_json::from_str(&data) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if value.get("tokens").is_none() {
            continue;
        }
        if value
            .get("time")
            .and_then(|time| time.get("completed"))
            .is_none()
        {
            has_incomplete_usage = true;
            continue;
        }
        if let Some(message) = parse_message_data(&value) {
            messages.push((message_id, message));
        }
    }

    Ok((messages, has_incomplete_usage))
}

fn parse_message_data(value: &Value) -> Option<OpenCodeMessageData> {
    let tokens = value.get("tokens")?;
    let input_tokens = tokens.get("input").and_then(Value::as_u64).unwrap_or(0) as u32;
    let output_tokens = tokens.get("output").and_then(Value::as_u64).unwrap_or(0) as u32;
    let reasoning_tokens = tokens.get("reasoning").and_then(Value::as_u64).unwrap_or(0) as u32;
    let cache = tokens.get("cache");
    let cache_read_tokens = cache
        .and_then(|value| value.get("read"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let cache_write_tokens = cache
        .and_then(|value| value.get("write"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    if input_tokens == 0
        && output_tokens == 0
        && reasoning_tokens == 0
        && cache_read_tokens == 0
        && cache_write_tokens == 0
    {
        return None;
    }

    Some(OpenCodeMessageData {
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_read_tokens,
        cache_write_tokens,
        cost: value.get("cost").and_then(Value::as_f64).unwrap_or(0.0),
        model_id: value
            .get("modelID")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        timestamp_ms: value
            .get("time")
            .and_then(|time| time.get("created"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
    })
}

fn insert_opencode_message(
    db: &Database,
    request_id: &str,
    message: &OpenCodeMessageData,
    session_id: &str,
) -> Result<bool> {
    db.with_conn(|conn| {
        let created_at = if message.timestamp_ms > 0 {
            message.timestamp_ms / 1000
        } else {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0)
        };
        let output_tokens = message.output_tokens.saturating_add(message.reasoning_tokens);
        let dedup_key = DedupKey {
            app_type: "opencode",
            model: &message.model_id,
            input_tokens: message.input_tokens,
            output_tokens,
            cache_read_tokens: message.cache_read_tokens,
            cache_creation_tokens: message.cache_write_tokens,
            created_at,
        };
        if should_skip_session_insert(conn, request_id, &dedup_key)? {
            return Ok(false);
        }

        let (input_cost, output_cost, cache_read_cost, cache_creation_cost, total_cost) =
            if message.cost > 0.0 {
                (
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    message.cost.to_string(),
                )
            } else {
                let usage = TokenUsage {
                    input_tokens: message.input_tokens,
                    output_tokens,
                    cache_read_tokens: message.cache_read_tokens,
                    cache_creation_tokens: message.cache_write_tokens,
                    model: Some(message.model_id.clone()),
                    message_id: None,
                };
                find_model_pricing(conn, &message.model_id)
                    .map(|pricing| {
                        let cost = CostCalculator::calculate(&usage, &pricing, Decimal::ONE);
                        (
                            cost.input_cost.to_string(),
                            cost.output_cost.to_string(),
                            cost.cache_read_cost.to_string(),
                            cost.cache_creation_cost.to_string(),
                            cost.total_cost.to_string(),
                        )
                    })
                    .unwrap_or_else(|| {
                        ("0".into(), "0".into(), "0".into(), "0".into(), "0".into())
                    })
            };

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
                "_opencode_session",
                "opencode",
                message.model_id,
                message.model_id,
                message.input_tokens,
                output_tokens,
                message.cache_read_tokens,
                message.cache_write_tokens,
                INPUT_TOKEN_SEMANTICS_LEGACY,
                input_cost,
                output_cost,
                cache_read_cost,
                cache_creation_cost,
                total_cost,
                0i64,
                Option::<i64>::None,
                200i64,
                Option::<String>::None,
                Some(session_id.to_string()),
                Some("opencode_session"),
                1i64,
                "1.0",
                created_at,
                "opencode_session",
            ],
        )?;

        Ok(inserted > 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_opencode_usage_and_reasoning_tokens() {
        let value = serde_json::json!({
            "role": "assistant",
            "modelID": "gpt-5",
            "cost": 0.01,
            "tokens": {
                "input": 100,
                "output": 20,
                "reasoning": 5,
                "cache": {"read": 30, "write": 4}
            },
            "time": {"created": 1780000000000i64, "completed": 1780000001000i64}
        });
        let parsed = parse_message_data(&value).unwrap();
        assert_eq!(parsed.input_tokens, 100);
        assert_eq!(parsed.output_tokens, 20);
        assert_eq!(parsed.reasoning_tokens, 5);
        assert_eq!(parsed.cache_read_tokens, 30);
        assert_eq!(parsed.cache_write_tokens, 4);
    }
}
