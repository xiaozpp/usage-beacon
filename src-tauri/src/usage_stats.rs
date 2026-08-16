//! 使用量聚合查询
//!
//! 移植自 cc-switch 的 services/usage_stats.rs，简化版：
//! - 只查询 proxy_request_logs 明细表（独立插件不写代理数据，无需冷热分层 UNION）
//! - 保留 fresh_input_sql 语义归一化（cache-inclusive app 的 input_tokens 含 cache）
//! - 保留 effective_usage_log_filter 跨源去重

use crate::database::Database;
use crate::error::{AppError, Result};
use crate::schema::{
    CACHE_INCLUSIVE_APP_TYPES, INPUT_TOKEN_SEMANTICS_FRESH, INPUT_TOKEN_SEMANTICS_LEGACY,
    INPUT_TOKEN_SEMANTICS_TOTAL, SESSION_PROXY_DEDUP_WINDOW_SECONDS,
};
use serde::{Deserialize, Serialize};

/// 查询参数
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageQuery {
    pub start_date: i64, // unix seconds
    pub end_date: i64,
    pub app_type: Option<String>,
    pub provider_name: Option<String>,
    pub model: Option<String>,
    pub device_id: Option<String>,
}

impl Default for UsageQuery {
    fn default() -> Self {
        // 默认最近 7 天
        let end = chrono::Utc::now().timestamp();
        let start = end - 7 * 24 * 3600;
        Self {
            start_date: start,
            end_date: end,
            app_type: None,
            provider_name: None,
            model: None,
            device_id: None,
        }
    }
}

/// 摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub total_requests: u32,
    pub total_cost: String,
    pub unpriced_requests: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub real_total_tokens: u64,
    pub success_rate: f32,
    pub cache_hit_rate: f64,
}

/// 各来源会话运行指标的统一聚合结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStats {
    pub source_count: u32,
    pub session_count: u32,
    pub turns: u32,
    pub steps: u32,
    pub llm_ms: u64,
    pub tool_ms: u64,
    pub ttft_ms: u64,
    pub ttft_steps: u32,
    pub decode_ms: u64,
    pub decode_tokens: u64,
    pub average_ttft_ms: Option<f64>,
    pub decode_tokens_per_second: Option<f64>,
}

/// 日/小时趋势
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyStats {
    pub date: String,
    pub request_count: u32,
    pub total_cost: String,
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
}

/// Provider 维度统计
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStats {
    pub provider_id: String,
    pub provider_name: String,
    pub request_count: u32,
    pub total_tokens: u64,
    pub total_cost: String,
    pub success_rate: f32,
    pub avg_latency_ms: Option<f64>,
}

/// 模型维度统计
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStats {
    pub model: String,
    pub request_count: u32,
    pub total_tokens: u64,
    pub total_cost: String,
    pub avg_cost_per_request: String,
}

/// 项目或会话维度统计
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBreakdownStats {
    pub key: String,
    pub request_count: u32,
    pub total_tokens: u64,
    pub total_cost: String,
    pub avg_cost_per_request: String,
    pub unpriced_requests: u32,
    pub source_name: String,
    pub app_type: String,
    pub latest_at: i64,
}

/// 请求日志筛选
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LogFilters {
    pub app_type: Option<String>,
    pub provider_name: Option<String>,
    pub model: Option<String>,
    pub device_id: Option<String>,
    pub status_code: Option<i32>,
    pub start_date: Option<i64>,
    pub end_date: Option<i64>,
}

/// 分页结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedLogs {
    pub data: Vec<RequestLogDetail>,
    pub total: u32,
    pub page: u32,
    pub page_size: u32,
}

/// 请求详情
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogDetail {
    pub request_id: String,
    pub provider_id: String,
    pub provider_name: String,
    pub app_type: String,
    pub model: String,
    pub request_model: Option<String>,
    pub pricing_model: Option<String>,
    pub input_tokens: u64,
    pub fresh_input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total_cost_usd: String,
    pub input_cost_usd: String,
    pub output_cost_usd: String,
    pub cache_read_cost_usd: String,
    pub cache_creation_cost_usd: String,
    pub latency_ms: i64,
    pub status_code: i32,
    pub error_message: Option<String>,
    pub session_id: Option<String>,
    pub is_streaming: bool,
    pub cost_multiplier: String,
    pub created_at: i64,
    pub data_source: String,
    pub project: String,
}

/// 模型定价信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPricingInfo {
    pub model_id: String,
    pub display_name: String,
    pub input_cost_per_million: String,
    pub output_cost_per_million: String,
    pub cache_read_cost_per_million: String,
    pub cache_creation_cost_per_million: String,
    pub price_source: String,
    pub price_fetched_at: Option<i64>,
}

