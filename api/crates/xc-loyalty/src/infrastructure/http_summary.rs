//! `HttpLoyaltySummary` (design §4.5, §6): HTTP client implementing `LoyaltySummarySource`.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;

use crate::domain::summary_source::{
    LoyaltyMemberSummary, LoyaltySummarySource, NextExpiry, SummaryTier, TierProgress,
};

#[derive(Clone)]
pub struct HttpLoyaltySummary {
    client: reqwest::Client,
    base_url: String,
    bearer_token: Option<String>,
}

impl HttpLoyaltySummary {
    pub fn new(base_url: impl Into<String>, bearer_token: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            bearer_token,
        }
    }

    pub fn with_client(
        client: reqwest::Client,
        base_url: impl Into<String>,
        bearer_token: Option<String>,
    ) -> Self {
        Self {
            client,
            base_url: base_url.into(),
            bearer_token,
        }
    }
}

impl LoyaltySummarySource for HttpLoyaltySummary {
    fn summary<'a>(&'a self, phone: &'a str) -> BoxFuture<'a, AppResult<LoyaltyMemberSummary>> {
        Box::pin(async move {
            let url = format!(
                "{}/api/v1/loyalty/members/summary",
                self.base_url.trim_end_matches('/')
            );
            let body = serde_json::json!({ "phone": phone });

            let mut req = self.client.post(&url).json(&body);
            if let Some(token) = &self.bearer_token {
                req = req.header("Authorization", format!("Bearer {token}"));
            }

            let resp = req
                .send()
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?;

            if !resp.status().is_success() {
                return Err(AppError::Internal(format!(
                    "loyalty summary answered {}",
                    resp.status()
                )));
            }

            let val: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?;

            let phone_key = val
                .get("phoneKey")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let available = val.get("available").and_then(|v| v.as_i64()).unwrap_or(0);
            let pending = val.get("pending").and_then(|v| v.as_i64()).unwrap_or(0);

            let next_expiry = val.get("nextExpiry").and_then(|ne| {
                let points = ne.get("points")?.as_i64()?;
                let at_str = ne.get("at")?.as_str()?;
                let at = DateTime::parse_from_rfc3339(at_str)
                    .ok()?
                    .with_timezone(&Utc);
                Some(NextExpiry { points, at })
            });

            let tier = val.get("tier").and_then(|t| {
                let code = t.get("code")?.as_str()?.to_string();
                let effective_until = t
                    .get("effectiveUntil")
                    .and_then(|v| v.as_str())
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc));
                let progress = t.get("progress").and_then(|p| {
                    let basis = p.get("basis")?.as_str()?.to_string();
                    let current_str = p.get("current")?.as_str()?;
                    let current = Decimal::from_str(current_str).ok()?;
                    let next = p
                        .get("next")
                        .and_then(|v| v.as_str())
                        .and_then(|s| Decimal::from_str(s).ok());
                    Some(TierProgress {
                        basis,
                        current,
                        next,
                    })
                });
                Some(SummaryTier {
                    code,
                    effective_until,
                    progress,
                })
            });

            Ok(LoyaltyMemberSummary {
                phone_key,
                available,
                pending,
                next_expiry,
                tier,
            })
        })
    }
}
