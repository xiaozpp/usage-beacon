//! 在线模型定价同步。
//!
//! OpenRouter 提供无需 API Key 的结构化模型目录，价格字段按 token 返回；
//! 这里将其转换为应用内部的「每百万 token」口径，并只更新本地已有或已经出现在
//! 用量日志中的模型。这样不会把完整的第三方模型目录灌进应用，也不会改变用量日志的取数逻辑。
//! 联网成功后，价格和来源时间会一起落到 model_pricing；后续成本计算只读这张本地表。

use crate::database::Database;
use crate::error::Result;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const PRICE_SOURCE: &str = "OpenRouter live";
const MAX_CATALOG_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingRefreshResult {
    pub source: String,
    pub fetched_at: i64,
    pub catalog_models: u32,
    pub updated_models: u32,
    pub added_models: u32,
    pub recosted_records: u32,
}

#[derive(Debug, Deserialize)]
struct OpenRouterPayload {
    #[serde(default)]
    data: Vec<OpenRouterModel>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    pricing: Option<OpenRouterPricing>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterPricing {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
    #[serde(default, alias = "cache_read")]
    input_cache_read: Option<String>,
    #[serde(default, alias = "cache_write")]
    input_cache_write: Option<String>,
}

#[derive(Debug)]
struct LocalPricingRow {
    model_id: String,
    cache_read: String,
    cache_creation: String,
}

/// 从互联网拉取并写入本地价格缓存，然后按最新缓存重算会话记录。
pub async fn refresh_model_pricing(
    db: Arc<Database>,
) -> std::result::Result<PricingRefreshResult, String> {
    let catalog = fetch_catalog().await?;
    tokio::task::spawn_blocking(move || {
        let mut result = apply_catalog(&db, &catalog).map_err(|error| error.to_string())?;
        match crate::session_usage::recost_session_logs(&db) {
            Ok(recost) => result.recosted_records = recost.updated,
            Err(error) => log::warn!("在线定价同步后按缓存重算成本失败: {error}"),
        }
        Ok::<_, String>(result)
    })
    .await
    .map_err(|error| format!("在线价格回填任务失败: {error}"))?
}

async fn fetch_catalog() -> std::result::Result<Vec<OpenRouterModel>, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent("UsagePulse/0.1 PricingSync")
        .build()
        .map_err(|error| format!("创建联网客户端失败: {error}"))?;

