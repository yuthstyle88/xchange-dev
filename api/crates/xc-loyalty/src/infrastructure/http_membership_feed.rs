//! `HttpMembershipFeed` (design §4.5, §6): HTTP client implementing `MembershipFeed`.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use uuid::Uuid;

use crate::domain::membership_feed::{MembershipChange, MembershipFeed, MembershipPage};

#[derive(Clone)]
pub struct HttpMembershipFeed {
    client: reqwest::Client,
    base_url: String,
    bearer_token: Option<String>,
}

impl HttpMembershipFeed {
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

impl MembershipFeed for HttpMembershipFeed {
    fn changes_after(&self, seq: i64) -> BoxFuture<'_, AppResult<MembershipPage>> {
        Box::pin(async move {
            let url = format!(
                "{}/api/v1/loyalty/memberships/changes?after={}&limit=500",
                self.base_url.trim_end_matches('/'),
                seq
            );

            let mut req = self.client.get(&url);
            if let Some(token) = &self.bearer_token {
                req = req.header("Authorization", format!("Bearer {token}"));
            }

            let resp = req
                .send()
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?;

            if !resp.status().is_success() {
                return Err(AppError::Internal(format!(
                    "loyalty feed answered {}",
                    resp.status()
                )));
            }

            let val: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?;

            let changed = val
                .get("changed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let next = val.get("next").and_then(|v| v.as_i64()).unwrap_or(0);
            let mut items = Vec::new();

            if let Some(items_val) = val.get("items").and_then(|v| v.as_array()) {
                for item in items_val {
                    let phone_key = item
                        .get("phoneKey")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned();
                    let customer_ref = item
                        .get("customerRef")
                        .and_then(|v| v.as_str())
                        .and_then(|s| Uuid::parse_str(s).ok());
                    let tier_code = item
                        .get("tierCode")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned();
                    let tier_rule_id = item
                        .get("tierRuleId")
                        .and_then(|v| v.as_str())
                        .and_then(|s| Uuid::parse_str(s).ok())
                        .unwrap_or_default();
                    let tier_rule_version = item
                        .get("tierRuleVersion")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0) as i32;
                    let effective_at = item
                        .get("effectiveAt")
                        .and_then(|v| v.as_str())
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_default();
                    let effective_until = item
                        .get("effectiveUntil")
                        .and_then(|v| v.as_str())
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.with_timezone(&Utc));
                    let seq = item.get("seq").and_then(|v| v.as_i64()).unwrap_or(0);

                    items.push(MembershipChange {
                        phone_key,
                        customer_ref,
                        tier_code,
                        tier_rule_id,
                        tier_rule_version,
                        effective_at,
                        effective_until,
                        seq,
                    });
                }
            }

            Ok(MembershipPage {
                changed,
                items,
                next,
            })
        })
    }
}
