//! Written before the implementation by the test writer gemini/studio-14 (controller claude/553782). Do not edit, delete or weaken.
//!
//! Ticket #1142 (Rate feed F7) — the kiosk prices at the tier of the amount and lists the first tier.
//! Design:
//! - `docs/design/rate-feed-connector.md` §9.3, §9.4
//! - `docs/design/blueprint/rates.md` §12.5
//! - Owner's rulings R-4, R-10 on #986
//!
//! What this pins:
//! - §9 Edge case 1: BUY with a foreign amount in T2 is priced at T2 in one repricing round.
//! - §9 Edge case 2: SELL with THB whose foreign amount falls in T2 is priced at T2 (loop settles).
//! - §9 Edge case 3: An untiered branch quotes unchanged (one round, tier_line_in answers None/same).
//! - §9 Edge case 4: A kiosk payment (confirm) under the current publication succeeds (checks line_by_id);
//!   confirm after a new publication is refused with RATE_CHANGED (line_by_id answers None).
//! - §9 Edge case 5: The kiosk lists on a tiered branch show one row per currency at the first tier (T1)
//!   for both POST /api/kiosk "rates" and GET /api/v3/views/kiosk_currencies.
//! - §9 Ruling follow-up 1 (comment 6100499439): A kiosk list on a publication whose (currency, group)
//!   has no open-below line shows no row for it, never a guessed one.
//! - §9 Ruling follow-up 2 (comment 6100499439, §9.3 step 4): The repricing loop stops after 4 rounds
//!   and refuses with AmountOrCurrencyInvalid when tiers oscillate / non-monotone.
//! - §9.4 / `src/presentation/http/kiosk.rs:204` & `v3/views/kiosk_currencies.rs:184`: Kiosk rate
//!   freshness on a tiered branch only evaluates the first tier (T1): fresh T1 with stale T2–T4 is not stale.
//! - §7 / §9.4 / `kiosk_repository.rs:652`: Currency on-switch requires an open-below tier:
//!   enabling a currency with no open-below tier is refused with RatesFirst and writes no switch or audit row.
//! - §4/§8: Stored kiosk request and exchange quote hold the tier line's line_id and rate.

mod common;
mod compat_common;
#[macro_use]
mod kiosk_http_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use chrono::{Duration, Utc};
use kiosk_http_common::rates_re4_common::d;
use kiosk_http_common::{Counter, audit_since};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::PgPool;

const MAIN: &str = rpc_common::MAIN;
const RATE_CHANGED: &str = "Rate changed or counter closed. Please start again.";
const AMOUNT_OR_CURRENCY: &str = "Please enter a valid amount and currency.";

