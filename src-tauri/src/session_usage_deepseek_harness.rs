//! DeepSeek Harness 会话使用量同步。
//!
//! 官方默认后端把每个会话写成带 Zstandard 帧的 JSONL 文件：
//! `DSH_HOME/sessions/**/session.jsonl.zstd`。本适配器只读取结构化的
//! `request/context`、`assistant/chunk` usage 和 `assistant/message.usage`，
//! 不读取或保存提示词、工具参数和回复正文。

use crate::calculator::{CostCalculator, TokenUsage};
use crate::database::Database;
use crate::error::{AppError, Result};
use crate::schema::{DATA_SOURCE_DEEPSEEK_HARNESS_SESSION, INPUT_TOKEN_SEMANTICS_FRESH};
use crate::session_usage::{
    find_model_pricing, get_sync_state, metadata_modified_nanos, modified_nanos_to_seconds,
    project_name_from_path, should_skip_session_insert, update_session_project, update_sync_state,
    DedupKey, SessionSyncResult,
};
use rusqlite::{Connection, OpenFlags};
use rust_decimal::Decimal;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

const APP_TYPE: &str = "deepseek_harness";
const PROVIDER_ID: &str = "_deepseek_harness_session";
const PROVIDER_TYPE: &str = DATA_SOURCE_DEEPSEEK_HARNESS_SESSION;
const SESSION_FORMAT_VERSION: i64 = 0;

#[derive(Debug, Clone)]
struct SessionMeta {
    id: String,
    version: i64,
    created_at: i64,
    cwd: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct UsageCounts {
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_creation_tokens: u32,
}

impl UsageCounts {
    fn has_billable_tokens(self) -> bool {
        self.input_tokens > 0
            || self.output_tokens > 0
            || self.cache_read_tokens > 0
            || self.cache_creation_tokens > 0
    }
}

#[derive(Debug, Clone)]
struct UsageRecord {
    event_seq: i64,
    turn: i64,
    step: i64,
    created_at: i64,
    model: String,
    counts: UsageCounts,
}

#[derive(Debug, Clone, Default)]
struct SessionRuntimeStats {
    model: String,
    started_at_ms: i64,
    last_event_at_ms: i64,
    turns: i64,
    steps: i64,
    llm_ms: i64,
    tool_ms: i64,
    ttft_ms: i64,
    ttft_steps: i64,
    decode_ms: i64,
    decode_tokens: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct StepKey {
    turn: i64,
    step: i64,
}

#[derive(Debug, Clone, Copy)]
struct OpenStep {
    turn: i64,
    step: i64,
    start_time_ms: i64,
    first_token_time_ms: Option<i64>,
}

#[derive(Default)]
struct RuntimeAccumulator {
    stats: SessionRuntimeStats,
    last_turn: Option<i64>,
    open_step: Option<OpenStep>,
    pending_calls: HashMap<String, i64>,
}

impl RuntimeAccumulator {
    fn push_event(&mut self, time: i64, event_type: &str, data: &Map<String, Value>) {
        if time > 0 {
            if self.stats.started_at_ms == 0 {
                self.stats.started_at_ms = time;
            } else {
                self.stats.started_at_ms = self.stats.started_at_ms.min(time);
            }
            self.stats.last_event_at_ms = self.stats.last_event_at_ms.max(time);
        }

        match event_type {
            "step/start" => {
                let Some(turn) = data.get("turn").and_then(json_i64) else {
                    return;
                };
                let Some(step) = data.get("step").and_then(json_i64) else {
                    return;
                };
                self.open_step = Some(OpenStep {
                    turn,
                    step,
                    start_time_ms: time,
                    first_token_time_ms: None,
                });
            }
            "assistant/chunk" => {
                let Some(open_step) = self.open_step else {
                    return;
                };
                if open_step.first_token_time_ms.is_some()
                    || !same_step(data, open_step.turn, open_step.step)
                {
                    return;
                }
                let Some(chunk) = data.get("chunk").and_then(Value::as_object) else {
                    return;
                };
                if is_non_empty_token_delta(chunk) {
                    self.open_step = Some(OpenStep {
                        first_token_time_ms: (time > 0).then_some(time),
                        ..open_step
                    });
                }
            }
            "assistant/message" => {
                let Some(open_step) = self.open_step else {
                    return;
                };
                if !same_step(data, open_step.turn, open_step.step) {
                    return;
                }

                self.stats.llm_ms = self
                    .stats
                    .llm_ms
                    .saturating_add(non_negative_delta(time, open_step.start_time_ms));
                if let Some(first_token_time_ms) = open_step.first_token_time_ms {
                    self.stats.ttft_ms = self.stats.ttft_ms.saturating_add(non_negative_delta(
                        first_token_time_ms,
                        open_step.start_time_ms,
                    ));
                    self.stats.ttft_steps = self.stats.ttft_steps.saturating_add(1);
                    if let Some(output_tokens) = data
                        .get("usage")
                        .and_then(Value::as_object)
                        .and_then(|usage| usage.get("outputTokens"))
                        .and_then(json_i64)
                    {
                        self.stats.decode_ms = self
                            .stats
                            .decode_ms
                            .saturating_add(non_negative_delta(time, first_token_time_ms));
                        self.stats.decode_tokens =
                            self.stats.decode_tokens.saturating_add(output_tokens);
                    }
                }
                self.open_step = None;
            }
            "tool/call" => {
                if let Some(call_id) = data.get("callId").and_then(non_empty_string) {
                    if time > 0 {
                        self.pending_calls.insert(call_id.to_string(), time);
                    }
                }
            }
            "tool/result" => {
                let Some(call_id) = tool_result_call_id(data) else {
                    return;
                };
                let Some(dispatched_at) = self.pending_calls.remove(call_id) else {
                    return;
                };
                self.stats.tool_ms = self
                    .stats
                    .tool_ms
                    .saturating_add(non_negative_delta(time, dispatched_at));
            }
            "step/end" => {
                let Some(turn) = data.get("turn").and_then(json_i64) else {
                    return;
                };
                self.stats.steps = self.stats.steps.saturating_add(1);
                if self.last_turn != Some(turn) {
                    self.stats.turns = self.stats.turns.saturating_add(1);
                    self.last_turn = Some(turn);
                }
                self.open_step = None;
            }
            "turn/end" => {
                self.pending_calls.clear();
            }
            _ => {}
        }
    }

