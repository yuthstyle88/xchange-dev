//! AL-4 (#1009): rate alerts publication outbox handler.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_alert::application::evaluator::RateAlertEvaluator;
use xc_kernel::ids::BranchId;
use xc_outbox::domain::event::OutboxEvent;
use xc_outbox::domain::event_kind::EventKind;
use xc_outbox::domain::handler::{DeliveryResult, FinalOutcome, OutboxHandler};

use crate::bootstrap::AppState;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct RateAlertsHandler {
    evaluator: Arc<RateAlertEvaluator>,
}

impl RateAlertsHandler {
    pub fn new(evaluator: Arc<RateAlertEvaluator>) -> Self {
        Self { evaluator }
    }
}

impl OutboxHandler for RateAlertsHandler {
    fn kind(&self) -> &'static EventKind {
        &xc_rate::application::publication::alert_event::RATE_ALERTS_PUBLICATION
    }

    fn deliver<'a>(&'a self, event: &'a OutboxEvent) -> BoxFuture<'a, DeliveryResult> {
        Box::pin(async move {
            let branch_id = match event
                .payload_redacted
                .get("branchId")
                .and_then(|v| v.as_str())
            {
                Some(b) => b,
                None => {
                    return DeliveryResult::Final(FinalOutcome::Refused {
                        http_status: 400,
                        error: "missing branchId in payload".to_string(),
                    });
                }
            };
            let pub_id_str = event
                .payload_redacted
                .get("publicationId")
                .and_then(|v| v.as_str())
                .unwrap_or(&event.idempotency_key);
            let publication_id = match Uuid::parse_str(pub_id_str) {
                Ok(p) => p,
                Err(e) => {
                    return DeliveryResult::Final(FinalOutcome::Refused {
                        http_status: 400,
                        error: format!("invalid publicationId: {e}"),
                    });
                }
            };

            let branch = BranchId::new(branch_id);
            match self
                .evaluator
                .evaluate_publication(&branch, publication_id)
                .await
            {
                Ok(()) => DeliveryResult::Final(FinalOutcome::Delivered { http_status: 200 }),
                Err(e) => DeliveryResult::Retry {
                    http_status: None,
                    error: e.to_string(),
                },
            }
        })
    }

    fn on_final<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _event: &'a OutboxEvent,
        _outcome: &'a FinalOutcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move { Ok(()) })
    }
}

pub fn handler(state: &AppState) -> Option<Arc<dyn OutboxHandler>> {
    let registry =
        Arc::new(xc_notify::application::template_registry::TemplateRegistry::generated());
    let notifications = Arc::new(
        xc_notify::application::notification_service::NotificationService::new(
            state.db.clone(),
            registry,
        ),
    );
    let evaluator = Arc::new(RateAlertEvaluator::with_pool(
        state.db.clone(),
        notifications,
    ));
    Some(Arc::new(RateAlertsHandler::new(evaluator)))
}