/// One publication on MAIN, made current, with an untiered JPY line and four USD tiers:
/// - T1: (None, 10000.00] THB, buy 31.50, sell 33.00, base_buy 32.00, base_sell 32.50
/// - T2: (10000.00, 50000.00] THB, buy 31.70, sell 32.80, base_buy 32.00, base_sell 32.50
/// - T3: (50000.00, 200000.00] THB, buy 31.90, sell 32.60, base_buy 32.00, base_sell 32.50
/// - T4: (200000.00, None] THB, buy 32.00, sell 32.50, base_buy 32.00, base_sell 32.50
///
/// Untiered JPY: buy 23.10, sell 23.50, unit_size 100.
///
/// Answers (publication_id, USD tier line ids T1..T4).
async fn publish_tiered_usd(pool: &PgPool) -> (String, Vec<String>) {
    let request: String = sqlx::query_scalar(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
         proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
         VALUES (gen_random_uuid(), 'RATE_PUBLICATION', $1, 'f7-seed', 'h', 'p', '{}'::jsonb, 'test', now(), \
         'APPROVED', gen_random_uuid()::text, '00000000-0000-0000-0000-000000000001') RETURNING id::text",
    )
    .bind(MAIN)
    .fetch_one(pool)
    .await
    .expect("an approved request");
    let publication: String = sqlx::query_scalar(
        "INSERT INTO rate_publications (id, branch_id, version, approved_request_id, published_by) \
         VALUES (gen_random_uuid(), $1, \
                 (SELECT coalesce(max(version), 0) + 1 FROM rate_publications WHERE branch_id = $1), \
                 $2::uuid, 'test') RETURNING id::text",
    )
    .bind(MAIN)
    .bind(&request)
    .fetch_one(pool)
    .await
    .expect("a publication");
    let snapshot = json!({ "buy_mode": "MANUAL", "sell_mode": "MANUAL" });
    sqlx::query(
        "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, \
         buy_rate, sell_rate, source_snapshot) VALUES (gen_random_uuid(), $1::uuid, 'JPY', 'ALL', 100, 23.1, 23.5, $2)",
    )
    .bind(&publication)
    .bind(&snapshot)
    .execute(pool)
    .await
    .expect("the JPY line");
    let mut ids = Vec::new();
    for (tier, lo, hi, buy, sell) in [
        ("T1", None, Some("10000.00"), "31.50", "33.00"),
        ("T2", Some("10000.00"), Some("50000.00"), "31.70", "32.80"),
        ("T3", Some("50000.00"), Some("200000.00"), "31.90", "32.60"),
        ("T4", Some("200000.00"), None, "32.00", "32.50"),
    ] {
        let id: String = sqlx::query_scalar(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, \
             buy_rate, sell_rate, source_snapshot, value_tier, tier_lower_exclusive_thb, tier_upper_inclusive_thb, \
             base_buy_rate, base_sell_rate) \
             VALUES (gen_random_uuid(), $1::uuid, 'USD', 'ALL', 1, $2::numeric, $3::numeric, $4, $5, \
                     $6::numeric, $7::numeric, 32.00, 32.50) RETURNING id::text",
        )
        .bind(&publication)
        .bind(buy)
        .bind(sell)
        .bind(&snapshot)
        .bind(tier)
        .bind(lo)
        .bind(hi)
        .fetch_one(pool)
        .await
        .expect("a USD tier line");
        ids.push(id);
    }
    sqlx::query("UPDATE branches SET current_publication_id = $1::uuid WHERE id = $2")
        .bind(&publication)
        .bind(MAIN)
        .execute(pool)
        .await
        .expect("make it current");
    (publication, ids)
}

/// Adds EUR lines to `publication_id` where neither band has `tier_lower_exclusive_thb IS NULL`.
async fn add_eur_without_open_below_tier(pool: &PgPool, publication_id: &str) {
    let snapshot = json!({ "buy_mode": "MANUAL", "sell_mode": "MANUAL" });
    for (tier, lo, hi, buy, sell) in [
        ("T1", "10000.00", Some("50000.00"), "35.00", "36.00"),
        ("T2", "50000.00", None, "35.50", "35.80"),
    ] {
        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, \
             buy_rate, sell_rate, source_snapshot, value_tier, tier_lower_exclusive_thb, tier_upper_inclusive_thb, \
             base_buy_rate, base_sell_rate) \
             VALUES (gen_random_uuid(), $1::uuid, 'EUR', 'ALL', 1, $2::numeric, $3::numeric, $4, $5, \
                     $6::numeric, $7::numeric, 35.00, 36.00)",
        )
        .bind(publication_id)
        .bind(buy)
        .bind(sell)
        .bind(&snapshot)
        .bind(tier)
        .bind(lo)
        .bind(hi)
        .execute(pool)
        .await
        .expect("EUR tier line with no open-below tier");
    }
}

/// Adds oscillating GBP lines to `publication_id`:
/// - T1: (None, 10000.00] THB, sell 30.00, base_sell 30.00, buy 29.00, base_buy 29.00
/// - T2: (10000.00, 50000.00] THB, sell 40.00, base_sell 30.00, buy 29.00, base_buy 29.00
///
/// When customer enters SELL 12,000 THB:
/// - Round 1 (at T1 sell 30.00): 12000 / 30 = 400 GBP; 400 * base 30 = 12000 THB -> falls in T2!
/// - Round 2 (at T2 sell 40.00): 12000 / 40 = 300 GBP; 300 * base 30 = 9000 THB -> falls in T1!
/// - Round 3 (at T1 sell 30.00): 12000 / 30 = 400 GBP -> falls in T2!
/// - Round 4 (at T2 sell 40.00): 12000 / 40 = 300 GBP -> falls in T1!
/// - Still moving after 4 rounds -> refuses AmountOrCurrencyInvalid.
async fn add_oscillating_gbp_tiers(pool: &PgPool, publication_id: &str) {
    let snapshot = json!({ "buy_mode": "MANUAL", "sell_mode": "MANUAL" });
    for (tier, lo, hi, buy, sell) in [
        ("T1", None, Some("10000.00"), "29.00", "30.00"),
        ("T2", Some("10000.00"), Some("50000.00"), "29.00", "40.00"),
    ] {
        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, \
             buy_rate, sell_rate, source_snapshot, value_tier, tier_lower_exclusive_thb, tier_upper_inclusive_thb, \
             base_buy_rate, base_sell_rate) \
             VALUES (gen_random_uuid(), $1::uuid, 'GBP', 'ALL', 1, $2::numeric, $3::numeric, $4, $5, \
                     $6::numeric, $7::numeric, 29.00, 30.00)",
        )
        .bind(publication_id)
        .bind(buy)
        .bind(sell)
        .bind(&snapshot)
        .bind(tier)
        .bind(lo)
        .bind(hi)
        .execute(pool)
        .await
        .expect("GBP oscillating tier line");
    }
}