    fn finish(mut self, model: String, fallback_created_at_ms: i64) -> SessionRuntimeStats {
        if self.stats.started_at_ms == 0 {
            self.stats.started_at_ms = fallback_created_at_ms;
        }
        if self.stats.last_event_at_ms == 0 {
            self.stats.last_event_at_ms = self.stats.started_at_ms;
        }
        self.stats.model = model;
        self.stats
    }
}

#[derive(Debug, Clone)]
struct ModelContext {
    model: String,
    provider: String,
}

impl Default for ModelContext {
    fn default() -> Self {
        Self {
            model: "unknown".to_string(),
            provider: "unknown".to_string(),
        }
    }
}

#[derive(Default)]
struct UsageAccumulator {
    context: ModelContext,
    chunk_usage: HashMap<StepKey, UsageRecord>,
    message_usage: HashMap<StepKey, UsageRecord>,
    runtime: RuntimeAccumulator,
}

/// 同步 DeepSeek Harness 的 JSONL 会话，并按显式环境变量支持其 SQLite 后端。
pub fn sync_deepseek_harness_usage(db: &Database) -> Result<SessionSyncResult> {
    let mut result = SessionSyncResult::default();

    for root in session_roots() {
        let files = match collect_session_files(&root) {
            Ok(files) => files,
            Err(error) => {
                result.errors.push(format!(
                    "DeepSeek Harness 目录扫描失败 {}: {error}",
                    root.display()
                ));
                continue;
            }
        };
        result.files_scanned = result.files_scanned.saturating_add(files.len() as u32);

        for path in files {
            match sync_jsonl_file(db, &path) {
                Ok((imported, skipped)) => {
                    result.imported = result.imported.saturating_add(imported);
                    result.skipped = result.skipped.saturating_add(skipped);
                }
                Err(error) => result.errors.push(format!(
                    "DeepSeek Harness 文件解析失败 {}: {error}",
                    path.display()
                )),
            }
        }
    }

    if let Some(path) = sqlite_path() {
        if path.exists() {
            result.files_scanned = result.files_scanned.saturating_add(1);
            match sync_sqlite_file(db, &path) {
                Ok((imported, skipped)) => {
                    result.imported = result.imported.saturating_add(imported);
                    result.skipped = result.skipped.saturating_add(skipped);
                }
                Err(error) => result.errors.push(format!(
                    "DeepSeek Harness SQLite 解析失败 {}: {error}",
                    path.display()
                )),
            }
        }
    }

    if result.imported > 0 {
        log::info!(
            "[DEEPSEEK-HARNESS-SYNC] 同步完成: 导入 {} 条, 跳过 {} 条, 扫描 {} 个文件",
            result.imported,
            result.skipped,
            result.files_scanned
        );
    }

    Ok(result)
}

fn session_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for key in ["DEEPSEEK_HARNESS_SESSIONS", "DSH_SESSION_ROOT"] {
        if let Some(value) = env::var_os(key).filter(|value| !value.is_empty()) {
            push_unique_path(&mut roots, resolve_configured_path(value));
        }
    }

