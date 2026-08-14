//! Claude Code 会话日志使用追踪
//!
//! 从 ~/.claude/projects/ 下的 JSONL 会话文件中提取 token 使用数据，
//! 实现无代理模式下的使用统计。
//!
//! 数据流:
//! ```text
//! ~/.claude/projects/*/*.jsonl → 增量解析 → 去重 → 费用计算 → proxy_request_logs 表
//! ```

use crate::calculator::{CostCalculator, ModelPricing, TokenUsage};
use crate::database::{get_claude_config_dir, Database};
use crate::error::Result;
use crate::schema::SESSION_REQUEST_ID_PREFIX;
use chrono::DateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 同步结果
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSyncResult {
    pub imported: u32,
    pub skipped: u32,
    pub files_scanned: u32,
    pub suspected_duplicates: u32,
    pub deferred_files: u32,
    pub errors: Vec<String>,
}

impl SessionSyncResult {
    pub fn merge(&mut self, other: SessionSyncResult) {
        self.imported = self.imported.saturating_add(other.imported);
        self.skipped = self.skipped.saturating_add(other.skipped);
        self.files_scanned = self.files_scanned.saturating_add(other.files_scanned);
        self.suspected_duplicates = self
            .suspected_duplicates
            .saturating_add(other.suspected_duplicates);
        self.deferred_files = self.deferred_files.saturating_add(other.deferred_files);
        self.errors.extend(other.errors);
    }
}

/// 从 JSONL 中解析出的 assistant 消息使用数据
#[derive(Debug)]
struct ParsedAssistantUsage {
    message_id: String,
    model: String,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_creation_tokens: u32,
    stop_reason: Option<String>,
    timestamp: Option<String>,
    session_id: Option<String>,
}

/// 去重检查的指纹
pub(crate) struct DedupKey<'a> {
    pub(crate) app_type: &'a str,
    pub(crate) model: &'a str,
    pub(crate) input_tokens: u32,
    pub(crate) output_tokens: u32,
    pub(crate) cache_read_tokens: u32,
    pub(crate) cache_creation_tokens: u32,
    pub(crate) created_at: i64,
}

/// 同步 Claude Code 会话日志到使用统计数据库
pub fn sync_claude_session_logs(db: &Database) -> Result<SessionSyncResult> {
    let projects_dir = get_claude_config_dir().join("projects");
    if !projects_dir.exists() {
        return Ok(SessionSyncResult::default());
    }

    let mut result = SessionSyncResult::default();
    let jsonl_files = collect_jsonl_files(&projects_dir);

    for file_path in &jsonl_files {
        result.files_scanned += 1;
        match sync_single_file(db, file_path) {
            Ok((imported, skipped)) => {
                result.imported += imported;
                result.skipped += skipped;
            }
            Err(e) => {
                let msg = format!("{}: {e}", file_path.display());
                log::warn!("[SESSION-SYNC] 文件解析失败: {msg}");
                result.errors.push(msg);
            }
        }
    }

    if result.imported > 0 {
        log::info!(
            "[SESSION-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 扫描 {} 个文件",
            result.imported,
            result.skipped,
            result.files_scanned
        );
    }

    Ok(result)
}

/// 同步所有支持的本地会话日志来源。
///
/// 每个来源独立执行：单个来源不存在或解析失败不会阻断其他来源，
/// 具体错误会汇总到结果中供前端展示。
pub fn sync_all_session_logs(db: &Database) -> Result<SessionSyncResult> {
    let mut result = SessionSyncResult::default();

    for (name, step) in [
        ("Claude", sync_claude_session_logs(db)),
        ("Codex", crate::session_usage_codex::sync_codex_usage(db)),
        ("Gemini", crate::session_usage_gemini::sync_gemini_usage(db)),
        (
            "OpenCode",
            crate::session_usage_opencode::sync_opencode_usage(db),
        ),
        ("ZCode", crate::session_usage_zcode::sync_zcode_usage(db)),
        ("Grok Build", crate::session_usage_grok::sync_grok_usage(db)),
    ] {
        match step {
            Ok(source_result) => result.merge(source_result),
            Err(error) => result.errors.push(format!("{name} 同步失败: {error}")),
        }
    }

    Ok(result)
}

