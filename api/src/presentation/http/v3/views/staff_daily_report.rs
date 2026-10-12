//! #1350: the staff report, `GET /api/v3/views/staff_daily_report?branch=&day=` (design
//! `xchange-v3-wave3-staff-report.md` §B). v2's page gives the look; every figure is the Blueprint's (§B.0), read here
//! with this file's own read-only SQL (§B.3) plus `CustodyPages` for the two custody tables kept as jsonb.
//!
//! - **Who:** the policy row `(StaffRpc, "staffDailyReport")` (D-B2, `areas/staff_report.rs`); a session holding none of
//!   its three permissions is refused `RoleNotAllowed` by the core before `load`, so no figure is sent.
//! - **The day:** `[day 00:00, day+1 00:00)` in `branches.timezone`, converted to UTC once (§B.3). No `day` = today's
//!   business date in that timezone (D-B3).
//! - **LIVE only:** DEMO exchanges (and reversals of them) are in no figure.
//! - Not versioned and never polled: the screen reads it on load and on a day change (D-B1, §B.4).

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::http::StatusCode;
use actix_web::web;
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_shift::domain::custody::repositories::custody_pages::CustodyPages;
use xc_shift::domain::custody::entities::cash_exception::ExceptionStatus;
use xc_shift::infrastructure::custody::custody_pages::PgCustodyPages;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::staff_daily_report::{
    ReportCurrencyRow, ReportNote, ReportSigner, ReportStatus, ReportTransit, SignerCapacity,
    StaffDailyReportData, StaffDailyReportFilter,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint, ViewFilter};

/// v2's seven currencies, in v2's order (B.1 row 13: the look).
const CODES: [&str; 7] = ["THB", "USD", "EUR", "GBP", "JPY", "CNY", "AUD"];
/// At most this many notes (Q-SR3).
const NOTES: i64 = 5;

fn parse_day(raw: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok()
}

impl ViewFilter for StaffDailyReportFilter {
    fn check(&self) -> Result<(), V3Error> {
        match self.day.as_deref() {
            None => Ok(()),
            Some(raw) if parse_day(raw).is_some() => Ok(()),
            Some(_) => Err(V3Error::new(
                "invalidRequest",
                Some("day"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )),
        }
    }
}

/// Money as a decimal string with two places (W7).
fn money(d: Decimal) -> String {
    let mut d = d;
    d.rescale(2);
    d.to_string()
}

/// A stored count figure, as stored.
fn stored(d: Option<Decimal>) -> Option<String> {
    d.map(|d| d.normalize().to_string())
}

/// The UTC instant local midnight of `day` falls on in `tz` (the earliest, where a DST gap or fold applies).
fn midnight(tz: Tz, day: NaiveDate) -> AppResult<DateTime<Utc>> {
    let naive = day
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| AppError::Internal("a midnight".into()))?;
    tz.from_local_datetime(&naive)
        .earliest()
        .or_else(|| tz.from_local_datetime(&(naive + Duration::hours(1))).earliest())
        .map(|t| t.with_timezone(&Utc))
        .ok_or_else(|| AppError::Internal(format!("no local midnight on {day} in {tz}")))
}

fn db(e: sqlx::Error) -> AppError {
    AppError::Database(e)
}

struct StaffDailyReportView {
    pool: PgPool,
    custody: Arc<dyn CustodyPages>,
}

impl StaffDailyReportView {
    async fn report(
        &self,
        branch: &BranchId,
        day: Option<NaiveDate>,
    ) -> AppResult<StaffDailyReportData> {
        let b = branch.as_str();
        let (branch_name, timezone, company, site): (
            String,
            String,
            Option<String>,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT b.name, b.timezone, o.entity->>'company', o.entity->>'site' \
             FROM branches b LEFT JOIN ops_state o ON o.branch_id = b.id WHERE b.id = $1",
        )
        .bind(b)
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;
        let tz: Tz = timezone
            .parse()
            .map_err(|_| AppError::Internal(format!("branch timezone {timezone:?}")))?;
        let now = Utc::now();
        let day = day.unwrap_or_else(|| now.with_timezone(&tz).date_naive());
        let next = day
            .succ_opt()
            .ok_or_else(|| AppError::Internal("the day after".into()))?;
        let (from, to) = (midnight(tz, day)?, midnight(tz, next)?);

        // B.1 rows 5, 10, 11: the day's COMPLETED LIVE exchanges, by `completed_at`; volumes per `shop_side`, gross.
        let (completed, buy, sell): (i64, Option<Decimal>, Option<Decimal>) = sqlx::query_as(
            "SELECT count(DISTINCT t.id), \
                    sum(l.thb_amount) FILTER (WHERE l.shop_side = 'BUY'), \
                    sum(l.thb_amount) FILTER (WHERE l.shop_side = 'SELL') \
             FROM transactions t LEFT JOIN transaction_lines l ON l.transaction_id = t.id \
             WHERE t.branch_id = $1 AND t.mode = 'LIVE' AND t.status = 'COMPLETED' \
               AND t.completed_at >= $2 AND t.completed_at < $3",
        )
        .bind(b)
        .bind(from)
        .bind(to)
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;

        // B.1 row 11: POSTED reversals of LIVE exchanges, by the business date of `posted_at`; shown apart.
        let reversed: Option<Decimal> = sqlx::query_scalar(
            "SELECT sum(r.amount_thb) FROM reversals r \
             JOIN transactions t ON t.id = r.original_transaction_id \
             WHERE r.branch_id = $1 AND r.status = 'POSTED' AND t.mode = 'LIVE' \
               AND r.posted_at >= $2 AND r.posted_at < $3",
        )
        .bind(b)
        .bind(from)
        .bind(to)
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;

        // B.1 row 12: the day's unfinished LIVE exchanges, by `recorded_at`.
        let pending_transactions: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM transactions \
             WHERE branch_id = $1 AND mode = 'LIVE' \
               AND status IN ('DRAFT', 'PENDING_APPROVAL', 'CONFIRMED') \
               AND recorded_at >= $2 AND recorded_at < $3",
        )
        .bind(b)
        .bind(from)
        .bind(to)
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;