    let dsh_home = env::var_os("DSH_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".dsh")));
    if let Some(home) = dsh_home {
        push_unique_path(&mut roots, home.join("sessions"));
    }
    roots
}

fn sqlite_path() -> Option<PathBuf> {
    ["DEEPSEEK_HARNESS_SQLITE", "DSH_SESSION_SQLITE"]
        .into_iter()
        .find_map(|key| {
            env::var_os(key)
                .filter(|value| !value.is_empty())
                .map(resolve_configured_path)
        })
}

fn resolve_configured_path(value: std::ffi::OsString) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn collect_session_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    collect_session_files_inner(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_session_files_inner(root: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_session_files_inner(&path, files)?;
        } else if file_type.is_file() && is_session_log(&path) {
            files.push(path);
        }
    }
    Ok(())
}

fn is_session_log(path: &Path) -> bool {
    matches!(
        path.file_name(),
        Some(name) if name == OsStr::new("session.jsonl.zstd") || name == OsStr::new("session.jsonl")
    )
}

fn sync_jsonl_file(db: &Database, path: &Path) -> Result<(u32, u32)> {
    let metadata = fs::metadata(path)?;
    let modified = metadata_modified_nanos(&metadata);
    let path_key = path.to_string_lossy().to_string();
    let (last_modified, _) = get_sync_state(db, &path_key)?;
    let needs_usage_sync = modified > last_modified;
    if !needs_usage_sync && has_runtime_stats(db, &path_key, None)? {
        return Ok((0, 0));
    }

    let (meta, records, runtime_stats, line_count) = parse_jsonl_file(path)?;
    let project = project_name_from_path(meta.cwd.as_deref());
    let fallback_created_at = modified_nanos_to_seconds(modified)
        .or_else(|| (meta.created_at > 0).then(|| normalize_epoch_seconds(meta.created_at)));

    let mut imported = 0u32;
    let mut skipped = 0u32;
    if needs_usage_sync {
        for record in &records {
            match insert_usage_record(db, &meta, &project, record, fallback_created_at)? {
                true => imported = imported.saturating_add(1),
                false => skipped = skipped.saturating_add(1),
            }
        }
    }

    upsert_runtime_stats(
        db,
        &meta,
        &project,
        &path_key,
        &runtime_stats,
        fallback_created_at,
    )?;

    if needs_usage_sync {
        update_sync_state(db, &path_key, modified, line_count)?;
    }
    Ok((imported, skipped))
}

fn parse_jsonl_file(
    path: &Path,
) -> Result<(SessionMeta, Vec<UsageRecord>, SessionRuntimeStats, i64)> {
    let file = File::open(path)?;
    let reader: Box<dyn Read> = if path.extension() == Some(OsStr::new("zstd")) {
        Box::new(zstd::stream::read::Decoder::new(file)?)
    } else {
        Box::new(file)
    };
    let mut reader = BufReader::new(reader);

    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(AppError::Config(
            "DeepSeek Harness 会话文件为空".to_string(),
        ));
    }
    let header: Value = serde_json::from_str(&line)?;
    let meta = parse_session_header(&header)?;
    let mut accumulator = UsageAccumulator::default();
    let mut line_count = 1i64;

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        line_count = line_count.saturating_add(1);
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let seq = value
            .get("seq")
            .and_then(json_i64)
            .unwrap_or(line_count - 1);
        let time = value.get("time").and_then(json_i64).unwrap_or(0);
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        accumulator.push_event(seq, time, event_type, value.get("data"));
    }

    let fallback_created_at_ms = meta.created_at.max(0);
    let (records, runtime_stats) = accumulator.finish(fallback_created_at_ms);
    Ok((meta, records, runtime_stats, line_count))
}

