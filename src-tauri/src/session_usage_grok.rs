//! Grok Build 会话日志使用追踪。
//!
//! Grok CLI 将每轮独立用量写入 `~/.grok/**/updates.jsonl` 的
//! `turn_completed` 事件。这里按 CC Switch 的规则逐轮入账：不做相邻事件
//! 差分，先等待 10 分钟沉降，再以 `prompt_id + model` 幂等写入。

use crate::calculator::{CostCalculator, TokenUsage};
use crate::database::{get_grok_config_dir, Database};
use crate::error::Result;
use crate::schema::INPUT_TOKEN_SEMANTICS_TOTAL;
use crate::session_usage::{
    find_model_pricing, get_sync_state, metadata_modified_nanos, should_skip_session_insert,
    update_sync_state, DedupKey, SessionSyncResult,
};
use rust_decimal::Decimal;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const SETTLE_WINDOW_SECONDS: i64 = 10 * 60;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct GrokCounters {
    input: u64,
    output: u64,
    cached: u64,
    api_ms: u64,
    cost_ticks: u64,
    cost_partial: bool,
}

impl GrokCounters {
    fn is_zero(self) -> bool {
        self.input == 0 && self.output == 0 && self.cached == 0
    }

    fn reported_cost(self) -> Option<Decimal> {
        (self.cost_ticks > 0)
            .then(|| Decimal::from(self.cost_ticks) / Decimal::from(10_000_000_000u64))
    }
}

#[derive(Debug)]
struct GrokUsageEvent {
    created_at: i64,
    prompt_id: String,
    cost_is_partial: bool,
    per_model: Vec<(String, GrokCounters)>,
}

/// 同步 Grok Build 会话日志。
pub fn sync_grok_usage(db: &Database) -> Result<SessionSyncResult> {
    let files = collect_grok_updates_files();
    let mut result = SessionSyncResult {
        files_scanned: files.len() as u32,
        ..Default::default()
    };

    for path in files {
        match sync_single_grok_file(db, &path) {
            Ok(file_result) => result.merge(file_result),
            Err(error) => result
                .errors
                .push(format!("Grok 会话文件解析失败 {}: {error}", path.display())),
        }
    }
    Ok(result)
}

fn collect_grok_updates_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for root in [
        get_grok_config_dir().join("sessions"),
        get_grok_config_dir().join("archived_sessions"),
    ] {
        collect_files_named(&root, &mut files);
    }
    files.sort();
    files
}

fn collect_files_named(root: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_named(&path, files);
        } else if path.file_name().and_then(|name| name.to_str()) == Some("updates.jsonl") {
            files.push(path);
        }
    }
}

fn sync_single_grok_file(db: &Database, file_path: &Path) -> Result<SessionSyncResult> {
    let file_path_string = file_path.to_string_lossy().to_string();
    let metadata = fs::metadata(file_path)?;
    let file_modified = metadata_modified_nanos(&metadata);
    let (last_modified, _) = get_sync_state(db, &file_path_string)?;
    if file_modified <= last_modified {
        return Ok(SessionSyncResult::default());
    }

    let content = fs::read_to_string(file_path)?;
    let events = parse_grok_usage_events(&content);
    let session_id = file_path
        .parent()
        .and_then(|directory| directory.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string();
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);

    let mut result = SessionSyncResult::default();
    let mut deferred = false;
    for (index, event) in events.iter().enumerate() {
        if now.saturating_sub(event.created_at) < SETTLE_WINDOW_SECONDS {
            deferred = true;
            break;
        }

        let turn_key = if event.prompt_id.is_empty() {
            format!("idx{index}")
        } else {
            event.prompt_id.clone()
        };
        for (model, counters) in &event.per_model {
            if counters.is_zero() {
                continue;
            }
            let request_id = format!("grok_session:{session_id}:{turn_key}:{model}");
            match insert_grok_session_entry(
                db,
                &request_id,
                counters,
                event.cost_is_partial || counters.cost_partial,
                model,
                &session_id,
                event.created_at,
            )? {
                true => result.imported = result.imported.saturating_add(1),
                false => result.skipped = result.skipped.saturating_add(1),
            }
        }
    }

    if deferred {
        result.deferred_files = 1;
    } else {
        update_sync_state(&db, &file_path_string, file_modified, events.len() as i64)?;
    }
    Ok(result)
}