    let response = client
        .get(OPENROUTER_MODELS_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| format!("在线价格请求失败: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("在线价格服务返回 HTTP {status}"));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_CATALOG_BYTES)
    {
        return Err("在线价格目录超过安全上限".to_string());
    }

    let body = response
        .bytes()
        .await
        .map_err(|error| format!("读取在线价格失败: {error}"))?;
    if body.len() as u64 > MAX_CATALOG_BYTES {
        return Err("在线价格目录超过安全上限".to_string());
    }

    let payload: OpenRouterPayload = serde_json::from_slice(&body)
        .map_err(|error| format!("在线价格 JSON 解析失败: {error}"))?;
    if payload.data.is_empty() {
        return Err("在线价格目录为空".to_string());
    }
    Ok(payload.data)
}

fn apply_catalog(db: &Database, catalog: &[OpenRouterModel]) -> Result<PricingRefreshResult> {
    let fetched_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    db.with_conn(|conn| {
        let transaction = conn.unchecked_transaction()?;
        let local_rows = {
            let mut stmt = transaction.prepare(
                "SELECT model_id, cache_read_cost_per_million, cache_creation_cost_per_million
                 FROM model_pricing",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(LocalPricingRow {
                    model_id: row.get(0)?,
                    cache_read: row.get(1)?,
                    cache_creation: row.get(2)?,
                })
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let existing_model_ids: Vec<String> =
            local_rows.iter().map(|row| row.model_id.clone()).collect();
        let usage_models = {
            let mut stmt = transaction.prepare(
                "SELECT DISTINCT model
                 FROM proxy_request_logs
                 WHERE TRIM(model) <> ''",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };

        let mut updated_models = 0u32;
        {
            let mut update = transaction.prepare(
                "UPDATE model_pricing
                 SET input_cost_per_million = ?1,
                     output_cost_per_million = ?2,
                     cache_read_cost_per_million = ?3,
                     cache_creation_cost_per_million = ?4,
                     price_source = ?5,
                     price_fetched_at = ?6
                 WHERE model_id = ?7",
            )?;

            for local in local_rows {
                let Some(remote) = find_match(catalog, &local.model_id) else {
                    continue;
                };
                let Some(remote_pricing) = remote.pricing.as_ref() else {
                    continue;
                };
                let (Some(input), Some(output)) = (
                    remote_pricing.prompt.as_deref(),
                    remote_pricing.completion.as_deref(),
                ) else {
                    continue;
                };
                let (Ok(input), Ok(output)) = (per_million(input), per_million(output)) else {
                    continue;
                };
                // 某些聚合目录条目没有缓存写入价；这种情况下保留内置值，
                // 避免把已有的缓存成本误降为 0。
                let cache_read = remote_pricing
                    .input_cache_read
                    .as_deref()
                    .and_then(|value| per_million(value).ok())
                    .unwrap_or(local.cache_read);
                let cache_creation = remote_pricing
                    .input_cache_write
                    .as_deref()
                    .and_then(|value| per_million(value).ok())
                    .unwrap_or(local.cache_creation);

                update.execute(rusqlite::params![
                    input,
                    output,
                    cache_read,
                    cache_creation,
                    PRICE_SOURCE,
                    fetched_at,
                    local.model_id,
                ])?;
                updated_models += 1;
            }
        }

        let mut added_models = 0u32;
        {
            let mut insert = transaction.prepare(
                "INSERT OR IGNORE INTO model_pricing (
                    model_id, display_name, input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_creation_cost_per_million,
                    price_source, price_fetched_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            let mut seen_usage_models = HashSet::new();

            for model_id in usage_models {
                if !seen_usage_models.insert(model_id.clone()) {
                    continue;
                }
                let model_key = normalize_model_key(&model_id);
                if existing_model_ids
                    .iter()
                    .any(|existing| model_keys_match(&normalize_model_key(existing), &model_key))
                {
                    continue;
                }
                let Some(remote) = find_match(catalog, &model_id) else {
                    continue;
                };
                let Some(remote_pricing) = remote.pricing.as_ref() else {
                    continue;
                };
                let (Some(input), Some(output)) = (
                    remote_pricing.prompt.as_deref(),
                    remote_pricing.completion.as_deref(),
                ) else {
                    continue;
                };
                let (Ok(input), Ok(output)) = (per_million(input), per_million(output)) else {
                    continue;
                };
                let cache_read = remote_pricing
                    .input_cache_read
                    .as_deref()
                    .and_then(|value| per_million(value).ok())
                    .unwrap_or_else(|| "0".to_string());
                let cache_creation = remote_pricing
                    .input_cache_write
                    .as_deref()
                    .and_then(|value| per_million(value).ok())
                    .unwrap_or_else(|| "0".to_string());
                let display_name = remote
                    .name
                    .as_deref()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or(&model_id);

                let inserted = insert.execute(rusqlite::params![
                    model_id,
                    display_name,
                    input,
                    output,
                    cache_read,
                    cache_creation,
                    PRICE_SOURCE,
                    fetched_at,
                ])?;
                if inserted > 0 {
                    added_models += 1;
                }
            }
        }

        transaction.execute(
            "INSERT INTO model_pricing_sync (
                id, source, fetched_at, catalog_models, matched_models
             ) VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                source = excluded.source,
                fetched_at = excluded.fetched_at,
                catalog_models = excluded.catalog_models,
                matched_models = excluded.matched_models",
            rusqlite::params![
                PRICE_SOURCE,
                fetched_at,
                catalog.len() as u32,
                updated_models + added_models
            ],
        )?;
        transaction.commit()?;

        Ok(PricingRefreshResult {
            source: PRICE_SOURCE.to_string(),
            fetched_at,
            catalog_models: catalog.len() as u32,
            updated_models,
            added_models,
            recosted_records: 0,
        })
    })
}

fn find_match<'a>(
    catalog: &'a [OpenRouterModel],
    local_model_id: &str,
) -> Option<&'a OpenRouterModel> {
    let key = normalize_model_key(local_model_id);
    let mut match_item = None;

    for item in catalog {
        // :free、:batch 等是不同计费档位，不拿它们覆盖标准价；
        // 若标准条目不存在，则让本地回退价继续生效。
        if item.id.contains(':') || !model_keys_match(&key, &normalize_model_key(&item.id)) {
            continue;
        }
        if match_item.is_some() {
            // 同名多供应商时不猜，避免错误覆盖成本。
            return None;
        }
        match_item = Some(item);
    }
    match_item
}

fn model_keys_match(left: &str, right: &str) -> bool {
    left == right
        || strip_date_suffix(left) == right
        || left == strip_date_suffix(right)
        || strip_date_suffix(left) == strip_date_suffix(right)
}

fn strip_date_suffix(value: &str) -> &str {
    let Some((prefix, suffix)) = value.rsplit_once('-') else {
        return value;
    };
    if suffix.len() == 8 && suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        prefix
    } else {
        value
    }
}

fn normalize_model_key(value: &str) -> String {
    value
        .rsplit('/')
        .next()
        .unwrap_or(value)
        .split(':')
        .next()
        .unwrap_or(value)
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '.'], "-")
}

