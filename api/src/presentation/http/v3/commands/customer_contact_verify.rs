//! BK-10 (#1412): `POST /api/v3/commands/customer_contact_verify` (`customerContactVerify`) — staff
//! verify the customer's phone at the desk (booking-slots design §6 `members` row). Gated by
//! `CustomerHoldPlace` (ruling (a), row in `xc-auth/.../policy/areas/staff_customers.rs`).
//!
//! Scaffold written with the up-front tests: the route, its types and its policy gate only.
//! BK-10's implementer writes `run`.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::web;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_customer::application::customer::contact_verify::ContactSummary;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, Locks};
use crate::presentation::http::v3::error::V3Error;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerContactVerifyInput {
    pub customer_id: String,
    pub phone: String,
}

pub type CustomerContactVerifyOutput = ContactSummary;

pub struct CustomerContactVerifyCommand {
    pool: PgPool,
}

impl CustomerContactVerifyCommand {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl CommandAction for CustomerContactVerifyCommand {
    const NAME: &'static str = "customerContactVerify";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::StaffRpc, "customerContactVerify");
    type Input = CustomerContactVerifyInput;
    type Output = CustomerContactVerifyOutput;
    const ROUTE: &'static str = "/api/v3/commands/customer_contact_verify";
    const METHOD: Method = Method::POST;

    fn locks(&self, _branch: &BranchId, _input: &Self::Input) -> Locks {
        Locks::Org
    }

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let branch_tx = BranchTx::new(pos_infra::transactions::Transactor::new(self.pool.clone()));
        let service =
            xc_customer::application::customer::contact_verify::ContactVerify::new(branch_tx);
        let now = chrono::Utc::now();
        let res = service
            .verify_in(tx, who, branch, &input.customer_id, &input.phone, now)
            .await;

        match res {
            Ok(summary) => Ok(summary),
            Err(xc_customer::application::customer::contact_verify::ContactVerifyError::ValidationFailed(msg)) => {
                Err(V3Error::new(
                    "validationFailed",
                    Some("phone"),
                    msg,
                    actix_web::http::StatusCode::BAD_REQUEST,
                ))
            }
            Err(xc_customer::application::customer::contact_verify::ContactVerifyError::OutOfScope) => {
                Err(crate::presentation::http::v3::command::out_of_scope())
            }
            Err(xc_customer::application::customer::contact_verify::ContactVerifyError::Internal(e)) => {
                tracing::error!(error = %e, "customerContactVerify internal error");
                Err(crate::presentation::http::v3::command::server_error())
            }
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(CustomerContactVerifyCommand::new(state.db.clone())),
    );
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    Ok(())
}
