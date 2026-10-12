//! `LoyaltyExchangeHandler` (design §6, D-4): OutboxHandler for loyalty events.

use std::sync::Arc;

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_outbox::domain::event::OutboxEvent;
use xc_outbox::domain::event_kind::EventKind;
use xc_outbox::domain::handler::{DeliveryResult, FinalOutcome, OutboxHandler};
use xc_outbox::domain::retry_policy::RetryPolicy;

use crate::domain::events::{LOYALTY_COMPLETED, LOYALTY_REVERSED};
use crate::domain::facts_source::LoyaltyFactsSource;
use crate::domain::wire_body::LoyaltyWireBody;
use crate::infrastructure::config::LoyaltyConfig;
use crate::infrastructure::http_client::LoyaltyHttp;

/// Seam to read the verified phone for a customer without coupling directly to DB details.
pub trait LoyaltyPhoneReader: Send + Sync {
    fn verified_phone<'a>(
        &'a self,
        organization_id: Uuid,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<String>>>;
}

pub struct LoyaltyExchangeHandler {
    kind: &'static EventKind,
    config: LoyaltyConfig,
    http: Arc<LoyaltyHttp>,
    facts_source: Arc<dyn LoyaltyFactsSource>,
    phone_reader: Arc<dyn LoyaltyPhoneReader>,
}

impl LoyaltyExchangeHandler {
    pub fn new(
        kind: &'static EventKind,
        config: LoyaltyConfig,
        http: Arc<LoyaltyHttp>,
        facts_source: Arc<dyn LoyaltyFactsSource>,
        phone_reader: Arc<dyn LoyaltyPhoneReader>,
    ) -> Self {
        Self {
            kind,
            config,
            http,
            facts_source,
            phone_reader,
        }
    }
}

