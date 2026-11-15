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
//!
//! Restated for GA-4 (#6928, `docs/design/blueprint/owner-set-values.md` row 5, G-7 to G-9): the cash step is the
//! Owner's `thb_cash_step`, never a constant. Every cash case first saves a `PricingRules` version with
//! `thb_cash_step` 0.25 (`cash_step_rule`), and keeps today's meaning; the CHK rows that carry a rounding record the
//! step 0.25 they were rounded with (the GA-4 line CHECK bounds a rounding by the line's own step). Added: each
//! stored line holds `cash_step_thb` (G-7), a 0.50 step stores 0.50 (G-8), and with no rule a cash quote is
//! refused `pricingRuleNotSet` / `thbCashStep` while a PromptPay quote is not (G-9).

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

/// GA-4: the Owner saves a pricing rule version for `ORG_1` with `thb_cash_step = step` (a fixture row, never a
/// constant in code).
async fn cash_step_rule(db: &TestDb, step: &str) {
    db.exec(&format!(
        "INSERT INTO pricing_rule_versions (version, organization_id, thb_cash_step, set_by, created_at) \
         VALUES ('{}', '{ORG_1}', {step}, 'owner-1', clock_timestamp())",
        Uuid::new_v4()
    ))
    .await;
}

// ============================================================ CR7

#[actix_web::test]
async fn cr7_omitted_payment_method_defaults_to_non_cash_and_unknown_method_is_refused() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD_32_10]).await;
    // GA-4: no payment method is an undecided payout, whose drawer estimate assumes cash and so needs the step.
    cash_step_rule(&db, "0.25").await;

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
    cash_step_rule(&db, "0.25").await;

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
    assert_eq!(
        q_buy.lines[0].cash_step_thb,
        Some(d("0.25")),
        "G-7: the line records the step"
    );

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
    assert_eq!(
        q_sell.lines[0].cash_step_thb,
        Some(d("0.25")),
        "G-7: the line records the step"
    );

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
    assert_eq!(
        q_pp.lines[0].cash_step_thb, None,
        "a non-cash line records no step"
    );

    db.finish().await;
}

// ============================================================ CR5v

#[actix_web::test]
async fn cr5v_cash_sell_whose_settle_rounds_to_zero_is_refused_on_quantity_and_stores_nothing() {
    let db = TestDb::start().await;
    // JPY sell rate 24.0 per 100 -> 1 JPY has thb_amount = 0.24, which cash SELL rounds down to 0.00.
    db.publish("MAIN", &[JPY]).await;
    cash_step_rule(&db, "0.25").await;

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

    // quote_lines: |settle_thb - (thb_amount ∓ fee_thb)| < 0.25, the step these rows record (GA-4)
    let ql_insert = |side: &str, thb: &str, fee: &str, settle: &str, line_no: i32| {
        let id = Uuid::new_v4();
        format!(
            "INSERT INTO quote_lines (id, organization_id, quote_id, published_rate_line_id, \
                 shop_side, currency, denomination_group, quantity, unit_size, base_rate, \
                 final_rate, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at, \
                 cash_step_thb) \
             VALUES ('{id}', '{ORG_1}', '{q_id}', '{line_id}', '{side}', 'USD', 'ALL', 1, 1, \
                 32.10, 32.10, {thb}, {fee}, {settle}, {line_no}, now(), now(), 0.25)"
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

    // transaction_lines: |settle_thb - (thb_amount ∓ fee_thb)| < 0.25, the step these rows record (GA-4)
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
                 line_no, created_at, updated_at, cash_step_thb) \
             VALUES ('{id}', '{ORG_1}', '{tx_id}', '{side}', 'USD', 1, 1, 32.10, \
                 {thb}, {fee}, {settle}, {line_no}, now(), now(), 0.25)"
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

// ============================================================ GA-4 (#6928): the step is the Owner's rule

/// G-8: with the Owner's `thb_cash_step` 0.50, a cash BUY of USD 1 at 32.10 settles to 32.50 and its line records
/// 0.50.
#[actix_web::test]
async fn g8_a_half_baht_step_from_the_rules_settles_and_is_recorded() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD_32_10]).await;
    cash_step_rule(&db, "0.50").await;

    let buy = post_quotes(
        &db,
        ME,
        Some("g8-buy-cash"),
        json!({
            "branch": "MAIN",
            "paymentMethod": "Cash",
            "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": "1" }]
        }),
    )
    .await;
    assert_eq!(buy.status, StatusCode::OK, "{}", buy.text);
    assert_eq!(buy.body["lines"][0]["settleThb"], json!("32.50"));
    assert_eq!(buy.body["lines"][0]["roundingThb"], json!("0.40"));
    let q = stored_quote(&db, buy.body["id"].as_str().expect("id")).await;
    assert_eq!(q.lines[0].cash_step_thb, Some(d("0.50")));
    assert_eq!(
        db.text("SELECT cash_step_thb::text FROM quote_lines").await,
        Some("0.50".to_owned()),
        "the stored row holds the step"
    );
    db.finish().await;
}

/// G-9: with no `thb_cash_step` saved, a cash quote is refused `pricingRuleNotSet` naming `thbCashStep` and stores
/// nothing; a PromptPay quote never reads the rule and succeeds.
#[actix_web::test]
async fn g9_with_no_cash_step_a_cash_quote_is_refused_and_promptpay_is_not() {
    let db = TestDb::start().await;
    db.publish("MAIN", &[USD_32_10]).await;
    // "not set": remove the fixture rule `TestDb::start` saves (`quote_common::save_cash_step_rule`).
    db.exec(&format!(
        "DELETE FROM pricing_rule_versions WHERE organization_id = '{ORG_1}'"
    ))
    .await;

    let cash = post_quotes(
        &db,
        ME,
        Some("g9-cash"),
        json!({
            "branch": "MAIN",
            "paymentMethod": "Cash",
            "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": "1" }]
        }),
    )
    .await;
    assert_eq!(cash.code(), "pricingRuleNotSet", "{}", cash.text);
    assert_eq!(cash.field(), Some("thbCashStep"), "{}", cash.text);
    assert_eq!(
        db.quote_rows().await,
        (0, 0),
        "a refused cash quote stores nothing"
    );

    let promptpay = post_quotes(
        &db,
        ME,
        Some("g9-promptpay"),
        json!({
            "branch": "MAIN",
            "paymentMethod": "PromptPay",
            "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": "1" }]
        }),
    )
    .await;
    assert_eq!(promptpay.status, StatusCode::OK, "{}", promptpay.text);
    assert_eq!(promptpay.body["lines"][0]["settleThb"], json!("32.10"));
    db.finish().await;
}
