//! Ticket #5122 (CR-b) — up-front tests for quote cash settlement rounding (`Settlement::of` wired
//! into `CreateQuote` and `POST /api/quotes`, `Quote.cash_settlement`, `QuoteLine::rounding_thb`,
//! and the `< 0.25` CHECK on `quote_lines` and `transaction_lines`).
//! Written before the implementation by the up-front test writer (gemini/pool-2). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §13; #5122 §3, §4 (CR7, CR7b, CR5v, CHK).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ME: &str = "staff:FRONT:MAIN";

/// USD with both buy and sell at 32.10 per 1, so CR7 and CR7b exercise BUY and SELL at 32.10.
const USD_32_10: Line = Line::manual("USD", "1", "32.10", "32.10");

struct HeaderSessions;

fn callers_of(spec: &str) -> Callers {
    if spec.is_empty() {
        return test_callers::callers(None, None);
    }
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = parts.next().expect("a role");
    let branch = parts.next().expect("a branch");
    let roles = test_callers::v2_staff_roles(role).expect("a known role");
    let staff = Some(test_callers::staff(
        &format!("employee-{role}"),
        roles,
        BranchId::new(branch),
    ));
    test_callers::callers(None, staff)
}

impl SessionReader for HeaderSessions {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { callers_of(&spec) })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    fn field(&self) -> Option<&str> {
        self.body["error"]["field"].as_str()
    }
}

async fn post_quotes(db: &TestDb, who: &str, key: Option<&str>, body: Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::post()
        .uri("/api/quotes")
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .set_payload(body.to_string());
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    if let Some(k) = key {
        req = req.insert_header(("idempotency-key", k));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn stored_quote(db: &TestDb, id: &str) -> xc_exchange::domain::quote::quote::Quote {
    let repo = PgQuoteRepository::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let q = repo
        .find_in(&mut tx, Uuid::parse_str(id).expect("a quote uuid"))
        .await
        .expect("find_in")
        .expect("quote stored");
    tx.commit().await.expect("commit");
    q
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

// ============================================================ CR7

#[actix_web::test]
async fn cr7_omitted_payment_method_defaults_to_non_cash_and_unknown_method_is_refused() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD_32_10]).await;

    let ok = post_quotes(
        &db,
        ME,
        Some("cr7-default"),
        json!({
            "branch": "MAIN",
            "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": "1" }]
        }),
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    assert_eq!(ok.body["cashSettlement"], json!(false));
    assert_eq!(ok.body["lines"][0]["thbAmount"], json!("32.10"));
    assert_eq!(ok.body["lines"][0]["feeThb"], json!("0.00"));
    assert_eq!(ok.body["lines"][0]["roundingThb"], json!("0.00"));
    assert_eq!(ok.body["lines"][0]["settleThb"], json!("32.10"));

    let id = ok.body["id"].as_str().expect("quote id");
    let q = stored_quote(&db, id).await;
    assert!(
        !q.cash_settlement,
        "omitted paymentMethod stores cash_settlement = false"
    );
    assert_eq!(q.lines[0].rounding_thb(), d("0.00"));
    assert_eq!(db.quote_rows().await, (1, 1));

    let bad = post_quotes(
        &db,
        ME,
        Some("cr7-bitcoin"),
        json!({
            "branch": "MAIN",
            "paymentMethod": "Bitcoin",
            "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": "1" }]
        }),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);
    assert_eq!(bad.code(), "validationFailed");
    assert_eq!(bad.field(), Some("paymentMethod"));
    assert_eq!(
        db.quote_rows().await,
        (1, 1),
        "refused paymentMethod stores nothing"
    );

    db.finish().await;
}

// ============================================================ CR7b