/// `(line id, rate, unit_size, foreign_amount, thb, input_amount, status)` of one kiosk request.
type Stored = (
    Option<String>,
    Decimal,
    Decimal,
    Decimal,
    Decimal,
    Decimal,
    String,
);

async fn stored(counter: &Counter, id: &str) -> Stored {
    sqlx::query_as(
        "SELECT published_rate_line_id::text, rate, unit_size, foreign_amount, thb, \
                input_amount, status \
         FROM kiosk_requests WHERE branch_id = 'MAIN' AND id = $1",
    )
    .bind(id)
    .fetch_one(&counter.desk.db.pool)
    .await
    .unwrap_or_else(|e| panic!("kiosk request {id}: {e}"))
}

/// The exchange quote line associated with a kiosk request.
async fn quote_line(
    counter: &Counter,
    kiosk_request_id: &str,
) -> (String, Decimal, Decimal, Decimal) {
    sqlx::query_as(
        "SELECT ql.published_rate_line_id::text, ql.final_rate, ql.quantity, ql.thb_amount \
         FROM kiosk_requests kr JOIN quote_lines ql ON ql.quote_id = kr.quote_id \
         WHERE kr.id = $1",
    )
    .bind(kiosk_request_id)
    .fetch_one(&counter.desk.db.pool)
    .await
    .unwrap_or_else(|e| panic!("quote_line for kiosk request {kiosk_request_id}: {e}"))
}

async fn requests(counter: &Counter) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kiosk_requests")
        .fetch_one(&counter.desk.db.pool)
        .await
        .expect("count kiosk_requests")
}

async fn audit_mark(counter: &Counter) -> i64 {
    audit_since(&counter.desk, 0).await.1
}

async fn quote(counter: &Counter, id: &str, side: &str, currency: &str, amount: Value) -> Value {
    counter.hands.ids.push(id);
    let answer = counter
        .ask(
            None,
            "quote",
            json!({ "type": side, "currency": currency, "amount": amount }),
        )
        .await;
    assert_eq!(
        answer.status, 200,
        "quote {side} {currency} {amount}: {}",
        answer.text
    );
    let body = answer.json();
    assert_eq!(body["id"], json!(id), "the route used the id it was given");
    body
}

fn number(value: &Value, what: &str) -> Decimal {
    match value {
        Value::Number(n) => d(&n.to_string()),
        other => panic!("{what} is not a JSON number: {other}"),
    }
}

fn listed(body: &Value, text: &str) -> Vec<(String, Decimal, Decimal)> {
    let mut listed: Vec<(String, Decimal, Decimal)> = body["rates"]
        .as_array()
        .unwrap_or_else(|| panic!("`rates` lists the rates: {text}"))
        .iter()
        .map(|r| {
            (
                r["code"].as_str().expect("a code").to_owned(),
                number(&r["buy"], "buy"),
                number(&r["sell"], "sell"),
            )
        })
        .collect();
    listed.sort();
    listed
}

async fn fetch_kiosk_currencies_view(counter: &Counter) -> Value {
    let host = counter
        .origin
        .split_once("://")
        .map_or(counter.origin.as_str(), |(_, host)| host)
        .to_string();
    let call = rpc_common::Call {
        method: actix_web::http::Method::GET,
        host,
        path: "/api/v3/views/kiosk_currencies?branch=MAIN".to_string(),
        headers: std::collections::BTreeMap::new(),
        body: Vec::new(),
    };
    let answer = counter
        .app
        .send(kiosk_http_common::as_who(call, &counter.cookies, "owner"))
        .await;
    assert_eq!(answer.status, 200, "kiosk_currencies view: {}", answer.text);
    answer.json()["data"].clone()
}