/// 摘要查询
pub fn get_usage_summary(db: &Database, q: &UsageQuery) -> Result<UsageSummary> {
    db.with_conn(|conn| {
        let mut sql = format!(
            "SELECT
                SUM(COALESCE(l.request_count, 1)) as cnt,
                COALESCE(SUM(CAST(l.total_cost_usd AS REAL)), 0) as total_cost,
                SUM({fresh_input}) as input_tokens,
                SUM(l.output_tokens) as output_tokens,
                SUM(l.cache_read_tokens) as cache_read_tokens,
                SUM(l.cache_creation_tokens) as cache_creation_tokens,
                SUM(CASE WHEN l.status_code >= 200 AND l.status_code < 300
                    THEN COALESCE(l.request_count, 1) ELSE 0 END) * 100.0
                    / NULLIF(SUM(COALESCE(l.request_count, 1)), 0) as success_rate,
                SUM(l.cache_read_tokens) * 100.0
                    / NULLIF(SUM({fresh_input} + l.cache_creation_tokens + l.cache_read_tokens), 0)
                    as cache_hit_rate,
                SUM(CASE
                    WHEN CAST(l.total_cost_usd AS REAL) = 0 AND ({real_total}) > 0
                        THEN COALESCE(l.request_count, 1)
                    ELSE 0
                END) as unpriced_requests
             FROM proxy_request_logs l
             WHERE {filter} AND l.created_at BETWEEN ?1 AND ?2",
            fresh_input = fresh_input_sql("l"),
            real_total = real_total_tokens_sql("l"),
            filter = effective_usage_log_filter("l")
        );

        let mut params: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(q.start_date), Box::new(q.end_date)];
        push_filters(&mut sql, &mut params, "l", q)?;

        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(
            params.iter().map(|b| b.as_ref()),
        ))?;

        let row = rows
            .next()?
            .ok_or_else(|| AppError::Database("无数据".into()))?;

        let input_tokens: Option<i64> = row.get(2)?;
        let output_tokens: Option<i64> = row.get(3)?;
        let cache_read: Option<i64> = row.get(4)?;
        let cache_creation: Option<i64> = row.get(5)?;

        // real_total_tokens = fresh_input + output + cache_read + cache_creation
        let real_total = input_tokens.unwrap_or(0) as u64
            + output_tokens.unwrap_or(0) as u64
            + cache_read.unwrap_or(0) as u64
            + cache_creation.unwrap_or(0) as u64;

        Ok(UsageSummary {
            total_requests: row.get::<_, i64>(0)? as u32,
            total_cost: format!("{:.6}", row.get::<_, f64>(1)?),
            unpriced_requests: row.get::<_, Option<i64>>(8)?.unwrap_or(0) as u32,
            input_tokens: input_tokens.unwrap_or(0) as u64,
            output_tokens: output_tokens.unwrap_or(0) as u64,
            cache_read_tokens: cache_read.unwrap_or(0) as u64,
            cache_creation_tokens: cache_creation.unwrap_or(0) as u64,
            real_total_tokens: real_total,
            success_rate: row.get::<_, Option<f32>>(6)?.unwrap_or(0.0),
            cache_hit_rate: row.get::<_, Option<f64>>(7)?.unwrap_or(0.0),
        })
    })
}

/// 获取当前筛选范围内可用的会话级运行指标。
///
/// 这些数字不从 proxy_request_logs 的 usage 行反推，而是直接读取会话级
/// 折叠结果，避免把一个会话的耗时重复写入每条请求。
pub fn get_runtime_stats(db: &Database, q: &UsageQuery) -> Result<RuntimeStats> {
    db.with_conn(|conn| {
        let mut sql = String::from(
            "SELECT
                COUNT(DISTINCT r.app_type) AS source_count,
                COUNT(*) AS session_count,
                COALESCE(SUM(r.turns), 0) AS turns,
                COALESCE(SUM(r.steps), 0) AS steps,
                COALESCE(SUM(r.llm_ms), 0) AS llm_ms,
                COALESCE(SUM(r.tool_ms), 0) AS tool_ms,
                COALESCE(SUM(r.ttft_ms), 0) AS ttft_ms,
                COALESCE(SUM(r.ttft_steps), 0) AS ttft_steps,
                COALESCE(SUM(r.decode_ms), 0) AS decode_ms,
                COALESCE(SUM(r.decode_tokens), 0) AS decode_tokens
             FROM session_runtime_stats r
             WHERE r.last_event_at >= ?1
               AND r.started_at <= ?2",
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(q.start_date), Box::new(q.end_date)];
        push_runtime_filters(&mut sql, &mut params, "r", q)?;

        let mut stmt = conn.prepare(&sql)?;
        let row = stmt.query_row(
            rusqlite::params_from_iter(params.iter().map(|value| value.as_ref())),
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            },
        )?;

        let source_count = non_negative_u32(row.0);
        let session_count = non_negative_u32(row.1);
        let turns = non_negative_u32(row.2);
        let steps = non_negative_u32(row.3);
        let llm_ms = non_negative_u64(row.4);
        let tool_ms = non_negative_u64(row.5);
        let ttft_ms = non_negative_u64(row.6);
        let ttft_steps = non_negative_u32(row.7);
        let decode_ms = non_negative_u64(row.8);
        let decode_tokens = non_negative_u64(row.9);

        Ok(RuntimeStats {
            source_count,
            session_count,
            turns,
            steps,
            llm_ms,
            tool_ms,
            ttft_ms,
            ttft_steps,
            decode_ms,
            decode_tokens,
            average_ttft_ms: (ttft_steps > 0).then(|| ttft_ms as f64 / ttft_steps as f64),
            decode_tokens_per_second: (decode_ms > 0)
                .then(|| decode_tokens as f64 * 1_000.0 / decode_ms as f64),
        })
    })
}