impl std::fmt::Debug for LoyaltyExchangeHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoyaltyExchangeHandler")
            .field("kind", &self.kind.event_type)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl OutboxHandler for LoyaltyExchangeHandler {
    fn kind(&self) -> &'static EventKind {
        self.kind
    }

    fn retry_policy(&self) -> RetryPolicy {
        RetryPolicy::default()
    }

    fn deliver<'a>(&'a self, event: &'a OutboxEvent) -> BoxFuture<'a, DeliveryResult> {
        Box::pin(async move {
            let event_id = match Uuid::parse_str(&event.id) {
                Ok(id) => id,
                Err(e) => {
                    return DeliveryResult::Final(FinalOutcome::Refused {
                        http_status: 400,
                        error: format!("invalid event id: {e}"),
                    });
                }
            };

            if event.event_type == LOYALTY_COMPLETED.event_type {
                let tx_id = event
                    .payload_redacted
                    .get("transactionId")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .or_else(|| {
                        event
                            .idempotency_key
                            .strip_prefix("exchange:")
                            .and_then(|s| Uuid::parse_str(s).ok())
                    });

                let tx_id = match tx_id {
                    Some(id) => id,
                    None => {
                        return DeliveryResult::Final(FinalOutcome::Refused {
                            http_status: 400,
                            error: "missing or invalid transactionId in payload".to_string(),
                        });
                    }
                };

                let facts = match self.facts_source.completed(tx_id).await {
                    Ok(f) => f,
                    Err(e) => {
                        return DeliveryResult::Retry {
                            http_status: None,
                            error: e.to_string(),
                        };
                    }
                };

                let customer_id = match facts.customer_id.as_deref() {
                    Some(c) if !c.trim().is_empty() => c.trim(),
                    _ => {
                        return DeliveryResult::Final(FinalOutcome::Refused {
                            http_status: 400,
                            error: "noVerifiedPhone: no customer on transaction".to_string(),
                        });
                    }
                };

                let phone = match self
                    .phone_reader
                    .verified_phone(facts.organization_id, customer_id)
                    .await
                {
                    Ok(Some(p)) => p,
                    Ok(None) => {
                        return DeliveryResult::Final(FinalOutcome::Refused {
                            http_status: 400,
                            error: "noVerifiedPhone: customer has no active verified phone"
                                .to_string(),
                        });
                    }
                    Err(e) => {
                        return DeliveryResult::Retry {
                            http_status: None,
                            error: e.to_string(),
                        };
                    }
                };

                let wire_body =
                    LoyaltyWireBody::completed(event_id, &facts, self.config.tenant_id, &phone);

                match self
                    .http
                    .post_event(event_id, &event.idempotency_key, &wire_body)
                    .await
                {
                    Ok(status) if status.is_success() => {
                        DeliveryResult::Final(FinalOutcome::Delivered {
                            http_status: status.as_u16(),
                        })
                    }
                    Ok(status) if status.as_u16() == 400 => {
                        DeliveryResult::Final(FinalOutcome::Refused {
                            http_status: 400,
                            error: "400 Bad Request".to_string(),
                        })
                    }
                    Ok(status) => DeliveryResult::Retry {
                        http_status: Some(status.as_u16()),
                        error: format!("HTTP {}", status.as_u16()),
                    },
                    Err(e) => DeliveryResult::Retry {
                        http_status: None,
                        error: e.to_string(),
                    },
                }
            } else if event.event_type == LOYALTY_REVERSED.event_type {
                let rev_id = event
                    .payload_redacted
                    .get("reversalId")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok())
                    .or_else(|| {
                        event
                            .idempotency_key
                            .strip_prefix("reversal:")
                            .and_then(|s| Uuid::parse_str(s).ok())
                    });

                let rev_id = match rev_id {
                    Some(id) => id,
                    None => {
                        return DeliveryResult::Final(FinalOutcome::Refused {
                            http_status: 400,
                            error: "missing or invalid reversalId in payload".to_string(),
                        });
                    }
                };

                let facts = match self.facts_source.reversed(rev_id).await {
                    Ok(f) => f,
                    Err(e) => {
                        return DeliveryResult::Retry {
                            http_status: None,
                            error: e.to_string(),
                        };
                    }
                };

                let customer_id = match facts.customer_id.as_deref() {
                    Some(c) if !c.trim().is_empty() => c.trim(),
                    _ => {
                        return DeliveryResult::Final(FinalOutcome::Refused {
                            http_status: 400,
                            error: "noVerifiedPhone: no customer on reversal".to_string(),
                        });
                    }
                };

                let phone = match self
                    .phone_reader
                    .verified_phone(facts.organization_id, customer_id)
                    .await
                {
                    Ok(Some(p)) => p,
                    Ok(None) => {
                        return DeliveryResult::Final(FinalOutcome::Refused {
                            http_status: 400,
                            error: "noVerifiedPhone: customer has no active verified phone"
                                .to_string(),
                        });
                    }
                    Err(e) => {
                        return DeliveryResult::Retry {
                            http_status: None,
                            error: e.to_string(),
                        };
                    }
                };

                let wire_body =
                    LoyaltyWireBody::reversed(event_id, &facts, self.config.tenant_id, &phone);

                match self
                    .http
                    .post_event(event_id, &event.idempotency_key, &wire_body)
                    .await
                {
                    Ok(status) if status.is_success() => {
                        DeliveryResult::Final(FinalOutcome::Delivered {
                            http_status: status.as_u16(),
                        })
                    }
                    Ok(status) if status.as_u16() == 400 => {
                        DeliveryResult::Final(FinalOutcome::Refused {
                            http_status: 400,
                            error: "400 Bad Request".to_string(),
                        })
                    }
                    Ok(status) => DeliveryResult::Retry {
                        http_status: Some(status.as_u16()),
                        error: format!("HTTP {}", status.as_u16()),
                    },
                    Err(e) => DeliveryResult::Retry {
                        http_status: None,
                        error: e.to_string(),
                    },
                }
            } else {
                DeliveryResult::Final(FinalOutcome::Refused {
                    http_status: 400,
                    error: format!("unsupported event type: {}", event.event_type),
                })
            }
        })
    }

    fn on_final<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        event: &'a OutboxEvent,
        outcome: &'a FinalOutcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            match outcome {
                FinalOutcome::Delivered { http_status } => {
                    tracing::info!(
                        event_id = %event.id,
                        event_type = %event.event_type,
                        http_status,
                        "loyalty event delivered"
                    );
                }
                FinalOutcome::Refused { http_status, error } => {
                    tracing::warn!(
                        event_id = %event.id,
                        event_type = %event.event_type,
                        http_status,
                        error = %error,
                        "loyalty event refused"
                    );
                }
                FinalOutcome::Expired => {
                    tracing::warn!(
                        event_id = %event.id,
                        event_type = %event.event_type,
                        "loyalty event expired"
                    );
                }
            }
            Ok(())
        })
    }
}