/// 收集目录下所有 .jsonl 文件（含子 agent 文件）
///
/// 扫描固定深度：
///   projects_dir/项目目录/*.jsonl                                      (主会话)
///   projects_dir/项目目录/SESSION_ID/subagents/*.jsonl                  (Task/Agent 子 agent)
///   projects_dir/项目目录/SESSION_ID/subagents/workflows/wf_*/*.jsonl   (Workflow 子 agent)
fn collect_jsonl_files(projects_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let entries = match fs::read_dir(projects_dir) {
        Ok(e) => e,
        Err(_) => return files,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Ok(sub_entries) = fs::read_dir(&path) {
            for sub_entry in sub_entries.flatten() {
                let sub_path = sub_entry.path();
                if sub_path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    files.push(sub_path);
                } else if sub_path.is_dir() {
                    let subagents_dir = sub_path.join("subagents");
                    if subagents_dir.is_dir() {
                        push_jsonl_children(&subagents_dir, &mut files);
                        let workflows_dir = subagents_dir.join("workflows");
                        if workflows_dir.is_dir() {
                            if let Ok(wf_entries) = fs::read_dir(&workflows_dir) {
                                for wf_entry in wf_entries.flatten() {
                                    let wf_path = wf_entry.path();
                                    if wf_path.is_dir() {
                                        push_jsonl_children(&wf_path, &mut files);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    files
}

fn push_jsonl_children(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                files.push(path);
            }
        }
    }
}

/// 同步单个 JSONL 文件，返回 (imported, skipped)
fn sync_single_file(db: &Database, file_path: &Path) -> Result<(u32, u32)> {
    let file_path_str = file_path.to_string_lossy().to_string();

    let metadata = fs::metadata(file_path)?;
    let file_modified = metadata_modified_nanos(&metadata);

    let (last_modified, last_offset) = get_sync_state(db, &file_path_str)?;

    // 文件未变化则跳过
    if file_modified <= last_modified {
        return Ok((0, 0));
    }

    let file = fs::File::open(file_path)?;
    let reader = BufReader::new(file);

    let mut line_offset: i64 = 0;
    let mut messages: HashMap<String, ParsedAssistantUsage> = HashMap::new();
    let mut current_session_id: Option<String> = None;

    for line_result in reader.lines() {
        line_offset += 1;
        if line_offset <= last_offset {
            continue;
        }

        let line = match line_result {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }

        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if current_session_id.is_none() {
            if let Some(sid) = value.get("sessionId").and_then(|v| v.as_str()) {
                current_session_id = Some(sid.to_string());
            }
        }

        // 只处理 assistant 类型的消息
        if value.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }

        let message = match value.get("message") {
            Some(m) => m,
            None => continue,
        };

        let msg_id = match message.get("id").and_then(|v| v.as_str()) {
            Some(id) => id.to_string(),
            None => continue,
        };

        let usage = match message.get("usage") {
            Some(u) => u,
            None => continue,
        };

        let parsed = ParsedAssistantUsage {
            message_id: msg_id.clone(),
            model: message
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            input_tokens: usage
                .get("input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            output_tokens: usage
                .get("output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            cache_read_tokens: usage
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            cache_creation_tokens: usage
                .get("cache_creation_input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32,
            stop_reason: message
                .get("stop_reason")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            timestamp: value
                .get("timestamp")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            session_id: current_session_id.clone(),
        };

        // 按 message.id 去重：优先保留有 stop_reason 的条目，否则保留最新且 output_tokens 更大的
        let should_replace = match messages.get(&msg_id) {
            None => true,
            Some(existing) => {
                if parsed.stop_reason.is_some() && existing.stop_reason.is_none() {
                    true
                } else if parsed.stop_reason.is_some() == existing.stop_reason.is_some() {
                    parsed.output_tokens > existing.output_tokens
                } else {
                    false
                }
            }
        };

        if should_replace {
            messages.insert(msg_id, parsed);
        }
    }

    let mut imported: u32 = 0;
    let mut skipped: u32 = 0;

    for msg in messages.values() {
        let has_billable_tokens = msg.input_tokens > 0
            || msg.output_tokens > 0
            || msg.cache_read_tokens > 0
            || msg.cache_creation_tokens > 0;
        if !has_billable_tokens {
            continue;
        }

        let request_id = format!("{}{}", SESSION_REQUEST_ID_PREFIX, msg.message_id);

        match insert_session_log_entry(db, &request_id, msg) {
            Ok(true) => imported += 1,
            Ok(false) => skipped += 1,
            Err(e) => {
                log::warn!("[SESSION-SYNC] 插入失败 ({}): {e}", msg.message_id);
                skipped += 1;
            }
        }
    }

    update_sync_state(db, &file_path_str, file_modified, line_offset)?;

    Ok((imported, skipped))
}

pub(crate) fn get_sync_state(db: &Database, file_path: &str) -> Result<(i64, i64)> {
    db.with_conn(|conn| {
        let result = conn.query_row(
            "SELECT last_modified, last_line_offset FROM session_log_sync WHERE file_path = ?1",
            rusqlite::params![file_path],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        );
        Ok(result.unwrap_or((0, 0)))
    })
}

pub(crate) fn metadata_modified_nanos(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

pub(crate) fn update_sync_state(
    db: &Database,
    file_path: &str,
    last_modified: i64,
    last_offset: i64,
) -> Result<()> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    db.with_conn(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO session_log_sync
             (file_path, last_modified, last_line_offset, last_synced_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![file_path, last_modified, last_offset, now],
        )?;
        Ok(())
    })
}

/// 插入单条会话日志到 proxy_request_logs
fn insert_session_log_entry(
    db: &Database,
    request_id: &str,
    msg: &ParsedAssistantUsage,
) -> Result<bool> {
    db.with_conn(|conn| {
        let created_at = msg
            .timestamp
            .as_ref()
            .and_then(|ts| {
                DateTime::parse_from_rfc3339(ts)
                    .ok()
                    .map(|dt| dt.timestamp())
            })
            .unwrap_or_else(|| {
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0)
            });

        let dedup_key = DedupKey {
            app_type: "claude",
            model: &msg.model,
            input_tokens: msg.input_tokens,
            output_tokens: msg.output_tokens,
            cache_read_tokens: msg.cache_read_tokens,
            cache_creation_tokens: msg.cache_creation_tokens,
            created_at,
        };

        if should_skip_session_insert(conn, request_id, &dedup_key)? {
            return Ok(false);
        }

        let usage = TokenUsage {
            input_tokens: msg.input_tokens,
            output_tokens: msg.output_tokens,
            cache_read_tokens: msg.cache_read_tokens,
            cache_creation_tokens: msg.cache_creation_tokens,
            model: Some(msg.model.clone()),
            message_id: None,
        };

        let pricing = find_model_pricing(conn, &msg.model);
        let multiplier = Decimal::from(1);
        let (input_cost, output_cost, cache_read_cost, cache_creation_cost, total_cost) =
            match pricing {
                Some(p) => {
                    let cost = CostCalculator::calculate(&usage, &p, multiplier);
                    (
                        cost.input_cost.to_string(),
                        cost.output_cost.to_string(),
                        cost.cache_read_cost.to_string(),
                        cost.cache_creation_cost.to_string(),
                        cost.total_cost.to_string(),
                    )
                }
                None => (
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                    "0".to_string(),
                ),
            };

        let inserted_rows = conn.execute(
            "INSERT OR IGNORE INTO proxy_request_logs (
                request_id, provider_id, app_type, model, request_model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd,
                latency_ms, first_token_ms, status_code, error_message, session_id,
                provider_type, is_streaming, cost_multiplier, created_at, data_source
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            rusqlite::params![
                request_id,
                "_session",
                "claude",
                msg.model,
                msg.model,
                msg.input_tokens,
                msg.output_tokens,
                msg.cache_read_tokens,
                msg.cache_creation_tokens,
                input_cost,
                output_cost,
                cache_read_cost,
                cache_creation_cost,
                total_cost,
                0i64,
                Option::<i64>::None,
                200i64,
                Option::<String>::None,
                msg.session_id,
                Some("session_log"),
                1i64,
                "1.0",
                created_at,
                "session_log",
            ],
        )?;

        Ok(inserted_rows > 0)
    })
}

/// 跨源去重检查：request_id 已存在 或 10 分钟窗口内有指纹匹配的 proxy 行则跳过
pub(crate) fn should_skip_session_insert(
    conn: &rusqlite::Connection,
    request_id: &str,
    key: &DedupKey,
) -> Result<bool> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM proxy_request_logs WHERE request_id = ?1)",
        rusqlite::params![request_id],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(true);
    }

    // 与 CC Switch 一致：只把成功的 proxy 行用于跨源去重；session 侧不暴露
    // cache_creation 时，0 表示未知，允许匹配 proxy 的任意 cache_creation 值。
    let allow_missing_cache_creation =
        matches!(key.app_type, "codex" | "gemini" | "opencode" | "zcode")
            && key.cache_creation_tokens == 0;
    let app_type_match =
        "l.app_type IN (?1, CASE WHEN ?1 = 'claude' THEN 'claude-desktop' ELSE ?1 END)";
    let sql = format!(
        "SELECT EXISTS(
            SELECT 1 FROM proxy_request_logs l
            WHERE COALESCE(l.data_source, 'proxy') = 'proxy'
              AND {app_type_match}
              AND l.status_code >= 200 AND l.status_code < 300
              AND l.input_tokens = ?3
              AND l.output_tokens = ?4
              AND l.cache_read_tokens = ?5
              AND (l.cache_creation_tokens = ?6 OR ?9 = 1)
              AND l.created_at BETWEEN ?7 - ?8 AND ?7 + ?8
              AND (
                  LOWER(l.model) = LOWER(?2)
                  OR LOWER(l.model) = 'unknown'
                  OR LOWER(?2) = 'unknown'
              )
        )"
    );
    conn.query_row(
        &sql,
        rusqlite::params![
            key.app_type,
            key.model,
            key.input_tokens,
            key.output_tokens,
            key.cache_read_tokens,
            key.cache_creation_tokens,
            key.created_at,
            crate::schema::SESSION_PROXY_DEDUP_WINDOW_SECONDS,
            allow_missing_cache_creation as i64,
        ],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

/// 从 model_pricing 表查找模型定价（支持模糊匹配）
pub fn find_model_pricing(conn: &rusqlite::Connection, model_id: &str) -> Option<ModelPricing> {
    // 精确匹配
    let row = conn
        .query_row(
            "SELECT input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_creation_cost_per_million
             FROM model_pricing WHERE LOWER(model_id) = LOWER(?1)",
            rusqlite::params![model_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .ok();

    let row = match row {
        Some(r) => r,
        None => {
            // 模糊匹配：去掉日期后缀
            let base = strip_date_suffix(model_id);
            if base == model_id {
                return None;
            }
            conn.query_row(
                "SELECT input_cost_per_million, output_cost_per_million,
                        cache_read_cost_per_million, cache_creation_cost_per_million
                 FROM model_pricing WHERE LOWER(model_id) = LOWER(?1)",
                rusqlite::params![base],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .ok()?
        }
    };

    let (input, output, cache_read, cache_creation) = row;
    ModelPricing::from_strings(&input, &output, &cache_read, &cache_creation).ok()
}

/// 去掉模型名的日期后缀（如 -20250514）
fn strip_date_suffix(model: &str) -> String {
    // 匹配 -YYYYMMDD 后缀
    if model.len() < 9 {
        return model.to_string();
    }
    let suffix = &model[model.len() - 9..];
    if suffix.starts_with('-') && suffix[1..].chars().all(|c| c.is_ascii_digit()) {
        model[..model.len() - 9].to_string()
    } else {
        model.to_string()
    }
}

/// 成本回填结果
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecostResult {
    pub updated: u32,
    pub skipped: u32,
    pub errors: Vec<String>,
}

/// 回填历史成本为 0 的 session 日志记录。
///
/// 这是离线或手动补价时的安全路径：只处理尚未计费的会话行。
pub fn recost_zero_cost_logs(db: &Database) -> Result<RecostResult> {
    recost_session_logs_with_filter(db, true)
}

/// 使用当前 model_pricing 本地缓存重算全部 session 日志记录。
///
/// 在线价格同步成功后调用，确保先前按内置回退价计算的历史记录也切换到
/// 最新缓存价格。只触碰会话来源，不会修改 proxy 请求记录。
pub fn recost_session_logs(db: &Database) -> Result<RecostResult> {
    recost_session_logs_with_filter(db, false)
}

fn recost_session_logs_with_filter(db: &Database, only_zero_cost: bool) -> Result<RecostResult> {
    db.with_conn(|conn| {
        let mut result = RecostResult::default();
        let zero_cost_filter = if only_zero_cost {
            " AND total_cost_usd = '0'"
        } else {
            ""
        };

        let select_sql = format!(
            "SELECT request_id, app_type, model, input_tokens, output_tokens,
                    cache_read_tokens, cache_creation_tokens
             FROM proxy_request_logs
             WHERE data_source IN ('session_log', 'codex_session', 'gemini_session', 'opencode_session', 'grok_session', 'zcode_session'){zero_cost_filter}"
        );
        let mut stmt = conn.prepare(&select_sql)?;
        let rows: Vec<(String, String, String, i64, i64, i64, i64)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        let update_sql = format!(
            "UPDATE proxy_request_logs
             SET input_cost_usd = ?1, output_cost_usd = ?2,
                 cache_read_cost_usd = ?3, cache_creation_cost_usd = ?4,
                 total_cost_usd = ?5
             WHERE request_id = ?6{zero_cost_filter}"
        );

        for (request_id, app_type, model, input, output, cache_read, cache_creation) in rows {
            let pricing = match find_model_pricing(conn, &model) {
                Some(p) => p,
                None => {
                    result.skipped += 1;
                    continue;
                }
            };

            let usage = TokenUsage {
                input_tokens: input as u32,
                output_tokens: output as u32,
                cache_read_tokens: cache_read as u32,
                cache_creation_tokens: cache_creation as u32,
                model: Some(model.clone()),
                message_id: None,
            };
            let cost = if crate::schema::is_cache_inclusive_app(&app_type) {
                CostCalculator::calculate_for_app(&app_type, &usage, &pricing, Decimal::from(1))
            } else {
                CostCalculator::calculate(&usage, &pricing, Decimal::from(1))
            };

            let updated = conn.execute(
                &update_sql,
                rusqlite::params![
                    cost.input_cost.to_string(),
                    cost.output_cost.to_string(),
                    cost.cache_read_cost.to_string(),
                    cost.cache_creation_cost.to_string(),
                    cost.total_cost.to_string(),
                    request_id,
                ],
            )?;

            if updated > 0 {
                result.updated += 1;
            } else {
                result.skipped += 1;
            }
        }

        if result.updated > 0 {
            log::info!(
                "[RECOST] 按本地价格缓存重算完成: 更新 {} 条, 跳过 {} 条",
                result.updated,
                result.skipped
            );
            crate::usage_events::notify_log_recorded();
        }
        Ok(result)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_strip_date_suffix() {
        assert_eq!(strip_date_suffix("claude-opus-4-20250514"), "claude-opus-4");
        assert_eq!(
            strip_date_suffix("claude-3-5-sonnet-20241022"),
            "claude-3-5-sonnet"
        );
        assert_eq!(strip_date_suffix("gpt-5"), "gpt-5");
        assert_eq!(strip_date_suffix("claude"), "claude");
    }

    #[test]
    fn test_find_model_pricing_is_case_insensitive() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE model_pricing (
                model_id TEXT PRIMARY KEY,
                display_name TEXT NOT NULL,
                input_cost_per_million TEXT NOT NULL,
                output_cost_per_million TEXT NOT NULL,
                cache_read_cost_per_million TEXT NOT NULL,
                cache_creation_cost_per_million TEXT NOT NULL
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO model_pricing (
                model_id, display_name, input_cost_per_million, output_cost_per_million,
                cache_read_cost_per_million, cache_creation_cost_per_million
             ) VALUES ('glm-5.2', 'GLM-5.2', '0.63', '1.98', '0.0945', '0')",
            [],
        )
        .unwrap();

        let pricing = find_model_pricing(&conn, "GLM-5.2").unwrap();
        assert_eq!(
            pricing.input_cost_per_million,
            Decimal::from_str("0.63").unwrap()
        );
        assert_eq!(
            pricing.output_cost_per_million,
            Decimal::from_str("1.98").unwrap()
        );
    }

    #[test]
    fn test_parse_usage_from_jsonl_line() {
        let line = r#"{"type":"assistant","message":{"id":"msg_test123","model":"claude-opus-4-6","usage":{"input_tokens":3,"output_tokens":150,"cache_read_input_tokens":5000,"cache_creation_input_tokens":10000},"stop_reason":"end_turn"},"timestamp":"2026-04-05T12:00:00Z","sessionId":"session-abc"}"#;

        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(
            value.get("type").and_then(|t| t.as_str()),
            Some("assistant")
        );

        let message = value.get("message").unwrap();
        let usage = message.get("usage").unwrap();
        assert_eq!(usage.get("input_tokens").unwrap().as_u64().unwrap(), 3);
        assert_eq!(usage.get("output_tokens").unwrap().as_u64().unwrap(), 150);
        assert_eq!(
            usage
                .get("cache_read_input_tokens")
                .unwrap()
                .as_u64()
                .unwrap(),
            5000
        );
    }

    #[test]
    fn test_dedup_by_message_id() {
        let mut messages: HashMap<String, ParsedAssistantUsage> = HashMap::new();
        let intermediate = ParsedAssistantUsage {
            message_id: "msg_1".to_string(),
            model: "claude-opus-4-6".to_string(),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            stop_reason: None,
            timestamp: None,
            session_id: None,
        };
        messages.insert("msg_1".to_string(), intermediate);

        let final_msg = ParsedAssistantUsage {
            message_id: "msg_1".to_string(),
            model: "claude-opus-4-6".to_string(),
            input_tokens: 100,
            output_tokens: 200,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            stop_reason: Some("end_turn".to_string()),
            timestamp: None,
            session_id: None,
        };

        let should_replace = match messages.get("msg_1") {
            None => true,
            Some(existing) => {
                if final_msg.stop_reason.is_some() && existing.stop_reason.is_none() {
                    true
                } else if final_msg.stop_reason.is_some() == existing.stop_reason.is_some() {
                    final_msg.output_tokens > existing.output_tokens
                } else {
                    false
                }
            }
        };

        assert!(should_replace);
    }
}