/// §9 Edge case: BUY with a foreign amount in T2 — priced at T2 in one repricing round.
/// §4/§8: The kiosk request's `rate`, `line_id`, `foreign` and `thb` are those of the tier line.
/// The exchange quote then has the same line and rate.
#[actix_web::test]
async fn kiosk_buy_foreign_amount_in_t2_is_priced_at_t2() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    let (_pub_id, usd_tier_ids) = publish_tiered_usd(&pool).await;

    // Customer enters BUY 500 USD.
    // 500 USD × base_buy 32.00 = 16,000.00 THB → falls in T2 ((10000, 50000] THB).
    // T2 buy rate is 31.70 → THB = 500 × 31.70 = 15,850.00 THB.
    let id = "00000000-0000-4000-8000-00000004b101";
    let answer = quote(&counter, id, "BUY", "USD", json!(500)).await;

    assert_eq!(
        number(&answer["rate"], "rate"),
        d("31.7"),
        "kiosk JSON rate is T2 buy rate: {answer}"
    );
    assert_eq!(number(&answer["foreign"], "foreign"), d("500"), "{answer}");
    assert_eq!(number(&answer["thb"], "thb"), d("15850"), "{answer}");

    let (line_id, rate, unit_size, foreign, thb, input, status) = stored(&counter, id).await;
    assert_eq!(
        line_id.as_deref(),
        Some(usd_tier_ids[1].as_str()),
        "stored line_id is T2 line"
    );
    assert_eq!(rate, d("31.70"), "stored rate is T2 buy rate");
    assert_eq!(unit_size, d("1"));
    assert_eq!(foreign, d("500"));
    assert_eq!(thb, d("15850.00"));
    assert_eq!(input, d("500"));
    assert_eq!(status, "QUOTED");

    // §8: The exchange quote has the same line and rate
    let (quote_line_id, quote_final_rate, quote_qty, quote_thb) = quote_line(&counter, id).await;
    assert_eq!(quote_line_id, usd_tier_ids[1], "exchange quote line is T2");
    assert_eq!(
        quote_final_rate,
        d("31.70"),
        "exchange quote final rate is T2"
    );
    assert_eq!(quote_qty, d("500"));
    assert_eq!(quote_thb, d("15850.00"));

    counter.finish().await;
}

/// §9 Edge case: SELL with THB whose foreign amount falls in T2 — priced at T2;
/// the loop settles because a higher tier sells cheaper.
/// §4/§8: Stored kiosk request and exchange quote hold T2's line and rate.
#[actix_web::test]
async fn kiosk_sell_thb_amount_falling_in_t2_is_priced_at_t2() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    let (_pub_id, usd_tier_ids) = publish_tiered_usd(&pool).await;

    // Customer enters SELL 16,400 THB.
    // Step 1: Counter line T1 sell rate 33.00 → 16,400 ÷ 33.00 = 496.96 USD (minor unit 2).
    //         base_value = 496.96 × 32.50 = 16,151.20 THB → falls in T2.
    // Step 2: Reprice at T2 sell rate 32.80 → 16,400 ÷ 32.80 = 500.00 USD.
    //         base_value = 500.00 × 32.50 = 16,250.00 THB → still in T2, loop settles.
    // Final THB = 500.00 × 32.80 = 16,400.00 THB.
    let id = "00000000-0000-4000-8000-00000004b102";
    let answer = quote(&counter, id, "SELL", "USD", json!(16400)).await;

    assert_eq!(
        number(&answer["rate"], "rate"),
        d("32.8"),
        "kiosk JSON rate is T2 sell rate: {answer}"
    );
    assert_eq!(number(&answer["foreign"], "foreign"), d("500"), "{answer}");
    assert_eq!(number(&answer["thb"], "thb"), d("16400"), "{answer}");

    let (line_id, rate, unit_size, foreign, thb, input, status) = stored(&counter, id).await;
    assert_eq!(
        line_id.as_deref(),
        Some(usd_tier_ids[1].as_str()),
        "stored line_id is T2 line"
    );
    assert_eq!(rate, d("32.80"), "stored rate is T2 sell rate");
    assert_eq!(unit_size, d("1"));
    assert_eq!(foreign, d("500.00"));
    assert_eq!(thb, d("16400.00"));
    assert_eq!(input, d("16400"));
    assert_eq!(status, "QUOTED");

    // §8: The exchange quote has the same line and rate
    let (quote_line_id, quote_final_rate, quote_qty, quote_thb) = quote_line(&counter, id).await;
    assert_eq!(quote_line_id, usd_tier_ids[1], "exchange quote line is T2");
    assert_eq!(
        quote_final_rate,
        d("32.80"),
        "exchange quote final rate is T2"
    );
    assert_eq!(quote_qty, d("500.00"));
    assert_eq!(quote_thb, d("16400.00"));

    counter.finish().await;
}

