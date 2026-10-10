//! `BrokerAttributionHandler` (RA-2 #980, design `docs/design/blueprint/broker-seam.md` §5.4): the
//! outbox handler that sends a captured referral code to Broker's `POST /api/v1/attributions`.
//!
//! Scaffold written by the ticket author: `deliver` and `on_final` are `todo!()`.

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_outbox::domain::event::OutboxEvent;
use xc_outbox::domain::event_kind::EventKind;
use xc_outbox::domain::handler::{DeliveryResult, FinalOutcome, OutboxHandler};
use xc_outbox::domain::retry_policy::RetryPolicy;

use crate::domain::customer_keys::FeedCustomerKeys;
use crate::domain::events::BROKER_ATTRIBUTION;

/// Broker's product name for xchange (Broker §7).
pub const PRODUCT: &str = "xchange-fx";

pub struct BrokerAttributionHandler {
    pool: PgPool,
    base_url: String,
    service_token: String,
    keys: Arc<dyn FeedCustomerKeys>,
    client: reqwest::Client,
}

impl BrokerAttributionHandler {
    /// `base_url` without a trailing path (`https://broker.example`); the handler adds
    /// `/api/v1/attributions`. `keys` gives the verified phone of a capture's exchange (D-6).
    pub fn new(
        pool: PgPool,
        base_url: String,
        service_token: String,
        keys: Arc<dyn FeedCustomerKeys>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            pool,
            base_url,
            service_token,
            keys,
            client,
        }
    }
}

/// Never prints the token.
impl std::fmt::Debug for BrokerAttributionHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrokerAttributionHandler")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl OutboxHandler for BrokerAttributionHandler {
    fn kind(&self) -> &'static EventKind {
        &BROKER_ATTRIBUTION
    }

    fn retry_policy(&self) -> RetryPolicy {
        RetryPolicy::default()
    }

    /// Read the capture named by `event.aggregate_id`, then send once (10 s timeout) with
    /// `Authorization: Bearer <token>` and `Idempotency-Key: <event.idempotency_key>`.
    fn deliver<'a>(&'a self, event: &'a OutboxEvent) -> BoxFuture<'a, DeliveryResult> {
        Box::pin(async move {
            let row = match sqlx::query(
                "SELECT code, customer_sub, transaction_id \
                 FROM referral_attributions \
                 WHERE id = $1::uuid",
            )
            .bind(&event.aggregate_id)
            .fetch_optional(&self.pool)
            .await
            {
                Ok(Some(r)) => r,
                Ok(None) => {
                    return DeliveryResult::Final(FinalOutcome::Refused {
                        http_status: 404,
                        error: "capture not found".to_string(),
                    });
                }
                Err(e) => {
                    return DeliveryResult::Retry {
                        http_status: None,
                        error: format!("database error: {e}"),
                    };
                }
            };

            let transaction_id: Option<Uuid> = row.get("transaction_id");
            let phone = if let Some(tx_id) = transaction_id {
                let mut tx = match self.pool.begin().await {
                    Ok(tx) => tx,
                    Err(e) => {
                        return DeliveryResult::Retry {
                            http_status: None,
                            error: format!("database error: {e}"),
                        };
                    }
                };
                let keys = match self.keys.keys_in(&mut tx, tx_id).await {
                    Ok(keys) => keys,
                    Err(e) => {
                        return DeliveryResult::Retry {
                            http_status: None,
                            error: format!("keys error: {e}"),
                        };
                    }
                };
                let _ = tx.commit().await;
                keys.phone
            } else {
                None
            };

            let code: String = row.get("code");
            let customer_sub: Option<String> = row.get("customer_sub");

            let mut body = serde_json::json!({
                "code": code,
                "product": PRODUCT,
                "customerSub": customer_sub,
            });
            if let Some(phone) = phone {
                body["phone"] = serde_json::Value::String(phone);
            }

            let url = format!(
                "{}/api/v1/attributions",
                self.base_url.trim_end_matches('/')
            );
            let res = match self
                .client
                .post(&url)
                .header(
                    reqwest::header::AUTHORIZATION,
                    format!("Bearer {}", self.service_token),
                )
                .header("Idempotency-Key", &event.idempotency_key)
                .json(&body)
                .send()
                .await
            {
                Ok(res) => res,
                Err(e) => {
                    let err_str =
                        format!("request error: {e}").replace(&self.service_token, "[REDACTED]");
                    return DeliveryResult::Retry {
                        http_status: None,
                        error: err_str,
                    };
                }
            };

            let status = res.status().as_u16();
            let body_text = res.text().await.unwrap_or_default();
            let safe_body = body_text.replace(&self.service_token, "[REDACTED]");

            match status {
                201 => DeliveryResult::Final(FinalOutcome::Delivered { http_status: 201 }),
                200 => DeliveryResult::Final(FinalOutcome::Delivered { http_status: 200 }),
                202..=299 => DeliveryResult::Final(FinalOutcome::Delivered {
                    http_status: status,
                }),
                401 | 408 | 429 => DeliveryResult::Retry {
                    http_status: Some(status),
                    error: format!("HTTP {status}: {safe_body}"),
                },
                500..=599 => DeliveryResult::Retry {
                    http_status: Some(status),
                    error: format!("HTTP {status}: {safe_body}"),
                },
                404 => DeliveryResult::Final(FinalOutcome::Refused {
                    http_status: 404,
                    error: "unknown_code".to_string(),
                }),
                409 => {
                    let error_val = serde_json::from_str::<serde_json::Value>(&body_text)
                        .ok()
                        .and_then(|v| {
                            v.get("error")
                                .and_then(|e| e.as_str().map(|s| s.to_string()))
                        })
                        .unwrap_or(safe_body);
                    DeliveryResult::Final(FinalOutcome::Refused {
                        http_status: 409,
                        error: error_val,
                    })
                }
                400..=499 => DeliveryResult::Final(FinalOutcome::Refused {
                    http_status: status,
                    error: safe_body,
                }),
                _ => DeliveryResult::Retry {
                    http_status: Some(status),
                    error: format!("HTTP {status}: {safe_body}"),
                },
            }
        })
    }

    /// Write `referral_attributions.broker_outcome` for the final answer (design §5.4 table).
    fn on_final<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        event: &'a OutboxEvent,
        outcome: &'a FinalOutcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let outcome_str = match outcome {
                FinalOutcome::Delivered { http_status: 201 } => "CREATED",
                FinalOutcome::Delivered { http_status: 200 } => "EXISTING",
                FinalOutcome::Delivered { .. } => "CREATED",
                FinalOutcome::Refused {
                    http_status: 404, ..
                } => "UNKNOWN_CODE",
                FinalOutcome::Refused {
                    http_status: 409,
                    error,
                } => match error.as_str() {
                    "self_referral" => "SELF_REFERRAL",
                    "held_by_other_broker" => "HELD_BY_OTHER",
                    _ => "REFUSED",
                },
                FinalOutcome::Refused { .. } => "REFUSED",
                FinalOutcome::Expired => "EXPIRED",
            };

            sqlx::query(
                "UPDATE referral_attributions \
                 SET broker_outcome = $1 \
                 WHERE id = $2::uuid",
            )
            .bind(outcome_str)
            .bind(&event.aggregate_id)
            .execute(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            Ok(())
        })
    }
}