        // B.1 row 7, Q-SR1: every verified CLOSE count of the day's counted shifts.
        let close_counts: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(
            "SELECT c.id, c.counted_by, c.verified_by \
             FROM cash_counts c JOIN shifts s ON s.id = c.shift_id \
             WHERE s.branch_id = $1 AND s.opened_by_count IS NOT NULL \
               AND s.opened_at >= $2 AND s.opened_at < $3 \
               AND c.kind = 'CLOSE' AND c.verified_at IS NOT NULL \
             ORDER BY c.verified_at, c.id",
        )
        .bind(b)
        .bind(from)
        .bind(to)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        let close_ids: Vec<Uuid> = close_counts.iter().map(|c| c.0).collect();
        let drawer: Vec<(String, Decimal, Option<Decimal>, Option<Decimal>)> = sqlx::query_as(
            "SELECT trim(currency), sum(counted), sum(expected), sum(difference) \
             FROM cash_count_totals WHERE count_id = ANY($1) GROUP BY trim(currency)",
        )
        .bind(&close_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;

        // B.1 row 13: the day's last verified safe count, by `verified_at`.
        let safe: Vec<(String, Decimal, Option<Decimal>, Option<Decimal>)> = sqlx::query_as(
            "SELECT trim(t.currency), t.counted, t.expected, t.difference FROM cash_count_totals t \
             WHERE t.count_id = ( \
               SELECT id FROM cash_counts \
               WHERE branch_id = $1 AND kind IN ('SAFE_OPEN', 'SAFE') AND shift_id IS NULL \
                 AND verified_at IS NOT NULL AND counted_at >= $2 AND counted_at < $3 \
               ORDER BY verified_at DESC, id DESC LIMIT 1)",
        )
        .bind(b)
        .bind(from)
        .bind(to)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;

        let currencies = CODES
            .iter()
            .map(|code| {
                let d = drawer.iter().find(|r| r.0 == *code);
                let s = safe.iter().find(|r| r.0 == *code);
                ReportCurrencyRow {
                    currency: (*code).to_owned(),
                    drawer_counted: stored(d.map(|r| r.1)),
                    drawer_expected: stored(d.and_then(|r| r.2)),
                    drawer_difference: stored(d.and_then(|r| r.3)),
                    safe_counted: stored(s.map(|r| r.1)),
                    safe_expected: stored(s.and_then(|r| r.2)),
                    safe_difference: stored(s.and_then(|r| r.3)),
                }
            })
            .collect();

        // B.1 row 8: reconciled, closed awaiting reconciliation, or preliminary.
        let certification: Option<Option<String>> = sqlx::query_scalar(
            "SELECT note FROM reconciliation_certifications \
             WHERE branch_id = $1 AND business_date = $2 \
             ORDER BY certified_at DESC, id DESC LIMIT 1",
        )
        .bind(b)
        .bind(day)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        let (counted_shifts, closed_shifts): (i64, i64) = sqlx::query_as(
            "SELECT count(*), \
                    count(*) FILTER (WHERE EXISTS ( \
                      SELECT 1 FROM cash_counts c WHERE c.shift_id = s.id AND c.kind = 'CLOSE' \
                        AND c.verified_at IS NOT NULL)) \
             FROM shifts s \
             WHERE s.branch_id = $1 AND s.opened_by_count IS NOT NULL \
               AND s.opened_at >= $2 AND s.opened_at < $3",
        )
        .bind(b)
        .bind(from)
        .bind(to)
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;
        let status = if certification.is_some() {
            ReportStatus::Reconciled
        } else if counted_shifts > 0 && closed_shifts == counted_shifts {
            ReportStatus::ClosedAwaitingReconciliation
        } else {
            ReportStatus::Preliminary
        };

        // B.1 row 18, D-B5: every verified CLOSE count's counter and verifier, names from `users`.
        let names: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, name FROM users WHERE id = ANY($1)",
        )
        .bind(
            close_counts
                .iter()
                .flat_map(|c| [c.1, c.2])
                .collect::<Vec<Uuid>>(),
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        let name_of = |id: Uuid| {
            names
                .iter()
                .find(|n| n.0 == id)
                .map(|n| n.1.clone())
                .unwrap_or_else(|| id.to_string())
        };
        let signers = close_counts
            .iter()
            .flat_map(|c| {
                [
                    ReportSigner {
                        name: name_of(c.1),
                        capacity: SignerCapacity::Counted,
                    },
                    ReportSigner {
                        name: name_of(c.2),
                        capacity: SignerCapacity::Verified,
                    },
                ]
            })
            .collect();

        // B.1 row 14, Q-SR2: every open hand-over and transfer, this branch at either end, not by day.
        let pending_requests: i64 = sqlx::query_scalar(
            "SELECT (SELECT count(*) FROM handovers WHERE branch_id = $1 AND status = 'PENDING') \
                  + (SELECT count(*) FROM cash_transfers \
                     WHERE status IN ('DRAFT', 'APPROVED', 'DISPATCHED') \
                       AND (from_branch_id = $1 OR to_branch_id = $1))",
        )
        .bind(b)
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;

        // B.1 row 14: the day's OPEN exceptions, through the kept custody reader (epoch ms `[from, to)`).
        let open_exceptions = self
            .custody
            .exceptions_between(branch, from.timestamp_millis(), to.timestamp_millis())
            .await?
            .iter()
            .filter(|e| e.status == ExceptionStatus::Open)
            .count() as u64;

        // B.1 row 15, Q-SR3: the day's count and transfer notes, non-empty, at most 5, in creation order.
        let notes: Vec<(String, String)> = sqlx::query_as(
            "SELECT kind, note FROM ( \
               SELECT kind, note, created_at, id FROM cash_counts \
               WHERE branch_id = $1 AND counted_at >= $2 AND counted_at < $3 \
               UNION ALL \
               SELECT kind, note, created_at, id FROM cash_transfers \
               WHERE (from_branch_id = $1 OR to_branch_id = $1) \
                 AND created_at >= $2 AND created_at < $3 \
             ) n WHERE coalesce(btrim(note), '') <> '' \
             ORDER BY created_at, id LIMIT $4",
        )
        .bind(b)
        .bind(from)
        .bind(to)
        .bind(NOTES)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;

        // B.1 row 16: every OPEN hold of the branch, not by day.
        let open_holds = self.custody.open_holds(branch).await?;

        // B.1 row 17, Q-SR4: every DISPATCHED transfer now, every kind, either end; Σ qty_sent × face_value.
        let in_transit: Vec<(String, Decimal, String)> = sqlx::query_as(
            "SELECT t.kind, sum(l.qty_sent * d.face_value), trim(l.currency) \
             FROM cash_transfers t \
             JOIN cash_transfer_lines l ON l.transfer_id = t.id \
             JOIN denominations d ON d.id = l.denomination_id \
             WHERE t.status = 'DISPATCHED' AND (t.from_branch_id = $1 OR t.to_branch_id = $1) \
             GROUP BY t.id, t.kind, t.dispatched_at, t.created_at, trim(l.currency) \
             ORDER BY t.dispatched_at NULLS LAST, t.created_at, t.id, trim(l.currency)",
        )
        .bind(b)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;

        Ok(StaffDailyReportData {
            day: day.to_string(),
            timezone,
            generated_at: now.to_rfc3339(),
            company,
            site,
            branch_name,
            status,
            completed: completed as u64,
            buy_volume: money(buy.unwrap_or_default()),
            sell_volume: money(sell.unwrap_or_default()),
            reversed_volume: money(reversed.unwrap_or_default()),
            pending_transactions: pending_transactions as u64,
            currencies,
            pending_requests: pending_requests as u64,
            open_exceptions,
            notes: notes
                .into_iter()
                .map(|(kind, note)| ReportNote { kind, note })
                .collect(),
            open_holds,
            in_transit: in_transit
                .into_iter()
                .map(|(kind, amount, currency)| ReportTransit {
                    kind,
                    amount: money(amount),
                    currency,
                })
                .collect(),
            signers,
            finance_note: certification.flatten(),
        })
    }
}

impl BranchView for StaffDailyReportView {
    const NAME: &'static str = "staff_daily_report";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::StaffRpc, "staffDailyReport");
    const VERSIONED: bool = false;
    type Data = StaffDailyReportData;
    type Filter = StaffDailyReportFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        // the filter's check has already refused a malformed day
        let day = filter.day.as_deref().and_then(parse_day);
        async move { self.report(branch, day).await }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(StaffDailyReportView {
            pool: state.db.clone(),
            custody: Arc::new(PgCustodyPages::new(state.db.clone())),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    StaffDailyReportFilter::export_all(&cfg)?;
    ReportStatus::export_all(&cfg)?;
    ReportCurrencyRow::export_all(&cfg)?;
    ReportNote::export_all(&cfg)?;
    ReportTransit::export_all(&cfg)?;
    SignerCapacity::export_all(&cfg)?;
    ReportSigner::export_all(&cfg)?;
    StaffDailyReportData::export_all(&cfg)?;
    Ok(())
}
