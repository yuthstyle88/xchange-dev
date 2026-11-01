//! Ticket #5864 (T2-BE, `docs/design/blueprint/staff-e2e-1101-reads.md` §1, PR 5863): the desk
//! lists only the currencies it can sell.
//! Written before the implementation by the ticket author (claude/b091f5e0). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! What this pins:
//! - `desk_config.rates` and `staff_desk_config.rates` are read through the same port and rules as
//!   the quote (`PublishedRates::line_for`): the branch's current publication, denomination group
//!   `ALL`, the one untiered line or else the tier whose lower bound is NULL, ordered by code.
//!   No publication ⇒ `[]`. A line with both sides NULL is not listed. A line with one side is
//!   listed with the other side `null`. Register currencies with no line and THB are not listed.
//! - Each listed side is the rate the quote prices with, per the line's own `unitSize`
//!   (`POST /api/quotes` answers the same `baseRate` and `unitSize`).
//! - A currency the desk does not list is refused by the quote: 409 `rateUnavailable`, and the
//!   message names the currency.
//! - Both paths answer the same `rates`.
//!
//! Publications are seeded with plain SQL through `quote_common::TestDb::publish` (and a tiered
//! line by hand, as `rate_feed_first_tier_views_test.rs` does). Every request goes through the
//! server's whole route table with the session read from a test header.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const MAIN: &str = "MAIN";
const OWNER: &str = "owner";
const FRONT: &str = "staff:FRONT:MAIN";
const OWNER_VIEW: &str = "desk_config";
const STAFF_VIEW: &str = "staff_desk_config";

/// USD both sides, 32.10 / 32.45 per 1.
const USD_BOTH: Line = Line::manual("USD", "1", "32.10", "32.45");
/// JPY buy only, 23.5 per 100; its sell side is NULL.
const JPY_BUY_ONLY: Line = Line {
    code: "JPY",
    unit_size: "100",
    buy: Some("23.5"),
    sell: None,
    stale_under: None,
};
/// GBP with both sides NULL.
const GBP_NEITHER: Line = Line {
    code: "GBP",
    unit_size: "1",
    buy: None,
    sell: None,
    stale_under: None,
};

static NEXT: AtomicU64 = AtomicU64::new(1);

fn key() -> String {
    format!(
        "t2-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

struct HeaderSessions;

fn callers_of(spec: &str) -> Callers {
    if spec.is_empty() {
        return test_callers::callers(None, None);
    }
    if spec == OWNER {
        return test_callers::callers(Some(test_callers::owner("owner")), None);
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

async fn send(db: &TestDb, req: atest::TestRequest) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// The view's `rates` as `who`.
async fn rates(db: &TestDb, who: &str, view: &str) -> Vec<Value> {
    let path = format!("/api/v3/views/{view}?branch={MAIN}");
    let req = atest::TestRequest::get()
        .uri(&path)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who));
    let answer = send(db, req).await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "GET {path} as {who}: {:.400}",
        answer.text
    );
    answer.body["data"]["rates"]
        .as_array()
        .unwrap_or_else(|| panic!("GET {path}: data.rates is a list: {:.400}", answer.text))
        .clone()
}

/// The owner's `desk_config.rates`, after checking FRONT's `staff_desk_config.rates` is the same.
async fn desk_rates(db: &TestDb) -> Vec<Value> {
    let owner = rates(db, OWNER, OWNER_VIEW).await;
    let front = rates(db, FRONT, STAFF_VIEW).await;
    assert_eq!(front, owner, "both paths answer the same rates (one core)");
    owner
}

async fn quote(db: &TestDb, code: &str, side: &str, quantity: &str) -> Answer {
    let body = json!({
        "branch": MAIN,
        "lines": [{ "shopSide": side, "currency": code, "quantity": quantity }]
    });
    let req = atest::TestRequest::post()
        .uri("/api/quotes")
        .insert_header((header::HOST, HOST))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header(("idempotency-key", key()))
        .insert_header((AS, FRONT))
        .set_payload(body.to_string());
    send(db, req).await
}

fn codes(rates: &[Value]) -> Vec<String> {
    rates
        .iter()
        .map(|r| r["code"].as_str().unwrap_or("?").to_owned())
        .collect()
}

fn rate_of<'a>(rates: &'a [Value], code: &str) -> &'a Value {
    rates
        .iter()
        .find(|r| r["code"] == json!(code))
        .unwrap_or_else(|| panic!("{code} in rates: {rates:?}"))
}

