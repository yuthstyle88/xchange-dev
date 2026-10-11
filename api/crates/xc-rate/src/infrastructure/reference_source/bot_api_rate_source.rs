//! `BotApiRateSource`: BoT reference rates from the official BoT API (design §8.6).

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, FixedOffset, SecondsFormat, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use tokio::sync::Mutex;

use crate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use crate::infrastructure::rate::bot_rate_source::{BOT_UNAVAILABLE, Clock};
use crate::infrastructure::reference_source::bot_rows::bot_rows;

/// `[proposed]` How long one upstream answer is kept (design §8.6).
pub const BOT_API_MIN_INTERVAL: Duration = Duration::from_secs(15 * 60);

const ALLOWED_CURRENCIES: [&str; 6] = ["USD", "EUR", "GBP", "JPY", "CNY", "AUD"];

/// Where and how to call the API. `Debug` must never print the token.
#[derive(Clone)]
pub struct BotApiConfig {
    pub base: String,
    pub token: String,
}

impl std::fmt::Debug for BotApiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BotApiConfig")
            .field("base", &self.base)
            .field("token", &"<redacted>")
            .finish()
    }
}

pub struct BotApiRateSource {
    config: BotApiConfig,
    timeout: Duration,
    clock: Clock,
    client: reqwest::Client,
    cache: Mutex<Option<(DateTime<Utc>, Result<ReferenceRates, String>)>>,
}

impl BotApiRateSource {
    pub fn new(config: BotApiConfig, timeout: Duration, clock: Clock) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_default();
        Self {
            config,
            timeout,
            clock,
            client,
            cache: Mutex::new(None),
        }
    }

    async fn fetch_cached(&self) -> AppResult<ReferenceRates> {
        let mut cache_guard = self.cache.lock().await;
        let now = (self.clock)();

        if let Some((cached_at, cached_res)) = cache_guard.as_ref() {
            let age = now.signed_duration_since(*cached_at);
            if age >= chrono::Duration::zero()
                && age < chrono::Duration::from_std(BOT_API_MIN_INTERVAL).unwrap()
            {
                return match cached_res {
                    Ok(rates) => Ok(rates.clone()),
                    Err(err) => Err(AppError::Domain(err.clone())),
                };
            }
        }

        let res = self.fetch_upstream(now).await;
        let cache_val = match &res {
            Ok(rates) => Ok(rates.clone()),
            Err(err) => Err(match err {
                AppError::Domain(s) => s.clone(),
                _ => BOT_UNAVAILABLE.to_string(),
            }),
        };
        *cache_guard = Some((now, cache_val));
        res
    }

    async fn fetch_upstream(&self, now: DateTime<Utc>) -> AppResult<ReferenceRates> {
        let bangkok = FixedOffset::east_opt(7 * 3600).expect("UTC+7");
        let bangkok_now = now.with_timezone(&bangkok);
        let end_period = bangkok_now.date_naive().format("%Y-%m-%d").to_string();
        let start_period = (bangkok_now.date_naive() - chrono::Duration::days(14))
            .format("%Y-%m-%d")
            .to_string();

        let base = self.config.base.trim_end_matches('/');
        let url = format!(
            "{base}/DAILY_AVG_EXG_RATE/?start_period={start_period}&end_period={end_period}"
        );

        let mut attempt = 0;
        let response = loop {
            attempt += 1;
            let req = self
                .client
                .get(&url)
                .header("Authorization", &self.config.token)
                .timeout(self.timeout);
            match req.send().await {
                Ok(res) => {
                    let status = res.status();
                    if status.is_success() {
                        break Ok(res);
                    } else if status.as_u16() == 401 || status.as_u16() == 403 {
                        tracing::error!(
                            status = status.as_u16(),
                            "BOT_API_AUTH_REJECTED: status {status}"
                        );
                        break Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
                    } else if status.as_u16() == 429 {
                        tracing::error!("BOT_API_RATE_LIMIT: status 429");
                        break Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
                    } else if status.is_client_error() {
                        tracing::warn!("BOT_API_CLIENT_ERROR: status {status}");
                        break Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
                    } else {
                        tracing::warn!("BOT_API_SERVER_ERROR: status {status}");
                        if attempt >= 2 {
                            break Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("BOT_API_REQUEST_ERROR: {e}");
                    if attempt >= 2 {
                        break Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
                    }
                }
            }
        };

        let res = response?;
        let bytes = res
            .bytes()
            .await
            .map_err(|_| AppError::Domain(BOT_UNAVAILABLE.to_string()))?;

        let body: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| AppError::Domain(BOT_UNAVAILABLE.to_string()))?;

        let Some(data_detail) = body
            .get("result")
            .and_then(|r| r.get("data"))
            .and_then(|d| d.get("data_detail"))
            .and_then(|v| v.as_array())
        else {
            return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
        };

        if data_detail.is_empty() {
            return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
        }

        let mut periods_map: BTreeMap<&str, Vec<&serde_json::Value>> = BTreeMap::new();
        for row in data_detail {
            if let Some(period) = row.get("period").and_then(|p| p.as_str()) {
                periods_map.entry(period).or_default().push(row);
            }
        }

        let mut chosen_period_rows = None;
        for (_period, rows) in periods_map.into_iter().rev() {
            let mut seen = HashSet::new();
            for r in &rows {
                if let Some(code) = r.get("currency_id").and_then(|c| c.as_str()) {
                    if ALLOWED_CURRENCIES.contains(&code) {
                        seen.insert(code);
                    }
                }
            }
            if seen.len() == 6 {
                chosen_period_rows = Some(rows);
                break;
            }
        }

        let Some(period_rows_refs) = chosen_period_rows else {
            return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
        };

        let period_rows: Vec<serde_json::Value> = period_rows_refs.into_iter().cloned().collect();
        let rows = bot_rows(&period_rows)?;

        Ok(ReferenceRates {
            source: self.config.base.clone(),
            fetched_at: now.to_rfc3339_opts(SecondsFormat::Millis, true),
            kind: "DAILY_COMMERCIAL_BANK_AVERAGE".to_string(),
            rows,
        })
    }
}

impl RateSource for BotApiRateSource {
    fn fetch(&self) -> BoxFuture<'_, AppResult<ReferenceRates>> {
        Box::pin(async move { self.fetch_cached().await })
    }
}