/// 日/小时趋势查询：短时间范围按小时聚合，较长范围按天聚合。
pub fn get_daily_trends(db: &Database, q: &UsageQuery) -> Result<Vec<DailyStats>> {
    db.with_conn(|conn| {
        let bucket_expression = if q.end_date.saturating_sub(q.start_date) <= 2_i64 * 24 * 3600 {
            "strftime('%Y-%m-%d %H:00:00', l.created_at, 'unixepoch', 'localtime')"
        } else {
            "date(l.created_at, 'unixepoch', 'localtime')"
        };
        let mut sql = format!(
            "SELECT
                {bucket_expression} as date,
                SUM(COALESCE(l.request_count, 1)) as cnt,
                COALESCE(SUM(CAST(l.total_cost_usd AS REAL)), 0) as cost,
                SUM({real_total}) as total_tokens,
                SUM({fresh_input}) as input_tokens,
                SUM(l.output_tokens) as output_tokens,
                SUM(l.cache_read_tokens) as cache_read_tokens,
                SUM(l.cache_creation_tokens) as cache_creation_tokens
             FROM proxy_request_logs l
             WHERE {filter} AND l.created_at BETWEEN ?1 AND ?2",
            bucket_expression = bucket_expression,
            real_total = real_total_tokens_sql("l"),
            fresh_input = fresh_input_sql("l"),
            filter = effective_usage_log_filter("l")
        );

        let mut params: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(q.start_date), Box::new(q.end_date)];
        push_filters(&mut sql, &mut params, "l", q)?;

        sql.push_str(" GROUP BY date ORDER BY date ASC");

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(params.iter().map(|b| b.as_ref())),
            |row| {
                Ok(DailyStats {
                    date: row.get(0)?,
                    request_count: row.get::<_, i64>(1)? as u32,
                    total_cost: format!("{:.6}", row.get::<_, f64>(2)?),
                    total_tokens: row.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                    input_tokens: row.get::<_, Option<i64>>(4)?.unwrap_or(0) as u64,
                    output_tokens: row.get::<_, Option<i64>>(5)?.unwrap_or(0) as u64,
                    cache_read_tokens: row.get::<_, Option<i64>>(6)?.unwrap_or(0) as u64,
                    cache_creation_tokens: row.get::<_, Option<i64>>(7)?.unwrap_or(0) as u64,
                })
            },
        )?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(row?);
        }
        Ok(stats)
    })
}

/// Provider 维度统计
pub fn get_provider_stats(db: &Database, q: &UsageQuery) -> Result<Vec<ProviderStats>> {
    db.with_conn(|conn| {
        let mut sql = format!(
            "SELECT
                l.provider_id,
                {provider_name} as provider_name,
                SUM(COALESCE(l.request_count, 1)) as cnt,
                SUM({real_total}) as total_tokens,
                COALESCE(SUM(CAST(l.total_cost_usd AS REAL)), 0) as total_cost,
                SUM(CASE WHEN l.status_code >= 200 AND l.status_code < 300
                    THEN COALESCE(l.request_count, 1) ELSE 0 END) * 100.0
                    / NULLIF(SUM(COALESCE(l.request_count, 1)), 0) as success_rate,
                AVG(CASE WHEN COALESCE(l.data_source, 'proxy') = 'proxy' THEN l.latency_ms END) as avg_latency
             FROM proxy_request_logs l
             WHERE {filter} AND l.created_at BETWEEN ?1 AND ?2",
            provider_name = provider_name_coalesce("l"),
            real_total = real_total_tokens_sql("l"),
            filter = effective_usage_log_filter("l")
        );

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(q.start_date),
            Box::new(q.end_date),
        ];
        push_filters(&mut sql, &mut params, "l", q)?;

        sql.push_str(" GROUP BY l.provider_id ORDER BY total_cost DESC");

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter().map(|b| b.as_ref())), |row| {
            Ok(ProviderStats {
                provider_id: row.get(0)?,
                provider_name: row.get(1)?,
                request_count: row.get::<_, i64>(2)? as u32,
                total_tokens: row.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                total_cost: format!("{:.6}", row.get::<_, f64>(4)?),
                success_rate: row.get::<_, Option<f32>>(5)?.unwrap_or(0.0),
                avg_latency_ms: row.get(6)?,
            })
        })?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(row?);
        }
        Ok(stats)
    })
}

/// 模型维度统计
pub fn get_model_stats(db: &Database, q: &UsageQuery) -> Result<Vec<ModelStats>> {
    db.with_conn(|conn| {
        let mut sql = format!(
            "SELECT
                COALESCE(NULLIF(l.pricing_model, ''), l.model) as model,
                SUM(COALESCE(l.request_count, 1)) as cnt,
                SUM({real_total}) as total_tokens,
                COALESCE(SUM(CAST(l.total_cost_usd AS REAL)), 0) as total_cost
             FROM proxy_request_logs l
             WHERE {filter} AND l.created_at BETWEEN ?1 AND ?2",
            real_total = real_total_tokens_sql("l"),
            filter = effective_usage_log_filter("l")
        );

        let mut params: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(q.start_date), Box::new(q.end_date)];
        push_filters(&mut sql, &mut params, "l", q)?;

        sql.push_str(" GROUP BY model ORDER BY total_cost DESC");

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(params.iter().map(|b| b.as_ref())),
            |row| {
                let count: i64 = row.get(1)?;
                let total_cost: f64 = row.get(3)?;
                let avg_cost = if count > 0 {
                    total_cost / count as f64
                } else {
                    0.0
                };
                Ok(ModelStats {
                    model: row.get(0)?,
                    request_count: count as u32,
                    total_tokens: row.get::<_, Option<i64>>(2)?.unwrap_or(0) as u64,
                    total_cost: format!("{:.6}", total_cost),
                    avg_cost_per_request: format!("{:.6}", avg_cost),
                })
            },
        )?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(row?);
        }
        Ok(stats)
    })
}

