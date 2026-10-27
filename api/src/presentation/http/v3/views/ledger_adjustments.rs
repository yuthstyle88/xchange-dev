//! LA-7: `GET /api/v3/views/ledger_adjustments?branch=&status=` (design `docs/design/blueprint/ledger-inventory.md`
//! §13.5 "Read"): the branch's FINANCE ledger adjustments, policy row `ledgerAdjustments` (`FinanceReconcile`).
//! One read joining `approval_requests` (type `LEDGER_ADJUSTMENT`, `proposed_payload->>'branch'` = the branch,
//! never `approval_requests.branch_id`, §13.3), its latest `approval_decisions` row and `ledger_postings` on
//! `object_id = source_id` (`source_type 'ADJUSTMENT'`), with the valuations from the posting's SAFE entries.
//! A `status` other than empty, `PENDING`, `APPROVED`, `REJECTED` or `POSTED` is `400 invalidRequest`.
//!
//! Not versioned: a decision runs under the decider's branch lock (`approval_decide.rs`), not the adjustment's,
//! so it need not move this branch's version. The screen reads it on demand; nothing polls it.
//!
//! Skeleton (LA-0, sub-controller claude/f32b85db): `load` is LA-7's.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::web;
use pos_error::AppResult;
use sqlx::PgPool;
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::ledger_adjustments::{
    LedgerAdjustmentRow, LedgerAdjustments, LedgerAdjustmentsFilter,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint, ViewFilter};

impl ViewFilter for LedgerAdjustmentsFilter {
    fn check(&self) -> Result<(), V3Error> {
        match self.status.as_str() {
            "" | "PENDING" | "APPROVED" | "REJECTED" | "POSTED" => Ok(()),
            _ => Err(V3Error::new(
                "invalidRequest",
                Some("status"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )),
        }
    }
}

struct LedgerAdjustmentsView {
    pool: PgPool,
}

impl BranchView for LedgerAdjustmentsView {
    const NAME: &'static str = "ledger_adjustments";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "ledgerAdjustments");
    const VERSIONED: bool = false;
    type Data = LedgerAdjustments;
    type Filter = LedgerAdjustmentsFilter;

    fn load(
        &self,
        branch: &BranchId,
        who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let _ = (&self.pool, branch, who, filter);
        async move { todo!("LA-7") }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(LedgerAdjustmentsView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    LedgerAdjustmentsFilter::export_all(&cfg)?;
    LedgerAdjustmentRow::export_all(&cfg)?;
    LedgerAdjustments::export_all(&cfg)?;
    Ok(())
}
