//! Codex 会话日志使用追踪。
//!
//! Codex 的 `token_count` 是线程内的累计快照。CC Switch 的统计口径还要
//! 处理 fork/subagent rollout 重放：子线程文件通常会包含父线程在 fork
//! 之前的历史快照，必须剔除这段回放后再把累计快照转换成增量。

use crate::calculator::{CostCalculator, TokenUsage};
use crate::database::Database;
use crate::error::Result;
use crate::schema::INPUT_TOKEN_SEMANTICS_LEGACY;
use crate::session_usage::{
    find_model_pricing, get_sync_state, metadata_modified_nanos, modified_nanos_to_seconds,
    parse_rfc3339_timestamp, project_name_from_path, should_skip_session_insert, update_sync_state,
    DedupKey, SessionSyncResult,
};
use chrono::{DateTime, Utc};
use rusqlite::OptionalExtension;
use rust_decimal::Decimal;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use uuid::Uuid;

const CODEX_THREAD_REQUEST_ID_PREFIX: &str = "codex_session:thread-v1";
const CODEX_USAGE_PARSER_VERSION_KEY: &str = "codex_usage_parser_version";
// CC Switch's Codex importer leaves this column at the schema default (legacy).
// Keep the same marker so fresh-input aggregation follows CC's stored-data
// semantics for Codex session rows.
const CODEX_USAGE_PARSER_VERSION: &str = "thread-v4-filter-codex-workspace";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CumulativeTokens {
    input: u64,
    cached_input: u64,
    output: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DeltaTokens {
    input: u32,
    cached_input: u32,
    output: u32,
}

impl DeltaTokens {
    fn is_zero(self) -> bool {
        self.input == 0 && self.cached_input == 0 && self.output == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TokenCountersSignature {
    input: Option<u64>,
    cached_input: Option<u64>,
    output: Option<u64>,
    reasoning_output: Option<u64>,
    total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TokenUsageSignature {
    total: Option<TokenCountersSignature>,
    last: Option<TokenCountersSignature>,
}

#[derive(Debug)]
struct TimestampedTokenSignature {
    timestamp: DateTime<Utc>,
    signature: TokenUsageSignature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParentFileStamp {
    modified_nanos: i64,
    size: u64,
}

impl ParentFileStamp {
    fn from_file(file: &fs::File) -> Option<Self> {
        let metadata = file.metadata().ok()?;
        Some(Self {
            modified_nanos: metadata_modified_nanos(&metadata),
            size: metadata.len(),
        })
    }
}

#[derive(Debug)]
struct ParentTokenTimeline {
    events: Vec<TimestampedTokenSignature>,
    max_timestamp: Option<DateTime<Utc>>,
    has_token_without_timestamp: bool,
}

impl ParentTokenTimeline {
    fn signatures_before(
        &self,
        parent_path: &Path,
        cutoff: DateTime<Utc>,
    ) -> std::result::Result<Vec<TokenUsageSignature>, String> {
        if self.has_token_without_timestamp {
            return Err(format!(
                "父 rollout {} 的 token_count 缺少有效 timestamp",
                parent_path.display()
            ));
        }
        if self
            .max_timestamp
            .map_or(true, |timestamp| timestamp < cutoff)
        {
            return Err(format!(
                "父 rollout {} 尚未写到 child fork 时刻",
                parent_path.display()
            ));
        }
        Ok(self
            .events
            .iter()
            .filter(|event| event.timestamp <= cutoff)
            .map(|event| event.signature.clone())
            .collect())
    }
}

#[derive(Debug)]
struct CachedParentTimeline {
    stamp: ParentFileStamp,
    timeline: Arc<ParentTokenTimeline>,
}

#[derive(Debug)]
struct CachedReplayPrefix {
    modified: i64,
    size: u64,
    prefix: usize,
}

#[derive(Debug)]
struct ParsedTokenEvent {
    line_offset: i64,
    signature: TokenUsageSignature,
    delta: DeltaTokens,
    event_index: Option<u32>,
    model: String,
    timestamp: Option<String>,
}

#[derive(Debug)]
enum ParentResolution {
    None,
    Parent(String),
    Deferred(String),
}

#[derive(Debug)]
struct ParsedCodexFile {
    root_thread_id: Option<String>,
    root_meta_seen: bool,
    root_timestamp: Option<DateTime<Utc>>,
    project: String,
    parent: ParentResolution,
    token_events: Vec<ParsedTokenEvent>,
    line_offset: i64,
    has_billable_tokens: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingReason {
    MissingParent(String),
    Stable(String),
    Retryable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingEntry {
    modified: i64,
    size: u64,
    reason: PendingReason,
}

#[derive(Debug, Default)]
struct CodexReplayCaches {
    parent_timelines: HashMap<PathBuf, CachedParentTimeline>,
    replay_prefixes: HashMap<PathBuf, CachedReplayPrefix>,
    pending: HashMap<PathBuf, PendingEntry>,
}

static CODEX_REPLAY_CACHES: OnceLock<Mutex<CodexReplayCaches>> = OnceLock::new();

fn replay_caches() -> &'static Mutex<CodexReplayCaches> {
    CODEX_REPLAY_CACHES.get_or_init(|| Mutex::new(CodexReplayCaches::default()))
}

pub(crate) fn clear_codex_replay_caches() {
    if let Ok(mut caches) = replay_caches().lock() {
        *caches = CodexReplayCaches::default();
    }
}

fn get_codex_config_dir() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".codex")
        })
}

fn is_rollout_filename(file_name: &str) -> bool {
    if !file_name.starts_with("rollout-") || !file_name.ends_with(".jsonl") {
        return false;
    }
    let stem = file_name.trim_end_matches(".jsonl");
    stem.get(stem.len().saturating_sub(36)..)
        .is_some_and(|candidate| Uuid::parse_str(candidate).is_ok())
}

fn is_codex_cursor_path(file_path: &str, codex_dir: &Path) -> bool {
    let path = Path::new(file_path);
    let file_name = file_path.rsplit(['/', '\\']).next().unwrap_or_default();
    if !is_rollout_filename(file_name) {
        return false;
    }
    path.starts_with(codex_dir.join("sessions"))
        || path.starts_with(codex_dir.join("archived_sessions"))
        || file_path
            .replace('\\', "/")
            .split('/')
            .any(|segment| matches!(segment, "sessions" | "archived_sessions"))
}

/// 清理旧解析器写入的 Codex 行和游标。request id 使用 thread-v1 的线程
/// 回放命名空间；切换解析口径后必须重建一次，避免旧重复行继续参与汇总。
///
/// 只清理当前仍存在的 rollout 文件对应的派生行/游标。已经归档或删除、且
/// 本地原始文件不可再读取的历史行不能被新解析器重建，保留它们比静默丢失
/// 历史用量更符合统计数据的可恢复性。
pub(crate) fn reset_codex_usage(db: &Database) -> Result<()> {
    let codex_dir = get_codex_config_dir();
    let files = collect_codex_session_files(&codex_dir);
    let current_paths: HashSet<String> = files
        .iter()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect();
    let current_session_ids: Vec<String> = files
        .iter()
        .filter_map(|path| thread_id_from_filename(path))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    db.with_conn(|conn| {
        if !current_session_ids.is_empty() {
            let placeholders = (0..current_session_ids.len())
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "DELETE FROM proxy_request_logs
                 WHERE data_source = 'codex_session' AND session_id IN ({placeholders})"
            );
            conn.execute(&sql, rusqlite::params_from_iter(current_session_ids.iter()))?;
        }

        let paths: Vec<String> = conn
            .prepare("SELECT file_path FROM session_log_sync")?
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        for path in paths.into_iter().filter(|path| {
            is_codex_cursor_path(path, &codex_dir)
                && current_paths.contains(&path.replace('\\', "/"))
        }) {
            conn.execute("DELETE FROM session_log_sync WHERE file_path = ?1", [path])?;
        }
        Ok(())
    })?;
    clear_codex_replay_caches();
    Ok(())
}

/// 首次切换到 thread-v2 解析口径时，重建 Codex 派生明细。
///
/// 旧版本 request id 不包含线程版本，也没有回放前缀去重；保留旧行会让
/// 新旧两套口径叠加，所以用 settings 做一次性迁移标记。原始 rollout 文件
/// 不会被修改，迁移失败也不会写入完成标记，下一次启动仍会重试。
pub(crate) fn migrate_codex_usage(db: &Database) -> Result<bool> {
    let already_migrated = db.with_conn(|conn| {
        let version: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [CODEX_USAGE_PARSER_VERSION_KEY],
                |row| row.get(0),
            )
            .optional()?;
        Ok(version.as_deref() == Some(CODEX_USAGE_PARSER_VERSION))
    })?;
    if already_migrated {
        return Ok(false);
    }

    reset_codex_usage(db)?;
    db.with_conn(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            [CODEX_USAGE_PARSER_VERSION_KEY, CODEX_USAGE_PARSER_VERSION],
        )?;
        Ok(())
    })?;
    Ok(true)
}