/// §9 Edge case: An untiered branch/currency quotes unchanged (one round, tier_line_in answers None/same).
#[actix_web::test]
async fn kiosk_untiered_branch_quotes_normally() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    let (_pub_id, _usd_tier_ids) = publish_tiered_usd(&pool).await;

    let jpy_line = kiosk_http_common::rates_re4_common::line(&pool, MAIN, "JPY").await;

    // BUY 1,000 JPY at 23.10 per 100 JPY → 231.00 THB.
    let id = "00000000-0000-4000-8000-00000004b103";
    let answer = quote(&counter, id, "BUY", "JPY", json!(1000)).await;

    assert_eq!(number(&answer["rate"], "rate"), d("0.231"), "{answer}");
    assert_eq!(number(&answer["thb"], "thb"), d("231"), "{answer}");
    assert_eq!(number(&answer["foreign"], "foreign"), d("1000"), "{answer}");

    let (line_id, rate, unit_size, foreign, thb, input, status) = stored(&counter, id).await;
    assert_eq!(
        line_id.as_deref(),
        Some(jpy_line.id.as_str()),
        "stored line_id is untiered JPY line"
    );
    assert_eq!(rate, d("23.1"));
    assert_eq!(unit_size, d("100"));
    assert_eq!(foreign, d("1000"));
    assert_eq!(thb, d("231.00"));
    assert_eq!(input, d("1000"));
    assert_eq!(status, "QUOTED");

    counter.finish().await;
}

/// §9 Edge case: A kiosk payment (confirm) on a tiered publication succeeds when the line is current
/// (checked via line_by_id); and is refused once a new publication makes the quoted line no longer current.
#[actix_web::test]
async fn kiosk_tiered_quote_confirm_succeeds_under_same_publication_and_refuses_after_new_publication()
 {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    let (_pub_id_1, _usd_tier_ids_1) = publish_tiered_usd(&pool).await;

    // 1. Quote on current tiered publication
    let id1 = "00000000-0000-4000-8000-00000004b104";
    quote(&counter, id1, "BUY", "USD", json!(500)).await;

    // Under F7, check_stale_in uses line_by_id: the quoted line is current -> confirm succeeds
    counter.desk.clock.advance(Duration::seconds(4));
    let confirmed = counter.ask(None, "confirm", json!({ "id": id1 })).await;
    assert_eq!(
        confirmed.status, 200,
        "confirm succeeds under current publication: {}",
        confirmed.text
    );
    let (_, _, _, _, _, _, status) = stored(&counter, id1).await;
    assert_eq!(status, "WAITING");

    // 2. Publish a new publication (version 2)
    let (_pub_id_2, _usd_tier_ids_2) = publish_tiered_usd(&pool).await;

    // Quote under version 2
    let id2 = "00000000-0000-4000-8000-00000004b105";
    quote(&counter, id2, "BUY", "USD", json!(500)).await;

    // 3. Publish version 3 (making id2's line no longer current)
    let (_pub_id_3, _usd_tier_ids_3) = publish_tiered_usd(&pool).await;

    // Confirm id2: refused because line_by_id answers None for an obsolete line
    counter.desk.clock.advance(Duration::seconds(4));
    let mark = audit_mark(&counter).await;
    let refused = counter.ask(None, "confirm", json!({ "id": id2 })).await;
    refused.assert_refused(400, RATE_CHANGED, "confirm refused after new publication");

    // Stays QUOTED and wrote no audit line
    let (_, _, _, _, _, _, status2) = stored(&counter, id2).await;
    assert_eq!(status2, "QUOTED");
    let (written, _) = audit_since(&counter.desk, mark).await;
    assert!(
        written.is_empty(),
        "refused confirm wrote no audit line: {written:?}"
    );

    counter.finish().await;
}

