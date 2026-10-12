//! The `desk_refunds` view (ticket #1265, DK-K).
//! Lists approved refunds inside their reversal window waiting to be posted.

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::web;
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Row};
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::NoFilter;
use crate::presentation::dto::v3::desk_refunds::{
    DeskRefundLine, DeskRefundPayment, DeskRefundRow, DeskRefunds,
};
use crate::presentation::dto::v3::reversal::{ReversalBy, load_reversal_summary};
use crate::presentation::http::v3::view::{BranchView, ViewEndpoint};

pub struct DeskRefundsView {
    pool: PgPool,
}

impl BranchView for DeskRefundsView {
    const NAME: &'static str = "desk_refunds";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "reversalRequest");
    const VERSIONED: bool = false;
    type Data = DeskRefunds;
    type Filter = NoFilter;

    fn load(
        &self,
        branch: &BranchId,
        _who: &Actor,
        _filter: &Self::Filter,
    ) -> impl Future<Output = AppResult<Self::Data>> + Send {
        let pool = self.pool.clone();
        let branch_id = branch.as_str().to_string();
        async move {
            let reversal_ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM reversals WHERE branch_id = $1 AND status = 'APPROVED' \
                 ORDER BY requested_at ASC, created_at ASC",
            )
            .bind(&branch_id)
            .fetch_all(&pool)
            .await
            .map_err(AppError::Database)?;

            let mut rows = Vec::new();
            for id in reversal_ids {
                let Some(summary) =
                    load_reversal_summary(&pool, ReversalBy::Id(id), &branch_id).await?
                else {
                    continue;
                };
                if summary.status != "APPROVED" {
                    continue;
                }

                let original_id = Uuid::from_str(&summary.original_transaction_id)
                    .map_err(|e| AppError::Internal(format!("invalid original_transaction_id: {e}")))?;

                let receipt_row = sqlx::query(
                    "SELECT number FROM receipts WHERE transaction_id = $1 AND revision = 1 LIMIT 1",
                )
                .bind(original_id)
                .fetch_optional(&pool)
                .await
                .map_err(AppError::Database)?;
                let receipt_number = receipt_row
                    .map(|r| r.get::<String, _>("number"))
                    .unwrap_or_default();

                let line_rows = sqlx::query(
                    "SELECT shop_side, currency, quantity::text AS quantity, settle_thb::text AS settle_thb \
                     FROM transaction_lines WHERE transaction_id = $1 ORDER BY line_no ASC",
                )
                .bind(original_id)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?;
                let lines = line_rows
                    .into_iter()
                    .map(|r| DeskRefundLine {
                        shop_side: r.get("shop_side"),
                        currency: r.get("currency"),
                        quantity: r.get("quantity"),
                        settle_thb: r.get("settle_thb"),
                    })
                    .collect();

                let payment_rows = sqlx::query(
                    "SELECT method, amount_thb::text AS amount_thb \
                     FROM transaction_payments WHERE transaction_id = $1 ORDER BY payment_no ASC",
                )
                .bind(original_id)
                .fetch_all(&pool)
                .await
                .map_err(AppError::Database)?;
                let payments = payment_rows
                    .into_iter()
                    .map(|r| DeskRefundPayment {
                        method: r.get("method"),
                        amount_thb: r.get("amount_thb"),
                    })
                    .collect();

                rows.push(DeskRefundRow {
                    reversal: summary,
                    receipt_number,
                    lines,
                    payments,
                });
            }

            rows.sort_by(|a, b| a.reversal.approved_at.cmp(&b.reversal.approved_at));
            Ok(DeskRefunds { rows })
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    ViewEndpoint::register(cfg, Arc::new(DeskRefundsView { pool: state.db.clone() }));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::desk_refunds::export_types(dir)
}
