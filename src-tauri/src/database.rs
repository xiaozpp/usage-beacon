//! 数据库连接与状态管理

use crate::error::{AppError, Result};
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Database {
    pub conn: Mutex<Connection>,
}

impl Database {
    pub fn open(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::Config(format!("无法创建数据库目录 {}: {e}", parent.display()))
            })?;
        }

        let conn = Connection::open(&path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;

        crate::schema::create_tables(&conn)?;
        crate::device_transfer::initialize_device_identity(&conn)?;
        crate::schema::seed_model_pricing(&conn)?;

        log::info!("数据库已初始化: {}", path.display());

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|e| AppError::Database(e.to_string()))?;
        f(&conn)
    }
}

/// 获取应用数据目录
pub fn get_app_data_dir() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| std::env::temp_dir());
    base.join("usage-pulse")
}

/// 获取数据库文件路径
pub fn get_db_path() -> PathBuf {
    get_app_data_dir().join("usage-pulse.db")
}

/// 获取 Claude 配置目录
pub fn get_claude_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
}

/// 获取 Grok Build 会话目录
pub fn get_grok_config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".grok")
}