/// §9 Edge case: The list on a tiered branch shows one row per currency at the first tier (T1)
/// (tier_lower_exclusive_thb IS NULL, R-10).
/// Covers both POST /api/kiosk action "rates" and GET /api/v3/views/kiosk_currencies.
#[actix_web::test]
async fn kiosk_lists_on_tiered_branch_show_first_tier_only() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    let (_pub_id, _usd_tier_ids) = publish_tiered_usd(&pool).await;

    // 1. POST /api/kiosk "rates"
    let rates_ans = counter.ask(None, "rates", json!({})).await;
    assert_eq!(rates_ans.status, 200, "{}", rates_ans.text);
    let rates_json = rates_ans.json();
    let rates_list = listed(&rates_json, &rates_ans.text);

    let usd_rates: Vec<&(String, Decimal, Decimal)> =
        rates_list.iter().filter(|(c, _, _)| c == "USD").collect();
    assert_eq!(
        usd_rates.len(),
        1,
        "kiosk rates lists USD exactly once, not once per tier: {rates_json}"
    );
    assert_eq!(
        usd_rates[0].1,
        d("31.5"),
        "kiosk rates lists first tier T1 buy rate"
    );
    assert_eq!(
        usd_rates[0].2,
        d("33"),
        "kiosk rates lists first tier T1 sell rate"
    );

    let jpy_rates: Vec<&(String, Decimal, Decimal)> =
        rates_list.iter().filter(|(c, _, _)| c == "JPY").collect();
    assert_eq!(jpy_rates.len(), 1, "JPY listed once");
    assert_eq!(jpy_rates[0].1, d("0.231"), "JPY per 1 buy");
    assert_eq!(jpy_rates[0].2, d("0.235"), "JPY per 1 sell");

    // 2. GET /api/v3/views/kiosk_currencies
    let view_data = fetch_kiosk_currencies_view(&counter).await;
    let currencies = view_data["currencies"]
        .as_array()
        .expect("currencies array");
    let usd_currencies: Vec<&Value> = currencies
        .iter()
        .filter(|c| c["code"] == "USD" && !c["buy"].is_null())
        .collect();
    assert_eq!(
        usd_currencies.len(),
        1,
        "kiosk_currencies view lists USD exactly once: {view_data}"
    );
    assert_eq!(
        usd_currencies[0]["buy"],
        json!("31.50000000"),
        "kiosk_currencies view shows first tier T1 buy rate"
    );
    assert_eq!(
        usd_currencies[0]["sell"],
        json!("33.00000000"),
        "kiosk_currencies view shows first tier T1 sell rate"
    );

    counter.finish().await;
}

/// §9 Ruling follow-up (comment 6100499439): A kiosk list on a publication whose (currency, group)
/// has no open-below line shows no row for it, never a guessed one.
#[actix_web::test]
async fn kiosk_list_shows_no_row_when_currency_has_no_open_below_tier() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    let (pub_id, _usd_tier_ids) = publish_tiered_usd(&pool).await;

    // Add EUR with two bands, neither open below (both lower bounds NOT NULL)
    add_eur_without_open_below_tier(&pool, &pub_id).await;

    // 1. POST /api/kiosk "rates": EUR must NOT appear in the list
    let rates_ans = counter.ask(None, "rates", json!({})).await;
    assert_eq!(rates_ans.status, 200, "{}", rates_ans.text);
    let rates_list = listed(&rates_ans.json(), &rates_ans.text);
    let eur_rates: Vec<&(String, Decimal, Decimal)> =
        rates_list.iter().filter(|(c, _, _)| c == "EUR").collect();
    assert!(
        eur_rates.is_empty(),
        "EUR has no open-below tier (tier_lower_exclusive_thb IS NULL), so it must not be listed: {rates_list:?}"
    );

    // 2. GET /api/v3/views/kiosk_currencies: EUR must have no rates / not be offered
    let view_data = fetch_kiosk_currencies_view(&counter).await;
    let currencies = view_data["currencies"]
        .as_array()
        .expect("currencies array");
    let eur_currencies: Vec<&Value> = currencies
        .iter()
        .filter(|c| c["code"] == "EUR" && !c["buy"].is_null())
        .collect();
    assert!(
        eur_currencies.is_empty(),
        "kiosk_currencies view must not offer EUR without an open-below tier: {view_data}"
    );

    counter.finish().await;
}