fn parse_session_header(value: &Value) -> Result<SessionMeta> {
    let version = value
        .get("version")
        .and_then(json_i64)
        .ok_or_else(|| AppError::Config("DeepSeek Harness 会话头缺少 version".to_string()))?;
    if version != SESSION_FORMAT_VERSION {
        return Err(AppError::Config(format!(
            "DeepSeek Harness 会话格式版本 {version} 暂不支持（当前支持 {SESSION_FORMAT_VERSION}）"
        )));
    }
    if value.get("type").and_then(Value::as_str) != Some("session") {
        return Err(AppError::Config(
            "DeepSeek Harness 会话文件首行不是 session header".to_string(),
        ));
    }
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| AppError::Config("DeepSeek Harness 会话头缺少 id".to_string()))?;
    let created_at = value.get("createdAt").and_then(json_i64).unwrap_or(0);
    let cwd = value
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .map(str::to_string);

    Ok(SessionMeta {
        id: id.to_string(),
        version,
        created_at,
        cwd,
    })
}

impl UsageAccumulator {
    fn push_event(&mut self, seq: i64, time: i64, event_type: &str, data: Option<&Value>) {
        let Some(data) = data.and_then(Value::as_object) else {
            return;
        };

        self.runtime.push_event(time, event_type, data);

        if event_type == "request/context" {
            self.update_context(data);
        }

        let turn = data.get("turn").and_then(json_i64).unwrap_or(-1);
        let step = data.get("step").and_then(json_i64).unwrap_or(seq);
        let key = StepKey { turn, step };
        let model = model_from_data(data).unwrap_or_else(|| self.context.model.clone());
        let created_at = normalize_epoch_seconds(time);

        if event_type == "assistant/chunk" {
            let Some(chunk) = data.get("chunk").and_then(Value::as_object) else {
                return;
            };
            if chunk.get("type").and_then(Value::as_str) != Some("usage") {
                return;
            }
            if let Some(counts) = parse_usage(chunk.get("usage")) {
                replace_record(
                    &mut self.chunk_usage,
                    key,
                    UsageRecord {
                        event_seq: seq,
                        turn,
                        step,
                        created_at,
                        model,
                        counts,
                    },
                );
            }
        } else if event_type == "assistant/message" {
            if let Some(counts) = parse_usage(data.get("usage")) {
                replace_record(
                    &mut self.message_usage,
                    key,
                    UsageRecord {
                        event_seq: seq,
                        turn,
                        step,
                        created_at,
                        model,
                        counts,
                    },
                );
            }
        }
    }

    fn update_context(&mut self, data: &Map<String, Value>) {
        if let Some(model) = data.get("model").and_then(non_empty_string) {
            self.context.model = model.to_string();
        }
        if let Some(provider) = data.get("provider").and_then(non_empty_string) {
            self.context.provider = provider.to_string();
        }
    }

    fn finish(self, fallback_created_at_ms: i64) -> (Vec<UsageRecord>, SessionRuntimeStats) {
        let model = self.context.model.clone();
        let runtime_stats = self.runtime.finish(model, fallback_created_at_ms);
        let chunk_keys: HashSet<StepKey> = self.chunk_usage.keys().copied().collect();
        let mut records: Vec<UsageRecord> = self.chunk_usage.into_values().collect();
        records.extend(
            self.message_usage
                .into_iter()
                .filter_map(|(key, record)| (!chunk_keys.contains(&key)).then_some(record)),
        );
        records.sort_by_key(|record| record.event_seq);
        (records, runtime_stats)
    }
}

