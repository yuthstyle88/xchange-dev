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

use actix_web::http::Method;
use actix_web::web;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_cash::application::adjustment::adjustment_service::LedgerAdjustmentService;
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::ledger_adjustment_post::{
    LedgerAdjustmentPostInput, LedgerAdjustmentPostOutput, LedgerAdjustmentPostedLine,
};
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use crate::presentation::http::v3::error::V3Error;

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
        let _ = (&self.pool, &self.service, tx, branch, who, input);
        todo!("LA-6")
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
