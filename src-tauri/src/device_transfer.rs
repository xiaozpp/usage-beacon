//! 多设备身份、使用记录导入导出。

use crate::database::Database;
use crate::error::{AppError, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const SCHEMA_VERSION: u32 = 1;
const LOCAL_DEVICE_ID_KEY: &str = "local_device_id";
const LOCAL_DEVICE_NAME_KEY: &str = "local_device_name";
const MAX_IMPORT_BYTES: usize = 250 * 1024 * 1024;
const MAX_IMPORT_RECORDS: usize = 500_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub is_local: bool,
    pub request_count: u32,
    pub last_seen: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageExportPayload {
    pub file_name: String,
    pub contents: String,
    pub record_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageImportResult {
    pub imported: u32,
    pub skipped: u32,
    pub device_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceIdentity {
    id: String,
    name: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageTransferFile {
    schema_version: u32,
    exported_at: i64,
    source_device: DeviceIdentity,
    records: Vec<TransferRecord>,
    #[serde(default)]
    runtime_stats: Vec<RuntimeTransferRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransferRecord {
    request_id: String,
    provider_id: String,
    app_type: String,
    model: String,
    request_model: Option<String>,
    pricing_model: Option<String>,
    #[serde(default = "default_request_count")]
    request_count: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_creation_tokens: i64,
    input_token_semantics: i64,
    input_cost_usd: String,
    output_cost_usd: String,
    cache_read_cost_usd: String,
    cache_creation_cost_usd: String,
    total_cost_usd: String,
    latency_ms: i64,
    first_token_ms: Option<i64>,
    duration_ms: Option<i64>,
    status_code: i64,
    error_message: Option<String>,
    session_id: Option<String>,
    provider_type: Option<String>,
    is_streaming: i64,
    cost_multiplier: String,
    created_at: i64,
    data_source: String,
    device_id: String,
    device_name: String,
    #[serde(default)]
    project: String,
}

fn default_request_count() -> i64 {
    1
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeTransferRecord {
    data_source: String,
    app_type: String,
    provider_id: String,
    session_id: String,
    model: String,
    project: String,
    turns: i64,
    steps: i64,
    llm_ms: i64,
    tool_ms: i64,
    ttft_ms: i64,
    ttft_steps: i64,
    decode_ms: i64,
    decode_tokens: i64,
    started_at: i64,
    last_event_at: i64,
    device_id: String,
    device_name: String,
}

pub fn initialize_device_identity(conn: &Connection) -> Result<DeviceInfo> {
    let existing_id = setting(conn, LOCAL_DEVICE_ID_KEY)?;
    let existing_name = setting(conn, LOCAL_DEVICE_NAME_KEY)?;
    let name = existing_name.unwrap_or_else(local_device_name);
    let id = existing_id.unwrap_or_else(|| {
        format!(
            "{}-{}",
            slug(&name),
            Utc::now().timestamp_millis().unsigned_abs()
        )
    });

    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        params![LOCAL_DEVICE_ID_KEY, id],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        params![LOCAL_DEVICE_NAME_KEY, name],
    )?;
    conn.execute(
        "UPDATE proxy_request_logs
         SET device_id = ?1, device_name = ?2
         WHERE COALESCE(device_id, '') = ''",
        params![id, name],
    )?;
    conn.execute(
        "UPDATE session_runtime_stats
         SET device_id = ?1, device_name = ?2
         WHERE COALESCE(device_id, '') = ''",
        params![id, name],
    )?;

    Ok(DeviceInfo {
        id,
        name,
        is_local: true,
        request_count: 0,
        last_seen: None,
    })
}

pub fn get_devices(db: &Database) -> Result<Vec<DeviceInfo>> {
    db.with_conn(|conn| {
        let local = current_device(conn)?;
        let mut stmt = conn.prepare(
            "SELECT device_id, COALESCE(NULLIF(MAX(device_name), ''), device_id),
                    COALESCE(SUM(COALESCE(request_count, 1)), 0), MAX(created_at)
             FROM proxy_request_logs
             WHERE COALESCE(device_id, '') <> ''
             GROUP BY device_id",
        )?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            Ok(DeviceInfo {
                is_local: id == local.id,
                id,
                name: row.get(1)?,
                request_count: row.get::<_, i64>(2)? as u32,
                last_seen: row.get(3)?,
            })
        })?;

        let mut devices = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        if !devices.iter().any(|device| device.id == local.id) {
            devices.push(DeviceInfo {
                id: local.id.clone(),
                name: local.name.clone(),
                is_local: true,
                request_count: 0,
                last_seen: None,
            });
        }
        devices.sort_by(|left, right| {
            right
                .is_local
                .cmp(&left.is_local)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(devices)
    })
}

pub fn export_usage_data(db: &Database) -> Result<UsageExportPayload> {
    db.with_conn(|conn| {
        let local = current_device(conn)?;
        let mut stmt = conn.prepare(
            "SELECT request_id, provider_id, app_type, model, request_model, pricing_model,
                    request_count, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    input_token_semantics, input_cost_usd, output_cost_usd,
                    cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd,
                    latency_ms, first_token_ms, duration_ms, status_code, error_message,
                    session_id, provider_type, is_streaming, cost_multiplier, created_at,
                    data_source, device_id, device_name, project
             FROM proxy_request_logs
             ORDER BY created_at ASC, request_id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(TransferRecord {
                request_id: row.get(0)?,
                provider_id: row.get(1)?,
                app_type: row.get(2)?,
                model: row.get(3)?,
                request_model: row.get(4)?,
                pricing_model: row.get(5)?,
                request_count: row.get(6)?,
                input_tokens: row.get(7)?,
                output_tokens: row.get(8)?,
                cache_read_tokens: row.get(9)?,
                cache_creation_tokens: row.get(10)?,
                input_token_semantics: row.get(11)?,
                input_cost_usd: row.get(12)?,
                output_cost_usd: row.get(13)?,
                cache_read_cost_usd: row.get(14)?,
                cache_creation_cost_usd: row.get(15)?,
                total_cost_usd: row.get(16)?,
                latency_ms: row.get(17)?,
                first_token_ms: row.get(18)?,
                duration_ms: row.get(19)?,
                status_code: row.get(20)?,
                error_message: row.get(21)?,
                session_id: row.get(22)?,
                provider_type: row.get(23)?,
                is_streaming: row.get(24)?,
                cost_multiplier: row.get(25)?,
                created_at: row.get(26)?,
                data_source: row.get(27)?,
                device_id: row.get(28)?,
                device_name: row.get(29)?,
                project: row.get(30)?,
            })
        })?;
        let records = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut runtime_stmt = conn.prepare(
            "SELECT data_source, app_type, provider_id, session_id, model, project,
                    turns, steps, llm_ms, tool_ms, ttft_ms, ttft_steps,
                    decode_ms, decode_tokens, started_at, last_event_at,
                    device_id, device_name
             FROM session_runtime_stats
             ORDER BY started_at ASC, session_id ASC",
        )?;
        let runtime_rows = runtime_stmt.query_map([], |row| {
            Ok(RuntimeTransferRecord {
                data_source: row.get(0)?,
                app_type: row.get(1)?,
                provider_id: row.get(2)?,
                session_id: row.get(3)?,
                model: row.get(4)?,
                project: row.get(5)?,
                turns: row.get(6)?,
                steps: row.get(7)?,
                llm_ms: row.get(8)?,
                tool_ms: row.get(9)?,
                ttft_ms: row.get(10)?,
                ttft_steps: row.get(11)?,
                decode_ms: row.get(12)?,
                decode_tokens: row.get(13)?,
                started_at: row.get(14)?,
                last_event_at: row.get(15)?,
                device_id: row.get(16)?,
                device_name: row.get(17)?,
            })
        })?;
        let runtime_stats = runtime_rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let record_count = records.len() as u32;
        let exported_at = Utc::now().timestamp();
        let transfer = UsageTransferFile {
            schema_version: SCHEMA_VERSION,
            exported_at,
            source_device: local.clone(),
            records,
            runtime_stats,
        };

        Ok(UsageExportPayload {
            file_name: format!("usage-pulse-{}-{exported_at}.json", slug(&local.name)),
            contents: serde_json::to_string(&transfer)?,
            record_count,
        })
    })
}

pub fn import_usage_data(db: &Database, contents: &str) -> Result<UsageImportResult> {
    if contents.len() > MAX_IMPORT_BYTES {
        return Err(AppError::Config("导入文件超过 250MB".into()));
    }
    let transfer: UsageTransferFile = serde_json::from_str(contents)?;
    if transfer.schema_version != SCHEMA_VERSION {
        return Err(AppError::Config(format!(
            "不支持的导入文件版本: {}",
            transfer.schema_version
        )));
    }
    if transfer.records.len() > MAX_IMPORT_RECORDS {
        return Err(AppError::Config("导入记录超过 50 万条".into()));
    }
    for record in &transfer.records {
        validate_record(record)?;
    }
    if transfer.runtime_stats.len() > MAX_IMPORT_RECORDS {
        return Err(AppError::Config("导入会话统计超过 50 万条".into()));
    }
    for runtime in &transfer.runtime_stats {
        validate_runtime_record(runtime)?;
    }

    db.with_conn(|conn| {
        let transaction = conn.unchecked_transaction()?;
        let mut imported = 0u32;
        let mut skipped = 0u32;
        let mut device_ids = BTreeSet::new();
        let source_device = transfer.source_device.clone();
        let records = transfer.records;
        let runtime_stats = transfer.runtime_stats;
        {
            let mut stmt = transaction.prepare(
                "INSERT OR IGNORE INTO proxy_request_logs (
                    request_id, provider_id, app_type, model, request_model, pricing_model,
                    request_count, input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    input_token_semantics, input_cost_usd, output_cost_usd,
                    cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd,
                    latency_ms, first_token_ms, duration_ms, status_code, error_message,
                    session_id, provider_type, is_streaming, cost_multiplier, created_at,
                    data_source, device_id, device_name, project
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                    ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31
                 )",
            )?;

            for mut record in records {
                if record.device_id.trim().is_empty() {
                    record.device_id = source_device.id.clone();
                }
                if record.device_name.trim().is_empty() {
                    record.device_name = source_device.name.clone();
                }
                device_ids.insert(record.device_id.clone());
                let changed = stmt.execute(params![
                    record.request_id,
                    record.provider_id,
                    record.app_type,
                    record.model,
                    record.request_model,
                    record.pricing_model,
                    record.request_count,
                    record.input_tokens,
                    record.output_tokens,
                    record.cache_read_tokens,
                    record.cache_creation_tokens,
                    record.input_token_semantics,
                    record.input_cost_usd,
                    record.output_cost_usd,
                    record.cache_read_cost_usd,
                    record.cache_creation_cost_usd,
                    record.total_cost_usd,
                    record.latency_ms,
                    record.first_token_ms,
                    record.duration_ms,
                    record.status_code,
                    record.error_message,
                    record.session_id,
                    record.provider_type,
                    record.is_streaming,
                    record.cost_multiplier,
                    record.created_at,
                    record.data_source,
                    record.device_id,
                    record.device_name,
                    record.project,
                ])?;
                if changed > 0 {
                    imported = imported.saturating_add(1);
                } else {
                    skipped = skipped.saturating_add(1);
                }
            }
        }
        {
            let mut runtime_stmt = transaction.prepare(
                "INSERT OR IGNORE INTO session_runtime_stats (
                    data_source, app_type, provider_id, session_id, model, project,
                    source_path, turns, steps, llm_ms, tool_ms, ttft_ms, ttft_steps,
                    decode_ms, decode_tokens, started_at, last_event_at, device_id, device_name
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, '', ?7, ?8, ?9,
                    ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
                 )",
            )?;

            for mut runtime in runtime_stats {
                if runtime.device_id.trim().is_empty() {
                    runtime.device_id = source_device.id.clone();
                }
                if runtime.device_name.trim().is_empty() {
                    runtime.device_name = source_device.name.clone();
                }
                device_ids.insert(runtime.device_id.clone());
                runtime_stmt.execute(params![
                    runtime.data_source,
                    runtime.app_type,
                    runtime.provider_id,
                    runtime.session_id,
                    runtime.model,
                    runtime.project,
                    runtime.turns,
                    runtime.steps,
                    runtime.llm_ms,
                    runtime.tool_ms,
                    runtime.ttft_ms,
                    runtime.ttft_steps,
                    runtime.decode_ms,
                    runtime.decode_tokens,
                    runtime.started_at,
                    runtime.last_event_at,
                    runtime.device_id,
                    runtime.device_name,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(UsageImportResult {
            imported,
            skipped,
            device_count: device_ids.len() as u32,
        })
    })
}

fn current_device(conn: &Connection) -> Result<DeviceIdentity> {
    let id = setting(conn, LOCAL_DEVICE_ID_KEY)?
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::Config("本机设备标识尚未初始化".into()))?;
    let name = setting(conn, LOCAL_DEVICE_NAME_KEY)?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| id.clone());
    Ok(DeviceIdentity { id, name })
}

fn setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn local_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .map(|value| clean_label(&value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "本机".to_string())
}

fn clean_label(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(80)
        .collect::<String>()
        .trim()
        .to_string()
}

fn slug(value: &str) -> String {
    let value = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let value = value.trim_matches('-').to_string();
    if value.is_empty() {
        "device".to_string()
    } else {
        value
    }
}

fn validate_record(record: &TransferRecord) -> Result<()> {
    if record.request_id.trim().is_empty()
        || record.provider_id.trim().is_empty()
        || record.app_type.trim().is_empty()
        || record.model.trim().is_empty()
        || record.created_at <= 0
        || record.request_count < 0
        || [
            record.input_tokens,
            record.output_tokens,
            record.cache_read_tokens,
            record.cache_creation_tokens,
        ]
        .iter()
        .any(|value| *value < 0)
    {
        return Err(AppError::Config("导入文件包含无效记录".into()));
    }
    Ok(())
}

fn validate_runtime_record(record: &RuntimeTransferRecord) -> Result<()> {
    if record.data_source.trim().is_empty()
        || record.app_type.trim().is_empty()
        || record.provider_id.trim().is_empty()
        || record.session_id.trim().is_empty()
        || record.model.trim().is_empty()
        || record.started_at <= 0
        || [
            record.turns,
            record.steps,
            record.llm_ms,
            record.tool_ms,
            record.ttft_ms,
            record.ttft_steps,
            record.decode_ms,
            record.decode_tokens,
            record.last_event_at,
        ]
        .iter()
        .any(|value| *value < 0)
    {
        return Err(AppError::Config("导入文件包含无效会话统计".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn test_db(id: &str, name: &str) -> Database {
        let conn = Connection::open_in_memory().unwrap();
        crate::schema::create_tables(&conn).unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![LOCAL_DEVICE_ID_KEY, id],
        )
        .unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![LOCAL_DEVICE_NAME_KEY, name],
        )
        .unwrap();
        initialize_device_identity(&conn).unwrap();
        Database {
            conn: Mutex::new(conn),
        }
    }

    #[test]
    fn export_import_is_deduplicated_and_keeps_device() {
        let source = test_db("device-a", "工作电脑");
        source
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO proxy_request_logs (
                        request_id, provider_id, app_type, model, input_tokens, output_tokens,
                        latency_ms, status_code, created_at, data_source
                     ) VALUES ('request-a', '_session', 'codex', 'gpt-test', 100, 20,
                               0, 200, 1700000000, 'codex_session')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();

        let payload = export_usage_data(&source).unwrap();
        let target = test_db("device-b", "家用电脑");
        let first = import_usage_data(&target, &payload.contents).unwrap();
        let second = import_usage_data(&target, &payload.contents).unwrap();

        assert_eq!(first.imported, 1);
        assert_eq!(first.skipped, 0);
        assert_eq!(second.imported, 0);
        assert_eq!(second.skipped, 1);
        assert!(get_devices(&target)
            .unwrap()
            .iter()
            .any(|device| device.id == "device-a" && !device.is_local));
    }
}
