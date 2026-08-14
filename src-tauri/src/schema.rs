//! 数据库 Schema 与初始化
//!
//! 仅保留 usage 子系统相关的三张表：
//! - proxy_request_logs: 请求明细
//! - usage_daily_rollups: 日聚合（冷热分层）
//! - model_pricing: 模型定价
//! - session_log_sync: 会话日志同步进度

use crate::error::Result;
use rusqlite::Connection;

/// 输入 token 语义
pub const INPUT_TOKEN_SEMANTICS_LEGACY: i64 = 0;
pub const INPUT_TOKEN_SEMANTICS_TOTAL: i64 = 1; // cache-inclusive
pub const INPUT_TOKEN_SEMANTICS_FRESH: i64 = 2; // fresh input

/// 缓存包含型应用（input_tokens 包含 cache_read + cache_creation）
pub const CACHE_INCLUSIVE_APP_TYPES: &[&str] = &["codex", "gemini", "grokbuild", "zcode"];

pub fn is_cache_inclusive_app(app_type: &str) -> bool {
    CACHE_INCLUSIVE_APP_TYPES.contains(&app_type)
}

/// 计费模式常量
pub const PRICING_SOURCE_RESPONSE: &str = "response";
pub const PRICING_SOURCE_REQUEST: &str = "request";

/// 数据源标识
pub const DATA_SOURCE_PROXY: &str = "proxy";
pub const DATA_SOURCE_SESSION_LOG: &str = "session_log";
pub const DATA_SOURCE_CODEX_SESSION: &str = "codex_session";
pub const DATA_SOURCE_GEMINI_SESSION: &str = "gemini_session";
pub const DATA_SOURCE_OPENCODE_SESSION: &str = "opencode_session";
pub const DATA_SOURCE_ZCODE_SESSION: &str = "zcode_session";
pub const DATA_SOURCE_GROK_SESSION: &str = "grok_session";

/// 跨源去重窗口（秒）：10 分钟内已有等价 proxy 行则跳过 session 行
pub const SESSION_PROXY_DEDUP_WINDOW_SECONDS: i64 = 600;

/// 会话日志 request_id 前缀
pub const SESSION_REQUEST_ID_PREFIX: &str = "session:";

/// 在指定连接上创建全部表
pub fn create_tables(conn: &Connection) -> Result<()> {
    // 1. 请求明细表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS proxy_request_logs (
            request_id TEXT PRIMARY KEY,
            provider_id TEXT NOT NULL,
            app_type TEXT NOT NULL,
            model TEXT NOT NULL,
            request_model TEXT,
            pricing_model TEXT,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            input_token_semantics INTEGER NOT NULL DEFAULT 0,
            input_cost_usd TEXT NOT NULL DEFAULT '0',
            output_cost_usd TEXT NOT NULL DEFAULT '0',
            cache_read_cost_usd TEXT NOT NULL DEFAULT '0',
            cache_creation_cost_usd TEXT NOT NULL DEFAULT '0',
            total_cost_usd TEXT NOT NULL DEFAULT '0',
            latency_ms INTEGER NOT NULL,
            first_token_ms INTEGER,
            duration_ms INTEGER,
            status_code INTEGER NOT NULL,
            error_message TEXT,
            session_id TEXT,
            provider_type TEXT,
            is_streaming INTEGER NOT NULL DEFAULT 0,
            cost_multiplier TEXT NOT NULL DEFAULT '1.0',
            created_at INTEGER NOT NULL,
            data_source TEXT NOT NULL DEFAULT 'proxy',
            device_id TEXT NOT NULL DEFAULT '',
            device_name TEXT NOT NULL DEFAULT ''
        )",
        [],
    )?;

    ensure_column(
        conn,
        "device_id",
        "ALTER TABLE proxy_request_logs ADD COLUMN device_id TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        conn,
        "device_name",
        "ALTER TABLE proxy_request_logs ADD COLUMN device_name TEXT NOT NULL DEFAULT ''",
    )?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_request_logs_provider
         ON proxy_request_logs(provider_id, app_type)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_request_logs_created_at
         ON proxy_request_logs(created_at)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_request_logs_model ON proxy_request_logs(model)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_request_logs_session ON proxy_request_logs(session_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_request_logs_status ON proxy_request_logs(status_code)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_request_logs_dedup_source
         ON proxy_request_logs(
             data_source, app_type, model, input_tokens, output_tokens,
             cache_read_tokens, cache_creation_tokens, created_at
         )",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_request_logs_app_created_at
         ON proxy_request_logs(app_type, created_at DESC)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_request_logs_device_created_at
         ON proxy_request_logs(device_id, created_at DESC)",
        [],
    )?;

    // 2. 模型定价表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS model_pricing (
            model_id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            input_cost_per_million TEXT NOT NULL,
            output_cost_per_million TEXT NOT NULL,
            cache_read_cost_per_million TEXT NOT NULL DEFAULT '0',
            cache_creation_cost_per_million TEXT NOT NULL DEFAULT '0',
            price_source TEXT NOT NULL DEFAULT 'Built-in fallback',
            price_fetched_at INTEGER
        )",
        [],
    )?;
    ensure_model_pricing_column(
        conn,
        "price_source",
        "ALTER TABLE model_pricing ADD COLUMN price_source TEXT NOT NULL DEFAULT 'Built-in fallback'",
    )?;
    ensure_model_pricing_column(
        conn,
        "price_fetched_at",
        "ALTER TABLE model_pricing ADD COLUMN price_fetched_at INTEGER",
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS model_pricing_sync (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            source TEXT NOT NULL DEFAULT 'Built-in fallback',
            fetched_at INTEGER,
            catalog_models INTEGER NOT NULL DEFAULT 0,
            matched_models INTEGER NOT NULL DEFAULT 0
        )",
        [],
    )?;

    // 3. 日聚合表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS usage_daily_rollups (
            date TEXT NOT NULL,
            app_type TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            model TEXT NOT NULL,
            request_model TEXT NOT NULL DEFAULT '',
            pricing_model TEXT NOT NULL DEFAULT '',
            request_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            input_token_semantics INTEGER NOT NULL DEFAULT 0,
            total_cost_usd TEXT NOT NULL DEFAULT '0',
            avg_latency_ms INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (date, app_type, provider_id, model, request_model, pricing_model)
        )",
        [],
    )?;

    // 4. 会话日志同步状态表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS session_log_sync (
            file_path TEXT PRIMARY KEY,
            last_modified INTEGER NOT NULL,
            last_line_offset INTEGER NOT NULL DEFAULT 0,
            last_synced_at INTEGER NOT NULL
        )",
        [],
    )?;

    // 5. 设置表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT)",
        [],
    )?;

    // 所有现有日志写入点继续使用原 SQL；触发器统一补上本机身份。
    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS trg_request_logs_device
         AFTER INSERT ON proxy_request_logs
         WHEN COALESCE(NEW.device_id, '') = ''
         BEGIN
             UPDATE proxy_request_logs
             SET device_id = COALESCE(
                     (SELECT value FROM settings WHERE key = 'local_device_id'), ''
                 ),
                 device_name = COALESCE(
                     (SELECT value FROM settings WHERE key = 'local_device_name'), ''
                 )
             WHERE request_id = NEW.request_id;
         END;",
    )?;

    Ok(())
}

