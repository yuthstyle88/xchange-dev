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

            page_from(&val)
        })
    }
}

fn item_from(item: &serde_json::Value) -> AppResult<MembershipChange> {
    let phone_key = item
        .get("phoneKey")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal("loyalty feed: phoneKey missing or not a string".into()))?
        .to_owned();

    let customer_ref =
        match item.get("customerRef") {
            Some(v) if !v.is_null() => {
                let s = v.as_str().ok_or_else(|| {
                    AppError::Internal("loyalty feed: customerRef present but not a string".into())
                })?;
                Some(Uuid::parse_str(s).map_err(|_| {
                    AppError::Internal("loyalty feed: customerRef invalid uuid".into())
                })?)
            }
            _ => None,
        };

    let tier_code = item
        .get("tierCode")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Internal("loyalty feed: tierCode missing or not a string".into()))?
        .to_owned();

    let tier_rule_id = {
        let s = item
            .get("tierRuleId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AppError::Internal("loyalty feed: tierRuleId missing or not a string".into())
            })?;
        Uuid::parse_str(s)
            .map_err(|_| AppError::Internal("loyalty feed: tierRuleId invalid uuid".into()))?
    };

    let tier_rule_version = {
        let v = item
            .get("tierRuleVersion")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                AppError::Internal("loyalty feed: tierRuleVersion missing or not an integer".into())
            })?;
        i32::try_from(v)
            .map_err(|_| AppError::Internal("loyalty feed: tierRuleVersion out of range".into()))?
    };

    let effective_at = {
        let s = item
            .get("effectiveAt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AppError::Internal("loyalty feed: effectiveAt missing or not a string".into())
            })?;
        DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|_| AppError::Internal("loyalty feed: effectiveAt invalid RFC 3339".into()))?
    };

    let effective_until = match item.get("effectiveUntil") {
        Some(v) if !v.is_null() => {
            let s = v.as_str().ok_or_else(|| {
                AppError::Internal("loyalty feed: effectiveUntil present but not a string".into())
            })?;
            Some(
                DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|_| {
                        AppError::Internal("loyalty feed: effectiveUntil invalid RFC 3339".into())
                    })?,
            )
        }
        _ => None,
    };

    let seq = item
        .get("seq")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| AppError::Internal("loyalty feed: seq missing or not an integer".into()))?;

    Ok(MembershipChange {
        phone_key,
        customer_ref,
        tier_code,
        tier_rule_id,
        tier_rule_version,
        effective_at,
        effective_until,
        seq,
    })
}

fn page_from(val: &serde_json::Value) -> AppResult<MembershipPage> {
    let changed = val
        .get("changed")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| AppError::Internal("loyalty feed: changed must be a bool".into()))?;

    let next = val
        .get("next")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| AppError::Internal("loyalty feed: next must be an integer".into()))?;

    let mut items = Vec::new();

    if changed {
        let items_val = val
            .get("items")
            .and_then(|v| v.as_array())
            .ok_or_else(|| AppError::Internal("loyalty feed: items must be an array".into()))?;

        for item in items_val {
            items.push(item_from(item)?);
        }
    } else if let Some(items_val) = val.get("items").and_then(|v| v.as_array()) {
        for item in items_val {
            items.push(item_from(item)?);
        }
    }

    Ok(MembershipPage {
        changed,
        items,
        next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn sample_item() -> serde_json::Value {
        json!({
            "phoneKey": "+66812345678",
            "customerRef": null,
            "tierCode": "GOLD",
            "tierRuleId": "11111111-1111-1111-1111-111111111111",
            "tierRuleVersion": 1,
            "effectiveAt": "2026-10-16T12:00:00Z",
            "seq": 100
        })
    }

    #[test]
    fn f1_full_item_with_null_customer_ref_and_no_effective_until_succeeds() {
        let val = json!({
            "changed": true,
            "next": 42,
            "items": [sample_item()]
        });
        let page = page_from(&val).expect("F1 must succeed");
        assert!(page.changed);
        assert_eq!(page.next, 42);
        assert_eq!(page.items.len(), 1);
        let item = &page.items[0];
        assert_eq!(item.phone_key, "+66812345678");
        assert_eq!(item.customer_ref, None);
        assert_eq!(item.tier_code, "GOLD");
        assert_eq!(
            item.tier_rule_id,
            Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
        );
        assert_eq!(item.tier_rule_version, 1);
        assert_eq!(
            item.effective_at,
            Utc.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap()
        );
        assert_eq!(item.effective_until, None);
        assert_eq!(item.seq, 100);
    }

    #[test]
    fn f2_missing_next_returns_err_naming_next() {
        let val = json!({
            "changed": true,
            "items": [sample_item()]
        });
        let err = page_from(&val).expect_err("F2 must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("next"),
            "error message should name 'next': {msg}"
        );
    }

    #[test]
    fn f3_non_integer_next_returns_err_naming_next() {
        let val = json!({
            "changed": false,
            "next": "42"
        });
        let err = page_from(&val).expect_err("F3 must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("next"),
            "error message should name 'next': {msg}"
        );
    }

    #[test]
    fn f4_item_without_tier_rule_id_returns_err_naming_tier_rule_id() {
        let mut item = sample_item();
        item.as_object_mut().unwrap().remove("tierRuleId");
        let val = json!({
            "changed": true,
            "next": 42,
            "items": [item]
        });
        let err = page_from(&val).expect_err("F4 must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("tierRuleId"),
            "error message should name 'tierRuleId': {msg}"
        );
    }

    #[test]
    fn f5_item_with_invalid_effective_at_returns_err_naming_effective_at() {
        let mut item = sample_item();
        item["effectiveAt"] = json!("yesterday");
        let val = json!({
            "changed": true,
            "next": 42,
            "items": [item]
        });
        let err = page_from(&val).expect_err("F5 must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("effectiveAt"),
            "error message should name 'effectiveAt': {msg}"
        );
    }

    #[test]
    fn f6_item_without_seq_returns_err_naming_seq() {
        let mut item = sample_item();
        item.as_object_mut().unwrap().remove("seq");
        let val = json!({
            "changed": true,
            "next": 42,
            "items": [item]
        });
        let err = page_from(&val).expect_err("F6 must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("seq"),
            "error message should name 'seq': {msg}"
        );
    }

    #[test]
    fn f7_unchanged_page_without_items_returns_ok_with_next() {
        let val = json!({
            "changed": false,
            "next": 42
        });
        let page = page_from(&val).expect("F7 must succeed");
        assert!(!page.changed);
        assert_eq!(page.next, 42);
        assert!(page.items.is_empty());
    }

    #[test]
    fn f8_tier_rule_version_at_i32_max_succeeds() {
        let mut item = sample_item();
        item["tierRuleVersion"] = json!(2147483647i64);
        let val = json!({
            "changed": true,
            "next": 42,
            "items": [item]
        });
        let page = page_from(&val).expect("2147483647 must succeed");
        assert_eq!(page.items[0].tier_rule_version, 2147483647);
    }

    #[test]
    fn f9_tier_rule_version_overflow_returns_err() {
        let mut item = sample_item();
        item["tierRuleVersion"] = json!(2147483648i64);
        let val = json!({
            "changed": true,
            "next": 42,
            "items": [item]
        });
        let err = page_from(&val).expect_err("2147483648 must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("tierRuleVersion out of range"),
            "error message should name 'tierRuleVersion out of range': {msg}"
        );
    }
}
