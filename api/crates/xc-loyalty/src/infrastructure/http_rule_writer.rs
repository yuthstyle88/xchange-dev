//! `HttpLoyaltyRuleWriter` (LY-6 #1411, design §6, §6.4 R3, R4): HTTP implementation of `LoyaltyRuleWriter`.

use std::time::Duration;

use futures::future::BoxFuture;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::application::rule_publisher::{
    LoyaltyRuleError, LoyaltyRuleWriter, RuleKind, RuleVersion, prepare_draft_fields,
};
use crate::infrastructure::config::LoyaltyConfig;

#[derive(Clone)]
pub struct HttpLoyaltyRuleWriter {
    client: reqwest::Client,
    base_url: String,
    service_token: String,
    timeout: Duration,
}

impl HttpLoyaltyRuleWriter {
    /// Builds `HttpLoyaltyRuleWriter` from `LoyaltyConfig`.
    /// Returns `None` when `config.service_token` is unset or blank (R3).
    pub fn from_config(config: &LoyaltyConfig) -> Option<Self> {
        let token = config
            .service_token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .ok()?;
        Some(Self {
            client,
            base_url: config.base_url.trim_end_matches('/').to_owned(),
            service_token: token.to_owned(),
            timeout: config.timeout,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.service_token)
    }

    fn collection_url(&self, kind: RuleKind) -> String {
        format!("{}/api/v1/loyalty/{}", self.base_url, kind.route_segment())
    }

    fn action_url(&self, kind: RuleKind, id: Uuid, action: &str) -> String {
        format!(
            "{}/api/v1/loyalty/{}/{id}/{action}",
            self.base_url,
            kind.route_segment()
        )
    }

    async fn send_mutation(
        &self,
        kind: RuleKind,
        url: String,
        body: Value,
        map_err: fn(u16, &Value) -> LoyaltyRuleError,
    ) -> Result<RuleVersion, LoyaltyRuleError> {
        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.bearer())
            .json(&body)
            .send()
            .await
            .map_err(|e| LoyaltyRuleError::Unavailable(e.to_string()))?;

        let status = resp.status();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| LoyaltyRuleError::Unavailable(e.to_string()))?;
        let payload: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

        if status.is_success() {
            return RuleVersion::from_dto(kind, payload);
        }

        Err(map_err(status.as_u16(), &payload))
    }
}

impl std::fmt::Debug for HttpLoyaltyRuleWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpLoyaltyRuleWriter")
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

fn error_message(status: u16, payload: &Value) -> String {
    payload
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("loyalty answered HTTP {status}"))
}

fn map_error_status(status: u16, payload: &Value) -> LoyaltyRuleError {
    let msg = error_message(status, payload);
    match status {
        400 => LoyaltyRuleError::Invalid(msg),
        404 => LoyaltyRuleError::NotFound,
        422 => LoyaltyRuleError::InvalidTransition(msg),
        _ => LoyaltyRuleError::Unavailable(msg),
    }
}

fn map_create_error(status: u16, payload: &Value) -> LoyaltyRuleError {
    match status {
        409 => LoyaltyRuleError::InvalidTransition(error_message(status, payload)),
        _ => map_error_status(status, payload),
    }
}

impl LoyaltyRuleWriter for HttpLoyaltyRuleWriter {
    fn create_draft<'a>(
        &'a self,
        kind: RuleKind,
        fields: &'a Map<String, Value>,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>> {
        Box::pin(async move {
            let cleaned = prepare_draft_fields(kind, fields);
            let url = self.collection_url(kind);
            self.send_mutation(kind, url, Value::Object(cleaned), map_create_error)
                .await
        })
    }

    fn submit<'a>(
        &'a self,
        kind: RuleKind,
        id: Uuid,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>> {
        Box::pin(async move {
            let url = self.action_url(kind, id, "submit");
            self.send_mutation(kind, url, json!({}), map_error_status)
                .await
        })
    }

    fn approve<'a>(
        &'a self,
        kind: RuleKind,
        id: Uuid,
        approved_ref: &'a str,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>> {
        Box::pin(async move {
            let url = self.action_url(kind, id, "approve");
            self.send_mutation(
                kind,
                url,
                json!({ "approvedRef": approved_ref }),
                map_error_status,
            )
            .await
        })
    }

    fn reject<'a>(
        &'a self,
        kind: RuleKind,
        id: Uuid,
        reason: &'a str,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>> {
        Box::pin(async move {
            let url = self.action_url(kind, id, "reject");
            self.send_mutation(kind, url, json!({ "reason": reason }), map_error_status)
                .await
        })
    }

    fn get<'a>(
        &'a self,
        kind: RuleKind,
        id: Uuid,
    ) -> BoxFuture<'a, Result<Option<RuleVersion>, LoyaltyRuleError>> {
        Box::pin(async move {
            let url = self.collection_url(kind);
            let resp = self
                .client
                .get(&url)
                .header("Authorization", self.bearer())
                .send()
                .await
                .map_err(|e| LoyaltyRuleError::Unavailable(e.to_string()))?;

            let status = resp.status();
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| LoyaltyRuleError::Unavailable(e.to_string()))?;
            let payload: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

            if !status.is_success() {
                if status.as_u16() == 404 {
                    return Ok(None);
                }
                return Err(map_error_status(status.as_u16(), &payload));
            }

            let Some(items) = payload.as_array() else {
                return Err(LoyaltyRuleError::Invalid(
                    "loyalty rule list response is not a JSON array".to_owned(),
                ));
            };

            for item in items {
                let item_id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|s| Uuid::parse_str(s).ok());
                if item_id == Some(id) {
                    return RuleVersion::from_dto(kind, item.clone()).map(Some);
                }
            }

            Ok(None)
        })
    }

    fn list<'a>(
        &'a self,
        kind: RuleKind,
    ) -> BoxFuture<'a, Result<Vec<RuleVersion>, LoyaltyRuleError>> {
        Box::pin(async move {
            let url = self.collection_url(kind);
            let resp = self
                .client
                .get(&url)
                .header("Authorization", self.bearer())
                .send()
                .await
                .map_err(|e| LoyaltyRuleError::Unavailable(e.to_string()))?;

            let status = resp.status();
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| LoyaltyRuleError::Unavailable(e.to_string()))?;
            let payload: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

            if !status.is_success() {
                return Err(LoyaltyRuleError::Unavailable(error_message(
                    status.as_u16(),
                    &payload,
                )));
            }

            let Some(items) = payload.as_array() else {
                return Err(LoyaltyRuleError::Invalid(
                    "loyalty rule list response is not a JSON array".to_owned(),
                ));
            };

            items
                .iter()
                .map(|item| RuleVersion::from_dto(kind, item.clone()))
                .collect()
        })
    }
}
