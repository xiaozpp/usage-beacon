//! Gemini CLI 会话日志使用追踪。
//!
//! Gemini CLI 将每个会话保存为 `~/.gemini/tmp/*/chats/session-*.json`，
//! 每条 Gemini 消息携带独立的 token 统计。

use crate::calculator::{CostCalculator, TokenUsage};
use crate::database::Database;
use crate::error::Result;
use crate::schema::INPUT_TOKEN_SEMANTICS_LEGACY;
use crate::session_usage::{
    find_model_pricing, get_sync_state, metadata_modified_nanos, modified_nanos_to_seconds,
    parse_rfc3339_timestamp, should_skip_session_insert, update_sync_state, DedupKey,
    SessionSyncResult,
};
use rust_decimal::Decimal;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct GeminiTokens {
    input: u32,
    output: u32,
    cached: u32,
    thoughts: u32,
}

/// 同步 Gemini CLI 会话日志。
pub fn sync_gemini_usage(db: &Database) -> Result<SessionSyncResult> {
    let gemini_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".gemini");
    let files = collect_gemini_session_files(&gemini_dir);
    let mut result = SessionSyncResult {
        files_scanned: files.len() as u32,
        ..Default::default()
    };

    for file_path in files {
        match sync_single_gemini_file(db, &file_path) {
            Ok((imported, skipped)) => {
                result.imported = result.imported.saturating_add(imported);
                result.skipped = result.skipped.saturating_add(skipped);
            }
            Err(error) => {
                let message = format!("Gemini 会话文件解析失败 {}: {error}", file_path.display());
                log::warn!("[GEMINI-SYNC] {message}");
                result.errors.push(message);
            }
        }
    }

    if result.imported > 0 {
        log::info!(
            "[GEMINI-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 扫描 {} 个文件",
            result.imported,
            result.skipped,
            result.files_scanned
        );
    }

    Ok(result)
}

fn collect_gemini_session_files(gemini_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let tmp_dir = gemini_dir.join("tmp");
    let Ok(project_dirs) = fs::read_dir(tmp_dir) else {
        return files;
    };

    for project_dir in project_dirs.flatten() {
        let chats_dir = project_dir.path().join("chats");
        let Ok(chat_files) = fs::read_dir(chats_dir) else {
            continue;
        };
        for file in chat_files.flatten() {
            let path = file.path();
            let is_session = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("session-") && name.ends_with(".json"))
                .unwrap_or(false);
            if is_session {
                files.push(path);
            }
        }
    }

    files.sort();
    files
}

fn sync_single_gemini_file(db: &Database, file_path: &Path) -> Result<(u32, u32)> {
    let file_path_str = file_path.to_string_lossy().to_string();
    let metadata = fs::metadata(file_path)?;
    let file_modified = metadata_modified_nanos(&metadata);
    let (last_modified, _) = get_sync_state(db, &file_path_str)?;
    if file_modified <= last_modified {
        return Ok((0, 0));
    }

    let content = fs::read_to_string(file_path)?;
    let value: Value = serde_json::from_str(&content)?;
    let session_id = value.get("sessionId").and_then(Value::as_str);
    let Some(messages) = value.get("messages").and_then(Value::as_array) else {
        update_sync_state(db, &file_path_str, file_modified, 0)?;
        return Ok((0, 0));
    };

    let mut imported = 0;
    let mut skipped = 0;
    for message in messages {
        if message.get("type").and_then(Value::as_str) != Some("gemini") {
            continue;
        }
        let Some(tokens_value) = message.get("tokens") else {
            continue;
        };
        let tokens = parse_gemini_tokens(tokens_value);
        if tokens.is_zero() {
            continue;
        }

        let message_id = message
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let model = message
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let timestamp = message.get("timestamp").and_then(Value::as_str);
        let request_id = format!(
            "gemini_session:{}:{message_id}",
            session_id.unwrap_or("unknown")
        );

        match insert_gemini_session_entry(
            db,
            &request_id,
            tokens,
            model,
            session_id,
            timestamp,
            modified_nanos_to_seconds(file_modified),
        )? {
            true => imported += 1,
            false => skipped += 1,
        }
    }

    update_sync_state(db, &file_path_str, file_modified, messages.len() as i64)?;
    Ok((imported, skipped))
}

fn parse_gemini_tokens(value: &Value) -> GeminiTokens {
    GeminiTokens {
        input: value.get("input").and_then(Value::as_u64).unwrap_or(0) as u32,
        output: value.get("output").and_then(Value::as_u64).unwrap_or(0) as u32,
        cached: value.get("cached").and_then(Value::as_u64).unwrap_or(0) as u32,
        thoughts: value.get("thoughts").and_then(Value::as_u64).unwrap_or(0) as u32,
    }
}

impl GeminiTokens {
    fn is_zero(self) -> bool {
        self.input == 0 && self.output == 0 && self.cached == 0 && self.thoughts == 0
    }
}

fn insert_gemini_session_entry(
    db: &Database,
    request_id: &str,
    tokens: GeminiTokens,
    model: &str,
    session_id: Option<&str>,
    timestamp: Option<&str>,
    fallback_created_at: Option<i64>,
) -> Result<bool> {
    db.with_conn(|conn| {
        let Some(created_at) = parse_rfc3339_timestamp(timestamp).or(fallback_created_at) else {
            return Ok(false);
        };
        let output_tokens = tokens.output.saturating_add(tokens.thoughts);
        let dedup_key = DedupKey {
            app_type: "gemini",
            model,
            input_tokens: tokens.input,
            output_tokens,
            cache_read_tokens: tokens.cached,
            cache_creation_tokens: 0,
            created_at,
        };
        if should_skip_session_insert(conn, request_id, &dedup_key)? {
            return Ok(false);
        }

        let usage = TokenUsage {
            input_tokens: tokens.input,
            output_tokens,
            cache_read_tokens: tokens.cached,
            cache_creation_tokens: 0,
            model: Some(model.to_string()),
            message_id: None,
        };
        let costs = find_model_pricing(conn, model)
            .map(|pricing| CostCalculator::calculate_for_app("gemini", &usage, &pricing, Decimal::ONE));
        let (input_cost, output_cost, cache_read_cost, cache_creation_cost, total_cost) = costs
            .map(|cost| {
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
                "_gemini_session",
                "gemini",
                model,
                model,
                tokens.input,
                output_tokens,
                tokens.cached,
                0i64,
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
                session_id.map(ToOwned::to_owned),
                Some("gemini_session"),
                1i64,
                "1.0",
                created_at,
                "gemini_session",
            ],
        )?;

        Ok(inserted > 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gemini_tokens_and_includes_thoughts() {
        let tokens = parse_gemini_tokens(&serde_json::json!({
            "input": 100,
            "output": 20,
            "cached": 30,
            "thoughts": 7
        }));
        assert_eq!(
            tokens,
            GeminiTokens {
                input: 100,
                output: 20,
                cached: 30,
                thoughts: 7
            }
        );
        assert!(!tokens.is_zero());
    }
}