#[actix_web::test]
async fn cr7b_cash_rounds_buy_and_sell_to_quarter_baht_while_promptpay_does_not() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD_32_10]).await;

    // 1. Cash BUY USD 1 at 32.10 -> settleThb "32.25", roundingThb "0.15", cashSettlement true.
    let buy_cash = post_quotes(
        &db,
        ME,
        Some("cr7b-buy-cash"),
        json!({
            "branch": "MAIN",
            "paymentMethod": "Cash",
            "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": "1" }]
        }),
    )
    .await;
    assert_eq!(buy_cash.status, StatusCode::OK, "{}", buy_cash.text);
    assert_eq!(buy_cash.body["cashSettlement"], json!(true));
    assert_eq!(buy_cash.body["lines"][0]["thbAmount"], json!("32.10"));
    assert_eq!(buy_cash.body["lines"][0]["feeThb"], json!("0.00"));
    assert_eq!(buy_cash.body["lines"][0]["settleThb"], json!("32.25"));
    assert_eq!(buy_cash.body["lines"][0]["roundingThb"], json!("0.15"));

    let q_buy = stored_quote(&db, buy_cash.body["id"].as_str().expect("id")).await;
    assert!(q_buy.cash_settlement);
    assert_eq!(q_buy.lines[0].settle_thb.to_string(), "32.25");
    assert_eq!(q_buy.lines[0].rounding_thb(), d("0.15"));

    // 2. Cash SELL USD 1 at 32.10 -> settleThb "32.00", roundingThb "-0.10", cashSettlement true.
    let sell_cash = post_quotes(
        &db,
        ME,
        Some("cr7b-sell-cash"),
        json!({
            "branch": "MAIN",
            "paymentMethod": "Cash",
            "lines": [{ "shopSide": "SELL", "currency": "USD", "quantity": "1" }]
        }),
    )
    .await;
    assert_eq!(sell_cash.status, StatusCode::OK, "{}", sell_cash.text);
    assert_eq!(sell_cash.body["cashSettlement"], json!(true));
    assert_eq!(sell_cash.body["lines"][0]["thbAmount"], json!("32.10"));
    assert_eq!(sell_cash.body["lines"][0]["feeThb"], json!("0.00"));
    assert_eq!(sell_cash.body["lines"][0]["settleThb"], json!("32.00"));
    assert_eq!(sell_cash.body["lines"][0]["roundingThb"], json!("-0.10"));

    let q_sell = stored_quote(&db, sell_cash.body["id"].as_str().expect("id")).await;
    assert!(q_sell.cash_settlement);
    assert_eq!(q_sell.lines[0].settle_thb.to_string(), "32.00");
    assert_eq!(q_sell.lines[0].rounding_thb(), d("-0.10"));

    // 3. PromptPay BUY USD 1 at 32.10 -> cashSettlement false, settleThb "32.10", roundingThb "0.00".
    let promptpay = post_quotes(
        &db,
        ME,
        Some("cr7b-promptpay"),
        json!({
            "branch": "MAIN",
            "paymentMethod": "PromptPay",
            "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": "1" }]
        }),
    )
    .await;
    assert_eq!(promptpay.status, StatusCode::OK, "{}", promptpay.text);
    assert_eq!(promptpay.body["cashSettlement"], json!(false));
    assert_eq!(promptpay.body["lines"][0]["settleThb"], json!("32.10"));
    assert_eq!(promptpay.body["lines"][0]["roundingThb"], json!("0.00"));

    let q_pp = stored_quote(&db, promptpay.body["id"].as_str().expect("id")).await;
    assert!(!q_pp.cash_settlement);
    assert_eq!(q_pp.lines[0].settle_thb.to_string(), "32.10");
    assert_eq!(q_pp.lines[0].rounding_thb(), d("0.00"));

    db.finish().await;
}

// ============================================================ CR5v

#[actix_web::test]
async fn cr5v_cash_sell_whose_settle_rounds_to_zero_is_refused_on_quantity_and_stores_nothing() {
    let db = TestDb::start().await;
    // JPY sell rate 24.0 per 100 -> 1 JPY has thb_amount = 0.24, which cash SELL rounds down to 0.00.
    db.publish("MAIN", &[JPY]).await;

    let refused = post_quotes(
        &db,
        ME,
        Some("cr5v-cash-zero"),
        json!({
            "branch": "MAIN",
            "paymentMethod": "Cash",
            "lines": [{ "shopSide": "SELL", "currency": "JPY", "quantity": "1" }]
        }),
    )
    .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
    assert_eq!(refused.code(), "validationFailed");
    assert_eq!(refused.field(), Some("quantity"));
    assert_eq!(
        db.quote_rows().await,
        (0, 0),
        "a cash SELL whose rounded settle is 0.00 must store nothing"
    );

    db.finish().await;
}

// ============================================================ CHK