/// §9 Ruling follow-up (comment 6100499439, §9.3 step 4): The repricing loop stops after 4 rounds
/// and refuses with AmountOrCurrencyInvalid when tiers oscillate.
#[actix_web::test]
async fn kiosk_repricing_loop_terminates_and_refuses_when_tiers_oscillate() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    let (pub_id, _usd_tier_ids) = publish_tiered_usd(&pool).await;

    // Add non-monotonic GBP tiers that oscillate between T1 and T2 for a SELL of 12,000 THB
    add_oscillating_gbp_tiers(&pool, &pub_id).await;

    let before_requests = requests(&counter).await;
    let mark = audit_mark(&counter).await;

    // SELL 12,000 THB:
    // Round 1: T1 sell 30.00 -> 400 GBP -> base 12,000 THB in T2
    // Round 2: T2 sell 40.00 -> 300 GBP -> base 9,000 THB in T1
    // Round 3: T1 sell 30.00 -> 400 GBP in T2
    // Round 4: T2 sell 40.00 -> 300 GBP in T1
    // Still moving after 4 rounds -> AmountOrCurrencyInvalid
    let id = "00000000-0000-4000-8000-00000004b107";
    counter.hands.ids.push(id);
    let refused = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        counter.ask(
            None,
            "quote",
            json!({ "type": "SELL", "currency": "GBP", "amount": 12000 }),
        ),
    )
    .await
    .expect("repricing loop must terminate within 5 seconds rather than hanging");

    refused.assert_refused(
        400,
        AMOUNT_OR_CURRENCY,
        "oscillating tiers refuse AmountOrCurrencyInvalid after 4 rounds",
    );

    assert_eq!(
        requests(&counter).await,
        before_requests,
        "the refused quote wrote no request"
    );
    let (written, _) = audit_since(&counter.desk, mark).await;
    assert!(
        written.is_empty(),
        "the refusal wrote no audit line: {written:?}"
    );

    counter.finish().await;
}

/// §9.4 / §4.10 / `src/presentation/http/kiosk.rs:204` & `src/presentation/http/v3/views/kiosk_currencies.rs:184`:
/// Kiosk rate freshness on a tiered branch only evaluates the first tier (T1):
/// when T1's source_snapshot is fresh and T2–T4 carry stale snapshots (fetched_at 3 days ago,
/// max_reference_age_hours 24), freshness.stale is false for both POST /api/kiosk "rates"
/// and GET /api/v3/views/kiosk_currencies.
#[actix_web::test]
async fn kiosk_rate_freshness_on_tiered_branch_is_not_stale_when_t1_is_fresh() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    let (_pub_id, usd_tier_ids) = publish_tiered_usd(&pool).await;

    let now = Utc::now();
    let fresh_snapshot = json!({
        "buy_mode": "FEED",
        "sell_mode": "FEED",
        "fetched_at": now.to_rfc3339(),
        "max_reference_age_hours": 24,
        "source_date": now.format("%Y-%m-%d").to_string(),
    });
    let three_days_ago = now - Duration::days(3);
    let stale_snapshot = json!({
        "buy_mode": "FEED",
        "sell_mode": "FEED",
        "fetched_at": three_days_ago.to_rfc3339(),
        "max_reference_age_hours": 24,
        "source_date": three_days_ago.format("%Y-%m-%d").to_string(),
    });

    sqlx::query("UPDATE published_rate_lines SET source_snapshot = $1 WHERE id = $2::uuid")
        .bind(&fresh_snapshot)
        .bind(&usd_tier_ids[0])
        .execute(&pool)
        .await
        .expect("update T1 snapshot to fresh");

    for stale_id in &usd_tier_ids[1..] {
        sqlx::query("UPDATE published_rate_lines SET source_snapshot = $1 WHERE id = $2::uuid")
            .bind(&stale_snapshot)
            .bind(stale_id)
            .execute(&pool)
            .await
            .expect("update T2-T4 snapshots to stale");
    }

    // 1. POST /api/kiosk "rates": freshness.stale must be false
    let rates_ans = counter.ask(None, "rates", json!({})).await;
    assert_eq!(rates_ans.status, 200, "{}", rates_ans.text);
    let rates_json = rates_ans.json();
    assert_eq!(
        rates_json["freshness"]["stale"],
        json!(false),
        "kiosk rates freshness must not be marked stale when T1 is fresh: {rates_json}"
    );

    // 2. GET /api/v3/views/kiosk_currencies: freshness.stale must be false
    let view_data = fetch_kiosk_currencies_view(&counter).await;
    assert_eq!(
        view_data["freshness"]["stale"],
        json!(false),
        "kiosk_currencies view freshness must not be marked stale when T1 is fresh: {view_data}"
    );

    counter.finish().await;
}