fn dec(value: &Value, what: &str) -> Decimal {
    let text = value
        .as_str()
        .unwrap_or_else(|| panic!("{what}: a decimal string, got {value}"));
    Decimal::from_str(text).unwrap_or_else(|e| panic!("{what}: {text:?}: {e}"))
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

/// One untiered line per (code, unit, buy, sell) and a two-tier EUR, on `MAIN`, made current.
/// Tier 1 (lower bound NULL) is 38.10 / 38.90; tier 2 (above 10000 THB) is 38.40 / 38.70.
async fn publish_with_tiered_eur(db: &TestDb) {
    let published = db.publish(MAIN, &[USD_BOTH]).await;
    for (tier, lo, hi, buy, sell) in [
        ("T2", Some("10000.00"), None, "38.40", "38.70"),
        ("T1", None, Some("10000.00"), "38.10", "38.90"),
    ] {
        sqlx::query(
            "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
                 denomination_group, unit_size, buy_rate, sell_rate, source_snapshot, value_tier, \
                 tier_lower_exclusive_thb, tier_upper_inclusive_thb, base_buy_rate, base_sell_rate) \
             SELECT gen_random_uuid(), organization_id, id, 'EUR', 'ALL', 1, $2::numeric, \
                 $3::numeric, $4, $5, $6::numeric, $7::numeric, 38.00, 39.00 \
             FROM rate_publications WHERE id = $1::uuid",
        )
        .bind(&published.id)
        .bind(buy)
        .bind(sell)
        .bind(json!({ "currency": "EUR", "buy_mode": "MANUAL", "sell_mode": "MANUAL",
                      "reason": "set by hand for the test" }))
        .bind(tier)
        .bind(lo)
        .bind(hi)
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("EUR tier {tier}: {e}"));
    }
}

// ── §1 table, rows 2 and 4: what is listed ─────────────────────────────────────

/// USD (both sides) and JPY (buy only): exactly `[JPY, USD]`, in code order; JPY's sell is null;
/// each side is the published rate per the line's own unit size; register currencies with no line
/// (EUR and the rest) and THB are absent.
#[actix_web::test]
async fn t2_lists_exactly_the_currencies_of_the_current_publication() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD_BOTH, JPY_BUY_ONLY]).await;

    let rates = desk_rates(&db).await;
    assert_eq!(
        codes(&rates),
        vec!["JPY".to_owned(), "USD".to_owned()],
        "only the currencies with a sellable line, ordered by code: {rates:?}"
    );
    for absent in ["EUR", "THB", "GBP", "CNY"] {
        assert!(
            rates.iter().all(|r| r["code"] != json!(absent)),
            "{absent} is not listed"
        );
    }

    let jpy = rate_of(&rates, "JPY");
    assert_eq!(dec(&jpy["buy"], "JPY.buy"), d("23.5"));
    assert!(jpy["sell"].is_null(), "JPY.sell: null, got {}", jpy["sell"]);
    assert_eq!(dec(&jpy["unitSize"], "JPY.unitSize"), d("100"));
    assert!(!jpy["publication"].is_null(), "JPY.publication filled");

    let usd = rate_of(&rates, "USD");
    assert_eq!(dec(&usd["buy"], "USD.buy"), d("32.10"));
    assert_eq!(dec(&usd["sell"], "USD.sell"), d("32.45"));
    assert_eq!(dec(&usd["unitSize"], "USD.unitSize"), d("1"));
    assert!(!usd["publication"].is_null(), "USD.publication filled");

    db.finish().await;
}

// ── §1 table, row 3 ────────────────────────────────────────────────────────────

/// A line with both sides NULL is not listed.
#[actix_web::test]
async fn t2_a_line_with_neither_side_is_not_listed() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD_BOTH, GBP_NEITHER]).await;

    let rates = desk_rates(&db).await;
    assert!(
        rates.iter().all(|r| r["code"] != json!("GBP")),
        "GBP has neither side, so it is not listed: {rates:?}"
    );
    assert_eq!(codes(&rates), vec!["USD".to_owned()], "{rates:?}");

    db.finish().await;
}

// ── §1 table, row 1 ────────────────────────────────────────────────────────────

