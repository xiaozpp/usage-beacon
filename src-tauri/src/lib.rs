#![allow(dead_code)]

mod calculator;
mod commands;
mod database;
mod error;
mod schema;
mod session_usage;
mod session_usage_codex;
mod session_usage_gemini;
mod session_usage_grok;
mod session_usage_opencode;
mod usage_events;
mod usage_stats;

use database::Database;
use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // 初始化数据库
            let db_path = database::get_db_path();
            let db = match Database::open(db_path) {
                Ok(db) => Arc::new(db),
                Err(e) => {
                    log::error!("数据库初始化失败: {e}");
                    return Err(Box::new(e));
                }
            };
            if let Err(error) = session_usage_codex::migrate_codex_usage(&db) {
                log::warn!("Codex 用量解析器迁移失败，将在下次启动重试: {error}");
            }
            app.manage(db.clone());

            // 设置事件通知的 AppHandle
            usage_events::set_app_handle(app.handle().clone());

            // 启动后台自动同步任务（每 60 秒同步一次本地会话日志）
            let db_for_sync = db.clone();
            tauri::async_runtime::spawn(async move {
                // 启动时先同步一次
                if let Err(e) = run_sync(&db_for_sync).await {
                    log::warn!("初始同步失败: {e}");
                }
                // 同步完成后回填历史成本为 0 的记录（修复补充定价后旧行未重算）
                if let Err(e) = run_recost(&db_for_sync).await {
                    log::warn!("初始成本回填失败: {e}");
                }
                // 周期同步
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
                interval.tick().await; // 跳过第一次立即触发
                loop {
                    interval.tick().await;
                    if let Err(e) = run_sync(&db_for_sync).await {
                        log::warn!("周期同步失败: {e}");
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::sync_claude_logs,
            commands::fetch_usage_summary,
            commands::fetch_daily_trends,
            commands::fetch_provider_stats,
            commands::fetch_model_stats,
            commands::fetch_request_logs,
            commands::fetch_request_detail,
            commands::fetch_model_pricing,
            commands::recost_logs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn run_sync(db: &Arc<Database>) -> Result<(), error::AppError> {
    let db = db.clone();
    tokio::task::spawn_blocking(move || {
        let result = session_usage::sync_all_session_logs(&db)?;
        if result.imported > 0 {
            usage_events::notify_log_recorded();
        }
        Ok::<_, error::AppError>(())
    })
    .await
    .map_err(|e| error::AppError::Config(format!("同步任务失败: {e}")))??;
    Ok(())
}

async fn run_recost(db: &Arc<Database>) -> Result<(), error::AppError> {
    let db = db.clone();
    tokio::task::spawn_blocking(move || {
        let _ = session_usage::recost_zero_cost_logs(&db)?;
        Ok::<_, error::AppError>(())
    })
    .await
    .map_err(|e| error::AppError::Config(format!("回填任务失败: {e}")))??;
    Ok(())
}