fn parse_grok_usage_events(content: &str) -> Vec<GrokUsageEvent> {
    let mut events = Vec::new();
    for line in content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if record.get("method").and_then(Value::as_str) != Some("_x.ai/session/update") {
            continue;
        }
        let update = record.get("params").and_then(|params| params.get("update"));
        let kind = update
            .and_then(|update| update.get("sessionUpdate"))
            .and_then(Value::as_str);
        if kind.is_some() && kind != Some("turn_completed") {
            continue;
        }
        let Some(usage) = update
            .and_then(|update| update.get("usage"))
            .filter(|usage| usage.is_object())
        else {
            continue;
        };
        let Some(created_at) = parse_event_timestamp(record.get("timestamp")) else {
            continue;
        };

        let prompt_id = update
            .and_then(|update| update.get("prompt_id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let mut per_model: Vec<(String, GrokCounters)> = usage
            .get("modelUsage")
            .and_then(Value::as_object)
            .map(|models| {
                models
                    .iter()
                    .map(|(model, counters)| (model.clone(), parse_grok_counters(counters)))
                    .collect()
            })
            .unwrap_or_default();
        if per_model.is_empty() {
            per_model.push(("unknown".to_string(), parse_grok_counters(usage)));
        }
        per_model.sort_by(|left, right| left.0.cmp(&right.0));

        events.push(GrokUsageEvent {
            created_at,
            prompt_id,
            cost_is_partial: usage
                .get("costIsPartial")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            per_model,
        });
    }
    events
}

fn parse_grok_counters(value: &Value) -> GrokCounters {
    let get = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
    GrokCounters {
        input: get("inputTokens"),
        output: get("outputTokens"),
        cached: get("cachedReadTokens"),
        api_ms: get("apiDurationMs"),
        cost_ticks: get("costUsdTicks"),
        cost_partial: value
            .get("costIsPartial")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

fn parse_event_timestamp(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    if let Some(number) = value.as_i64() {
        return Some(if number > 100_000_000_000 {
            number / 1000
        } else {
            number
        });
    }
    value
        .as_str()
        .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| timestamp.timestamp())
}

fn has_recent_proxy_activity(db: &Database, created_at: i64) -> Result<bool> {
    db.with_conn(|conn| {
        conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM proxy_request_logs
                WHERE COALESCE(data_source, 'proxy') = 'proxy'
                  AND app_type = 'grokbuild'
                  AND created_at BETWEEN ?1 - ?2 AND ?1 + ?2
            )",
            rusqlite::params![created_at, SETTLE_WINDOW_SECONDS],
            |row| row.get(0),
        )
        .map_err(Into::into)
    })
}

fn insert_grok_session_entry(
    db: &Database,
    request_id: &str,
    counters: &GrokCounters,
    cost_is_partial: bool,
    model: &str,
    session_id: &str,
    created_at: i64,
) -> Result<bool> {
    if has_recent_proxy_activity(db, created_at)? {
        return Ok(false);
    }

    db.with_conn(|conn| {
        let clamp = |value: u64| value.min(u32::MAX as u64) as u32;
        let usage = TokenUsage {
            input_tokens: clamp(counters.input),
            output_tokens: clamp(counters.output),
            cache_read_tokens: clamp(counters.cached),
            cache_creation_tokens: 0,
            model: Some(model.to_string()),
            message_id: None,
        };
        let key = DedupKey {
            app_type: "grokbuild",
            model,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            cache_creation_tokens: 0,
            created_at,
        };
        if should_skip_session_insert(conn, request_id, &key)? {
            return Ok(false);
        }

        let pricing = find_model_pricing(conn, model);
        let cost = pricing.map(|pricing| {
            CostCalculator::calculate_for_app("grokbuild", &usage, &pricing, Decimal::ONE)
        });
        let (input_cost, output_cost, cache_read_cost, cache_creation_cost, total_cost) =
            match cost {
                Some(cost) => {
                    let total = if !cost_is_partial {
                        counters.reported_cost().unwrap_or(cost.total_cost)
                    } else {
                        cost.total_cost
                    };
                    (
                        cost.input_cost.to_string(),
                        cost.output_cost.to_string(),
                        cost.cache_read_cost.to_string(),
                        cost.cache_creation_cost.to_string(),
                        total.to_string(),
                    )
                }
                None => (
                    "0".into(),
                    "0".into(),
                    "0".into(),
                    "0".into(),
                    counters
                        .reported_cost()
                        .unwrap_or(Decimal::ZERO)
                        .to_string(),
                ),
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
                "_grok_session",
                "grokbuild",
                model,
                model,
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                0i64,
                INPUT_TOKEN_SEMANTICS_TOTAL,
                input_cost,
                output_cost,
                cache_read_cost,
                cache_creation_cost,
                total_cost,
                counters.api_ms.min(i64::MAX as u64) as i64,
                Option::<i64>::None,
                200i64,
                Option::<String>::None,
                session_id,
                Some("grok_session"),
                1i64,
                "1.0",
                created_at,
                "grok_session",
            ],
        )?;
        Ok(inserted > 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_completed_turns_and_model_usage() {
        let content = concat!(
            r#"{"timestamp":1700000000,"method":"_x.ai/session/update","params":{"update":{"sessionUpdate":"usage_snapshot","usage":{"inputTokens":999}}}}"#,
            "\n",
            r#"{"timestamp":1700000060,"method":"_x.ai/session/update","params":{"update":{"sessionUpdate":"turn_completed","prompt_id":"p1","usage":{"modelUsage":{"grok-4.5-build":{"inputTokens":100,"outputTokens":10,"cachedReadTokens":20}}}}}}"#,
        );
        let events = parse_grok_usage_events(content);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].prompt_id, "p1");
        assert_eq!(events[0].per_model[0].0, "grok-4.5-build");
        assert_eq!(events[0].per_model[0].1.input, 100);
        assert_eq!(events[0].per_model[0].1.cached, 20);
    }

    #[test]
    fn parses_millisecond_epoch_timestamps() {
        assert_eq!(
            parse_event_timestamp(Some(&Value::from(1_700_000_000_000i64))),
            Some(1_700_000_000)
        );
    }
}
