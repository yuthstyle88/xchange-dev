//! FN-1 #1068: F01, `GET /api/v3/views/finance_reconciliation?branch=&date=` (design `executive-finance.md` §5.1,
//! §8.2): a branch-day's sheet for FINANCE, `FinanceReconcile` on that branch (policy row `financeReconciliation`).
//! The core answers `outOfScope` for another branch and `noSuchBranch` for an unknown one; a `date` that is not
//! `YYYY-MM-DD` is `400 invalidRequest`.
//!
//! Not versioned: the sheet's figures (completions, reversals, certifications) do not all move the branch version,
//! so a `since` shortcut could answer "unchanged" wrongly. The screen reads it on demand; nothing polls it.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::web;
use pos_error::{AppError, AppResult};
use ts_rs::TS;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::application::finance::reconciliation::{ReconciliationService, business_date};
use crate::bootstrap::AppState;
use crate::presentation::dto::v3::finance_reconciliation::{
    CertificationRow, CountLine, CountRow, ExceptionRow, FinanceReconciliationFilter,
    FinanceSheet, InTransitRow, IntegrityRow, MethodTotals, Part, ReconciliationCertifyInput,
    ReversalPendingRow, ShortReceiptRow, TransactionTotals, TransferStatusCount,
    TransferSummary,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint, ViewFilter};

impl ViewFilter for FinanceReconciliationFilter {
    fn check(&self) -> Result<(), V3Error> {
        match business_date(&self.date) {
            Some(_) => Ok(()),
            None => Err(V3Error::new(
                "invalidRequest",
                Some("date"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )),
        }
    }
}

struct FinanceReconciliationView {
    service: ReconciliationService,
}

impl BranchView for FinanceReconciliationView {
    const NAME: &'static str = "finance_reconciliation";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "financeReconciliation");
    const VERSIONED: bool = false;
    type Data = FinanceSheet;
    type Filter = FinanceReconciliationFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let date = business_date(&filter.date);
        async move {
            // the filter's check has already refused a bad date
            let date = date.ok_or_else(|| AppError::Internal("an unchecked date".into()))?;
            self.service.sheet(branch, date).await
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(FinanceReconciliationView {
            service: ReconciliationService::standard(state.db.clone()),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    Part::<()>::export_all(&cfg)?;
    MethodTotals::export_all(&cfg)?;
    TransactionTotals::export_all(&cfg)?;
    CountLine::export_all(&cfg)?;
    CountRow::export_all(&cfg)?;
    TransferStatusCount::export_all(&cfg)?;
    InTransitRow::export_all(&cfg)?;
    ShortReceiptRow::export_all(&cfg)?;
    TransferSummary::export_all(&cfg)?;
    ExceptionRow::export_all(&cfg)?;
    IntegrityRow::export_all(&cfg)?;
    ReversalPendingRow::export_all(&cfg)?;
    CertificationRow::export_all(&cfg)?;
    FinanceSheet::export_all(&cfg)?;
    FinanceReconciliationFilter::export_all(&cfg)?;
    ReconciliationCertifyInput::export_all(&cfg)?;
    Ok(())
}