fn replace_record(records: &mut HashMap<StepKey, UsageRecord>, key: StepKey, record: UsageRecord) {
    let should_replace = records
        .get(&key)
        .map(|existing| record.event_seq >= existing.event_seq)
        .unwrap_or(true);
    if should_replace {
        records.insert(key, record);
    }
}

fn same_step(data: &Map<String, Value>, turn: i64, step: i64) -> bool {
    data.get("turn").and_then(json_i64) == Some(turn)
        && data.get("step").and_then(json_i64) == Some(step)
}

fn non_negative_delta(end: i64, start: i64) -> i64 {
    if end > 0 && start > 0 {
        end.saturating_sub(start).max(0)
    } else {
        0
    }
}

/// 与官方 dsh-session-stats 的 isTokenDelta 对齐：空心跳、空工具帧不算首 token。
fn is_non_empty_token_delta(chunk: &Map<String, Value>) -> bool {
    match chunk.get("type").and_then(Value::as_str) {
        Some("text-delta") | Some("reasoning-delta") => chunk
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| !text.is_empty()),
        Some("tool-call-delta") => {
            chunk
                .get("argumentsDelta")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
                || chunk.get("name").is_some()
        }
        _ => false,
    }
}

fn tool_result_call_id(data: &Map<String, Value>) -> Option<&str> {
    data.get("message")
        .and_then(Value::as_object)
        .and_then(|message| message.get("source"))
        .and_then(Value::as_object)
        .and_then(|source| source.get("callId"))
        .and_then(non_empty_string)
        .or_else(|| data.get("callId").and_then(non_empty_string))
}

fn parse_usage(value: Option<&Value>) -> Option<UsageCounts> {
    let value = value?.as_object()?;
    let counts = UsageCounts {
        input_tokens: json_u32(value.get("inputTokens")),
        output_tokens: json_u32(value.get("outputTokens")),
        cache_read_tokens: json_u32(value.get("cacheReadTokens")),
        cache_creation_tokens: json_u32(value.get("cacheWriteTokens")),
    };
    counts.has_billable_tokens().then_some(counts)
}

fn model_from_data(data: &Map<String, Value>) -> Option<String> {
    data.get("model")
        .and_then(non_empty_string)
        .map(str::to_string)
        .or_else(|| {
            data.get("message")
                .and_then(Value::as_object)
                .and_then(|message| message.get("model"))
                .and_then(non_empty_string)
                .map(str::to_string)
        })
}

fn non_empty_string(value: &Value) -> Option<&str> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| value.try_into().ok()))
}

fn json_u32(value: Option<&Value>) -> u32 {
    value
        .and_then(json_i64)
        .unwrap_or(0)
        .clamp(0, u32::MAX as i64) as u32
}

fn normalize_epoch_seconds(value: i64) -> i64 {
    if value > 100_000_000_000 {
        value / 1_000
    } else {
        value
    }
}

fn has_runtime_stats(db: &Database, source_path: &str, session_id: Option<&str>) -> Result<bool> {
    db.with_conn(|conn| {
        let result = if let Some(session_id) = session_id {
            conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM session_runtime_stats
                    WHERE data_source = ?1 AND source_path = ?2 AND session_id = ?3
                 )",
                rusqlite::params![PROVIDER_TYPE, source_path, session_id],
                |row| row.get(0),
            )
        } else {
            conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM session_runtime_stats
                    WHERE data_source = ?1 AND source_path = ?2
                 )",
                rusqlite::params![PROVIDER_TYPE, source_path],
                |row| row.get(0),
            )
        };
        Ok(result.unwrap_or(false))
    })
}

