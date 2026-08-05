//! Tauri 命令桥接层

use crate::database::Database;
use crate::error::AppError;
use crate::session_usage::{
    recost_zero_cost_logs, sync_all_session_logs, RecostResult, SessionSyncResult,
};
use crate::usage_stats::{self, LogFilters, ModelPricingInfo, PaginatedLogs, UsageQuery};
use crate::usage_stats::{DailyStats, ModelStats, ProviderStats, RequestLogDetail, UsageSummary};
use std::sync::Arc;
use tauri::State;

/// 同步所有支持的本地会话日志
#[tauri::command]
pub async fn sync_session_logs(db: State<'_, Arc<Database>>) -> Result<SessionSyncResult, String> {
    let db = db.inner().clone();
    let result = tokio::task::spawn_blocking(move || sync_all_session_logs(&db))
        .await
        .map_err(|e| AppError::Config(format!("同步任务失败: {e}")).to_string())?
        .map_err(|e| e.to_string())?;

    if result.imported > 0 {
        crate::usage_events::notify_log_recorded();
    }
    Ok(result)
}

/// 获取使用量摘要
#[tauri::command]
pub fn fetch_usage_summary(
    db: State<'_, Arc<Database>>,
    start_date: i64,
    end_date: i64,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
) -> Result<UsageSummary, String> {
    let q = UsageQuery {
        start_date,
        end_date,
        app_type,
        provider_name,
        model,
    };
    usage_stats::get_usage_summary(db.inner(), &q).map_err(|e| e.to_string())
}

/// 获取日趋势
#[tauri::command]
pub fn fetch_daily_trends(
    db: State<'_, Arc<Database>>,
    start_date: i64,
    end_date: i64,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
) -> Result<Vec<DailyStats>, String> {
    let q = UsageQuery {
        start_date,
        end_date,
        app_type,
        provider_name,
        model,
    };
    usage_stats::get_daily_trends(db.inner(), &q).map_err(|e| e.to_string())
}

/// 获取 Provider 统计
#[tauri::command]
pub fn fetch_provider_stats(
    db: State<'_, Arc<Database>>,
    start_date: i64,
    end_date: i64,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
) -> Result<Vec<ProviderStats>, String> {
    let q = UsageQuery {
        start_date,
        end_date,
        app_type,
        provider_name,
        model,
    };
    usage_stats::get_provider_stats(db.inner(), &q).map_err(|e| e.to_string())
}

/// 获取模型统计
#[tauri::command]
pub fn fetch_model_stats(
    db: State<'_, Arc<Database>>,
    start_date: i64,
    end_date: i64,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
) -> Result<Vec<ModelStats>, String> {
    let q = UsageQuery {
        start_date,
        end_date,
        app_type,
        provider_name,
        model,
    };
    usage_stats::get_model_stats(db.inner(), &q).map_err(|e| e.to_string())
}

/// 获取请求日志
#[tauri::command]
pub fn fetch_request_logs(
    db: State<'_, Arc<Database>>,
    filters: LogFilters,
    page: u32,
    page_size: u32,
) -> Result<PaginatedLogs, String> {
    usage_stats::get_request_logs(db.inner(), &filters, page, page_size).map_err(|e| e.to_string())
}

/// 获取请求详情
#[tauri::command]
pub fn fetch_request_detail(
    db: State<'_, Arc<Database>>,
    request_id: String,
) -> Result<Option<RequestLogDetail>, String> {
    usage_stats::get_request_detail(db.inner(), &request_id).map_err(|e| e.to_string())
}

/// 获取模型定价列表
#[tauri::command]
pub fn fetch_model_pricing(db: State<'_, Arc<Database>>) -> Result<Vec<ModelPricingInfo>, String> {
    usage_stats::get_model_pricing_list(db.inner()).map_err(|e| e.to_string())
}

/// 回填历史成本为 0 的 session 日志记录
#[tauri::command]
pub async fn recost_logs(db: State<'_, Arc<Database>>) -> Result<RecostResult, String> {
    let db = db.inner().clone();
    tokio::task::spawn_blocking(move || recost_zero_cost_logs(&db))
        .await
        .map_err(|e| AppError::Config(format!("回填任务失败: {e}")).to_string())?
        .map_err(|e| e.to_string())
}