fn per_million(value: &str) -> std::result::Result<String, String> {
    let price =
        Decimal::from_str(value.trim()).map_err(|error| format!("价格不是有效数字: {error}"))?;
    if price < Decimal::ZERO {
        return Err("价格不能为负数".to_string());
    }
    Ok((price * Decimal::from(1_000_000)).normalize().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_token_prices_to_per_million() {
        assert_eq!(per_million("0.0000025").unwrap(), "2.5");
        assert_eq!(per_million("0.000015").unwrap(), "15");
        assert_eq!(per_million("0").unwrap(), "0");
    }

    #[test]
    fn normalizes_provider_and_punctuation() {
        assert_eq!(normalize_model_key("openai/gpt-5.4"), "gpt-5-4");
        assert_eq!(normalize_model_key("gpt_5.4:batch"), "gpt-5-4");
        assert!(model_keys_match(
            "claude-sonnet-4-5-20250929",
            "claude-sonnet-4-5"
        ));
    }

    #[test]
    fn parses_openrouter_cache_fields() {
        let payload: OpenRouterPayload = serde_json::from_str(
            r#"{"data":[{"id":"openai/gpt-5.4","pricing":{"prompt":"0.0000025","completion":"0.000015","input_cache_read":"0.00000025","input_cache_write":"0.00000625"}}]}"#,
        )
        .unwrap();
        let pricing = payload.data[0].pricing.as_ref().unwrap();
        assert_eq!(pricing.input_cache_read.as_deref(), Some("0.00000025"));
        assert_eq!(pricing.input_cache_write.as_deref(), Some("0.00000625"));
    }

    #[test]
    fn applies_matching_model_and_keeps_unknown_models_out() {
        let db = Database {
            conn: std::sync::Mutex::new(rusqlite::Connection::open_in_memory().unwrap()),
        };
        db.with_conn(|conn| {
            crate::schema::create_tables(conn)?;
            crate::schema::seed_model_pricing(conn)?;
            conn.execute(
                "INSERT INTO proxy_request_logs (
                    request_id, provider_id, app_type, model, latency_ms, status_code, created_at
                 ) VALUES ('test-pricing-row', '_zcode_session', 'zcode', 'FUTURE-MODEL', 0, 200, 0)",
                [],
            )?;
            Ok(())
        })
        .unwrap();

        let catalog = vec![
            OpenRouterModel {
                id: "openai/gpt-5.4".to_string(),
                name: None,
                pricing: Some(OpenRouterPricing {
                    prompt: Some("0.0000025".to_string()),
                    completion: Some("0.000015".to_string()),
                    input_cache_read: Some("0.00000025".to_string()),
                    input_cache_write: None,
                }),
            },
            OpenRouterModel {
                id: "vendor/not-in-local-table".to_string(),
                name: None,
                pricing: Some(OpenRouterPricing {
                    prompt: Some("0.000001".to_string()),
                    completion: Some("0.000002".to_string()),
                    input_cache_read: None,
                    input_cache_write: None,
                }),
            },
            OpenRouterModel {
                id: "vendor/future-model".to_string(),
                name: Some("Future Model".to_string()),
                pricing: Some(OpenRouterPricing {
                    prompt: Some("0.000001".to_string()),
                    completion: Some("0.000002".to_string()),
                    input_cache_read: Some("0.0000001".to_string()),
                    input_cache_write: None,
                }),
            },
        ];

        let result = apply_catalog(&db, &catalog).unwrap();
        assert_eq!(result.updated_models, 1);
        assert_eq!(result.added_models, 1);
        let prices: (String, String, String) = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT input_cost_per_million, output_cost_per_million, cache_read_cost_per_million
                     FROM model_pricing WHERE model_id = 'gpt-5.4'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .map_err(Into::into)
            })
            .unwrap();
        assert_eq!(
            prices,
            ("2.5".to_string(), "15".to_string(), "0.25".to_string())
        );
        let future_price: (String, String, String) = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT input_cost_per_million, output_cost_per_million, cache_read_cost_per_million
                     FROM model_pricing WHERE model_id = 'FUTURE-MODEL'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .map_err(Into::into)
            })
            .unwrap();
        assert_eq!(
            future_price,
            ("1".to_string(), "2".to_string(), "0.1".to_string())
        );
    }
}
