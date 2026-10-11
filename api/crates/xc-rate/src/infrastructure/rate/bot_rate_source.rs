//! BoT rate source implementation.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};

use crate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};

pub const BOT_BASE: &str = "https://www.bot.or.th/content/bot/th/statistics/exchange-rate/jcr:content/root/container/statisticstable2";
pub const BOT_SOURCE_PAGE: &str = "https://www.bot.or.th/th/statistics/exchange-rate.html";
pub const BOT_TIMEOUT: Duration = Duration::from_secs(10);
pub const BOT_UNAVAILABLE: &str = "ไม่สามารถดึงเรทล่าสุดจาก ธปท. ได้ กรุณาลองใหม่ก่อนบันทึกเรท";

pub type Clock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

#[derive(Clone)]
pub struct BotRateSource {
    base: String,
    timeout: Duration,
    clock: Clock,
    client: reqwest::Client,
}

impl BotRateSource {
    pub fn bot() -> Self {
        Self::new(BOT_BASE, BOT_TIMEOUT, Arc::new(Utc::now))
    }

    pub fn new(base: impl Into<String>, timeout: Duration, clock: Clock) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_default();
        Self {
            base: base.into(),
            timeout,
            clock,
            client,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

fn parse_instant(val: Option<&serde_json::Value>) -> Option<DateTime<Utc>> {
    let s = val?.as_str()?;
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn window_contains(
    now: DateTime<Utc>,
    start_val: Option<&serde_json::Value>,
    end_val: Option<&serde_json::Value>,
) -> bool {
    match (parse_instant(start_val), parse_instant(end_val)) {
        (Some(start), Some(end)) => start <= now && now < end,
        _ => false,
    }
}

impl RateSource for BotRateSource {
    fn fetch(&self) -> BoxFuture<'_, AppResult<ReferenceRates>> {
        Box::pin(async move {
            let now = (self.clock)();

            // 1. GET <base>.labels.json
            let labels_url = format!("{}.labels.json", self.base);
            let res = self
                .client
                .get(&labels_url)
                .timeout(self.timeout)
                .send()
                .await
                .map_err(|_| AppError::Domain(BOT_UNAVAILABLE.to_string()))?;

            if !res.status().is_success() {
                return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
            }

            let body_bytes = res
                .bytes()
                .await
                .map_err(|_| AppError::Domain(BOT_UNAVAILABLE.to_string()))?;

            let labels_val: serde_json::Value = serde_json::from_slice(&body_bytes)
                .map_err(|_| AppError::Domain(BOT_UNAVAILABLE.to_string()))?;

            // 2. Choose level
            let in_level1 = window_contains(
                now,
                labels_val.get("level1StartTime"),
                labels_val.get("level1EndTime"),
            );
            let in_level2 = window_contains(
                now,
                labels_val.get("level2StartTime"),
                labels_val.get("level2EndTime"),
            );

            let level = if in_level1 {
                1
            } else if in_level2 {
                2
            } else {
                3
            };

            // 3. GET <base>.results.level<N>cache.json
            let level_url = format!("{}.results.level{}cache.json", self.base, level);
            let res = self
                .client
                .get(&level_url)
                .timeout(self.timeout)
                .send()
                .await
                .map_err(|_| AppError::Domain(BOT_UNAVAILABLE.to_string()))?;

            if !res.status().is_success() {
                return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
            }

            let body_bytes = res
                .bytes()
                .await
                .map_err(|_| AppError::Domain(BOT_UNAVAILABLE.to_string()))?;

            let table_val: serde_json::Value = serde_json::from_slice(&body_bytes)
                .map_err(|_| AppError::Domain(BOT_UNAVAILABLE.to_string()))?;

            // 4. Validate table
            if table_val.get("noDataFoundFlag").and_then(|v| v.as_bool()) == Some(true) {
                return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
            }

            let Some(response_content) =
                table_val.get("responseContent").and_then(|v| v.as_array())
            else {
                return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
            };

            // 5. Filter and process rows
            let rows =
                crate::infrastructure::reference_source::bot_rows::bot_rows(response_content)?;

            Ok(ReferenceRates {
                source: BOT_SOURCE_PAGE.to_string(),
                fetched_at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
                kind: "DAILY_COMMERCIAL_BANK_AVERAGE".to_string(),
                rows,
            })
        })
    }
}