/// 项目维度统计。只对有项目标识的记录展示，避免把未解析出的空值混成一个项目。
pub fn get_project_stats(db: &Database, q: &UsageQuery) -> Result<Vec<UsageBreakdownStats>> {
    get_breakdown_stats(db, q, "l.project", None)
}

/// 会话维度统计。用于定位具体会话的 token 与成本消耗。
pub fn get_session_stats(db: &Database, q: &UsageQuery) -> Result<Vec<UsageBreakdownStats>> {
    get_breakdown_stats(db, q, "l.session_id", Some(12))
}

fn get_breakdown_stats(
    db: &Database,
    q: &UsageQuery,
    dimension: &str,
    limit: Option<u32>,
) -> Result<Vec<UsageBreakdownStats>> {
    db.with_conn(|conn| {
        let mut sql = format!(
            "SELECT
                TRIM(COALESCE({dimension}, '')) as dimension_key,
                {provider_name} as source_name,
                SUM(COALESCE(l.request_count, 1)) as cnt,
                SUM({real_total}) as total_tokens,
                COALESCE(SUM(CAST(l.total_cost_usd AS REAL)), 0) as total_cost,
                SUM(CASE
                    WHEN CAST(l.total_cost_usd AS REAL) = 0 AND ({real_total}) > 0
                        THEN COALESCE(l.request_count, 1)
                    ELSE 0
                END) as unpriced_requests,
                MAX(l.app_type) as app_type,
                MAX(l.created_at) as latest_at
             FROM proxy_request_logs l
             WHERE {filter}
               AND l.created_at BETWEEN ?1 AND ?2
               AND TRIM(COALESCE({dimension}, '')) <> ''",
            real_total = real_total_tokens_sql("l"),
            provider_name = provider_name_coalesce("l"),
            filter = effective_usage_log_filter("l")
        );

        let mut params: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(q.start_date), Box::new(q.end_date)];
        push_filters(&mut sql, &mut params, "l", q)?;
        sql.push_str(" GROUP BY dimension_key ORDER BY total_cost DESC, cnt DESC");
        if let Some(limit) = limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(params.iter().map(|b| b.as_ref())),
            |row| {
                let count: i64 = row.get(2)?;
                let total_cost: f64 = row.get(4)?;
                Ok(UsageBreakdownStats {
                    key: row.get(0)?,
                    source_name: row.get(1)?,
                    request_count: count as u32,
                    total_tokens: row.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                    total_cost: format!("{total_cost:.6}"),
                    avg_cost_per_request: format!(
                        "{:.6}",
                        if count > 0 {
                            total_cost / count as f64
                        } else {
                            0.0
                        }
                    ),
                    unpriced_requests: row.get::<_, Option<i64>>(5)?.unwrap_or(0) as u32,
                    app_type: row.get(6)?,
                    latest_at: row.get(7)?,
                })
            },
        )?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(row?);
        }
        Ok(stats)
    })
}

/// 请求日志分页查询
pub fn get_request_logs(
    db: &Database,
    filters: &LogFilters,
    page: u32,
    page_size: u32,
) -> Result<PaginatedLogs> {
    db.with_conn(|conn| {
        let page = if page == 0 { 1 } else { page };
        let page_size = if page_size == 0 {
            10
        } else {
            page_size.min(100)
        };
        let offset = (page - 1) * page_size;

        let mut where_clauses = vec![effective_usage_log_filter("l").to_string()];
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref app_type) = filters.app_type {
            if app_type == "claude" {
                where_clauses.push("(l.app_type = ? OR l.app_type = 'claude-desktop')".to_string());
                params.push(Box::new(app_type.clone()));
            } else {
                where_clauses.push("l.app_type = ?".to_string());
                params.push(Box::new(app_type.clone()));
            }
        }
        if let Some(ref provider_name) = filters.provider_name {
            where_clauses.push(format!("{} = ?", provider_name_coalesce("l")));
            params.push(Box::new(provider_name.clone()));
        }
        if let Some(ref model) = filters.model {
            where_clauses.push("COALESCE(NULLIF(l.pricing_model, ''), l.model) = ?".to_string());
            params.push(Box::new(model.clone()));
        }
        if let Some(ref device_id) = filters.device_id {
            where_clauses.push("l.device_id = ?".to_string());
            params.push(Box::new(device_id.clone()));
        }
        if let Some(status) = filters.status_code {
            where_clauses.push("l.status_code = ?".to_string());
            params.push(Box::new(status));
        }
        if let Some(start) = filters.start_date {
            where_clauses.push("l.created_at >= ?".to_string());
            params.push(Box::new(start));
        }
        if let Some(end) = filters.end_date {
            where_clauses.push("l.created_at <= ?".to_string());
            params.push(Box::new(end));
        }

        let where_sql = where_clauses.join(" AND ");

        // 总数
        let count_sql = format!("SELECT COUNT(*) FROM proxy_request_logs l WHERE {where_sql}");
        let total: i64 = conn.query_row(
            &count_sql,
            rusqlite::params_from_iter(params.iter().map(|b| b.as_ref())),
            |row| row.get(0),
        )?;

        // 数据
        let data_sql = format!(
            "SELECT
                l.request_id, l.provider_id, {provider_name} as provider_name,
                l.app_type, l.model, l.request_model, l.pricing_model,
                l.input_tokens, {fresh_input} as fresh_input_tokens,
                l.output_tokens, l.cache_read_tokens, l.cache_creation_tokens,
                l.total_cost_usd, l.input_cost_usd, l.output_cost_usd,
                l.cache_read_cost_usd, l.cache_creation_cost_usd,
                l.latency_ms, l.status_code, l.error_message, l.session_id,
                l.is_streaming, l.cost_multiplier, l.created_at, l.data_source, l.project
             FROM proxy_request_logs l
             WHERE {where_sql}
             ORDER BY l.created_at DESC
             LIMIT ? OFFSET ?",
            provider_name = provider_name_coalesce("l"),
            fresh_input = fresh_input_sql("l")
        );

        params.push(Box::new(page_size as i64));
        params.push(Box::new(offset as i64));

        let mut stmt = conn.prepare(&data_sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(params.iter().map(|b| b.as_ref())),
            |row| {
                Ok(RequestLogDetail {
                    request_id: row.get(0)?,
                    provider_id: row.get(1)?,
                    provider_name: row.get(2)?,
                    app_type: row.get(3)?,
                    model: row.get(4)?,
                    request_model: row.get(5)?,
                    pricing_model: row.get(6)?,
                    input_tokens: row.get::<_, i64>(7)? as u64,
                    fresh_input_tokens: row.get::<_, i64>(8)? as u64,
                    output_tokens: row.get::<_, i64>(9)? as u64,
                    cache_read_tokens: row.get::<_, i64>(10)? as u64,
                    cache_creation_tokens: row.get::<_, i64>(11)? as u64,
                    total_cost_usd: row.get(12)?,
                    input_cost_usd: row.get(13)?,
                    output_cost_usd: row.get(14)?,
                    cache_read_cost_usd: row.get(15)?,
                    cache_creation_cost_usd: row.get(16)?,
                    latency_ms: row.get(17)?,
                    status_code: row.get(18)?,
                    error_message: row.get(19)?,
                    session_id: row.get(20)?,
                    is_streaming: row.get::<_, i64>(21)? != 0,
                    cost_multiplier: row.get(22)?,
                    created_at: row.get(23)?,
                    data_source: row.get(24)?,
                    project: row.get(25)?,
                })
            },
        )?;

        let mut data = Vec::new();
        for row in rows {
            data.push(row?);
        }

        Ok(PaginatedLogs {
            data,
            total: total as u32,
            page,
            page_size,
        })
    })
}

