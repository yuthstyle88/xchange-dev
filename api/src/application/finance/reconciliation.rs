//! FN-1 #1068: F01, FINANCE's reconcile-and-certify sheet (design `executive-finance.md` §5, §8.2; owner decision
//! E-8). `ReconciliationService::sheet` assembles a branch-day's six parts and the hash of its figures;
//! `certify_in` appends a certification of exactly the figures the caller saw.
//!
//! Rulings on #1068 (issuecomment-6109917312):
//! - Q1: the transactions part is EX-3's `ExchangeFacts::branch_day_totals`, `count` included. No second reader.
//! - Q2: the exceptions part is the branch's open `cash_exceptions`, not day-scoped, and **not** in the hash.
//! - Q3: E-8's "that day" is a count's `counted_at` business date in the branch's timezone.
//!
//! The four parts FN-2 fills come through `SheetPartsPort`; until then `UnavailableParts` answers each one
//! unavailable, naming the ticket it waits for.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_shift::domain::custody::entities::cash_exception::ExceptionStatus;
use xc_shift::infrastructure::custody::decode::exception_from_row;

use crate::application::executive::ports::{BranchDayTotals, ExchangeFacts};
use crate::infrastructure::executive::wide_transaction_facts::WideTransactionFacts;
use crate::infrastructure::finance::certification_repository::{
    Certification, NewCertification, PgCertificationRepository,
};
pub use crate::presentation::dto::v3::finance_reconciliation::{
    CertificationRow, CountRow, ExceptionRow, FinanceSheet, IntegrityRow, MethodTotals, Part,
    ReversalPendingRow, TransactionTotals, TransferSummary,
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The four parts FN-2 fills (§7). FN-2 implements this in its own file and swaps the one line that builds the
/// service.
pub trait SheetPartsPort: Send + Sync {
    fn counts<'a>(
        &'a self,
        branch: &'a BranchId,
        date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<Part<Vec<CountRow>>>>;

    fn transfers<'a>(
        &'a self,
        branch: &'a BranchId,
        date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<Part<TransferSummary>>>;

    fn integrity<'a>(
        &'a self,
        branch: &'a BranchId,
        date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<Part<Vec<IntegrityRow>>>>;

    fn reversals_pending<'a>(
        &'a self,
        branch: &'a BranchId,
        date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<Part<Vec<ReversalPendingRow>>>>;
}

/// Today's `SheetPartsPort`: each part unavailable, naming the ticket whose source it waits for.
pub struct UnavailableParts;

fn waiting<T: ts_rs::TS + Send + 'static>(
    ticket: &'static str,
) -> BoxFuture<'static, AppResult<Part<T>>> {
    Box::pin(async move { Ok(Part::waiting_for(format!("waits for {ticket}"))) })
}

impl SheetPartsPort for UnavailableParts {
    fn counts<'a>(
        &'a self,
        _branch: &'a BranchId,
        _date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<Part<Vec<CountRow>>>> {
        waiting("CA-3")
    }

    fn transfers<'a>(
        &'a self,
        _branch: &'a BranchId,
        _date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<Part<TransferSummary>>> {
        waiting("CA-5")
    }

    fn integrity<'a>(
        &'a self,
        _branch: &'a BranchId,
        _date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<Part<Vec<IntegrityRow>>>> {
        waiting("LG-7")
    }

    fn reversals_pending<'a>(
        &'a self,
        _branch: &'a BranchId,
        _date: NaiveDate,
    ) -> BoxFuture<'a, AppResult<Part<Vec<ReversalPendingRow>>>> {
        waiting("RV-1")
    }
}

/// Why a certification was refused; nothing is written for either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertifyRefusal {
    /// The figures moved since the caller read them (`staleVersion`).
    StaleVersion,
    /// The caller verified one of that day's counts (E-8, `samePerson`).
    SamePerson,
}