fn non_empty_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn thread_id_from_filename(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let start = stem.len().checked_sub(36)?;
    let candidate = stem.get(start..)?;
    Uuid::parse_str(candidate)
        .ok()
        .map(|value| value.hyphenated().to_string())
}

fn explicit_parent_from_meta(payload: &Value) -> ParentResolution {
    let forked_from = non_empty_string(payload.get("forked_from_id"));
    let spawned_from = payload
        .get("source")
        .and_then(|source| source.get("subagent"))
        .and_then(|subagent| subagent.get("thread_spawn"))
        .and_then(|spawn| non_empty_string(spawn.get("parent_thread_id")));

    match (forked_from, spawned_from) {
        (None, None) => ParentResolution::None,
        (Some(parent), None) | (None, Some(parent)) => ParentResolution::Parent(parent),
        (Some(forked), Some(spawned)) if forked == spawned => ParentResolution::Parent(forked),
        (Some(forked), Some(spawned)) => ParentResolution::Deferred(format!(
            "forked_from_id ({forked}) 与 thread_spawn.parent_thread_id ({spawned}) 不一致"
        )),
    }
}

fn parse_timestamp(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

/// Codex Desktop 会把任务工作区放在
/// `Documents/Codex/YYYY-MM-DD/<workspace>` 下。这里的最后一级是
/// Codex 任务工作区 slug，不是用户项目名，不能进入项目排行。
fn codex_project_name_from_path(value: Option<&str>) -> String {
    if is_codex_managed_workspace(value) {
        String::new()
    } else {
        project_name_from_path(value)
    }
}

fn is_codex_managed_workspace(value: Option<&str>) -> bool {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let segments = value
        .trim_end_matches(['\\', '/'])
        .split(['\\', '/'])
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    segments.windows(4).any(|window| {
        window[0].eq_ignore_ascii_case("Documents")
            && window[1].eq_ignore_ascii_case("Codex")
            && is_codex_workspace_date(window[2])
            && !window[3].is_empty()
    })
}

fn is_codex_workspace_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn parse_signature_counters(value: Option<&Value>) -> Option<TokenCountersSignature> {
    let value = value?.as_object()?;
    Some(TokenCountersSignature {
        input: value.get("input_tokens").and_then(Value::as_u64),
        cached_input: value
            .get("cached_input_tokens")
            .or_else(|| value.get("cache_read_input_tokens"))
            .and_then(Value::as_u64),
        output: value.get("output_tokens").and_then(Value::as_u64),
        reasoning_output: value.get("reasoning_output_tokens").and_then(Value::as_u64),
        total: value.get("total_tokens").and_then(Value::as_u64),
    })
}

fn parse_token_signature(info: &Value) -> Option<TokenUsageSignature> {
    let total = parse_signature_counters(info.get("total_token_usage"));
    let last = parse_signature_counters(info.get("last_token_usage"));
    (total.is_some() || last.is_some()).then_some(TokenUsageSignature { total, last })
}

fn normalize_codex_model(raw: &str) -> String {
    let mut name = raw.to_lowercase();
    if let Some(pos) = name.rfind('/') {
        name = name[pos + 1..].to_string();
    }

    if name.len() > 11 {
        let suffix = &name[name.len() - 11..];
        let bytes = suffix.as_bytes();
        if bytes.len() == 11
            && bytes[0] == b'-'
            && bytes[5] == b'-'
            && bytes[8] == b'-'
            && bytes[1..5].iter().all(u8::is_ascii_digit)
            && bytes[6..8].iter().all(u8::is_ascii_digit)
            && bytes[9..11].iter().all(u8::is_ascii_digit)
        {
            name.truncate(name.len() - 11);
        }
    }

    if name.len() > 9 {
        let (base, suffix) = name.split_at(name.len() - 9);
        if suffix.starts_with('-') && suffix[1..].chars().all(|c| c.is_ascii_digit()) {
            name = base.to_string();
        }
    }
    name
}

fn compute_delta(prev: &Option<CumulativeTokens>, current: &CumulativeTokens) -> DeltaTokens {
    let previous = prev.as_ref();
    let input = previous.map_or(current.input, |value| {
        current.input.saturating_sub(value.input)
    });
    let cached_input = previous.map_or(current.cached_input, |value| {
        current.cached_input.saturating_sub(value.cached_input)
    });
    let output = previous.map_or(current.output, |value| {
        current.output.saturating_sub(value.output)
    });

    DeltaTokens {
        input: input.min(u32::MAX as u64) as u32,
        cached_input: cached_input.min(input).min(u32::MAX as u64) as u32,
        output: output.min(u32::MAX as u64) as u32,
    }
}

fn parse_cumulative_tokens(value: &Value) -> Option<CumulativeTokens> {
    if !value.is_object() {
        return None;
    }
    Some(CumulativeTokens {
        input: value
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cached_input: value
            .get("cached_input_tokens")
            .or_else(|| value.get("cache_read_input_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output: value
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    })
}

type RolloutIndex = HashMap<String, Vec<PathBuf>>;

#[derive(Debug, Default)]
struct CodexFileSyncResult {
    imported: u32,
    skipped: u32,
    deferred: bool,
}

/// 同步 Codex 使用数据（从 JSONL 会话日志）。
pub fn sync_codex_usage(db: &Database) -> Result<SessionSyncResult> {
    let codex_dir = get_codex_config_dir();
    let files = collect_codex_session_files(&codex_dir);
    let rollout_index = build_rollout_index(&files);
    let mut result = SessionSyncResult {
        files_scanned: files.len() as u32,
        ..Default::default()
    };

    for file_path in &files {
        match sync_single_codex_file(db, file_path, &rollout_index) {
            Ok(file_result) => {
                result.imported = result.imported.saturating_add(file_result.imported);
                result.skipped = result.skipped.saturating_add(file_result.skipped);
                if file_result.deferred {
                    result.deferred_files = result.deferred_files.saturating_add(1);
                }
            }
            Err(error) => {
                let message = format!("Codex 会话文件解析失败 {}: {error}", file_path.display());
                log::warn!("[CODEX-SYNC] {message}");
                result.errors.push(message);
            }
        }
    }

    if result.imported > 0 || result.deferred_files > 0 {
        log::info!(
            "[CODEX-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 延后 {} 个文件, 扫描 {} 个文件",
            result.imported,
            result.skipped,
            result.deferred_files,
            result.files_scanned
        );
    }
    Ok(result)
}

fn collect_codex_session_files(codex_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let sessions_dir = codex_dir.join("sessions");
    if sessions_dir.is_dir() {
        collect_jsonl_recursive(&sessions_dir, &mut files, 0, 3);
    }

    let archived_dir = codex_dir.join("archived_sessions");
    if let Ok(entries) = fs::read_dir(archived_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn collect_jsonl_recursive(dir: &Path, files: &mut Vec<PathBuf>, depth: u32, max_depth: u32) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && depth < max_depth {
            collect_jsonl_recursive(&path, files, depth + 1, max_depth);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
}

fn build_rollout_index(files: &[PathBuf]) -> RolloutIndex {
    let mut index = RolloutIndex::new();
    for path in files {
        if let Some(thread_id) = thread_id_from_filename(path) {
            index.entry(thread_id).or_default().push(path.clone());
        }
    }
    for paths in index.values_mut() {
        paths.sort();
    }
    index
}

fn parse_codex_file(file_path: &Path, root_thread_id: Option<String>) -> Result<ParsedCodexFile> {
    let file = fs::File::open(file_path)?;
    let reader = BufReader::new(file);
    let mut root_meta_seen = false;
    let mut root_timestamp = None;
    let mut project = String::new();
    let mut parent = ParentResolution::None;
    let mut current_model = "unknown".to_string();
    let mut previous_total = None;
    let mut event_index = 0u32;
    let mut token_events = Vec::new();
    let mut line_offset = 0i64;
    let mut has_billable_tokens = false;

    for line_result in reader.lines() {
        line_offset += 1;
        let line = match line_result {
            Ok(line) => line,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }

        let is_event_msg = line.contains("\"event_msg\"");
        let is_turn_context = line.contains("\"turn_context\"");
        let is_session_meta = line.contains("\"session_meta\"");
        if !is_event_msg && !is_turn_context && !is_session_meta {
            continue;
        }
        if is_event_msg && !line.contains("\"token_count\"") {
            continue;
        }

        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Some(event_type) = value.get("type").and_then(Value::as_str) else {
            continue;
        };

        match event_type {
            "session_meta" if !root_meta_seen => {
                root_meta_seen = true;
                root_timestamp = parse_timestamp(value.get("timestamp"));
                let payload = value.get("payload").unwrap_or(&Value::Null);
                project = codex_project_name_from_path(payload.get("cwd").and_then(Value::as_str));
                parent = explicit_parent_from_meta(payload);

                let meta_thread_id = non_empty_string(
                    payload
                        .get("id")
                        .or_else(|| payload.get("thread_id"))
                        .or_else(|| payload.get("threadId")),
                );
                if let (Some(filename_id), Some(meta_id)) = (&root_thread_id, meta_thread_id) {
                    if filename_id != &meta_id {
                        parent = ParentResolution::Deferred(format!(
                            "文件名线程 ID ({filename_id}) 与 root meta ID ({meta_id}) 不一致"
                        ));
                    }
                }

                if let ParentResolution::Parent(parent_id) = &mut parent {
                    match Uuid::parse_str(parent_id) {
                        Ok(value) => *parent_id = value.hyphenated().to_string(),
                        Err(_) => {
                            parent = ParentResolution::Deferred(format!(
                                "显式 parent_thread_id 不是有效 UUID: {parent_id}"
                            ));
                        }
                    }
                }
                if matches!((&root_thread_id, &parent), (Some(root), ParentResolution::Parent(parent_id)) if root == parent_id)
                {
                    parent = ParentResolution::Deferred(
                        "parent_thread_id 与 root_thread_id 相同".to_string(),
                    );
                }
            }
            "turn_context" => {
                if let Some(payload) = value.get("payload") {
                    if let Some(model) = payload
                        .get("model")
                        .or_else(|| payload.get("info").and_then(|info| info.get("model")))
                        .and_then(Value::as_str)
                    {
                        current_model = normalize_codex_model(model);
                    }
                }
            }
            "event_msg" => {
                let Some(payload) = value.get("payload") else {
                    continue;
                };
                if payload.get("type").and_then(Value::as_str) != Some("token_count") {
                    continue;
                }
                let Some(info) = payload.get("info").filter(|info| !info.is_null()) else {
                    continue;
                };
                let Some(signature) = parse_token_signature(info) else {
                    continue;
                };

                if let Some(model) = info
                    .get("model")
                    .or_else(|| info.get("model_name"))
                    .or_else(|| payload.get("model"))
                    .and_then(Value::as_str)
                {
                    current_model = normalize_codex_model(model);
                }

                let (cumulative, is_total) = if let Some(total) = info.get("total_token_usage") {
                    (parse_cumulative_tokens(total), true)
                } else if let Some(last) = info.get("last_token_usage") {
                    (parse_cumulative_tokens(last), false)
                } else {
                    continue;
                };
                let Some(cumulative) = cumulative else {
                    continue;
                };

                let mut delta = if is_total {
                    let delta = compute_delta(&previous_total, &cumulative);
                    previous_total = Some(cumulative);
                    delta
                } else {
                    DeltaTokens {
                        input: cumulative.input.min(u32::MAX as u64) as u32,
                        cached_input: cumulative.cached_input.min(u32::MAX as u64) as u32,
                        output: cumulative.output.min(u32::MAX as u64) as u32,
                    }
                };
                delta.cached_input = delta.cached_input.min(delta.input);

                let nonzero_index = if delta.is_zero() {
                    None
                } else {
                    has_billable_tokens = true;
                    event_index = event_index.saturating_add(1);
                    Some(event_index)
                };

                token_events.push(ParsedTokenEvent {
                    line_offset,
                    signature,
                    delta,
                    event_index: nonzero_index,
                    model: current_model.clone(),
                    timestamp: value
                        .get("timestamp")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                });
            }
            _ => {}
        }
    }

    Ok(ParsedCodexFile {
        root_thread_id,
        root_meta_seen,
        root_timestamp,
        project,
        parent,
        token_events,
        line_offset,
        has_billable_tokens,
    })
}

fn parent_signatures_before(
    parent_path: &Path,
    cutoff: DateTime<Utc>,
) -> std::result::Result<Vec<TokenUsageSignature>, String> {
    let file = fs::File::open(parent_path)
        .map_err(|error| format!("无法打开父 rollout {}: {error}", parent_path.display()))?;
    let stamp = ParentFileStamp::from_file(&file);
    let cached_timeline = stamp.and_then(|stamp| {
        replay_caches().lock().ok().and_then(|caches| {
            caches
                .parent_timelines
                .get(parent_path)
                .filter(|entry| entry.stamp == stamp)
                .map(|entry| Arc::clone(&entry.timeline))
        })
    });
    if let Some(timeline) = cached_timeline {
        return timeline.signatures_before(parent_path, cutoff);
    }

    let mut events = Vec::new();
    let mut max_timestamp = None;
    let mut has_token_without_timestamp = false;

    for line in BufReader::new(file)
        .lines()
        .map_while(std::result::Result::ok)
    {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let timestamp = parse_timestamp(value.get("timestamp"));
        if let Some(timestamp) = timestamp {
            max_timestamp = Some(
                max_timestamp.map_or(timestamp, |current: DateTime<Utc>| current.max(timestamp)),
            );
        }
        if value.get("type").and_then(Value::as_str) != Some("event_msg")
            || value
                .get("payload")
                .and_then(|payload| payload.get("type"))
                .and_then(Value::as_str)
                != Some("token_count")
        {
            continue;
        }
        let Some(info) = value
            .get("payload")
            .and_then(|payload| payload.get("info"))
            .filter(|info| !info.is_null())
        else {
            continue;
        };
        let Some(signature) = parse_token_signature(info) else {
            continue;
        };
        let Some(timestamp) = timestamp else {
            has_token_without_timestamp = true;
            continue;
        };
        events.push(TimestampedTokenSignature {
            timestamp,
            signature,
        });
    }

    let timeline = Arc::new(ParentTokenTimeline {
        events,
        max_timestamp,
        has_token_without_timestamp,
    });
    let result = timeline.signatures_before(parent_path, cutoff);
    if let (Some(stamp), Ok(mut caches)) = (stamp, replay_caches().lock()) {
        caches.parent_timelines.insert(
            parent_path.to_path_buf(),
            CachedParentTimeline {
                stamp,
                timeline: Arc::clone(&timeline),
            },
        );
    }
    result
}

fn resolve_parent_signatures(
    parent_id: &str,
    cutoff: DateTime<Utc>,
    rollout_index: &RolloutIndex,
) -> std::result::Result<Vec<TokenUsageSignature>, String> {
    let Some(candidates) = rollout_index.get(parent_id) else {
        return Err(format!("找不到父 rollout: {parent_id}"));
    };
    let mut snapshots = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        snapshots.push(parent_signatures_before(candidate, cutoff)?);
    }
    let Some(first) = snapshots.first() else {
        return Err(format!("找不到父 rollout: {parent_id}"));
    };
    if snapshots.iter().skip(1).any(|snapshot| snapshot != first) {
        return Err(format!(
            "父 rollout UUID {parent_id} 对应多个内容不一致的文件"
        ));
    }
    Ok(first.clone())
}

fn matching_replay_prefix(child: &[ParsedTokenEvent], parent: &[TokenUsageSignature]) -> usize {
    let mut parent_offset = 0usize;
    let mut matched = 0usize;
    for event in child {
        if parent_offset >= parent.len() {
            break;
        }
        let Some(relative_match) = parent[parent_offset..]
            .iter()
            .position(|signature| signature == &event.signature)
        else {
            break;
        };
        parent_offset += relative_match + 1;
        matched += 1;
    }
    matched
}

fn mark_deferred(
    file_path: &Path,
    modified: i64,
    size: u64,
    reason: PendingReason,
) -> CodexFileSyncResult {
    let entry = PendingEntry {
        modified,
        size,
        reason,
    };
    let should_warn = replay_caches()
        .lock()
        .ok()
        .and_then(|mut caches| {
            caches
                .pending
                .insert(file_path.to_path_buf(), entry.clone())
        })
        .as_ref()
        != Some(&entry);
    if should_warn {
        let reason = match &entry.reason {
            PendingReason::MissingParent(parent) => format!("找不到父 rollout {parent}"),
            PendingReason::Stable(reason) | PendingReason::Retryable(reason) => reason.clone(),
        };
        log::warn!("[CODEX-SYNC] deferred {}: {reason}", file_path.display());
    }
    CodexFileSyncResult {
        deferred: true,
        ..Default::default()
    }
}

fn get_codex_sync_state(db: &Database, file_path: &Path) -> Result<(i64, i64)> {
    let file_path_string = file_path.to_string_lossy().to_string();
    let state = get_sync_state(db, &file_path_string)?;
    if state != (0, 0)
        || file_path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some("archived_sessions")
    {
        return Ok(state);
    }

    let Some(file_name) = file_path.file_name().and_then(|name| name.to_str()) else {
        return Ok(state);
    };
    let slash_suffix = format!("/{file_name}");
    let backslash_suffix = format!("\\{file_name}");
    let inherited = db.with_conn(|conn| {
        conn.query_row(
            "SELECT last_modified, last_line_offset
             FROM session_log_sync
             WHERE file_path <> ?1
               AND (substr(file_path, -length(?2)) = ?2
                    OR substr(file_path, -length(?3)) = ?3)
             ORDER BY last_line_offset DESC, last_modified DESC
            LIMIT 1",
            rusqlite::params![file_path_string, slash_suffix, backslash_suffix],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(crate::error::AppError::from)
    });

    match inherited {
        Ok(Some(value)) => {
            update_sync_state(db, &file_path_string, value.0, value.1)?;
            Ok(value)
        }
        Ok(None) => Ok(state),
        Err(error) => Err(error),
    }
}

fn sync_single_codex_file(
    db: &Database,
    file_path: &Path,
    rollout_index: &RolloutIndex,
) -> Result<CodexFileSyncResult> {
    let file_path_string = file_path.to_string_lossy().to_string();
    let metadata = fs::metadata(file_path)?;
    let file_modified = metadata_modified_nanos(&metadata);
    let file_size = metadata.len();
    let (last_modified, last_offset) = get_codex_sync_state(db, file_path)?;

    if file_modified <= last_modified {
        return Ok(CodexFileSyncResult::default());
    }

    if let Ok(mut caches) = replay_caches().lock() {
        if let Some(pending) = caches.pending.get(file_path).cloned() {
            if pending.modified == file_modified && pending.size == file_size {
                match &pending.reason {
                    PendingReason::MissingParent(parent) if !rollout_index.contains_key(parent) => {
                        return Ok(CodexFileSyncResult {
                            deferred: true,
                            ..Default::default()
                        });
                    }
                    PendingReason::Stable(_) => {
                        return Ok(CodexFileSyncResult {
                            deferred: true,
                            ..Default::default()
                        });
                    }
                    PendingReason::Retryable(_) | PendingReason::MissingParent(_) => {
                        caches.pending.remove(file_path);
                    }
                }
            }
        }
    }

    let parsed = parse_codex_file(file_path, thread_id_from_filename(file_path))?;
    if !parsed.has_billable_tokens {
        update_sync_state(db, &file_path_string, file_modified, parsed.line_offset)?;
        return Ok(CodexFileSyncResult::default());
    }

    let Some(root_thread_id) = parsed.root_thread_id.as_deref() else {
        return Ok(mark_deferred(
            file_path,
            file_modified,
            file_size,
            PendingReason::Stable("文件名缺少有效的尾部 UUID".to_string()),
        ));
    };
    if !parsed.root_meta_seen {
        return Ok(mark_deferred(
            file_path,
            file_modified,
            file_size,
            PendingReason::Stable("含计费 token 但尚无 session_meta".to_string()),
        ));
    }

    let replay_prefix = match &parsed.parent {
        ParentResolution::None => 0,
        ParentResolution::Deferred(reason) => {
            return Ok(mark_deferred(
                file_path,
                file_modified,
                file_size,
                PendingReason::Stable(reason.clone()),
            ));
        }
        ParentResolution::Parent(parent_id) => {
            let Some(cutoff) = parsed.root_timestamp else {
                return Ok(mark_deferred(
                    file_path,
                    file_modified,
                    file_size,
                    PendingReason::Stable(
                        "parented rollout 的 root meta 缺少有效 timestamp".to_string(),
                    ),
                ));
            };
            if let Ok(caches) = replay_caches().lock() {
                if let Some(prefix) = caches
                    .replay_prefixes
                    .get(file_path)
                    .filter(|cached| cached.modified == file_modified && cached.size == file_size)
                    .map(|cached| cached.prefix)
                {
                    prefix
                } else {
                    drop(caches);
                    let parent_signatures =
                        match resolve_parent_signatures(parent_id, cutoff, rollout_index) {
                            Ok(signatures) => signatures,
                            Err(reason) => {
                                let pending_reason = if rollout_index.contains_key(parent_id) {
                                    PendingReason::Retryable(reason)
                                } else {
                                    PendingReason::MissingParent(parent_id.clone())
                                };
                                return Ok(mark_deferred(
                                    file_path,
                                    file_modified,
                                    file_size,
                                    pending_reason,
                                ));
                            }
                        };
                    let prefix = matching_replay_prefix(&parsed.token_events, &parent_signatures);
                    if let Ok(mut caches) = replay_caches().lock() {
                        caches.replay_prefixes.insert(
                            file_path.to_path_buf(),
                            CachedReplayPrefix {
                                modified: file_modified,
                                size: file_size,
                                prefix,
                            },
                        );
                    }
                    prefix
                }
            } else {
                let parent_signatures = resolve_parent_signatures(parent_id, cutoff, rollout_index)
                    .map_err(crate::error::AppError::Config)?;
                matching_replay_prefix(&parsed.token_events, &parent_signatures)
            }
        }
    };

    let fallback_created_at = parsed
        .root_timestamp
        .map(|timestamp| timestamp.timestamp())
        .or_else(|| modified_nanos_to_seconds(file_modified));

    if let Ok(mut caches) = replay_caches().lock() {
        caches.pending.remove(file_path);
    }

    let mut result = CodexFileSyncResult::default();
    for (token_offset, event) in parsed.token_events.iter().enumerate() {
        let Some(event_index) = event.event_index else {
            continue;
        };
        if token_offset < replay_prefix {
            if event.line_offset > last_offset {
                result.skipped = result.skipped.saturating_add(1);
            }
            continue;
        }
        if event.line_offset <= last_offset {
            continue;
        }

        let request_id = format!("{CODEX_THREAD_REQUEST_ID_PREFIX}:{root_thread_id}:{event_index}");
        match insert_codex_session_entry(
            db,
            &request_id,
            event,
            root_thread_id,
            &parsed.project,
            fallback_created_at,
        )? {
            true => result.imported = result.imported.saturating_add(1),
            false => result.skipped = result.skipped.saturating_add(1),
        }
    }

    update_sync_state(db, &file_path_string, file_modified, parsed.line_offset)?;
    Ok(result)
}

fn insert_codex_session_entry(
    db: &Database,
    request_id: &str,
    event: &ParsedTokenEvent,
    session_id: &str,
    project: &str,
    fallback_created_at: Option<i64>,
) -> Result<bool> {
    db.with_conn(|conn| {
        let Some(created_at) = parse_rfc3339_timestamp(event.timestamp.as_deref())
            .or(fallback_created_at)
        else {
            return Ok(false);
        };

        let key = DedupKey {
            app_type: "codex",
            model: &event.model,
            input_tokens: event.delta.input,
            output_tokens: event.delta.output,
            cache_read_tokens: event.delta.cached_input,
            cache_creation_tokens: 0,
            created_at,
        };
        if should_skip_session_insert(conn, request_id, &key)? {
            return Ok(false);
        }

        let usage = TokenUsage {
            input_tokens: event.delta.input,
            output_tokens: event.delta.output,
            cache_read_tokens: event.delta.cached_input,
            cache_creation_tokens: 0,
            model: Some(event.model.clone()),
            message_id: None,
        };
        let costs = find_model_pricing(conn, &event.model)
            .map(|pricing| CostCalculator::calculate_for_app("codex", &usage, &pricing, Decimal::ONE));
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
                provider_type, is_streaming, cost_multiplier, created_at, data_source, project
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
            rusqlite::params![
                request_id,
                "_codex_session",
                "codex",
                event.model,
                event.model,
                event.delta.input,
                event.delta.output,
                event.delta.cached_input,
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
                Some(session_id.to_string()),
                Some("codex_session"),
                1i64,
                "1.0",
                created_at,
                "codex_session",
                project,
            ],
        )?;
        Ok(inserted > 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cumulative_delta_uses_saturating_subtraction() {
        let previous = Some(CumulativeTokens {
            input: 100,
            cached_input: 80,
            output: 20,
        });
        let current = CumulativeTokens {
            input: 90,
            cached_input: 70,
            output: 25,
        };
        assert_eq!(
            compute_delta(&previous, &current),
            DeltaTokens {
                input: 0,
                cached_input: 0,
                output: 5,
            }
        );
    }

    #[test]
    fn normalizes_provider_and_date_suffixes() {
        assert_eq!(
            normalize_codex_model("OpenAI/GPT-5.6-SOL-20260305"),
            "gpt-5.6-sol"
        );
        assert_eq!(
            normalize_codex_model("azure/gpt-5.6-luna-20260305"),
            "gpt-5.6-luna"
        );
    }

    #[test]
    fn ignores_codex_managed_workspaces_as_projects() {
        let workspace = r"C:\Users\Administrator\Documents\Codex\2026-08-16\she";
        assert!(is_codex_managed_workspace(Some(workspace)));
        assert_eq!(codex_project_name_from_path(Some(workspace)), "");
        assert_eq!(
            codex_project_name_from_path(Some(r"D:\work\LLM tools\usage-pulse")),
            "usage-pulse"
        );
    }

    #[test]
    fn parent_metadata_accepts_fork_and_spawn_parent() {
        let payload = serde_json::json!({
            "forked_from_id": "00000000-0000-4000-8000-000000000001",
            "source": { "subagent": { "thread_spawn": {
                "parent_thread_id": "00000000-0000-4000-8000-000000000001"
            }}}
        });
        assert!(matches!(
            explicit_parent_from_meta(&payload),
            ParentResolution::Parent(value) if value == "00000000-0000-4000-8000-000000000001"
        ));
    }

    #[test]
    fn replay_prefix_matches_only_the_parent_prefix() {
        let signature = |value| TokenUsageSignature {
            total: Some(TokenCountersSignature {
                input: Some(value),
                cached_input: Some(0),
                output: Some(0),
                reasoning_output: Some(0),
                total: Some(value),
            }),
            last: None,
        };
        let event = |value| ParsedTokenEvent {
            line_offset: value as i64,
            signature: signature(value),
            delta: DeltaTokens::default(),
            event_index: None,
            model: "gpt-5.6-sol".to_string(),
            timestamp: None,
        };
        assert_eq!(
            matching_replay_prefix(
                &[event(1), event(2), event(3)],
                &[signature(1), signature(2)]
            ),
            2
        );
    }
}