fn upsert_runtime_stats(
    db: &Database,
    meta: &SessionMeta,
    project: &str,
    source_path: &str,
    stats: &SessionRuntimeStats,
    fallback_created_at: Option<i64>,
) -> Result<()> {
    let started_at = (stats.started_at_ms > 0)
        .then(|| normalize_epoch_seconds(stats.started_at_ms))
        .or(fallback_created_at)
        .or_else(|| (meta.created_at > 0).then(|| normalize_epoch_seconds(meta.created_at)))
        .unwrap_or(0);
    let last_event_at = (stats.last_event_at_ms > 0)
        .then(|| normalize_epoch_seconds(stats.last_event_at_ms))
        .unwrap_or(started_at);

    db.with_conn(|conn| {
        conn.execute(
            "INSERT OR REPLACE INTO session_runtime_stats (
                data_source, app_type, provider_id, session_id, model, project, source_path,
                turns, steps, llm_ms, tool_ms, ttft_ms, ttft_steps, decode_ms, decode_tokens,
                started_at, last_event_at, device_id, device_name
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                COALESCE((SELECT value FROM settings WHERE key = 'local_device_id'), ''),
                COALESCE((SELECT value FROM settings WHERE key = 'local_device_name'), '')
            )",
            rusqlite::params![
                PROVIDER_TYPE,
                APP_TYPE,
                PROVIDER_ID,
                meta.id,
                stats.model,
                project,
                source_path,
                stats.turns,
                stats.steps,
                stats.llm_ms,
                stats.tool_ms,
                stats.ttft_ms,
                stats.ttft_steps,
                stats.decode_ms,
                stats.decode_tokens,
                started_at,
                last_event_at,
            ],
        )?;
        Ok(())
    })
}

fn insert_usage_record(
    db: &Database,
    meta: &SessionMeta,
    project: &str,
    record: &UsageRecord,
    fallback_created_at: Option<i64>,
) -> Result<bool> {
    let created_at = (record.created_at > 0)
        .then(|| normalize_epoch_seconds(record.created_at))
        .or(fallback_created_at)
        .or_else(|| (meta.created_at > 0).then(|| normalize_epoch_seconds(meta.created_at)));
    let Some(created_at) = created_at else {
        return Ok(false);
    };

    db.with_conn(|conn| {
        let dedup_key = DedupKey {
            app_type: APP_TYPE,
            model: &record.model,
            input_tokens: record.counts.input_tokens,
            output_tokens: record.counts.output_tokens,
            cache_read_tokens: record.counts.cache_read_tokens,
            cache_creation_tokens: record.counts.cache_creation_tokens,
            created_at,
        };
        let request_id = format!("{DATA_SOURCE_DEEPSEEK_HARNESS_SESSION}:{}:{}", meta.id, record.event_seq);
        if should_skip_session_insert(conn, &request_id, &dedup_key)? {
            return Ok(false);
        }

        let usage = TokenUsage {
            input_tokens: record.counts.input_tokens,
            output_tokens: record.counts.output_tokens,
            cache_read_tokens: record.counts.cache_read_tokens,
            cache_creation_tokens: record.counts.cache_creation_tokens,
            model: Some(record.model.clone()),
            message_id: None,
        };
        let (input_cost, output_cost, cache_read_cost, cache_creation_cost, total_cost) =
            match find_model_pricing(conn, &record.model) {
                Some(pricing) => {
                    let cost = CostCalculator::calculate(&usage, &pricing, Decimal::from(1));
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
                PROVIDER_ID,
                APP_TYPE,
                record.model,
                record.model,
                record.counts.input_tokens,
                record.counts.output_tokens,
                record.counts.cache_read_tokens,
                record.counts.cache_creation_tokens,
                INPUT_TOKEN_SEMANTICS_FRESH,
                input_cost,
                output_cost,
                cache_read_cost,
                cache_creation_cost,
                total_cost,
                0i64,
                Option::<i64>::None,
                200i64,
                Option::<String>::None,
                Some(meta.id.clone()),
                PROVIDER_TYPE,
                1i64,
                "1.0",
                created_at,
                DATA_SOURCE_DEEPSEEK_HARNESS_SESSION,
                project,
            ],
        )?;
        Ok(inserted > 0)
    })
}