fn ensure_column(conn: &Connection, name: &str, migration: &str) -> Result<()> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM pragma_table_info('proxy_request_logs') WHERE name = ?1
         )",
        rusqlite::params![name],
        |row| row.get(0),
    )?;
    if !exists {
        conn.execute(migration, [])?;
    }
    Ok(())
}

fn ensure_model_pricing_column(conn: &Connection, name: &str, migration: &str) -> Result<()> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM pragma_table_info('model_pricing') WHERE name = ?1
         )",
        rusqlite::params![name],
        |row| row.get(0),
    )?;
    if !exists {
        conn.execute(migration, [])?;
    }
    Ok(())
}

/// 初始化默认模型定价数据
pub fn seed_model_pricing(conn: &Connection) -> Result<()> {
    let pricing_data: &[(&str, &str, &str, &str, &str, &str)] = &[
        // Claude 系列
        ("claude-opus-5", "Claude Opus 5", "5", "25", "0.50", "6.25"),
        (
            "claude-opus-4-8",
            "Claude Opus 4.8",
            "5",
            "25",
            "0.50",
            "6.25",
        ),
        (
            "claude-opus-4-7",
            "Claude Opus 4.7",
            "5",
            "25",
            "0.50",
            "6.25",
        ),
        (
            "claude-opus-4-6",
            "Claude Opus 4.6",
            "5",
            "25",
            "0.50",
            "6.25",
        ),
        (
            "claude-fable-5",
            "Claude Fable 5",
            "10",
            "50",
            "1.00",
            "12.50",
        ),
        (
            "claude-mythos-5",
            "Claude Mythos 5",
            "10",
            "50",
            "1.00",
            "12.50",
        ),
        (
            "claude-sonnet-5",
            "Claude Sonnet 5",
            "3",
            "15",
            "0.30",
            "3.75",
        ),
        (
            "claude-sonnet-4-6",
            "Claude Sonnet 4.6",
            "3",
            "15",
            "0.30",
            "3.75",
        ),
        (
            "claude-sonnet-4-5-20250929",
            "Claude Sonnet 4.5",
            "3",
            "15",
            "0.30",
            "3.75",
        ),
        (
            "claude-haiku-4-5-20251001",
            "Claude Haiku 4.5",
            "1",
            "5",
            "0.10",
            "1.25",
        ),
        (
            "claude-opus-4-1-20250805",
            "Claude Opus 4.1",
            "15",
            "75",
            "1.50",
            "18.75",
        ),
        (
            "claude-opus-4-20250514",
            "Claude Opus 4",
            "15",
            "75",
            "1.50",
            "18.75",
        ),
        (
            "claude-sonnet-4-20250514",
            "Claude Sonnet 4",
            "3",
            "15",
            "0.30",
            "3.75",
        ),
        (
            "claude-3-5-haiku-20241022",
            "Claude 3.5 Haiku",
            "0.80",
            "4",
            "0.08",
            "1",
        ),
        (
            "claude-3-5-sonnet-20241022",
            "Claude 3.5 Sonnet",
            "3",
            "15",
            "0.30",
            "3.75",
        ),
        // GPT 系列
        ("gpt-5.6-sol", "GPT-5.6 Sol", "5", "30", "0.50", "6.25"),
        ("gpt-5.6-terra", "GPT-5.6 Terra", "2", "12", "0.20", "2.50"),
        (
            "gpt-5.6-luna",
            "GPT-5.6 Luna",
            "0.20",
            "1.20",
            "0.02",
            "0.25",
        ),
        ("gpt-5.5", "GPT-5.5", "5", "30", "0.50", "0"),
        ("gpt-5.4", "GPT-5.4", "2.50", "15", "0.25", "0"),
        ("gpt-5.4-mini", "GPT-5.4 Mini", "0.75", "4.50", "0.075", "0"),
        ("gpt-5.2", "GPT-5.2", "1.75", "14", "0.175", "0"),
        ("gpt-5.2-codex", "GPT-5.2 Codex", "1.75", "14", "0.175", "0"),
        ("gpt-5.1", "GPT-5.1", "1.25", "10", "0.125", "0"),
        ("gpt-5", "GPT-5", "1.25", "10", "0.125", "0"),
        ("gpt-4.1", "GPT-4.1", "2", "8", "0.50", "0"),
        ("gpt-4.1-mini", "GPT-4.1 Mini", "0.40", "1.60", "0.10", "0"),
        ("o3", "OpenAI o3", "2", "8", "0.50", "0"),
        ("o4-mini", "OpenAI o4-mini", "1.10", "4.40", "0.275", "0"),
        // Gemini 系列
        (
            "gemini-3.5-flash",
            "Gemini 3.5 Flash",
            "1.50",
            "9.00",
            "0.15",
            "0",
        ),
        (
            "gemini-2.5-pro",
            "Gemini 2.5 Pro",
            "1.25",
            "10",
            "0.125",
            "0",
        ),
        (
            "gemini-2.5-flash",
            "Gemini 2.5 Flash",
            "0.3",
            "2.5",
            "0.03",
            "0",
        ),
        // DeepSeek
        (
            "deepseek-v3.2",
            "DeepSeek V3.2",
            "0.28",
            "0.42",
            "0.028",
            "0",
        ),
        (
            "deepseek-v3.1",
            "DeepSeek V3.1",
            "0.55",
            "1.67",
            "0.055",
            "0",
        ),
        ("deepseek-v3", "DeepSeek V3", "0.28", "1.11", "0.028", "0"),
        // GLM
        ("glm-4.7", "GLM-4.7", "0.6", "2.2", "0.11", "0"),
        ("glm-4.6", "GLM-4.6", "0.6", "2.2", "0.11", "0"),
        ("glm-5", "GLM-5", "1", "3.2", "0.2", "0"),
        // GLM-5.2：在线价格同步成功后会以实时目录覆盖，内置值用于离线回退
        ("glm-5.2", "GLM-5.2", "0.63", "1.98", "0.0945", "0"),
        // Qwen
        (
            "qwen3-coder-plus",
            "Qwen3 Coder Plus",
            "0.65",
            "3.25",
            "0.13",
            "0",
        ),
        (
            "qwen3-coder-flash",
            "Qwen3 Coder Flash",
            "0.195",
            "0.975",
            "0.039",
            "0",
        ),
        // Kimi
        (
            "kimi-k2-thinking",
            "Kimi K2 Thinking",
            "0.55",
            "2.20",
            "0.10",
            "0",
        ),
        ("kimi-k2-0905", "Kimi K2", "0.55", "2.20", "0.10", "0"),
        // Grok
        ("grok-4.5", "Grok 4.5", "2", "6", "0.50", "0"),
        ("grok-4.5-build", "Grok 4.5 Build", "2", "6", "0.30", "0"),
        ("grok-4", "Grok 4", "3", "15", "0.75", "0"),
    ];

    let mut stmt = conn.prepare(
        "INSERT OR IGNORE INTO model_pricing (
            model_id, display_name, input_cost_per_million, output_cost_per_million,
            cache_read_cost_per_million, cache_creation_cost_per_million
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;

    for (model_id, display_name, input, output, cache_read, cache_creation) in pricing_data {
        stmt.execute(rusqlite::params![
            model_id,
            display_name,
            input,
            output,
            cache_read,
            cache_creation
        ])?;
    }

    log::info!("已初始化 {} 条模型定价数据", pricing_data.len());
    Ok(())
}
