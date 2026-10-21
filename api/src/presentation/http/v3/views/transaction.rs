//! The B04 `transaction` view (ticket #678, design `quote-transaction.md` §4.2).

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{PgPool, Row};
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::transaction_detail::{
    TransactionDetailDto, TransactionFilter, TransactionViewData, TxApprovalDto, TxBenefitDto,
    TxBookingRefDto, TxLineDto, TxPaymentDto, TxReceiptDto,
};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct TransactionView {
    pool: PgPool,
}

fn ts(v: Option<DateTime<Utc>>) -> Option<String> {
    v.map(|t| t.to_rfc3339())
}

fn parse_benefits(snapshot: Option<serde_json::Value>, tx_id: Uuid) -> Vec<TxBenefitDto> {
    let Some(items) = snapshot
        .and_then(|mut s| s.get_mut("applied").map(serde_json::Value::take))
        .and_then(|v| match v {
            serde_json::Value::Array(arr) => Some(arr),
            _ => None,
        })
    else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match serde_json::from_value::<TxBenefitDto>(item) {
            Ok(b) => out.push(b),
            Err(err) => {
                tracing::warn!(
                    transaction_id = %tx_id,
                    error = %err,
                    "skipping malformed applied benefit in quote benefit_snapshot"
                );
            }
        }
    }
    out
}

impl BranchView for TransactionView {
    const NAME: &'static str = "transaction";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "transactionView");
    const VERSIONED: bool = false;
    type Data = TransactionViewData;
    type Filter = TransactionFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch = branch.as_str().to_string();
        let id = filter.id.clone();
        async move {
            let none = Ok(TransactionViewData { transaction: None });
            let Some(id) = id.and_then(|s| Uuid::from_str(&s).ok()) else {
                return none;
            };
            let Some(r) = sqlx::query(
                "SELECT t.id, t.status, t.document_ref, t.branch_id, t.customer_id, t.quote_id, \
                 q.expires_at, t.approval_request_id, a.status AS a_status, t.completed_at, \
                 t.cancelled_at, t.cancel_reason, q.benefit_snapshot \
                 FROM transactions t LEFT JOIN quotes q ON q.id = t.quote_id \
                 LEFT JOIN approval_requests a ON a.id = t.approval_request_id \
                 WHERE t.id = $1 AND t.branch_id = $2",
            )
            .bind(id)
            .bind(&branch)
            .fetch_optional(&pool)
            .await
            .map_err(AppError::Database)?
            else {
                return none;
            };
            let lines = sqlx::query(
                "SELECT line_no, currency, shop_side, quantity, unit_size, rate_snapshot, thb_amount, \
                 fee_thb, settle_thb FROM transaction_lines WHERE transaction_id = $1 ORDER BY line_no",
            )
            .bind(id)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?
            .into_iter()
            .map(|l| TxLineDto {
                line_no: l.get(0),
                currency: l.get::<String, _>(1).trim().to_string(),
                side: l.get(2),
                quantity: format!("{:.4}", l.get::<Decimal, _>(3)),
                unit_size: format!("{:.4}", l.get::<Decimal, _>(4)),
                rate_snapshot: format!("{:.8}", l.get::<Decimal, _>(5)),
                thb_amount: format!("{:.2}", l.get::<Decimal, _>(6)),
                fee_thb: format!("{:.2}", l.get::<Decimal, _>(7)),
                settle_thb: format!("{:.2}", l.get::<Decimal, _>(8)),
            })
            .collect();
            let payment = sqlx::query(
                "SELECT method, reference, account, slip_file_key, amount_thb, bank_fee_thb, paid_at \
                 FROM transaction_payments WHERE transaction_id = $1 ORDER BY payment_no LIMIT 1",
            )
            .bind(id)
            .fetch_optional(&pool)
            .await
            .map_err(AppError::Database)?
            .map(|p| TxPaymentDto {
                method: p.get(0),
                reference: p.get(1),
                account: p.get(2),
                slip_file_key: p.get(3),
                amount_thb: format!("{:.2}", p.get::<Decimal, _>(4)),
                bank_fee_thb: format!("{:.2}", p.get::<Decimal, _>(5)),
                paid_at: p.get::<DateTime<Utc>, _>(6).to_rfc3339(),
            });
            let receipts = sqlx::query(
                "SELECT number, revision, issued_at FROM receipts WHERE transaction_id = $1 ORDER BY revision",
            )
            .bind(id)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?
            .into_iter()
            .map(|x| TxReceiptDto {
                number: x.get(0),
                revision: x.get(1),
                issued_at: x.get::<DateTime<Utc>, _>(2).to_rfc3339(),
            })
            .collect();
            let approval = r.get::<Option<Uuid>, _>(7).map(|a| TxApprovalDto {
                id: a.to_string(),
                status: r.get("a_status"),
            });
            let reversal = crate::presentation::dto::v3::reversal::load_reversal_summary(
                &pool,
                crate::presentation::dto::v3::reversal::ReversalBy::Original(id),
                &branch,
            )
            .await?;
            let benefits = parse_benefits(r.get(12), id);
            let booking =
                sqlx::query("SELECT id, booking_no FROM bookings WHERE transaction_id = $1")
                    .bind(id)
                    .fetch_optional(&pool)
                    .await
                    .map_err(AppError::Database)?
                    .map(|b| TxBookingRefDto {
                        id: b.get(0),
                        booking_no: b.get(1),
                    });
            Ok(TransactionViewData {
                transaction: Some(TransactionDetailDto {
                    id: r.get::<Uuid, _>(0).to_string(),
                    status: r.get(1),
                    document_ref: r.get(2),
                    branch_id: r.get(3),
                    customer_id: r.get(4),
                    quote_id: r.get::<Option<Uuid>, _>(5).map(|q| q.to_string()),
                    quote_expires_at: ts(r.get(6)),
                    lines,
                    payment,
                    receipts,
                    approval_request: approval,
                    completed_at: ts(r.get(9)),
                    cancelled_at: ts(r.get(10)),
                    cancel_reason: r.get(11),
                    reversal,
                    benefits,
                    booking,
                }),
            })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(
        cfg,
        Arc::new(TransactionView {
            pool: state.db.clone(),
        }),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::transaction_detail::export_types(dir)
}
