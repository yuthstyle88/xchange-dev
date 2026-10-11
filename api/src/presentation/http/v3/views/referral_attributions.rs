//! RA-3 (#981, design `broker-seam.md` §4.3): the `referral_attributions` view, a branch's
//! captures, newest first, `?transaction=` narrowing to one exchange.
//!
//! `customer_sub` is masked as `Redactor` masks (`***` + the last four characters), applied here
//! because `Redactor`'s deny-list has no `customerSub` key; no phone is ever read.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, SecondsFormat, Utc};
use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::referral_attributions::{
    ReferralAttributionRow, ReferralAttributionsFilter,
};
use crate::presentation::http::v3::view::{PagedView, RowId, ViewEndpoint};

pub struct ReferralAttributionsPagedView {
    pool: PgPool,
}

impl ReferralAttributionsPagedView {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct CaptureSqlRow {
    id: Uuid,
    code: String,
    status: String,
    source: String,
    transaction_id: Option<Uuid>,
    booking_id: Option<String>,
    customer_sub: Option<String>,
    captured_at: DateTime<Utc>,
    captured_by: String,
    supersedes_id: Option<Uuid>,
    reason: Option<String>,
    broker_outcome: Option<String>,
    pushed_at: Option<DateTime<Utc>>,
}

/// `Redactor`'s format (`xc-approval/src/domain/redactor.rs` `mask_value`): `***` + the last four.
fn mask(sub: &str) -> String {
    let count = sub.chars().count();
    let last4: String = sub.chars().skip(count.saturating_sub(4)).collect();
    format!("***{last4}")
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

impl From<CaptureSqlRow> for ReferralAttributionRow {
    fn from(r: CaptureSqlRow) -> Self {
        Self {
            id: r.id.to_string(),
            code: r.code,
            status: r.status,
            source: r.source,
            transaction_id: r.transaction_id.map(|t| t.to_string()),
            booking_id: r.booking_id,
            customer_sub: r.customer_sub.as_deref().map(mask),
            captured_at: stamp(r.captured_at),
            captured_by: r.captured_by,
            supersedes_id: r.supersedes_id.map(|s| s.to_string()),
            reason: r.reason,
            broker_outcome: r.broker_outcome,
            pushed_at: r.pushed_at.map(stamp),
        }
    }
}

impl PagedView for ReferralAttributionsPagedView {
    const NAME: &'static str = "referral_attributions";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "referralAttributions");
    type Row = ReferralAttributionRow;
    type Filter = ReferralAttributionsFilter;

    fn page(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
        before: Option<RowId>,
        limit: u16,
    ) -> impl Future<Output = AppResult<Vec<Self::Row>>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_owned();
        // `ViewFilter::check` has already refused a `transaction` that is not a uuid.
        let transaction = filter
            .transaction
            .as_deref()
            .and_then(|t| Uuid::parse_str(t).ok());
        async move {
            // A cursor that names no row of this branch continues after nothing: an empty page.
            let before = match before {
                None => None,
                Some(b) => match Uuid::parse_str(&b) {
                    Ok(id) => Some(id),
                    Err(_) => return Ok(Vec::new()),
                },
            };
            // Newest first by `captured_at`, then `id`; `before` continues after that row.
            let rows: Vec<CaptureSqlRow> = sqlx::query_as(
                "SELECT r.id, r.code, r.status, r.source, r.transaction_id, r.booking_id, \
                        r.customer_sub, r.captured_at, r.captured_by, r.supersedes_id, r.reason, \
                        r.broker_outcome, r.pushed_at \
                 FROM referral_attributions r \
                 WHERE r.branch_id = $1 \
                   AND ($2::uuid IS NULL OR r.transaction_id = $2) \
                   AND ($3::uuid IS NULL OR (r.captured_at, r.id) < \
                        (SELECT c.captured_at, c.id FROM referral_attributions c \
                         WHERE c.id = $3 AND c.branch_id = $1)) \
                 ORDER BY r.captured_at DESC, r.id DESC \
                 LIMIT $4",
            )
            .bind(&branch_id)
            .bind(transaction)
            .bind(before)
            .bind(i64::from(limit))
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;
            Ok(rows.into_iter().map(ReferralAttributionRow::from).collect())
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register_paged(
        cfg,
        Arc::new(ReferralAttributionsPagedView::new(state.db.clone())),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    ReferralAttributionsFilter::export_all(&cfg)?;
    ReferralAttributionRow::export_all(&cfg)?;
    Ok(())
}
