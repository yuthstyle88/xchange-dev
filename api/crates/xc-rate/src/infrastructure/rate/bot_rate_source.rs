//! BoT rate source implementation.

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;

use crate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};

pub const BOT_BASE: &str = "https://www.bot.or.th/content/bot/th/statistics/exchange-rate/jcr:content/root/container/statisticstable2";
pub const BOT_SOURCE_PAGE: &str = "https://www.bot.or.th/th/statistics/exchange-rate.html";
pub const BOT_TIMEOUT: Duration = Duration::from_secs(10);
pub const BOT_UNAVAILABLE: &str = "ไม่สามารถดึงเรทล่าสุดจาก ธปท. ได้ กรุณาลองใหม่ก่อนบันทึกเรท";

const ALLOWED_CURRENCIES: [&str; 6] = ["USD", "EUR", "GBP", "JPY", "CNY", "AUD"];

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

/// A rate as the bank writes it: digits, optionally one dot with digits on
/// both sides. `Decimal::from_str` alone also takes `33_42` (as 3342), a sign,
/// an exponent, `33.` and `.5`; none of those is repaired here.
fn is_plain_decimal(text: &str) -> bool {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    match text.split_once('.') {
        Some((whole, fraction)) => digits(whole) && digits(fraction),
        None => digits(text),
    }
}

fn parse_rate(val: Option<&serde_json::Value>) -> Option<Decimal> {
    let text = match val? {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => return None,
    };
    if !is_plain_decimal(&text) {
        return None;
    }
    let d = Decimal::from_str(&text).ok()?;
    if d <= Decimal::ZERO {
        return None;
    }
    Some(d)
}

fn decimal_to_number(d: Decimal) -> serde_json::Number {
    let normalized = d.normalize();
    serde_json::from_str::<serde_json::Number>(&normalized.to_string())
        .expect("normalized decimal is valid json number")
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
            let mut kept_rows = Vec::new();
            for row_val in response_content {
                let Some(currency_id) = row_val.get("currency_id").and_then(|v| v.as_str()) else {
                    continue;
                };
                if !ALLOWED_CURRENCIES.contains(&currency_id) {
                    continue;
                }

                let Some(period) = row_val.get("period").and_then(|v| v.as_str()) else {
                    return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
                };
                if period.len() != 10 {
                    return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
                }
                let bytes = period.as_bytes();
                let is_date_shape = bytes.iter().enumerate().all(|(i, &b)| {
                    if i == 4 || i == 7 {
                        b == b'-'
                    } else {
                        b.is_ascii_digit()
                    }
                });
                if !is_date_shape {
                    return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
                }

                let bs_val = row_val.get("buying_sight");
                let bt_val = row_val.get("buying_transfer");
                let selling_val = row_val.get("selling");

                let (Some(bs_dec), Some(bt_dec), Some(selling_dec)) = (
                    parse_rate(bs_val),
                    parse_rate(bt_val),
                    parse_rate(selling_val),
                ) else {
                    return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
                };

                let unit: u64 = if currency_id == "JPY" { 100 } else { 1 };
                let unit_dec = Decimal::from(unit);

                let bs_rate = decimal_to_number(bs_dec / unit_dec);
                let bt_rate = decimal_to_number(bt_dec / unit_dec);
                let selling_rate = decimal_to_number(selling_dec / unit_dec);

                let original = serde_json::json!({
                    "buyingSight": bs_val.unwrap().clone(),
                    "buyingTransfer": bt_val.unwrap().clone(),
                    "selling": selling_val.unwrap().clone(),
                });

                let row = serde_json::json!({
                    "code": currency_id,
                    "date": period,
                    "unit": unit,
                    "buyingSight": bs_rate,
                    "buyingTransfer": bt_rate,
                    "selling": selling_rate,
                    "original": original,
                });

                kept_rows.push((currency_id.to_string(), row));
            }

            if kept_rows.len() != 6 {
                return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
            }

            let mut seen_currencies = HashSet::new();
            for (code, _) in &kept_rows {
                if !seen_currencies.insert(code) {
                    return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
                }
            }
            if seen_currencies.len() != 6 {
                return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
            }

            let rows: Vec<serde_json::Value> = kept_rows.into_iter().map(|(_, r)| r).collect();

            Ok(ReferenceRates {
                source: BOT_SOURCE_PAGE.to_string(),
                fetched_at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
                kind: "DAILY_COMMERCIAL_BANK_AVERAGE".to_string(),
                rows,
            })
        })
    }
}