/// A branch with no current publication lists nothing.
#[actix_web::test]
async fn t2_no_current_publication_lists_nothing() {
    let db = TestDb::start().await;
    db.exec("UPDATE branches SET current_publication_id = NULL WHERE id = 'MAIN'")
        .await;

    let rates = desk_rates(&db).await;
    assert_eq!(rates, Vec::<Value>::new(), "no publication ⇒ rates == []");

    db.finish().await;
}

// ── §1 table, row 2: the tier `line_for` picks ─────────────────────────────────

/// A tiered currency is listed once, with tier 1's rates (the tier whose lower bound is NULL),
/// whatever order the tiers were written in.
#[actix_web::test]
async fn t2_a_tiered_currency_is_listed_once_with_tier_one() {
    let db = TestDb::start().await;
    publish_with_tiered_eur(&db).await;

    let rates = desk_rates(&db).await;
    assert_eq!(
        codes(&rates),
        vec!["EUR".to_owned(), "USD".to_owned()],
        "EUR listed once: {rates:?}"
    );
    let eur = rate_of(&rates, "EUR");
    assert_eq!(dec(&eur["buy"], "EUR.buy"), d("38.10"), "tier 1's buy");
    assert_eq!(dec(&eur["sell"], "EUR.sell"), d("38.90"), "tier 1's sell");
    assert_eq!(dec(&eur["unitSize"], "EUR.unitSize"), d("1"));

    db.finish().await;
}

// ── the list and the quote agree ───────────────────────────────────────────────

/// Every listed side quotes at the listed rate and unit size; a side listed `null` and a currency
/// not listed are refused 409 `rateUnavailable`, the message naming the currency.
#[actix_web::test]
async fn t2_every_listed_side_quotes_and_an_unlisted_currency_is_refused_by_name() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD_BOTH, JPY_BUY_ONLY]).await;

    let rates = desk_rates(&db).await;
    assert_eq!(
        codes(&rates),
        vec!["JPY".to_owned(), "USD".to_owned()],
        "{rates:?}"
    );
    for listed in &rates {
        let code = listed["code"].as_str().expect("a code");
        let quantity = if code == "JPY" { "1000" } else { "10" };
        for (field, side) in [("buy", "BUY"), ("sell", "SELL")] {
            let what = format!("{code}.{field}");
            let answer = quote(&db, code, side, quantity).await;
            if listed[field].is_null() {
                assert_eq!(
                    answer.status,
                    StatusCode::CONFLICT,
                    "{what}: {}",
                    answer.text
                );
                assert_eq!(
                    answer.body["error"]["code"],
                    json!("rateUnavailable"),
                    "{what}"
                );
                continue;
            }
            assert_eq!(
                answer.status,
                StatusCode::OK,
                "{what} is listed, so the quote prices it: {}",
                answer.text
            );
            let line = &answer.body["lines"][0];
            assert_eq!(
                dec(&line["baseRate"], &format!("{what} quote baseRate")),
                dec(&listed[field], &what),
                "{what}: the listed rate is the rate the quote prices with"
            );
            assert_eq!(
                dec(&line["unitSize"], &format!("{what} quote unitSize")),
                dec(&listed["unitSize"], &format!("{code}.unitSize")),
                "{what}: the listed unit size is the quote's"
            );
        }
    }

    let eur = quote(&db, "EUR", "BUY", "10").await;
    assert_eq!(eur.status, StatusCode::CONFLICT, "EUR: {}", eur.text);
    assert_eq!(eur.body["error"]["code"], json!("rateUnavailable"));
    let message = eur.body["error"]["message"].as_str().unwrap_or("");
    assert!(
        message.contains("EUR"),
        "the refusal names the currency: {message:?}"
    );

    db.finish().await;
}

// ── both paths ─────────────────────────────────────────────────────────────────

/// `desk_config` (owner) and `staff_desk_config` (FRONT) answer the same, sellable-only `rates`.
#[actix_web::test]
async fn t2_both_paths_answer_the_same_sellable_rates() {
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD_BOTH, JPY_BUY_ONLY, GBP_NEITHER])
        .await;

    let owner = rates(&db, OWNER, OWNER_VIEW).await;
    let front = rates(&db, FRONT, STAFF_VIEW).await;
    let expected = vec!["JPY".to_owned(), "USD".to_owned()];
    assert_eq!(codes(&owner), expected, "desk_config: {owner:?}");
    assert_eq!(codes(&front), expected, "staff_desk_config: {front:?}");
    assert_eq!(front, owner, "one core, two paths");

    db.finish().await;
}
