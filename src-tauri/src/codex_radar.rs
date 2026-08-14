use regex::Regex;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const IQ_URL: &str = "https://codexradar.com/api/intelligence-efficiency-metrics";
const HOME_URL: &str = "https://codexradar.com/";
const MAX_IQ_BYTES: u64 = 512 * 1024;
const MAX_HOME_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRadarSnapshot {
    pub iq: Option<RadarIqSnapshot>,
    pub quota: Option<RadarQuotaSnapshot>,
    pub fetched_at: i64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RadarIqSnapshot {
    pub source_updated_at: String,
    pub runs_24h_total: u32,
    pub runs_total: u32,
    pub benchmark_id: String,
    pub score_label: String,
    pub points: Vec<RadarIqPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RadarIqPoint {
    pub model: String,
    pub effort: String,
    pub iq: f64,
    pub average_price_usd: Option<f64>,
    pub average_minutes: Option<f64>,
    pub total: u32,
    pub runs_24h: u32,
    pub runs_total: u32,
    pub source_updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RadarQuotaSnapshot {
    pub source_updated_at: String,
    pub tiers: Vec<RadarQuotaTier>,
    pub history: Vec<RadarQuotaHistoryPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RadarQuotaTier {
    pub plan: String,
    pub weekly_usd: f64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RadarQuotaHistoryPoint {
    pub label: String,
    pub weekly_usd: f64,
}

#[derive(Debug, Deserialize)]
struct IqApiPayload {
    #[serde(default)]
    source_updated_at: String,
    #[serde(default)]
    runs_24h_total: u32,
    #[serde(default)]
    runs_total: u32,
    #[serde(default)]
    benchmark_id: String,
    #[serde(default)]
    score_label: String,
    #[serde(default)]
    points: Vec<IqApiPoint>,
}

#[derive(Debug, Deserialize)]
struct IqApiPoint {
    #[serde(default)]
    model: String,
    #[serde(default)]
    effort: String,
    iq: Option<f64>,
    average_price_usd: Option<f64>,
    average_minutes: Option<f64>,
    #[serde(default)]
    total: u32,
    #[serde(default)]
    runs_24h: u32,
    #[serde(default)]
    runs_total: u32,
    source_updated_at: Option<String>,
}

pub async fn fetch_codex_radar() -> Result<CodexRadarSnapshot, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent("UsagePulse/0.1 CodexRadarWidget")
        .build()
        .map_err(|error| format!("创建联网客户端失败: {error}"))?;

    let (iq_result, quota_result) = tokio::join!(
        fetch_text(&client, IQ_URL, MAX_IQ_BYTES),
        fetch_text(&client, HOME_URL, MAX_HOME_BYTES)
    );

    let mut warnings = Vec::new();
    let iq = match iq_result.and_then(|body| parse_iq(&body)) {
        Ok(value) => Some(value),
        Err(error) => {
            warnings.push(format!("智商雷达: {error}"));
            None
        }
    };
    let quota = match quota_result.and_then(|body| parse_quota(&body)) {
        Ok(value) => Some(value),
        Err(error) => {
            warnings.push(format!("额度雷达: {error}"));
            None
        }
    };

    if iq.is_none() && quota.is_none() {
        return Err(format!("Codex Radar 联网失败：{}", warnings.join("；")));
    }

    Ok(CodexRadarSnapshot {
        iq,
        quota,
        fetched_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64,
        warnings,
    })
}

async fn fetch_text(client: &reqwest::Client, url: &str, max_bytes: u64) -> Result<String, String> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("请求失败: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("服务返回 HTTP {status}"));
    }
    if response
        .content_length()
        .is_some_and(|size| size > max_bytes)
    {
        return Err("响应数据超过安全上限".to_string());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("读取响应失败: {error}"))?
    {
        let remaining = max_bytes.saturating_sub(bytes.len() as u64);
        if chunk.len() as u64 > remaining {
            return Err("响应数据超过安全上限".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| "响应不是有效 UTF-8".to_string())
}

fn parse_iq(body: &str) -> Result<RadarIqSnapshot, String> {
    let payload: IqApiPayload =
        serde_json::from_str(body).map_err(|error| format!("JSON 解析失败: {error}"))?;
    let points = payload
        .points
        .into_iter()
        .filter_map(|point| {
            let iq = point.iq?;
            if point.model.is_empty()
                || point.effort.is_empty()
                || point.total == 0
                || !iq.is_finite()
                || !(0.0..=150.0).contains(&iq)
            {
                return None;
            }
            Some(RadarIqPoint {
                model: point.model,
                effort: point.effort,
                iq,
                average_price_usd: point
                    .average_price_usd
                    .filter(|value| value.is_finite() && *value >= 0.0),
                average_minutes: point
                    .average_minutes
                    .filter(|value| value.is_finite() && *value >= 0.0),
                total: point.total,
                runs_24h: point.runs_24h,
                runs_total: point.runs_total,
                source_updated_at: point.source_updated_at,
            })
        })
        .collect::<Vec<_>>();
    if points.is_empty() {
        return Err("没有可用的智商样本".to_string());
    }
    Ok(RadarIqSnapshot {
        source_updated_at: payload.source_updated_at,
        runs_24h_total: payload.runs_24h_total,
        runs_total: payload.runs_total,
        benchmark_id: payload.benchmark_id,
        score_label: payload.score_label,
        points,
    })
}

fn parse_quota(body: &str) -> Result<RadarQuotaSnapshot, String> {
    let section_regex =
        Regex::new(r#"(?is)<section[^>]*class="[^"]*\bquota-radar\b[^"]*"[^>]*>(.*?)</section>"#)
            .map_err(|error| error.to_string())?;
    let section = section_regex
        .captures(body)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str())
        .ok_or_else(|| "页面中未找到额度雷达".to_string())?;

    let updated_regex =
        Regex::new(r#"(?is)额度雷达\s*<span>([^<]+)</span>"#).map_err(|error| error.to_string())?;
    let row_regex = Regex::new(
        r#"(?is)<strong>([^<]+)</strong>\s*<span>\$([0-9,]+(?:\.[0-9]+)?)</span>\s*<em>([^<]+)</em>"#,
    )
    .map_err(|error| error.to_string())?;
    let history_regex =
        Regex::new(r#"(?is)<title>([^<]+?)\s+20x Pro 7d\s+\$([0-9,]+(?:\.[0-9]+)?)</title>"#)
            .map_err(|error| error.to_string())?;

    let source_updated_at = updated_regex
        .captures(section)
        .and_then(|captures| captures.get(1))
        .map(|value| decode_text(value.as_str()))
        .unwrap_or_default();
    let tiers = row_regex
        .captures_iter(section)
        .filter_map(|captures| {
            let weekly_usd = parse_usd(captures.get(2)?.as_str())?;
            Some(RadarQuotaTier {
                plan: decode_text(captures.get(1)?.as_str()),
                weekly_usd,
                source: decode_text(captures.get(3)?.as_str()),
            })
        })
        .collect::<Vec<_>>();
    if tiers.is_empty() {
        return Err("额度档位解析失败".to_string());
    }
    let history = history_regex
        .captures_iter(section)
        .filter_map(|captures| {
            Some(RadarQuotaHistoryPoint {
                label: decode_text(captures.get(1)?.as_str()),
                weekly_usd: parse_usd(captures.get(2)?.as_str())?,
            })
        })
        .collect();

    Ok(RadarQuotaSnapshot {
        source_updated_at,
        tiers,
        history,
    })
}

fn parse_usd(value: &str) -> Option<f64> {
    let parsed = value.replace(',', "").parse::<f64>().ok()?;
    (parsed.is_finite() && parsed >= 0.0).then_some(parsed)
}

fn decode_text(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_validates_iq_points() {
        let payload = r#"{
          "source_updated_at":"2026-08-14T10:46:14+08:00",
          "runs_24h_total":648,
          "runs_total":32015,
          "benchmark_id":"deep-swe",
          "score_label":"Pass rate",
          "points":[
            {"model":"gpt-5.6-sol","effort":"max","iq":102.14,"total":336,"runs_24h":23,"runs_total":1905},
            {"model":"grok-4.5","effort":"medium","iq":null,"total":0,"runs_24h":0,"runs_total":0}
          ]
        }"#;
        let parsed = parse_iq(payload).unwrap();
        assert_eq!(parsed.points.len(), 1);
        assert_eq!(parsed.points[0].model, "gpt-5.6-sol");
        assert_eq!(parsed.runs_24h_total, 648);
    }

    #[test]
    fn parses_quota_tiers_and_history() {
        let html = r#"
          <section class="quota-radar" aria-label="额度雷达">
            <h2>额度雷达 <span>8月9日19:49更新</span></h2>
            <strong>20x Pro</strong><span>$1,649.72</span><em>分布式雷达</em>
            <strong>Plus</strong><span>$82.49</span><em>推算</em>
            <title>2026-08-03 20x Pro 7d $1,948.00</title>
            <title>2026-08-09 20x Pro 7d $1,649.72</title>
          </section>
        "#;
        let parsed = parse_quota(html).unwrap();
        assert_eq!(parsed.source_updated_at, "8月9日19:49更新");
        assert_eq!(parsed.tiers.len(), 2);
        assert_eq!(parsed.history.len(), 2);
        assert_eq!(parsed.tiers[0].weekly_usd, 1649.72);
    }

    #[tokio::test]
    #[ignore = "requires codexradar.com"]
    async fn fetches_live_snapshot() {
        let snapshot = fetch_codex_radar().await.unwrap();
        assert!(snapshot.iq.is_some());
        assert!(snapshot.quota.is_some());
    }
}
