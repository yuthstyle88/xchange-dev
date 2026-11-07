//! F56a up-front tests (`docs/design/backdated-keying.md` §16.3): `GET /api/v3/views/backdate_safe` answers the branch's
//! SAFE balance (the place a backdated sale posts at), every non-zero currency ordered by code and written to its minor
//! unit the way `staff_floor` writes `safe`. Written before the implementation by the sub-controller claude/3b81c9d5.
//! Do not edit, delete or weaken; add tests in a file of your own. Needs `TEST_DATABASE_URL`.

mod common;
mod quote_common;
mod tx4_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use serde_json::{Value, json};
use sqlx::PgPool;
use tx4_common::*;
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const OWNER: &str = "owner-f56";
static FUND: AtomicU64 = AtomicU64::new(0);

struct OwnerSession;

impl SessionReader for OwnerSession {
    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let callers = test_callers::callers(Some(test_callers::owner(OWNER)), None);
        Box::pin(async move { callers })
    }
}

async fn read(desk: &Desk) -> (StatusCode, Value) {
    let state = AppState::new(Arc::new(common::settings(&[])), desk.db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(OwnerSession);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri("/api/v3/views/backdate_safe?branch=MAIN")
        .insert_header((header::HOST, "xc.test"))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Posts `amount` of `currency` from the bank into MAIN's SAFE through the ledger.
async fn fund(pool: &PgPool, currency: &str, amount: &str) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    let mut tx = pool.begin().await.expect("begin");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to: CashPlace::Safe(BranchId::new("MAIN")),
                currency: xc_kernel::money::CurrencyCode::parse(currency).expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: format!("f56-fund:{}", FUND.fetch_add(1, Ordering::Relaxed)),
                    source_key: "fund".to_string(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("fund through the ledger");
    tx.commit().await.expect("commit");
}

#[actix_web::test]
async fn f56a_1_lists_the_safe_balance_by_code_at_each_minor_unit() {
    let desk = Desk::start().await;
    fund(&desk.db.pool, "USD", "520").await;
    fund(&desk.db.pool, "THB", "150000").await;
    fund(&desk.db.pool, "JPY", "5000").await;

    let (status, body) = read(&desk).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["data"]["balances"],
        json!([
            { "currency": "JPY", "amount": "5000" },
            { "currency": "THB", "amount": "150000.00" },
            { "currency": "USD", "amount": "520.00" },
        ])
    );
}

#[actix_web::test]
async fn f56a_2_an_empty_safe_answers_an_empty_list() {
    let desk = Desk::start().await;
    let (status, body) = read(&desk).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["balances"], json!([]));
}