/// 请求详情
pub fn get_request_detail(db: &Database, request_id: &str) -> Result<Option<RequestLogDetail>> {
    db.with_conn(|conn| {
        let sql = format!(
            "SELECT
                l.request_id, l.provider_id, {provider_name} as provider_name,
                l.app_type, l.model, l.request_model, l.pricing_model,
                l.input_tokens, {fresh_input} as fresh_input_tokens,
                l.output_tokens, l.cache_read_tokens, l.cache_creation_tokens,
                l.total_cost_usd, l.input_cost_usd, l.output_cost_usd,
                l.cache_read_cost_usd, l.cache_creation_cost_usd,
                l.latency_ms, l.status_code, l.error_message, l.session_id,
                l.is_streaming, l.cost_multiplier, l.created_at, l.data_source, l.project
             FROM proxy_request_logs l
             WHERE l.request_id = ?1",
            provider_name = provider_name_coalesce("l"),
            fresh_input = fresh_input_sql("l")
        );

        let result = conn.query_row(&sql, rusqlite::params![request_id], |row| {
            Ok(RequestLogDetail {
                request_id: row.get(0)?,
                provider_id: row.get(1)?,
                provider_name: row.get(2)?,
                app_type: row.get(3)?,
                model: row.get(4)?,
                request_model: row.get(5)?,
                pricing_model: row.get(6)?,
                input_tokens: row.get::<_, i64>(7)? as u64,
                fresh_input_tokens: row.get::<_, i64>(8)? as u64,
                output_tokens: row.get::<_, i64>(9)? as u64,
                cache_read_tokens: row.get::<_, i64>(10)? as u64,
                cache_creation_tokens: row.get::<_, i64>(11)? as u64,
                total_cost_usd: row.get(12)?,
                input_cost_usd: row.get(13)?,
                output_cost_usd: row.get(14)?,
                cache_read_cost_usd: row.get(15)?,
                cache_creation_cost_usd: row.get(16)?,
                latency_ms: row.get(17)?,
                status_code: row.get(18)?,
                error_message: row.get(19)?,
                session_id: row.get(20)?,
                is_streaming: row.get::<_, i64>(21)? != 0,
                cost_multiplier: row.get(22)?,
                created_at: row.get(23)?,
                data_source: row.get(24)?,
                project: row.get(25)?,
            })
        });

        match result {
            Ok(detail) => Ok(Some(detail)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    })
}

/// 获取全部模型定价
pub fn get_model_pricing_list(db: &Database) -> Result<Vec<ModelPricingInfo>> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT model_id, display_name, input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_creation_cost_per_million,
                    price_source, price_fetched_at
             FROM model_pricing
             ORDER BY model_id ASC",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(ModelPricingInfo {
                model_id: row.get(0)?,
                display_name: row.get(1)?,
                input_cost_per_million: row.get(2)?,
                output_cost_per_million: row.get(3)?,
                cache_read_cost_per_million: row.get(4)?,
                cache_creation_cost_per_million: row.get(5)?,
                price_source: row.get(6)?,
                price_fetched_at: row.get(7)?,
            })
        })?;

        let mut list = Vec::new();
        for row in rows {
            list.push(row?);
        }
        Ok(list)
    })
}

