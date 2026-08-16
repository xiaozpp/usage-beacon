//! Tauri 命令桥接层

use crate::codex_radar::CodexRadarSnapshot;
use crate::database::Database;
use crate::device_transfer::{DeviceInfo, UsageExportPayload, UsageImportResult};
use crate::error::AppError;
use crate::pricing_sync::PricingRefreshResult;
use crate::session_usage::{
    recost_zero_cost_logs, sync_all_session_logs, RecostResult, SessionSyncResult,
};
use crate::usage_stats::{self, LogFilters, ModelPricingInfo, PaginatedLogs, UsageQuery};
use crate::usage_stats::{
    DailyStats, ModelStats, ProviderStats, RequestLogDetail, RuntimeStats, UsageBreakdownStats,
    UsageSummary,
};
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
    device_id: Option<String>,
) -> Result<UsageSummary, String> {
    let q = UsageQuery {
        start_date,
        end_date,
        app_type,
        provider_name,
        model,
        device_id,
    };
    usage_stats::get_usage_summary(db.inner(), &q).map_err(|e| e.to_string())
}

/// 获取当前筛选范围内可用的会话运行指标。
#[tauri::command]
pub fn fetch_runtime_stats(
    db: State<'_, Arc<Database>>,
    start_date: i64,
    end_date: i64,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
    device_id: Option<String>,
) -> Result<RuntimeStats, String> {
    let q = UsageQuery {
        start_date,
        end_date,
        app_type,
        provider_name,
        model,
        device_id,
    };
    usage_stats::get_runtime_stats(db.inner(), &q).map_err(|e| e.to_string())
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
    device_id: Option<String>,
) -> Result<Vec<DailyStats>, String> {
    let q = UsageQuery {
        start_date,
        end_date,
        app_type,
        provider_name,
        model,
        device_id,
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
    device_id: Option<String>,
) -> Result<Vec<ProviderStats>, String> {
    let q = UsageQuery {
        start_date,
        end_date,
        app_type,
        provider_name,
        model,
        device_id,
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
    device_id: Option<String>,
) -> Result<Vec<ModelStats>, String> {
    let q = UsageQuery {
        start_date,
        end_date,
        app_type,
        provider_name,
        model,
        device_id,
    };
    usage_stats::get_model_stats(db.inner(), &q).map_err(|e| e.to_string())
}

/// 获取项目统计
#[tauri::command]
pub fn fetch_project_stats(
    db: State<'_, Arc<Database>>,
    start_date: i64,
    end_date: i64,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
    device_id: Option<String>,
) -> Result<Vec<UsageBreakdownStats>, String> {
    let q = UsageQuery {
        start_date,
        end_date,
        app_type,
        provider_name,
        model,
        device_id,
    };
    usage_stats::get_project_stats(db.inner(), &q).map_err(|e| e.to_string())
}

/// 获取会话统计
#[tauri::command]
pub fn fetch_session_stats(
    db: State<'_, Arc<Database>>,
    start_date: i64,
    end_date: i64,
    app_type: Option<String>,
    provider_name: Option<String>,
    model: Option<String>,
    device_id: Option<String>,
) -> Result<Vec<UsageBreakdownStats>, String> {
    let q = UsageQuery {
        start_date,
        end_date,
        app_type,
        provider_name,
        model,
        device_id,
    };
    usage_stats::get_session_stats(db.inner(), &q).map_err(|e| e.to_string())
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

/// 从公开在线目录刷新模型 Token 定价，并回填历史零成本会话记录。
#[tauri::command]
pub async fn refresh_model_pricing(
    db: State<'_, Arc<Database>>,
) -> Result<PricingRefreshResult, String> {
    crate::pricing_sync::refresh_model_pricing(db.inner().clone()).await
}

/// 获取本机和已导入设备。
#[tauri::command]
pub fn fetch_devices(db: State<'_, Arc<Database>>) -> Result<Vec<DeviceInfo>, String> {
    crate::device_transfer::get_devices(db.inner()).map_err(|e| e.to_string())
}

/// 获取 Codex Radar 公开的社区智商与额度数据。
#[tauri::command]
pub async fn fetch_codex_radar() -> Result<CodexRadarSnapshot, String> {
    crate::codex_radar::fetch_codex_radar().await
}

/// 导出全部设备记录；前端负责保存为 JSON 文件。
#[tauri::command]
pub async fn export_usage_data(db: State<'_, Arc<Database>>) -> Result<UsageExportPayload, String> {
    let db = db.inner().clone();
    tokio::task::spawn_blocking(move || crate::device_transfer::export_usage_data(&db))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// 导入另一台电脑导出的 JSON，并按 request_id 去重。
#[tauri::command]
pub async fn import_usage_data(
    db: State<'_, Arc<Database>>,
    contents: String,
) -> Result<UsageImportResult, String> {
    let db = db.inner().clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::device_transfer::import_usage_data(&db, &contents)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    if result.imported > 0 {
        crate::usage_events::notify_log_recorded();
    }
    Ok(result)
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