/// The parts the hash covers, in declaration order: every part except the exceptions (ruling Q2).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HashedFigures<'a> {
    transactions: &'a Part<TransactionTotals>,
    counts: &'a Part<Vec<CountRow>>,
    transfers: &'a Part<TransferSummary>,
    integrity: &'a Part<Vec<IntegrityRow>>,
    reversals_pending: &'a Part<Vec<ReversalPendingRow>>,
}

/// What a certification stores in `figures`: the six parts as the certifier saw them.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShownFigures<'a> {
    transactions: &'a Part<TransactionTotals>,
    counts: &'a Part<Vec<CountRow>>,
    transfers: &'a Part<TransferSummary>,
    exceptions: &'a Part<Vec<ExceptionRow>>,
    integrity: &'a Part<Vec<IntegrityRow>>,
    reversals_pending: &'a Part<Vec<ReversalPendingRow>>,
}

/// Lower-hex SHA-256 of the compact JSON of the hashed parts: availability and reasons are figures too.
fn figures_hash(s: &FinanceSheet) -> AppResult<String> {
    let hashed = HashedFigures {
        transactions: &s.transactions,
        counts: &s.counts,
        transfers: &s.transfers,
        integrity: &s.integrity,
        reversals_pending: &s.reversals_pending,
    };
    let bytes = serde_json::to_vec(&hashed)
        .map_err(|e| AppError::Internal(format!("hash the figures: {e}")))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

/// A sheet's business date: `YYYY-MM-DD`.
pub fn business_date(raw: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok()
}

/// Money as a decimal string with two places (W7).
fn money(d: Decimal) -> String {
    let mut d = d;
    d.rescale(2);
    d.to_string()
}

fn transaction_totals(t: BranchDayTotals) -> TransactionTotals {
    TransactionTotals {
        by_method: t
            .by_method
            .into_iter()
            .map(|m| MethodTotals {
                method: m.method,
                buy_thb: money(m.buy_thb),
                sell_thb: money(m.sell_thb),
                fees: money(m.fees_thb),
                bank_fees: money(m.bank_fees_thb),
                count: m.count,
            })
            .collect(),
        reversed_thb: money(t.reversals_thb),
    }
}

fn certification_row(c: &Certification, current_hash: &str) -> CertificationRow {
    CertificationRow {
        id: c.id.to_string(),
        certified_by: c.certified_by.to_string(),
        certified_at: c.certified_at.to_rfc3339(),
        note: c.note.clone(),
        figures_hash: c.figures_hash.clone(),
        stale: c.figures_hash != current_hash,
    }
}

pub struct ReconciliationService {
    pool: PgPool,
    facts: Arc<dyn ExchangeFacts>,
    parts: Arc<dyn SheetPartsPort>,
    certifications: PgCertificationRepository,
}

impl ReconciliationService {
    pub fn new(
        pool: PgPool,
        facts: Arc<dyn ExchangeFacts>,
        parts: Arc<dyn SheetPartsPort>,
    ) -> Self {
        Self {
            certifications: PgCertificationRepository::new(pool.clone()),
            pool,
            facts,
            parts,
        }
    }

    /// What F01's view and command run on: EX-3's exchange facts and FN-2's `SheetParts`.
    pub fn standard(pool: PgPool) -> Self {
        Self::new(
            pool.clone(),
            Arc::new(WideTransactionFacts::new(pool.clone())),
            Arc::new(super::sheet_parts::SheetParts::new(
                pool,
                Arc::new(xc_cash::application::reconcile::Reconcile::standard()),
            )),
        )
    }

    /// The branch's open exceptions (ruling Q2). A row the custody reader cannot decode (the new cash flow's
    /// shape) is open, and its summary is its id; nothing parses its data.
    async fn open_exceptions(&self, branch: &BranchId) -> AppResult<Vec<ExceptionRow>> {
        let rows = sqlx::query(
            "SELECT id, data, position FROM cash_exceptions WHERE branch_id = $1 ORDER BY position",
        )
        .bind(branch.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(AppError::Database)?;
        let mut open = Vec::new();
        for row in &rows {
            let id: String = sqlx::Row::get(row, 0);
            let position: i32 = sqlx::Row::get(row, 2);
            let summary = match exception_from_row(row) {
                Ok(e) if e.status != ExceptionStatus::Open => continue,
                Ok(e) if e.note.is_empty() => e.id,
                Ok(e) => format!("{}: {}", e.id, e.note),
                Err(_) => id.clone(),
            };
            open.push(ExceptionRow {
                id,
                position,
                reviewed: false,
                summary,
            });
        }
        Ok(open)
    }

    /// The sheet of `branch` on business date `date`, with its certifications marked stale against its hash.
    pub async fn sheet(&self, branch: &BranchId, date: NaiveDate) -> AppResult<FinanceSheet> {
        let totals = self.facts.branch_day_totals(branch, date).await?;
        let mut sheet = FinanceSheet {
            branch: branch.as_str().to_owned(),
            date: date.format("%Y-%m-%d").to_string(),
            transactions: Part::of(transaction_totals(totals)),
            counts: self.parts.counts(branch, date).await?,
            transfers: self.parts.transfers(branch, date).await?,
            exceptions: Part::of(self.open_exceptions(branch).await?),
            integrity: self.parts.integrity(branch, date).await?,
            reversals_pending: self.parts.reversals_pending(branch, date).await?,
            figures_hash: String::new(),
            certifications: Vec::new(),
        };
        sheet.figures_hash = figures_hash(&sheet)?;
        sheet.certifications = self
            .certifications
            .for_day(branch, date)
            .await?
            .iter()
            .map(|c| certification_row(c, &sheet.figures_hash))
            .collect();
        Ok(sheet)
    }

    /// Certifies the sheet of `branch` on `date` in the caller's transaction (design §8.2):
    /// 1. the sheet is recomputed; another hash than `figures_hash` is `StaleVersion`;
    /// 2. a caller who verified a count of that day (its `counted_at` business date in the branch's timezone, Q3)
    ///    is `SamePerson` (E-8);
    /// 3. one certification row is appended, holding the six parts.
    ///
    /// A refusal writes nothing. The caller's command writes the audit row on the same transaction.
    #[allow(clippy::too_many_arguments)]
    pub async fn certify_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        date: NaiveDate,
        certifier: Uuid,
        figures_hash: &str,
        note: Option<&str>,
        now: DateTime<Utc>,
    ) -> AppResult<Result<CertificationRow, CertifyRefusal>> {
        let sheet = self.sheet(branch, date).await?;
        if sheet.figures_hash != figures_hash {
            return Ok(Err(CertifyRefusal::StaleVersion));
        }
        let verified_that_day: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
               SELECT 1 FROM cash_counts c JOIN branches b ON b.id = c.branch_id \
                WHERE c.branch_id = $1 AND c.verified_by = $2 \
                  AND (c.counted_at AT TIME ZONE b.timezone)::date = $3)",
        )
        .bind(branch.as_str())
        .bind(certifier)
        .bind(date)
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        if verified_that_day {
            return Ok(Err(CertifyRefusal::SamePerson));
        }
        let figures = serde_json::to_value(ShownFigures {
            transactions: &sheet.transactions,
            counts: &sheet.counts,
            transfers: &sheet.transfers,
            exceptions: &sheet.exceptions,
            integrity: &sheet.integrity,
            reversals_pending: &sheet.reversals_pending,
        })
        .map_err(|e| AppError::Internal(format!("store the figures: {e}")))?;
        let stored = self
            .certifications
            .append_in(
                tx,
                NewCertification {
                    branch,
                    business_date: date,
                    figures_hash,
                    figures,
                    certified_by: certifier,
                    certified_at: now,
                    note,
                },
            )
            .await?;
        Ok(Ok(certification_row(&stored, figures_hash)))
    }
}