#[actix_web::test]
async fn chk_quote_and_transaction_lines_refuse_rounding_at_or_above_one_quarter_baht() {
    let db = TestDb::start().await;
    let pub_1 = db.publish("MAIN", &[USD_32_10]).await;
    let line_id = pub_1.line("USD");
    db.customer("CUST-CHK", "MAIN").await;

    let q_id = Uuid::new_v4();
    db.exec(&format!(
        "INSERT INTO quotes (id, organization_id, branch_id, type, channel, status, \
             publication_id, expires_at, created_by, created_at, updated_at) \
         VALUES ('{q_id}', '{ORG_1}', 'MAIN', 'INDICATIVE', 'DESK', 'ACTIVE', '{}', \
             now() + interval '5 minutes', 'test', now(), now())",
        pub_1.id
    ))
    .await;

    // quote_lines: |settle_thb - (thb_amount ∓ fee_thb)| < 0.25
    let ql_insert = |side: &str, thb: &str, fee: &str, settle: &str, line_no: i32| {
        let id = Uuid::new_v4();
        format!(
            "INSERT INTO quote_lines (id, organization_id, quote_id, published_rate_line_id, \
                 shop_side, currency, denomination_group, quantity, unit_size, base_rate, \
                 final_rate, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
             VALUES ('{id}', '{ORG_1}', '{q_id}', '{line_id}', '{side}', 'USD', 'ALL', 1, 1, \
                 32.10, 32.10, {thb}, {fee}, {settle}, {line_no}, now(), now())"
        )
    };

    // BUY: net = thb_amount - fee_thb = 32.10 - 0.00 = 32.10
    assert_eq!(
        db.outcome(&ql_insert("BUY", "32.10", "0.00", "32.34", 1))
            .await,
        "ok",
        "quote_lines BUY with +0.24 rounding is within a coin"
    );
    assert_eq!(
        db.outcome(&ql_insert("BUY", "32.10", "0.00", "32.35", 2))
            .await,
        "23514",
        "quote_lines BUY with +0.25 rounding is refused by CHECK (23514)"
    );
    assert_eq!(
        db.outcome(&ql_insert("BUY", "32.10", "0.00", "31.85", 3))
            .await,
        "23514",
        "quote_lines BUY with -0.25 rounding is refused by CHECK (23514)"
    );

    // SELL: net = thb_amount + fee_thb = 32.10 + 0.00 = 32.10
    assert_eq!(
        db.outcome(&ql_insert("SELL", "32.10", "0.00", "31.86", 4))
            .await,
        "ok",
        "quote_lines SELL with -0.24 rounding is within a coin"
    );
    assert_eq!(
        db.outcome(&ql_insert("SELL", "32.10", "0.00", "31.85", 5))
            .await,
        "23514",
        "quote_lines SELL with -0.25 rounding is refused by CHECK (23514)"
    );
    assert_eq!(
        db.outcome(&ql_insert("SELL", "32.10", "0.00", "32.35", 6))
            .await,
        "23514",
        "quote_lines SELL with +0.25 rounding is refused by CHECK (23514)"
    );

    // transaction_lines: |settle_thb - (thb_amount ∓ fee_thb)| < 0.25
    let tx_id = Uuid::new_v4();
    db.exec(&format!(
        "INSERT INTO transactions (id, organization_id, branch_id, mode, status, document_ref, \
             submission_id, payload_hash, customer_id, customer_name, nationality, staff_id, \
             booking_no, notes, value_date, paid_at, recorded_at, recorded_by, count_confirmed) \
         VALUES ('{tx_id}', '{ORG_1}', 'MAIN', 'DEMO', 'DRAFT', NULL, 'sub-chk', 'hash', \
             'CUST-CHK', 'Test Customer', 'THA', 'staff-1', 'WALK-IN', '', '2026-10-06', \
             now(), now(), 'staff-1', false)"
    ))
    .await;

    let tl_insert = |side: &str, thb: &str, fee: &str, settle: &str, line_no: i32| {
        let id = Uuid::new_v4();
        format!(
            "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, \
                 currency, quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, \
                 line_no, created_at, updated_at) \
             VALUES ('{id}', '{ORG_1}', '{tx_id}', '{side}', 'USD', 1, 1, 32.10, \
                 {thb}, {fee}, {settle}, {line_no}, now(), now())"
        )
    };

    assert_eq!(
        db.outcome(&tl_insert("BUY", "32.10", "0.00", "32.34", 1))
            .await,
        "ok",
        "transaction_lines BUY with +0.24 rounding is within a coin"
    );
    assert_eq!(
        db.outcome(&tl_insert("BUY", "32.10", "0.00", "32.35", 2))
            .await,
        "23514",
        "transaction_lines BUY with +0.25 rounding is refused by CHECK (23514)"
    );
    assert_eq!(
        db.outcome(&tl_insert("SELL", "32.10", "0.00", "31.86", 3))
            .await,
        "ok",
        "transaction_lines SELL with -0.24 rounding is within a coin"
    );
    assert_eq!(
        db.outcome(&tl_insert("SELL", "32.10", "0.00", "31.85", 4))
            .await,
        "23514",
        "transaction_lines SELL with -0.25 rounding is refused by CHECK (23514)"
    );

    db.finish().await;
}