// ============== SQL 辅助函数 ==============

/// 跨源去重 SQL 片段：排除 10 分钟窗口内已有等价 proxy 行的 session 行
fn effective_usage_log_filter(alias: &str) -> String {
    let data_source = format!("COALESCE({alias}.data_source, 'proxy')");
    let proxy_data_source = "COALESCE(p.data_source, 'proxy')";
    let app_type_match = format!(
        "p.app_type IN ({alias}.app_type, CASE WHEN {alias}.app_type = 'claude' THEN 'claude-desktop' ELSE {alias}.app_type END)"
    );
    format!(
        "NOT (
            {data_source} IN ('session_log', 'codex_session', 'gemini_session', 'opencode_session', 'zcode_session', 'grok_session', 'deepseek_harness_session', 'hermes_session')
            AND EXISTS (
                SELECT 1 FROM proxy_request_logs p
                WHERE {proxy_data_source} = 'proxy'
                  AND {app_type_match}
                  AND p.status_code >= 200 AND p.status_code < 300
                  AND p.input_tokens = {alias}.input_tokens
                  AND p.output_tokens = {alias}.output_tokens
                  AND p.cache_read_tokens = {alias}.cache_read_tokens
                  AND (
                      p.cache_creation_tokens = {alias}.cache_creation_tokens
                      OR (
                          {alias}.cache_creation_tokens = 0
                          AND {data_source} IN ('codex_session', 'gemini_session', 'opencode_session', 'zcode_session', 'grok_session', 'deepseek_harness_session', 'hermes_session')
                      )
                  )
                  AND p.created_at BETWEEN {alias}.created_at - {window} AND {alias}.created_at + {window}
                  AND (
                      LOWER(p.model) = LOWER({alias}.model)
                      OR LOWER(p.model) = 'unknown'
                      OR LOWER({alias}.model) = 'unknown'
                  )
            )
        )",
        window = SESSION_PROXY_DEDUP_WINDOW_SECONDS
    )
}

