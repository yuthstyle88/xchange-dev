//! `GET /api/v3/views/my_transactions` (ticket #1003, APP-11).
//! Design `docs/design/blueprint/customer-app.md` §3 (C04), §4.4, §9.1.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Row};
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::common::{NoFilter, ViewAnswer};
use crate::presentation::dto::v3::my_transactions::{MyTransactionItem, MyTransactionsData};
use crate::presentation::http::v3::command::out_of_scope;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{
    BranchView, authenticate_and_authorize, parse_branch, require_branch,
};

pub struct MyTransactionsView {
    db: PgPool,
}

impl MyTransactionsView {
    async fn fetch_transactions(
        &self,
        customer_id: &str,
    ) -> Result<Vec<MyTransactionItem>, sqlx::Error> {
        let rows = sqlx::query(
            r#"
        SELECT
            t.id,
            t.value_date,
            t.branch_id,
            b.name AS branch_name,
            l1.shop_side AS type,
            l1.currency,
            l1.quantity AS amount,
            l1.rate_snapshot AS rate,
            totals.total_principal AS principal,
            totals.total_fee AS fee,
            totals.total_settlement AS settlement,
            r.number AS receipt_ref,
            rev.id AS reversal_id
        FROM transactions t
        JOIN branches b ON b.id = t.branch_id
        JOIN LATERAL (
            SELECT
                l1.shop_side,
                l1.currency,
                l1.quantity,
                l1.rate_snapshot
            FROM transaction_lines l1
            WHERE l1.transaction_id = t.id AND l1.line_no = 1
            LIMIT 1
        ) l1 ON true
        JOIN LATERAL (
            SELECT
                COALESCE(SUM(l.thb_amount), 0) AS total_principal,
                COALESCE(SUM(l.fee_thb), 0) AS total_fee,
                COALESCE(SUM(l.settle_thb), 0) AS total_settlement
            FROM transaction_lines l
            WHERE l.transaction_id = t.id
        ) totals ON true
        LEFT JOIN LATERAL (
            SELECT number
            FROM receipts
            WHERE transaction_id = t.id
            ORDER BY revision DESC
            LIMIT 1
        ) r ON true
        LEFT JOIN reversals rev ON rev.original_transaction_id = t.id AND rev.status = 'POSTED'
        WHERE t.customer_id = $1 AND t.status = 'COMPLETED'
        ORDER BY t.recorded_at DESC, t.id DESC
        "#,
        )
        .bind(customer_id)
        .fetch_all(&self.db)
        .await?;

        let items = rows
            .into_iter()
            .map(|row| {
                let id: uuid::Uuid = row.get("id");
                let value_date: chrono::NaiveDate = row.get("value_date");
                let branch: String = row.get("branch_id");
                let branch_name: String = row.get("branch_name");
                let tx_type: String = row.get("type");
                let currency: String = row.get("currency");
                let amount: rust_decimal::Decimal = row.get("amount");
                let rate: rust_decimal::Decimal = row.get("rate");
                let principal: rust_decimal::Decimal = row.get("principal");
                let fee: rust_decimal::Decimal = row.get("fee");
                let settlement: rust_decimal::Decimal = row.get("settlement");
                let receipt_ref: Option<String> = row.get("receipt_ref");
                let rev_id: Option<uuid::Uuid> = row.get("reversal_id");

                let status = if rev_id.is_some() {
                    "REVERSED".to_string()
                } else {
                    "COMPLETED".to_string()
                };

                MyTransactionItem {
                    id: id.to_string(),
                    date: value_date.to_string(),
                    branch,
                    branch_name,
                    r#type: tx_type,
                    currency,
                    amount: format!("{:.2}", amount),
                    rate: rate.to_string(),
                    principal: format!("{:.2}", principal),
                    fee: format!("{:.2}", fee),
                    settlement: format!("{:.2}", settlement),
                    receipt_ref: receipt_ref.unwrap_or_default(),
                    status,
                }
            })
            .collect();

        Ok(items)
    }
}

impl BranchView for MyTransactionsView {
    const NAME: &'static str = "my_transactions";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "my_transactions");
    const VERSIONED: bool = false;
    type Data = MyTransactionsData;
    type Filter = NoFilter;

    async fn load(
        &self,
        _branch: &BranchId,
        who: &Actor,
        _filter: &Self::Filter,
    ) -> AppResult<Self::Data> {
        let own = who.customer_id.as_deref().ok_or_else(|| AppError::NotFound {
            entity: "customer",
            id: String::new(),
        })?;
        let transactions = self
            .fetch_transactions(own)
            .await
            .map_err(AppError::Database)?;
        Ok(MyTransactionsData { transactions })
    }
}

fn server_error_refusal() -> HttpResponse {
    V3Error::new(
        "serverError",
        None,
        "เซิร์ฟเวอร์ขัดข้อง",
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .error_response()
}

async fn handle(req: HttpRequest, view: web::Data<Arc<MyTransactionsView>>) -> HttpResponse {
    let named = web::Query::<std::collections::HashMap<String, String>>::from_query(
        req.query_string(),
    )
    .ok()
    .and_then(|q| q.into_inner().remove("branch"));
    match parse_branch(named.as_deref()) {
        Ok(Some(branch)) => {
            if let Err(e) = require_branch(&req, &branch).await {
                return e.error_response();
            }
        }
        Ok(None) => {}
        Err(()) => {
            return V3Error::new(
                "invalidRequest",
                Some("branch"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    }
    let who = match authenticate_and_authorize(&req, MyTransactionsView::INHERITS).await {
        Ok(who) => who,
        Err(refusal) => return refusal,
    };
    let Some(own_customer_id) = who.customer_id.as_deref() else {
        return out_of_scope().error_response();
    };

    match view.fetch_transactions(own_customer_id).await {
        Ok(items) => HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(ViewAnswer {
                version: 0,
                changed: true,
                data: Some(MyTransactionsData {
                    transactions: items,
                }),
            }),
        Err(e) => {
            tracing::error!(error = %e, "my_transactions query failed");
            server_error_refusal()
        }
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let view = Arc::new(MyTransactionsView {
        db: state.db.clone(),
    });
    cfg.service(
        web::resource("/api/v3/views/my_transactions")
            .app_data(web::Data::new(view))
            .route(web::get().to(handle)),
    );
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    crate::presentation::dto::v3::my_transactions::export_types(dir)
}