/// §7 / §9.4 / `crates/xc-kiosk/src/infrastructure/kiosk/kiosk_repository.rs:652-669`:
/// On a currency with no open-below tier (such as EUR with only T1 (10000, 50000] and T2 (50000, None]),
/// switching EUR on is refused with RatesFirst ("Set valid counter buy and sell rates first.")
/// and writes no switch or audit row.
/// Meanwhile, a tiered currency with an open-below band (USD) still switches on successfully.
#[actix_web::test]
async fn kiosk_currency_on_switch_refuses_rates_first_when_no_open_below_tier() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    let (pub_id, _usd_tier_ids) = publish_tiered_usd(&pool).await;

    // Add EUR lines without an open-below tier (tier_lower_exclusive_thb IS NULL)
    add_eur_without_open_below_tier(&pool, &pub_id).await;

    // 1. Refusal case: EUR has rates, but neither is open-below (no counter line)
    let mark_eur = audit_mark(&counter).await;
    let eur_before: Option<(bool,)> = sqlx::query_as(
        "SELECT enabled FROM kiosk_currencies WHERE branch_id = 'MAIN' AND code = 'EUR'",
    )
    .fetch_optional(&pool)
    .await
    .expect("query kiosk_currencies for EUR before");

    let refused_eur = counter
        .ask(
            Some("owner"),
            "configure",
            json!({
                "code": "EUR",
                "enabled": true,
                "buy": "35.5",
                "sell": "35.8",
            }),
        )
        .await;

    refused_eur.assert_refused(
        400,
        "Set valid counter buy and sell rates first.",
        "enabling currency with no open-below tier must be refused with RatesFirst",
    );

    let (eur_audit, _) = audit_since(&counter.desk, mark_eur).await;
    assert!(
        eur_audit.is_empty(),
        "refused EUR configure wrote no audit row: {eur_audit:?}"
    );

    let eur_after: Option<(bool,)> = sqlx::query_as(
        "SELECT enabled FROM kiosk_currencies WHERE branch_id = 'MAIN' AND code = 'EUR'",
    )
    .fetch_optional(&pool)
    .await
    .expect("query kiosk_currencies for EUR after");
    assert_eq!(
        eur_after, eur_before,
        "refused EUR configure did not write any switch in kiosk_currencies"
    );

    // 2. Positive case: USD has an open-below tier (T1), so enabling USD succeeds
    let mark_usd = audit_mark(&counter).await;
    let answer_usd = counter
        .ask(
            Some("owner"),
            "configure",
            json!({
                "code": "USD",
                "enabled": true,
                "buy": "31.5",
                "sell": "33.0",
            }),
        )
        .await;
    assert_eq!(
        answer_usd.status, 200,
        "USD configure with open-below tier succeeds: {}",
        answer_usd.text
    );
    assert_eq!(answer_usd.json()["saved"], json!(true));

    let (usd_audit, _) = audit_since(&counter.desk, mark_usd).await;
    assert_eq!(
        usd_audit.len(),
        1,
        "successful USD configure wrote exactly 1 audit row: {usd_audit:?}"
    );

    let (usd_enabled,): (bool,) = sqlx::query_as(
        "SELECT enabled FROM kiosk_currencies WHERE branch_id = 'MAIN' AND code = 'USD'",
    )
    .fetch_one(&pool)
    .await
    .expect("query kiosk_currencies for USD");
    assert!(
        usd_enabled,
        "USD switch must be stored as enabled in kiosk_currencies"
    );

    counter.finish().await;
}
