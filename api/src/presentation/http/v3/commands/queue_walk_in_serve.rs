//! XC-WALKIN-QUEUE (`docs/design/walkin-queue.md` W1, W4–W7, §5): `POST /api/queue/tickets/serve`
//! (`ticketServeWalkIn`). The desk's booking step calls it for a sale with no booking and no kiosk
//! request; it answers the ticket the customer at the counter is served under, SERVING for the
//! caller (`WalkInDesk::serve_in`). The command's Idempotency-Key is the sale's key (W5).
//!
//! W6: a new ticket needs `ServiceWindow::allows_begin`, read through
//! `CustodyFactsPort::service_window_in`; the answer is passed to `serve_in` as `may_issue`, so
//! `xc-queue` stays free of `xc-shift`.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xc_auth::domain::access::Actor as User;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_queue::application::issue::walk_in_desk::WalkInDesk;
use xc_shift::application::custody::service::CustodyFactsPort;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;

use super::queue::{QueueServices, QueueTicketDto};
use crate::bootstrap::AppState;
use crate::infrastructure::custody::custody_ports::PgCustodyPorts;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

/// `ticketServeWalkIn`'s body: the sale's customer, when the desk already knows them (W4 (3)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "queue.ts")]
pub struct QueueServeWalkInInput {
    pub customer_id: Option<String>,
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    QueueServeWalkInInput::export_all(&cfg)?;
    Ok(())
}

pub struct ServeWalkIn(pub QueueServices);

impl CommandAction for ServeWalkIn {
    const NAME: &'static str = "ticketServeWalkIn";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ticketServeWalkIn");
    type Input = QueueServeWalkInInput;
    type Output = QueueTicketDto;
    const ROUTE: &'static str = "/api/queue/tickets/serve";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &User,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let now = Utc::now();
        let now_time = CustodyTime::from_epoch_millis(now.timestamp_millis());
        let (window, _) = PgCustodyPorts
            .service_window_in(tx, branch, now_time)
            .await
            .map_err(|_| server_error())?;
        let may_issue = window.allows_begin(now_time);
        let user = super::queue::user_in(tx, branch, who).await?;
        let customer_id = input
            .customer_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        WalkInDesk::new(
            self.0.issuer.clone(),
            self.0.desk.clone(),
            self.0.repo.clone(),
            self.0.status.clone(),
        )
        .serve_in(tx, &user, branch, customer_id, key.as_str(), may_issue, now)
        .await
        .map(QueueTicketDto::from)
        .map_err(super::queue::refusal)
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(ServeWalkIn(QueueServices::new(state.db.clone()))),
    );
}
