//! LA-6: `POST /api/v3/commands/ledger_adjustment_post` `{branch, id}` (design
//! `docs/design/blueprint/ledger-inventory.md` §13.5 "Post", AD-7): any `FinanceReconcile` holder of the branch
//! posts an APPROVED adjustment, the creator included (owner OA-7). In the seam's one transaction under the branch
//! lock: `LedgerAdjustmentService::post_in`, then one `audit_events` row `LEDGER_ADJUSTMENT_POST`,
//! `object_ref = "ledger_adjustment:<id>"`, `after` = the posted figures (AD-10). Answers
//! `{id, postingId, lines: [{currency, quantity, valuationThb}]}`; a repeat answers the same `postingId`.
//!
//! Refusals, as `AdjustmentRefusal` says: `Invalid` → `400 validationFailed`; `NotFound` → `404 notFound`;
//! `ApprovalRequired` → `409 approvalRequired`; `Rejected` → `409 staleVersion`; `StockInsufficient` →
//! `409 stockInsufficient` carrying the shortfall. This command never calls `stock_refusal_answer`
//! (`presentation/http/v3/negative_stock.rs`): that would open a `NEGATIVE_STOCK` request (AD-7).
//!
//! Skeleton (LA-0, sub-controller claude/f32b85db): `run` is LA-6's.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::Utc;
use serde_json::json;
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
use crate::presentation::dto::v3::ledger_adjustment_post::{
    LedgerAdjustmentPostInput, LedgerAdjustmentPostOutput, LedgerAdjustmentPostedLine,
};
use crate::presentation::http::v3::command::{
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::me::me_roles;

pub const AUDIT_ACTION: &str = "LEDGER_ADJUSTMENT_POST";

pub struct LedgerAdjustmentPostCommand {
    pool: PgPool,
    service: LedgerAdjustmentService,
}

impl LedgerAdjustmentPostCommand {
    pub fn new(pool: PgPool) -> Self {
        let approvals =
            crate::presentation::http::v3::commands::make_approval_service(pool.clone());
        Self {
            service: LedgerAdjustmentService::new(approvals, Arc::new(LedgerCashPostings::new())),
            pool,
        }
    }
}

impl CommandAction for LedgerAdjustmentPostCommand {
    const NAME: &'static str = "ledgerAdjustmentPost";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ledgerAdjustmentPost");
    type Input = LedgerAdjustmentPostInput;
    type Output = LedgerAdjustmentPostOutput;
    const ROUTE: &'static str = "/api/v3/commands/ledger_adjustment_post";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let answer = self
            .service
            .post_in(tx, branch, &input.id, who)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "ledger adjustment post failed");
                server_error()
            })?;
        let posted = match answer {
            Ok(posted) => posted,
            Err(AdjustmentRefusal::Invalid(e)) => {
                return Err(V3Error::with_field_path(
                    "validationFailed",
                    e.field,
                    e.message,
                    StatusCode::BAD_REQUEST,
                ));
            }
            Err(AdjustmentRefusal::NotFound) => {
                return Err(V3Error::new(
                    "notFound",
                    None,
                    "notFound",
                    StatusCode::NOT_FOUND,
                ));
            }
            Err(AdjustmentRefusal::ApprovalRequired(id)) => {
                return Err(V3Error::new(
                    "approvalRequired",
                    None,
                    format!("approvalRequired: {id}"),
                    StatusCode::CONFLICT,
                ));
            }
            Err(AdjustmentRefusal::Rejected(_)) => {
                return Err(V3Error::new(
                    "staleVersion",
                    None,
                    "staleVersion",
                    StatusCode::CONFLICT,
                ));
            }
            Err(AdjustmentRefusal::StockInsufficient(s)) => {
                let detail: Vec<String> = s
                    .lines
                    .iter()
                    .map(|l| {
                        format!(
                            "{} {} short by {}",
                            l.currency.as_str(),
                            l.place.as_str(),
                            l.short_by.normalize()
                        )
                    })
                    .collect();
                let msg = if detail.is_empty() {
                    "stockInsufficient".to_owned()
                } else {
                    format!("stockInsufficient: {}", detail.join(", "))
                };
                return Err(V3Error::new(
                    "stockInsufficient",
                    None,
                    msg,
                    StatusCode::CONFLICT,
                ));
            }
            Err(AdjustmentRefusal::OutOfScope) => return Err(out_of_scope()),
        };

        let posting_id = posted.posting_id.0.to_string();
        let lines: Vec<LedgerAdjustmentPostedLine> = posted
            .lines
            .into_iter()
            .map(|l| LedgerAdjustmentPostedLine {
                currency: l.currency.as_str().to_owned(),
                quantity: l.quantity.normalize().to_string(),
                valuation_thb: format!("{:.2}", l.valuation_thb),
            })
            .collect();

        let mut roles: Vec<String> = me_roles(who, Utc::now())
            .into_iter()
            .map(|r| r.role)
            .collect();
        roles.dedup();
        PgAuditLog::new(self.pool.clone())
            .append_in(
                tx,
                &who.organization_id,
                &AuditEvent {
                    actor_id: who.user_id.clone(),
                    actor_role: roles.join(","),
                    scope: "branch".to_owned(),
                    action: AUDIT_ACTION.to_owned(),
                    object_ref: format!("ledger_adjustment:{}", posted.id),
                    branch_id: Some(branch.as_str().to_owned()),
                    before: None,
                    after: Some(json!({ "postingId": &posting_id, "lines": &lines })),
                },
            )
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "ledger adjustment post audit failed");
                server_error()
            })?;

        Ok(LedgerAdjustmentPostOutput {
            id: posted.id,
            posting_id,
            lines,
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    CommandEndpoint::register(
        cfg,
        Arc::new(LedgerAdjustmentPostCommand::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    LedgerAdjustmentPostInput::export_all(&cfg)?;
    LedgerAdjustmentPostedLine::export_all(&cfg)?;
    LedgerAdjustmentPostOutput::export_all(&cfg)?;
    Ok(())
}