fn sync_sqlite_file(db: &Database, path: &Path) -> Result<(u32, u32)> {
    let modified = sqlite_modified_nanos(path)?;
    let path_key = path.to_string_lossy().to_string();
    let (last_modified, _) = get_sync_state(db, &path_key)?;
    let source = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    source.busy_timeout(Duration::from_secs(2))?;

    let sessions: Vec<SessionMeta> = {
        let mut statement = source
            .prepare("SELECT id, version, created_at, cwd FROM sessions ORDER BY created_at, id")?;
        let mapped = statement
            .query_map([], |row| {
                Ok(SessionMeta {
                    id: row.get(0)?,
                    version: row.get(1)?,
                    created_at: row.get(2)?,
                    cwd: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        mapped
    };

    let mut imported = 0u32;
    let mut skipped = 0u32;
    let mut event_count = 0i64;
    for meta in &sessions {
        if meta.version != SESSION_FORMAT_VERSION {
            return Err(AppError::Config(format!(
                "DeepSeek Harness SQLite 会话 {} 格式版本 {} 暂不支持",
                meta.id, meta.version
            )));
        }
        let project = project_name_from_path(meta.cwd.as_deref());
        update_session_project(db, APP_TYPE, &meta.id, &project)?;
        let runtime_stats_present = has_runtime_stats(db, &path_key, Some(&meta.id))?;
        let needs_usage_sync = modified > last_modified;
        if !needs_usage_sync && runtime_stats_present {
            continue;
        }

        let mut statement = source.prepare(
            "SELECT seq, type, time, data FROM events WHERE session_id = ?1 ORDER BY seq",
        )?;
        let mut rows = statement.query([&meta.id])?;
        let mut accumulator = UsageAccumulator::default();
        while let Some(row) = rows.next()? {
            let seq: i64 = row.get(0)?;
            let event_type: String = row.get(1)?;
            let time: i64 = row.get(2)?;
            let data: String = row.get(3)?;
            let value: Value = match serde_json::from_str(&data) {
                Ok(value) => value,
                Err(_) => continue,
            };
            event_count = event_count.saturating_add(1);
            accumulator.push_event(seq, time, &event_type, Some(&value));
        }

        let fallback_created_at = (meta.created_at > 0)
            .then(|| normalize_epoch_seconds(meta.created_at))
            .or_else(|| modified_nanos_to_seconds(modified));
        let (records, runtime_stats) = accumulator.finish(meta.created_at.max(0));
        if needs_usage_sync {
            for record in &records {
                match insert_usage_record(db, meta, &project, record, fallback_created_at)? {
                    true => imported = imported.saturating_add(1),
                    false => skipped = skipped.saturating_add(1),
                }
            }
        }
        upsert_runtime_stats(
            db,
            meta,
            &project,
            &path_key,
            &runtime_stats,
            fallback_created_at,
        )?;
    }

    if modified > last_modified {
        update_sync_state(db, &path_key, modified, event_count)?;
    }
    Ok((imported, skipped))
}

fn sqlite_modified_nanos(path: &Path) -> Result<i64> {
    let mut modified = metadata_modified_nanos(&fs::metadata(path)?);
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        if let Ok(metadata) = fs::metadata(PathBuf::from(sidecar)) {
            modified = modified.max(metadata_modified_nanos(&metadata));
        }
    }
    Ok(modified)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_chunk_is_authoritative_and_reasoning_is_not_double_counted() {
        let mut accumulator = UsageAccumulator::default();
        let context = serde_json::json!({
            "provider": "deepseek",
            "model": "deepseek-chat"
        });
        accumulator.push_event(1, 1_780_000_000_000, "request/context", Some(&context));
        let chunk = serde_json::json!({
            "turn": 1,
            "step": 1,
            "chunk": {
                "type": "usage",
                "usage": {
                    "inputTokens": 100,
                    "outputTokens": 20,
                    "reasoningTokens": 8,
                    "cacheReadTokens": 30
                }
            }
        });
        accumulator.push_event(2, 1_780_000_001_000, "assistant/chunk", Some(&chunk));
        let fallback = serde_json::json!({
            "turn": 1,
            "step": 1,
            "usage": {"inputTokens": 999, "outputTokens": 999}
        });
        accumulator.push_event(3, 1_780_000_001_100, "assistant/message", Some(&fallback));
        let (records, _) = accumulator.finish(0);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].model, "deepseek-chat");
        assert_eq!(records[0].counts.output_tokens, 20);
        assert_eq!(records[0].counts.cache_read_tokens, 30);
    }

    #[test]
    fn message_usage_fills_only_missing_steps() {
        let mut accumulator = UsageAccumulator::default();
        let message = serde_json::json!({
            "turn": 2,
            "step": 3,
            "usage": {
                "inputTokens": 10,
                "outputTokens": 5,
                "cacheWriteTokens": 2
            }
        });
        accumulator.push_event(7, 1_780_000_001_000, "assistant/message", Some(&message));
        let (records, _) = accumulator.finish(0);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].counts.cache_creation_tokens, 2);
        assert_eq!(records[0].turn, 2);
        assert_eq!(records[0].step, 3);
    }

    #[test]
    fn runtime_stats_follow_official_session_stats_fold() {
        let mut accumulator = UsageAccumulator::default();
        accumulator.push_event(
            1,
            1_780_000_000_000,
            "turn/start",
            Some(&serde_json::json!({"turn": 1})),
        );
        accumulator.push_event(
            2,
            1_780_000_001_000,
            "step/start",
            Some(&serde_json::json!({"turn": 1, "step": 1})),
        );
        accumulator.push_event(
            3,
            1_780_000_001_100,
            "assistant/chunk",
            Some(&serde_json::json!({
                "turn": 1,
                "step": 1,
                "chunk": {"type": "reasoning-delta", "text": "token"}
            })),
        );
        accumulator.push_event(
            4,
            1_780_000_001_200,
            "assistant/chunk",
            Some(&serde_json::json!({
                "turn": 1,
                "step": 1,
                "chunk": {"type": "tool-call-delta", "argumentsDelta": ""}
            })),
        );
        accumulator.push_event(
            5,
            1_780_000_001_500,
            "assistant/message",
            Some(&serde_json::json!({
                "turn": 1,
                "step": 1,
                "usage": {"outputTokens": 80}
            })),
        );
        accumulator.push_event(
            6,
            1_780_000_001_600,
            "tool/call",
            Some(&serde_json::json!({"turn": 1, "step": 1, "callId": "call-1"})),
        );
        accumulator.push_event(
            7,
            1_780_000_002_100,
            "tool/result",
            Some(&serde_json::json!({
                "turn": 1,
                "step": 1,
                "message": {"source": {"callId": "call-1"}}
            })),
        );
        accumulator.push_event(
            8,
            1_780_000_002_200,
            "step/end",
            Some(&serde_json::json!({"turn": 1, "step": 1})),
        );
        accumulator.push_event(
            9,
            1_780_000_002_300,
            "turn/end",
            Some(&serde_json::json!({"turn": 1})),
        );

        let (_, stats) = accumulator.finish(0);
        assert_eq!(stats.turns, 1);
        assert_eq!(stats.steps, 1);
        assert_eq!(stats.llm_ms, 500);
        assert_eq!(stats.tool_ms, 500);
        assert_eq!(stats.ttft_ms, 100);
        assert_eq!(stats.ttft_steps, 1);
        assert_eq!(stats.decode_ms, 400);
        assert_eq!(stats.decode_tokens, 80);
    }

    #[test]
    fn runtime_stats_count_closed_steps_without_assistant_message() {
        let mut accumulator = UsageAccumulator::default();
        accumulator.push_event(
            1,
            1_780_000_000_000,
            "step/start",
            Some(&serde_json::json!({"turn": 1, "step": 1})),
        );
        accumulator.push_event(
            2,
            1_780_000_000_100,
            "step/end",
            Some(&serde_json::json!({"turn": 1, "step": 1})),
        );

        let (_, stats) = accumulator.finish(0);
        assert_eq!(stats.turns, 1);
        assert_eq!(stats.steps, 1);
        assert_eq!(stats.llm_ms, 0);
        assert_eq!(stats.ttft_steps, 0);
    }
}
