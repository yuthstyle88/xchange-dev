//! S7 (#3410): `POST /api/app/queue/tickets` (`queueTicketTake`) — a signed-in customer takes a
//! walk-in queue number at a branch from the app (`IssueSource::App`).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::{SecondsFormat, Utc};
use pos_error::AppError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_queue::application::status::status_service::TicketRef;
use xc_queue::domain::ticket::issue_source::IssueSource;

use super::queue::QueueServices;
use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::my_queue_ticket::MyQueueTicketData;
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueTicketTakeInput {
    pub branch: BranchId,
}

pub struct QueueTicketTake(pub QueueServices);

fn refusal(err: AppError) -> V3Error {
    if let AppError::Conflict(ref msg) = err {
        if let Some(ticket_id) = msg.strip_prefix("alreadyQueued:") {
            return V3Error::new(
                "alreadyQueued",
                None,
                ticket_id.to_owned(),
                StatusCode::CONFLICT,
            );
        }
        if msg == "alreadyQueued" {
            return V3Error::new("alreadyQueued", None, "alreadyQueued", StatusCode::CONFLICT);
        }
    }
    super::queue::refusal(err)
}

impl CommandAction for QueueTicketTake {
    const NAME: &'static str = "queueTicketTake";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::OwnerRoute, "queueTicketTake");
    type Input = QueueTicketTakeInput;
    type Output = MyQueueTicketData;
    const ROUTE: &'static str = "/api/app/queue/tickets";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        _input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let customer_id = who.customer_id.clone().ok_or_else(out_of_scope)?;
        let now = Utc::now();
        let source = IssueSource::App { customer_id };
        let ticket = self
            .0
            .issuer
            .issue_in(tx, branch, &source, key.as_str(), now)
            .await
            .map_err(refusal)?;

        let ticket_uuid = Uuid::parse_str(&ticket.id).map_err(|_| server_error())?;
        let (tickets_ahead, estimate) = self
            .0
            .status
            .ticket_wait_in(
                tx,
                TicketRef {
                    id: ticket_uuid,
                    branch,
                    service_type: ticket.service_type.as_str(),
                    sequence: ticket.sequence,
                    status: ticket.status.as_str(),
                },
                now,
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "queueTicketTake wait calculation failed");
                server_error()
            })?;

        let (estimate_seconds, estimate_low_seconds, estimate_high_seconds, estimate_as_of) =
            match estimate {
                Some(e) => (
                    Some(e.seconds),
                    Some(e.low_seconds),
                    Some(e.high_seconds),
                    Some(now.to_rfc3339_opts(SecondsFormat::Secs, true)),
                ),
                None => (None, None, None, None),
            };

        Ok(MyQueueTicketData {
            ticket_id: ticket.id,
            branch: branch.as_str().to_owned(),
            display_number: ticket.display_number,
            service_type: ticket.service_type.as_str().to_owned(),
            status: ticket.status.as_str().to_owned(),
            tickets_ahead,
            estimate_seconds,
            estimate_low_seconds,
            estimate_high_seconds,
            estimate_as_of,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let q = QueueServices::new(state.db.clone());
    CommandEndpoint::register(cfg, Arc::new(QueueTicketTake(q)));
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    Ok(())
}