/// fresh input SQL：cache-inclusive app 的 input_tokens 含 cache，需扣减
fn fresh_input_sql(alias: &str) -> String {
    let app_types = CACHE_INCLUSIVE_APP_TYPES
        .iter()
        .map(|app| format!("'{app}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "CASE
            WHEN {a}.input_token_semantics = {fresh}
                THEN {a}.input_tokens
            WHEN {a}.app_type IN ({apps})
                AND {a}.input_token_semantics = {total}
                AND {a}.input_tokens >= ({a}.cache_read_tokens + {a}.cache_creation_tokens)
                THEN {a}.input_tokens - {a}.cache_read_tokens - {a}.cache_creation_tokens
            WHEN {a}.app_type IN ({apps})
                AND {a}.input_token_semantics = {legacy}
                AND {a}.input_tokens >= {a}.cache_read_tokens
                THEN {a}.input_tokens - {a}.cache_read_tokens
            ELSE {a}.input_tokens
        END",
        a = alias,
        apps = app_types,
        fresh = INPUT_TOKEN_SEMANTICS_FRESH,
        total = INPUT_TOKEN_SEMANTICS_TOTAL,
        legacy = INPUT_TOKEN_SEMANTICS_LEGACY,
    )
}

/// 真实 token 总量：新增输入 + 输出 + 缓存读 + 缓存写。
///
/// 这一定义与摘要卡片的 real_total_tokens 保持一致，供趋势、供应商、模型
/// 以及请求详情共享，避免不同界面展示不同的“Tokens”口径。
fn real_total_tokens_sql(alias: &str) -> String {
    format!(
        "({fresh_input} + {a}.output_tokens + {a}.cache_read_tokens + {a}.cache_creation_tokens)",
        fresh_input = fresh_input_sql(alias),
        a = alias,
    )
}

/// provider_id 占位符到可读名的映射
fn provider_name_coalesce(alias: &str) -> String {
    format!(
        "CASE {a}.provider_id
            WHEN '_session' THEN 'Claude (Session)'
            WHEN '_codex_session' THEN 'Codex (Session)'
            WHEN '_gemini_session' THEN 'Gemini (Session)'
            WHEN '_opencode_session' THEN 'OpenCode (Session)'
            WHEN '_zcode_session' THEN 'ZCode (Session)'
            WHEN '_grok_session' THEN 'Grok Build (Session)'
            WHEN '_deepseek_harness_session' THEN 'DeepSeek Harness (Session)'
            WHEN '_hermes_session' THEN 'Hermes (Session)'
            ELSE {a}.provider_id
        END",
        a = alias
    )
}

fn non_negative_u32(value: i64) -> u32 {
    value.max(0).min(u32::MAX as i64) as u32
}

fn non_negative_u64(value: i64) -> u64 {
    value.max(0) as u64
}

fn push_runtime_filters(
    sql: &mut String,
    params: &mut Vec<Box<dyn rusqlite::ToSql>>,
    alias: &str,
    q: &UsageQuery,
) -> Result<()> {
    if let Some(ref app_type) = q.app_type {
        sql.push_str(&format!(" AND {alias}.app_type = ?", alias = alias));
        params.push(Box::new(app_type.clone()));
    }
    if let Some(ref provider_name) = q.provider_name {
        sql.push_str(&format!(
            " AND {provider} = ?",
            provider = provider_name_coalesce(alias)
        ));
        params.push(Box::new(provider_name.clone()));
    }
    if let Some(ref model) = q.model {
        sql.push_str(&format!(" AND {alias}.model = ?", alias = alias));
        params.push(Box::new(model.clone()));
    }
    if let Some(ref device_id) = q.device_id {
        sql.push_str(&format!(" AND {alias}.device_id = ?", alias = alias));
        params.push(Box::new(device_id.clone()));
    }
    Ok(())
}

/// 追加过滤条件
fn push_filters(
    sql: &mut String,
    params: &mut Vec<Box<dyn rusqlite::ToSql>>,
    alias: &str,
    q: &UsageQuery,
) -> Result<()> {
    if let Some(ref app_type) = q.app_type {
        // claude-desktop 折叠到 claude
        if app_type == "claude" {
            sql.push_str(&format!(
                " AND ({a}.app_type = ? OR {a}.app_type = 'claude-desktop')",
                a = alias
            ));
            params.push(Box::new(app_type.clone()));
        } else {
            sql.push_str(&format!(" AND {a}.app_type = ?", a = alias));
            params.push(Box::new(app_type.clone()));
        }
    }
    if let Some(ref provider_name) = q.provider_name {
        sql.push_str(&format!(
            " AND {provider} = ?",
            provider = provider_name_coalesce(alias)
        ));
        params.push(Box::new(provider_name.clone()));
    }
    if let Some(ref model) = q.model {
        sql.push_str(&format!(
            " AND COALESCE(NULLIF({a}.pricing_model, ''), {a}.model) = ?",
            a = alias,
        ));
        params.push(Box::new(model.clone()));
    }
    if let Some(ref device_id) = q.device_id {
        sql.push_str(&format!(" AND {alias}.device_id = ?", alias = alias));
        params.push(Box::new(device_id.clone()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn test_fresh_input_sql() {
        let sql = fresh_input_sql("l");
        assert!(sql.contains("codex"));
        assert!(sql.contains("input_token_semantics"));
    }

    #[test]
    fn test_provider_name_coalesce() {
        let sql = provider_name_coalesce("l");
        assert!(sql.contains("_session"));
    }

    #[test]
    fn test_fresh_input_matches_cc_semantics() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE proxy_request_logs (
                app_type TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_read_tokens INTEGER,
                cache_creation_tokens INTEGER,
                input_token_semantics INTEGER
            )",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO proxy_request_logs VALUES ('codex', 1000, 1, 600, 100, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO proxy_request_logs VALUES ('codex', 1000, 1, 600, 100, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO proxy_request_logs VALUES ('claude', 200, 1, 5000, 0, 0)",
            [],
        )
        .unwrap();

        let expression = fresh_input_sql("l");
        let sql = format!("SELECT SUM({expression}) FROM proxy_request_logs l");
        let fresh: i64 = conn.query_row(&sql, [], |row| row.get(0)).unwrap();
        // TOTAL: 1000 - 600 - 100 = 300; LEGACY: 1000 - 600 = 400;
        // Claude remains 200.
        assert_eq!(fresh, 900);
    }

    #[test]
    fn test_cache_hit_rate_is_token_based() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE proxy_request_logs (
                app_type TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_read_tokens INTEGER,
                cache_creation_tokens INTEGER,
                input_token_semantics INTEGER
            )",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO proxy_request_logs VALUES ('codex', 1000, 0, 900, 0, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO proxy_request_logs VALUES ('claude', 100, 0, 0, 0, 0)",
            [],
        )
        .unwrap();
        let expression = fresh_input_sql("l");
        let sql = format!(
            "SELECT SUM(l.cache_read_tokens) * 100.0 /
                    NULLIF(SUM({expression} + l.cache_creation_tokens + l.cache_read_tokens), 0)
             FROM proxy_request_logs l"
        );
        let hit_rate: f64 = conn.query_row(&sql, [], |row| row.get(0)).unwrap();
        assert!((hit_rate - (900.0 / 1_100.0 * 100.0)).abs() < 1e-9);
    }

    #[test]
    fn summary_and_breakdowns_use_the_same_real_token_columns() {
        let conn = Connection::open_in_memory().unwrap();
        crate::schema::create_tables(&conn).unwrap();
        conn.execute(
            "INSERT INTO proxy_request_logs (
                request_id, provider_id, app_type, model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                input_token_semantics, latency_ms, status_code, created_at, data_source,
                session_id, project
             ) VALUES ('test-1', '_codex_session', 'codex', 'gpt-test',
                       1000, 50, 600, 100, 1, 0, 200, 1700000000,
                       'codex_session', 'session-1', 'project-a')",
            [],
        )
        .unwrap();
        let db = Database {
            conn: std::sync::Mutex::new(conn),
        };
        let query = UsageQuery {
            start_date: 1699999999,
            end_date: 1700000001,
            app_type: None,
            provider_name: None,
            model: None,
            device_id: None,
        };

        let summary = get_usage_summary(&db, &query).unwrap();
        assert_eq!(summary.input_tokens, 300);
        assert_eq!(summary.output_tokens, 50);
        assert_eq!(summary.real_total_tokens, 1050);
        assert_eq!(summary.unpriced_requests, 1);
        assert!((summary.cache_hit_rate - 60.0).abs() < 1e-9);

        let models = get_model_stats(&db, &query).unwrap();
        assert_eq!(models[0].total_tokens, 1050);

        let providers = get_provider_stats(&db, &query).unwrap();
        assert_eq!(providers[0].total_tokens, 1050);
        assert_eq!(providers[0].avg_latency_ms, None);

        let projects = get_project_stats(&db, &query).unwrap();
        assert_eq!(projects[0].key, "project-a");
        assert_eq!(projects[0].total_tokens, 1050);
        assert_eq!(projects[0].unpriced_requests, 1);

        let sessions = get_session_stats(&db, &query).unwrap();
        assert_eq!(sessions[0].key, "session-1");

        let trends = get_daily_trends(&db, &query).unwrap();
        assert_eq!(trends[0].total_tokens, 1050);

        let logs = get_request_logs(
            &db,
            &LogFilters {
                start_date: Some(1699999999),
                end_date: Some(1700000001),
                ..LogFilters::default()
            },
            1,
            0,
        )
        .unwrap();
        assert_eq!(logs.page_size, 10);
        assert_eq!(logs.data[0].input_tokens, 1000);
        assert_eq!(logs.data[0].fresh_input_tokens, 300);
        assert_eq!(logs.data[0].project, "project-a");
    }

    #[test]
    fn runtime_stats_include_sessions_overlapping_the_time_range() {
        let conn = Connection::open_in_memory().unwrap();
        crate::schema::create_tables(&conn).unwrap();
        conn.execute(
            "INSERT INTO session_runtime_stats (
                data_source, app_type, provider_id, session_id, model,
                turns, steps, llm_ms, tool_ms, ttft_ms, ttft_steps,
                decode_ms, decode_tokens, started_at, last_event_at, device_id
             ) VALUES (?1, 'deepseek_harness', '_deepseek_harness_session',
                       'runtime-overlap', 'deepseek-chat',
                       2, 3, 1200, 300, 180, 2,
                       900, 90, 900, 1500, 'device-a')",
            [crate::schema::DATA_SOURCE_DEEPSEEK_HARNESS_SESSION],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_runtime_stats (
                data_source, app_type, provider_id, session_id, model,
                turns, steps, llm_ms, tool_ms, ttft_ms, ttft_steps,
                decode_ms, decode_tokens, started_at, last_event_at, device_id
             ) VALUES ('codex_session', 'codex', '_codex_session',
                       'runtime-other-source', 'gpt-test',
                       1, 1, 100, 0, 50, 1,
                       100, 20, 1100, 1600, 'device-a')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_runtime_stats (
                data_source, app_type, provider_id, session_id, model,
                turns, steps, llm_ms, tool_ms, ttft_ms, ttft_steps,
                decode_ms, decode_tokens, started_at, last_event_at, device_id
             ) VALUES (?1, 'deepseek_harness', '_deepseek_harness_session',
                       'runtime-outside', 'deepseek-chat',
                       1, 1, 100, 0, 50, 1,
                       100, 10, 2100, 2200, 'device-a')",
            [crate::schema::DATA_SOURCE_DEEPSEEK_HARNESS_SESSION],
        )
        .unwrap();
        let db = Database {
            conn: std::sync::Mutex::new(conn),
        };

        let stats = get_runtime_stats(
            &db,
            &UsageQuery {
                start_date: 1000,
                end_date: 2000,
                ..UsageQuery::default()
            },
        )
        .unwrap();

        assert_eq!(stats.source_count, 2);
        assert_eq!(stats.session_count, 2);
        assert_eq!(stats.turns, 3);
        assert_eq!(stats.steps, 4);
        assert_eq!(stats.llm_ms, 1300);
        assert_eq!(stats.tool_ms, 300);
        assert_eq!(stats.ttft_steps, 3);
        assert_eq!(stats.decode_tokens, 110);
        assert_eq!(stats.average_ttft_ms, Some(230.0 / 3.0));
        assert_eq!(stats.decode_tokens_per_second, Some(110.0));

        let deepseek_stats = get_runtime_stats(
            &db,
            &UsageQuery {
                start_date: 1000,
                end_date: 2000,
                app_type: Some("deepseek_harness".to_owned()),
                ..UsageQuery::default()
            },
        )
        .unwrap();
        assert_eq!(deepseek_stats.source_count, 1);
        assert_eq!(deepseek_stats.session_count, 1);
        assert_eq!(deepseek_stats.steps, 3);
    }

    #[test]
    fn trends_switch_between_hour_and_day_buckets() {
        let conn = Connection::open_in_memory().unwrap();
        crate::schema::create_tables(&conn).unwrap();
        conn.execute(
            "INSERT INTO proxy_request_logs (
                request_id, provider_id, app_type, model,
                input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                input_token_semantics, latency_ms, status_code, created_at, data_source
             ) VALUES ('trend-test-1', '_codex_session', 'codex', 'gpt-test',
                       1000, 50, 600, 100, 1, 0, 200, 1700000000, 'codex_session')",
            [],
        )
        .unwrap();
        let db = Database {
            conn: std::sync::Mutex::new(conn),
        };

        let hourly = get_daily_trends(
            &db,
            &UsageQuery {
                start_date: 1699999999,
                end_date: 1700000001,
                ..UsageQuery::default()
            },
        )
        .unwrap();
        assert_eq!(hourly.len(), 1);
        assert_eq!(hourly[0].date.len(), 19);
        assert!(hourly[0].date.ends_with(":00:00"));

        let daily = get_daily_trends(
            &db,
            &UsageQuery {
                start_date: 1699000000,
                end_date: 1701000000,
                ..UsageQuery::default()
            },
        )
        .unwrap();
        assert_eq!(daily.len(), 1);
        assert_eq!(daily[0].date.len(), 10);
    }
}
