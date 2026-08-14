//! 使用量事件通知
//!
//! 200ms 防抖的全局事件桥，用于在前端收到通知后刷新查询缓存

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static DEBOUNCE_FLAG: AtomicBool = AtomicBool::new(false);

pub fn set_app_handle(handle: AppHandle) {
    let _ = APP_HANDLE.set(handle);
}

/// 通知前端有新日志写入（200ms 防抖）
pub fn notify_log_recorded() {
    let Some(handle) = APP_HANDLE.get() else {
        return;
    };

    if DEBOUNCE_FLAG.swap(true, Ordering::SeqCst) {
        return;
    }

    let handle = handle.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        DEBOUNCE_FLAG.store(false, Ordering::SeqCst);
        let _ = handle.emit("usage-log-recorded", ());
    });
}
