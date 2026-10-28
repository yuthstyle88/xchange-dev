//! LA-5: `POST /api/v3/commands/ledger_adjustment_create` (design `docs/design/blueprint/ledger-inventory.md`
//! §13.4, §13.5 "Create"): FINANCE asks for a ledger adjustment of the branch. In the seam's one transaction under
//! the branch lock: `LedgerAdjustmentService::create_in` opens the `LEDGER_ADJUSTMENT` request, then one
//! `audit_events` row `LEDGER_ADJUSTMENT_CREATE`, `object_ref = "ledger_adjustment:<id>"`, `after` = the payload
//! (AD-10). Answers `{id, approvalRequestId, status: "REQUESTED"}`. A refusal maps as `AdjustmentRefusal` says:
//! `Invalid` → `400 validationFailed` with its field, `OutOfScope` → `403 outOfScope`. The seam replays a repeated
//! key and refuses the same key with another body (`idempotencyConflict`).
//!
//! Skeleton (LA-0, sub-controller claude/f32b85db): `run` is LA-5's.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use sqlx::PgPool;
use ts_rs::TS;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_cash::application::adjustment::adjustment_service::{
    AdjustmentRefusal, LedgerAdjustmentService,
};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::ledger_adjustment_create::{
    LedgerAdjustmentCreateInput, LedgerAdjustmentCreateOutput, LedgerAdjustmentEvidence,
    LedgerAdjustmentLine,
};
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::me::me_roles;

pub const AUDIT_ACTION: &str = "LEDGER_ADJUSTMENT_CREATE";

pub struct LedgerAdjustmentCreateCommand {
    pool: PgPool,
    service: LedgerAdjustmentService,
}

impl LedgerAdjustmentCreateCommand {
    pub fn new(pool: PgPool) -> Self {
        let approvals =
            crate::presentation::http::v3::commands::make_approval_service(pool.clone());
        Self {
            service: LedgerAdjustmentService::new(approvals, Arc::new(LedgerCashPostings::new())),
            pool,
        }
    }
}

impl CommandAction for LedgerAdjustmentCreateCommand {
    const NAME: &'static str = "ledgerAdjustmentCreate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ledgerAdjustmentCreate");
    type Input = LedgerAdjustmentCreateInput;
    type Output = LedgerAdjustmentCreateOutput;
    const ROUTE: &'static str = "/api/v3/commands/ledger_adjustment_create";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let payload = serde_json::to_value(&input).map_err(|_| server_error())?;
        let created = match self
            .service
            .create_in(tx, branch, &payload, who)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "ledger adjustment create failed");
                server_error()
            })? {
            Ok(created) => created,
            Err(AdjustmentRefusal::Invalid(e)) => {
                return Err(V3Error::with_field_path(
                    "validationFailed",
                    e.field,
                    e.message,
                    StatusCode::BAD_REQUEST,
                ));
            }
            Err(AdjustmentRefusal::OutOfScope) => return Err(out_of_scope()),
            Err(_) => return Err(server_error()),
        };

        let mut roles: Vec<String> = me_roles(who, Utc::now())
            .into_iter()
            .map(|r| r.role)
            .collect();
        roles.dedup();
        PgAuditLog::new(self.pool.clone())
            .append(
                tx,
                &AuditEvent {
                    actor_id: who.user_id.clone(),
                    actor_role: roles.join(","),
                    scope: "branch".to_owned(),
                    action: AUDIT_ACTION.to_owned(),
                    object_ref: format!("ledger_adjustment:{}", created.id),
                    branch_id: Some(branch.as_str().to_owned()),
                    before: None,
                    after: Some(payload),
                },
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "ledger adjustment create audit failed");
                server_error()
            })?;

        Ok(LedgerAdjustmentCreateOutput {
            id: created.id,
            approval_request_id: created.approval_request_id.to_string(),
            status: "REQUESTED".to_owned(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(LedgerAdjustmentCreateCommand::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    LedgerAdjustmentLine::export_all(&cfg)?;
    LedgerAdjustmentEvidence::export_all(&cfg)?;
    LedgerAdjustmentCreateInput::export_all(&cfg)?;
    LedgerAdjustmentCreateOutput::export_all(&cfg)?;
    Ok(())
}
