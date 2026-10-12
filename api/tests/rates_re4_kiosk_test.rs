//! Ticket #756 (RE-4b) — the kiosk quotes from the branch's current publication, per
//! `unit_size`, through `ExchangeArithmetic`; it stores the line it quoted; it refuses a confirm
//! once that line is no longer current; and its JSON keeps v2's rate per 1.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md`
//! - §3.2: the kiosk BUY is `ExchangeArithmetic::thb`; the kiosk SELL by baht is
//!   `foreign_for_thb` (truncated to the currency's minor unit, JPY 0), then `thb`;
//! - §3.3: THB is rounded HALF_UP (away from zero) to 2 places once, at the end;
//! - §4.8: the kiosk quote reads `PublishedRates::line_in` for its side; a quote and its stored
//!   row keep the line id and the rate; a later publication never rewrites them; stale at read
//!   time, `StalePolicy::admits(side, thb)`;
//! - §4.9 R1 (the row stores the line's `rate` per `unit_size`, its `unit_size` and its
//!   `published_rate_line_id`), R2 (every THB figure is `ExchangeArithmetic::thb`), R3 (the kiosk
//!   JSON `rate` stays per 1, through `CompatRate::per_one`), R4 (confirm refuses when the current
//!   line is not the stored one, with the existing "the rate changed" refusal, and writes
//!   nothing), R5 (stale: the policy version the line was published under; a refusal is the
//!   reader's existing "no rate for this currency" answer);
//! - §8 migration 5 (`kiosk_requests.published_rate_line_id`, merged `d6ce058`), D-3 (the current
//!   rate is the line of `branches.current_publication_id`; a currency without a line is
//!   unavailable);
//! - ticket #756 §8–9, the edge cases, one or more tests each.
//!
//! Every request goes through the server's route table (`POST /api/kiosk`) with the harness of
//! `kiosk_http_common` (`Counter`: `MAIN` with an open shift, its dollar saved directly at
//! 32.1 / 32.6 and published at the same rates). Publications are made through the rate engine
//! (`rates_re4_common`, and `publish_auto` below for a line with a reference). JPY's `unit_size`
//! is 100 and its minor unit 0 (RU-1's register).
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.
//!
//! Signatures this test assumes:
//! - [required] column `kiosk_requests.published_rate_line_id uuid NULL REFERENCES
//!   published_rate_lines(id)` (migration 5); `kiosk_requests.unit_size` holds the line's
//!   `unit_size`; `kiosk_requests.rate` the line's rate per that unit;
//! - [required] the kiosk's JSON answers keep their v2 keys; `rate` (in `quote`, `list`, `get`)
//!   and `buy`/`sell` (in `rates`) are per 1; a quote's `thb` is the stored THB;
//! - [required] `CompatRate::per_one` (`xc_kernel::money::CompatRate`, pinned by
//!   `crates/xc-kernel/tests/compat_rate_test.rs`);
//! - the refusal sentences are today's `KioskError` texts (`kiosk_error.rs`);
//! - the rate engine as `rates_re4_common` names it (RE-1, merged), with
//!   `LineSide::Auto` for a line priced from the newest reference.

mod common;
mod compat_common;
mod kiosk_http_common;
mod rpc_common;
mod snapshot_common;

use chrono::{Duration, Utc};
use kiosk_http_common::rates_re4_common::{d, line, publish};
use kiosk_http_common::{Counter, audit_since};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::PgPool;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate};
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xchange::presentation::http::v3::commands::make_approval_service;

/// Amended for QU-6 #1354 (ruling issuecomment-6117792804), setup only: `accept` starts the
/// request's queue ticket as the accepting teller (queue-tickets.md §4.5), and the queue serves
/// only a teller with an ACTIVE counter, so the accepting staff of `MAIN` get one (and the `users`
/// row it references), seeded as `kiosk_accept_queue_ticket_test.rs` seeds them.
async fn open_counters(pool: &sqlx::PgPool, tellers: &[&str]) {
    for teller in tellers {
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             SELECT $2::uuid, organization_id, 'Teller', 'ACTIVE', 'test' FROM branches WHERE id = $1 \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(rpc_common::MAIN)
        .bind(teller)
        .execute(pool)
        .await
        .expect("the teller's user row");
        sqlx::query(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             SELECT organization_id, id, $2::uuid, 'ACTIVE' FROM branches WHERE id = $1 \
             ON CONFLICT (branch_id, teller_id) DO NOTHING",
        )
        .bind(rpc_common::MAIN)
        .bind(teller)
        .execute(pool)
        .await
        .expect("open the teller's counter");
    }
}

const RATE_CHANGED: &str = "Rate changed or counter closed. Please start again.";
const AMOUNT_OR_CURRENCY: &str = "Please enter a valid amount and currency.";

/// `(line id, rate, unit_size, foreign_amount, thb, input_amount, status)` of one request.
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

async fn requests(counter: &Counter) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kiosk_requests")
        .fetch_one(&counter.desk.db.pool)
        .await
        .expect("count kiosk_requests")
}

/// The last audit line's sequence number now.
async fn audit_mark(counter: &Counter) -> i64 {
    audit_since(&counter.desk, 0).await.1
}

/// A quote with the id `id`, answered 200; the answer.
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

/// A quote that must be refused with `sentence`, and that writes no request and no audit line.
async fn refused_quote(
    counter: &Counter,
    side: &str,
    currency: &str,
    amount: Value,
    sentence: &str,
    label: &str,
) {
    let before = requests(counter).await;
    let mark = audit_mark(counter).await;
    counter
        .ask(
            None,
            "quote",
            json!({ "type": side, "currency": currency, "amount": amount }),
        )
        .await
        .assert_refused(400, sentence, label);
    assert_eq!(
        requests(counter).await,
        before,
        "{label}: the refusal wrote no request"
    );
    let (written, _) = audit_since(&counter.desk, mark).await;
    assert!(
        written.is_empty(),
        "{label}: the refusal wrote no audit line: {written:?}"
    );
}

/// The JSON number of `value` as decimal text (`235`, `999.93`, `0.231`).
fn number(value: &Value, what: &str) -> Decimal {
    match value {
        Value::Number(n) => d(&n.to_string()),
        other => panic!("{what} is not a JSON number: {other}"),
    }
}

fn unique(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    format!(
        "{prefix}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// The next publication of `MAIN`, as the owner publishes it: USD **AUTO** against a BOT
/// reference fetched `age_hours` ago (buy 32.1, sell 32.45, no adjustment), and JPY MANUAL
/// 23.1 / 23.5 per 100 (no reference, never stale), under a new policy version whose references
/// go stale after 24 h, with `on_stale` and `stale_limit_thb` as given (§4.3, §4.8).
/// The reference and the policy are seeded rows, as `crates/xc-rate/tests/published_rates_test.rs`
/// seeds them; the draft, its approval and the publication go through the rate engine.
async fn publish_auto(
    pool: &PgPool,
    age_hours: i64,
    on_stale: Option<&str>,
    stale_limit_thb: Option<&str>,
) {
    let fetched_at = Utc::now() - Duration::hours(age_hours);
    let batch: String = sqlx::query_scalar(
        "INSERT INTO rate_reference_batches \
           (id, provider, source_date, fetched_at, raw_checksum, status) \
         VALUES (gen_random_uuid(), 'BOT', ($1::timestamptz AT TIME ZONE 'UTC')::date, $1, $2, \
                 'COMPLETE') \
         RETURNING id::text",
    )
    .bind(fetched_at)
    .bind(unique("re4b-checksum"))
    .fetch_one(pool)
    .await
    .expect("insert into rate_reference_batches");
    sqlx::query(
        "INSERT INTO rate_reference_lines \
           (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
         VALUES (gen_random_uuid(), $1::uuid, 'USD', 'BOT', 1, 32.1, 32.45, '{}'::jsonb)",
    )
    .bind(&batch)
    .execute(pool)
    .await
    .expect("insert into rate_reference_lines");
    let guard = json!({ "min_pct": "-5.0000", "max_pct": "5.0000" });
    sqlx::query(
        "INSERT INTO rate_policy_versions \
           (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
            max_reference_age_hours, on_stale, stale_limit_thb, valid_from, valid_to, \
            approved_by, reason, source_version) \
         VALUES (gen_random_uuid(), 'MAIN', \
                 COALESCE((SELECT max(version) FROM rate_policy_versions WHERE branch_id = 'MAIN'), 0) + 1, \
                 0, 0, $1, 24, $2, $3::numeric, now() - interval '1 day', NULL, 'owner', \
                 'seeded by the RE-4b test', NULL)",
    )
    .bind(json!({ "USD": { "buy": guard, "sell": guard } }))
    .bind(on_stale)
    .bind(stale_limit_thb)
    .execute(pool)
    .await
    .expect("insert into rate_policy_versions");

    let owner = Actor::owner("owner");
    // OR-7.1 #1020 (design `org-roles-branches.md` §3.3, §4.3), by #1020's ticket author: `submit` takes the
    // owner's B1-mapped user, EXECUTIVE + ADMIN organisation-wide.
    let owner_user = xc_auth::domain::access::Actor::new(
        "owner",
        "00000000-0000-0000-0000-000000000001",
        vec![
            xc_auth::domain::access::ScopedRole::new(
                xc_auth::domain::access::Role::Executive,
                None,
                chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
                None,
            ),
            xc_auth::domain::access::ScopedRole::new(
                xc_auth::domain::access::Role::Admin,
                None,
                chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
                None,
            ),
        ],
        None,
    );
    let main = BranchId::new("MAIN");
    let drafts = RateDraftService::new(pool.clone(), make_approval_service(pool.clone()));
    let publisher = PublishService::new(pool.clone());
    let manual = |text: &str| LineSide::Manual(Rate::parse(text).expect("a rate"));
    let lines = vec![
        DraftLine::new(
            CurrencyCode::parse("USD").expect("USD"),
            LineSide::Auto,
            LineSide::Auto,
            None,
        ),
        DraftLine::new(
            CurrencyCode::parse("JPY").expect("JPY"),
            manual("23.1"),
            manual("23.5"),
            Some("published by the RE-4b test".to_owned()),
        ),
    ];
    let mut tx = pool.begin().await.expect("begin");
    let draft = drafts
        .create(&mut tx, &main, &owner, lines)
        .await
        .unwrap_or_else(|e| panic!("create the draft: {}", e.code()));
    tx.commit().await.expect("commit the draft");
    let mut tx = pool.begin().await.expect("begin");
    drafts
        .submit(&mut tx, &main, &owner_user, &draft.id, &unique("re4b-key"))
        .await
        .unwrap_or_else(|e| panic!("submit the draft: {}", e.code()));
    tx.commit().await.expect("commit the submit");
    let expected: Option<i32> = sqlx::query_scalar(
        "SELECT p.version FROM branches b JOIN rate_publications p \
           ON p.id = b.current_publication_id WHERE b.id = 'MAIN'",
    )
    .fetch_optional(pool)
    .await
    .expect("read the current version");
    let mut tx = pool.begin().await.expect("begin");
    publisher
        .publish(&mut tx, &main, &owner, &draft.id, expected)
        .await
        .unwrap_or_else(|e| panic!("publish the draft: {}", e.code()));
    tx.commit().await.expect("commit the publish");

    let usd = line(pool, "MAIN", "USD").await;
    assert_eq!(
        (usd.buy, usd.sell),
        (Some(d("32.1")), Some(d("32.45"))),
        "the setup: USD is the reference, unadjusted"
    );
}

/// §9 RE-4b "a quote stores the line id, rate and unit", R1, with §3.2's kiosk BUY: 1,000 JPY at
/// 23.1 per 100 JPY is 231.00 THB (`amount × rate ÷ unit_size`), never 23,100.00
/// (`amount × rate`). The answer's `rate` is per 1 (R3): 0.231.
#[actix_web::test]
async fn a_kiosk_quote_stores_the_published_line_its_rate_and_unit_size() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    publish(
        &pool,
        "MAIN",
        &[("USD", "33", "34"), ("JPY", "23.1", "23.5")],
    )
    .await;
    let jpy = line(&pool, "MAIN", "JPY").await;
    assert_eq!(jpy.unit_size, d("100"), "RU-1 registers JPY per 100");

    let id = "00000000-0000-4000-8000-00000004b001";
    let answer = quote(&counter, id, "BUY", "JPY", json!(1000)).await;
    assert_eq!(number(&answer["thb"], "thb"), d("231"), "{answer}");
    assert_eq!(number(&answer["foreign"], "foreign"), d("1000"), "{answer}");
    assert_eq!(
        number(&answer["rate"], "rate"),
        d("0.231"),
        "R3: the kiosk JSON rate is per 1: {answer}"
    );

    let (line_id, rate, unit_size, foreign, thb, input, status) = stored(&counter, id).await;
    assert_eq!(
        line_id.as_deref(),
        Some(jpy.id.as_str()),
        "the request stores the line it quoted"
    );
    assert_eq!(rate, d("23.1"), "R1: the line's buy rate, per 100 JPY");
    assert_eq!(unit_size, d("100"), "R1: the line's unit_size");
    assert_eq!(foreign, d("1000"));
    assert_eq!(thb, d("231.00"), "1000 × 23.1 ÷ 100");
    assert_eq!(input, d("1000"));
    assert_eq!(status, "QUOTED");
    counter.finish().await;
}

/// D-3 and §4.8: the kiosk reads the current publication, not the newest `rates` row. The
/// counter's direct save says USD 32.1 / 32.6; the publication says 33 / 34. The quote, its
/// stored row and the kiosk's `rates` list all say 33 / 34.
#[actix_web::test]
async fn a_kiosk_quote_reads_the_publication_not_the_direct_save() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    publish(&pool, "MAIN", &[("USD", "33", "34")]).await;
    let usd = line(&pool, "MAIN", "USD").await;

    let id = "00000000-0000-4000-8000-00000004b002";
    let answer = quote(&counter, id, "BUY", "USD", json!(100)).await;
    assert_eq!(number(&answer["rate"], "rate"), d("33"), "{answer}");
    assert_eq!(number(&answer["thb"], "thb"), d("3300"), "{answer}");
    let (line_id, rate, unit_size, _, thb, _, _) = stored(&counter, id).await;
    assert_eq!(line_id.as_deref(), Some(usd.id.as_str()));
    assert_eq!(rate, d("33"));
    assert_eq!(unit_size, d("1"));
    assert_eq!(thb, d("3300.00"));

    let rates = counter.ask(None, "rates", json!({})).await;
    assert_eq!(rates.status, 200, "{}", rates.text);
    assert_eq!(
        listed(&rates.json(), &rates.text),
        vec![("USD".to_owned(), d("33"), d("34"))],
        "the kiosk lists the publication's rates: {}",
        rates.text
    );
    counter.finish().await;
}

/// `(code, buy, sell)` of every entry of a `rates` answer, sorted by code.
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

/// R3: every rate the kiosk JSON prints is per 1, through `CompatRate::per_one`: the `rates`
/// list, a quote, and the desk's `list` and `get` of the request. JPY 23.1 / 23.5 per 100 is
/// 0.231 / 0.235; the stored row keeps 23.5 per 100 (R1).
#[actix_web::test]
async fn the_kiosk_json_rate_is_per_one() {
    let fr1 = kiosk_http_common::person("FR1", "FRONT", 0x11);
    let counter = Counter::open(&[fr1]).await;
    let pool = counter.desk.db.pool.clone();
    open_counters(&pool, &["00000000-0000-4000-8000-000000000011"]).await;
    publish(
        &pool,
        "MAIN",
        &[("USD", "33", "34"), ("JPY", "23.1", "23.5")],
    )
    .await;

    let rates = counter.ask(None, "rates", json!({})).await;
    assert_eq!(rates.status, 200, "{}", rates.text);
    assert_eq!(
        listed(&rates.json(), &rates.text),
        vec![
            ("JPY".to_owned(), d("0.231"), d("0.235")),
            ("USD".to_owned(), d("33"), d("34")),
        ],
        "{}",
        rates.text
    );

    // 10,000 JPY for 2,350.00 THB: a SELL by baht, at the sell rate
    let id = "00000000-0000-4000-8000-00000004b003";
    let answer = quote(&counter, id, "SELL", "JPY", json!(2350)).await;
    assert_eq!(number(&answer["rate"], "rate"), d("0.235"), "{answer}");
    assert_eq!(
        number(&answer["foreign"], "foreign"),
        d("10000"),
        "{answer}"
    );
    assert_eq!(number(&answer["thb"], "thb"), d("2350"), "{answer}");
    let (_, rate, unit_size, _, _, _, _) = stored(&counter, id).await;
    assert_eq!((rate, unit_size), (d("23.5"), d("100")), "R1: per 100");

    counter.desk.clock.advance(Duration::seconds(4));
    let confirmed = counter.ask(None, "confirm", json!({ "id": id })).await;
    assert_eq!(confirmed.status, 200, "{}", confirmed.text);

    let list = counter.ask(Some("owner"), "list", json!({})).await;
    assert_eq!(list.status, 200, "{}", list.text);
    let item = list.json()["requests"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["id"] == json!(id)).cloned())
        .unwrap_or_else(|| panic!("the request is listed: {}", list.text));
    assert_eq!(number(&item["rate"], "list rate"), d("0.235"), "{item}");
    assert_eq!(number(&item["thb"], "list thb"), d("2350"), "{item}");

    let accepted = counter
        .ask(Some("FR1"), "accept", json!({ "id": id }))
        .await;
    assert_eq!(accepted.status, 200, "{}", accepted.text);
    let got = counter.ask(Some("FR1"), "get", json!({ "id": id })).await;
    assert_eq!(got.status, 200, "{}", got.text);
    assert_eq!(
        number(&got.json()["rate"], "get rate"),
        d("0.235"),
        "{}",
        got.text
    );
    counter.finish().await;
}

/// R2, §3.3: THB is rounded HALF_UP once. 15 JPY at 23.1 per 100 is 3.465 THB, which is 3.47
/// half up (3.46 by banker's rounding, 3.46 truncated).
#[actix_web::test]
async fn a_kiosk_buy_rounds_thb_half_up_once() {
    let counter = Counter::open(&[]).await;
    publish(&counter.desk.db.pool, "MAIN", &[("JPY", "23.1", "23.5")]).await;

    let id = "00000000-0000-4000-8000-00000004b004";
    let answer = quote(&counter, id, "BUY", "JPY", json!(15)).await;
    assert_eq!(number(&answer["thb"], "thb"), d("3.47"), "{answer}");
    let (_, _, _, foreign, thb, _, _) = stored(&counter, id).await;
    assert_eq!(foreign, d("15"));
    assert_eq!(thb, d("3.47"), "15 × 23.1 ÷ 100 = 3.465, half up");
    counter.finish().await;
}

/// §3.2 the kiosk SELL by baht: the customer pays 1,000 THB for JPY at 23.5 per 100.
/// `foreign_for_thb` = 1000 × 100 ÷ 23.5 = 4255.319…, truncated to JPY's minor unit 0 = 4,255
/// JPY; then `thb` = 4255 × 23.5 ÷ 100 = 999.925 = 999.93 half up (999.92 by banker's).
#[actix_web::test]
async fn a_kiosk_sell_by_baht_truncates_to_whole_yen_then_rounds_half_up() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    publish(&pool, "MAIN", &[("JPY", "23.1", "23.5")]).await;
    let jpy = line(&pool, "MAIN", "JPY").await;

    let id = "00000000-0000-4000-8000-00000004b005";
    let answer = quote(&counter, id, "SELL", "JPY", json!(1000)).await;
    assert_eq!(number(&answer["foreign"], "foreign"), d("4255"), "{answer}");
    assert_eq!(number(&answer["thb"], "thb"), d("999.93"), "{answer}");
    let (line_id, rate, unit_size, foreign, thb, input, _) = stored(&counter, id).await;
    assert_eq!(line_id.as_deref(), Some(jpy.id.as_str()));
    assert_eq!(rate, d("23.5"), "the line's sell rate");
    assert_eq!(unit_size, d("100"));
    assert_eq!(foreign, d("4255"));
    assert_eq!(thb, d("999.93"));
    assert_eq!(input, d("1000"), "the baht the customer entered");
    counter.finish().await;
}

/// R4, §9 RE-4b "a publish between quote and confirm refuses the confirm", with a real
/// interleaving: quote on version 1, publish version 2 **at the same USD rate** (a new line id),
/// then confirm. The confirm is refused because the line moved, not because the rate text did,
/// and it writes nothing: the request stays QUOTED on version 1's line, and no audit line is
/// written. A quote made after version 2 confirms (the refusal is about the line, not the
/// counter).
#[actix_web::test]
async fn a_publish_between_quote_and_confirm_refuses_the_confirm() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    publish(&pool, "MAIN", &[("USD", "33", "34")]).await;
    let first = line(&pool, "MAIN", "USD").await;

    let early = "00000000-0000-4000-8000-00000004b006";
    quote(&counter, early, "BUY", "USD", json!(100)).await;
    let mark = audit_mark(&counter).await;

    publish(&pool, "MAIN", &[("USD", "33", "34")]).await;
    let second = line(&pool, "MAIN", "USD").await;
    assert_ne!(first.id, second.id, "version 2 has a line of its own");
    assert_eq!(second.buy, first.buy, "at the same rate");

    counter.desk.clock.advance(Duration::seconds(4));
    let refused = counter.ask(None, "confirm", json!({ "id": early })).await;
    refused.assert_refused(400, RATE_CHANGED, "confirm after a publish");
    let (line_id, rate, unit_size, _, thb, _, status) = stored(&counter, early).await;
    assert_eq!(status, "QUOTED", "the refused confirm moved nothing");
    assert_eq!(
        line_id.as_deref(),
        Some(first.id.as_str()),
        "a later publication never rewrites the stored line (§4.8)"
    );
    assert_eq!((rate, unit_size, thb), (d("33"), d("1"), d("3300.00")));
    let (written, _) = audit_since(&counter.desk, mark).await;
    assert!(
        written.is_empty(),
        "the refused confirm wrote no audit line: {written:?}"
    );

    let late = "00000000-0000-4000-8000-00000004b007";
    quote(&counter, late, "BUY", "USD", json!(100)).await;
    counter.desk.clock.advance(Duration::seconds(4));
    let confirmed = counter.ask(None, "confirm", json!({ "id": late })).await;
    assert_eq!(confirmed.status, 200, "{}", confirmed.text);
    let (line_id, _, _, _, _, _, status) = stored(&counter, late).await;
    assert_eq!(status, "WAITING");
    assert_eq!(line_id.as_deref(), Some(second.id.as_str()));
    counter.finish().await;
}

/// R4, the same when version 2 changes the rate: refused, nothing written, the quoted rate kept.
#[actix_web::test]
async fn a_publish_that_changes_the_rate_refuses_the_confirm() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    publish(&pool, "MAIN", &[("JPY", "23.1", "23.5")]).await;
    let first = line(&pool, "MAIN", "JPY").await;

    let id = "00000000-0000-4000-8000-00000004b008";
    quote(&counter, id, "BUY", "JPY", json!(1000)).await;
    let mark = audit_mark(&counter).await;
    publish(&pool, "MAIN", &[("JPY", "23.2", "23.5")]).await;

    counter.desk.clock.advance(Duration::seconds(4));
    let refused = counter.ask(None, "confirm", json!({ "id": id })).await;
    refused.assert_refused(400, RATE_CHANGED, "confirm after a new rate");
    let (line_id, rate, unit_size, _, thb, _, status) = stored(&counter, id).await;
    assert_eq!(status, "QUOTED");
    assert_eq!(line_id.as_deref(), Some(first.id.as_str()));
    assert_eq!(
        (rate, unit_size, thb),
        (d("23.1"), d("100"), d("231.00")),
        "the quoted line's rate and THB are kept"
    );
    let (written, _) = audit_since(&counter.desk, mark).await;
    assert!(written.is_empty(), "nothing written: {written:?}");
    counter.finish().await;
}

/// D-3, §9 RE-4b "a currency with no published line is not quoted", even though the direct save
/// (`rates`, the counter's USD 32.1 / 32.6) has it. The refusal is the existing "no rate"
/// answer and writes nothing.
#[actix_web::test]
async fn a_currency_without_a_published_line_is_not_quoted() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();

    publish(&pool, "MAIN", &[("JPY", "23.1", "23.5")]).await;
    refused_quote(
        &counter,
        "BUY",
        "USD",
        json!(100),
        AMOUNT_OR_CURRENCY,
        "USD has no line",
    )
    .await;
    refused_quote(
        &counter,
        "SELL",
        "USD",
        json!(1000),
        AMOUNT_OR_CURRENCY,
        "USD has no line, SELL by baht",
    )
    .await;
    assert_eq!(requests(&counter).await, 0);
    counter.finish().await;
}

/// R5, §9 RE-4b "a stale line under STOP gets the existing 'no rate' answer": USD is AUTO
/// against a reference fetched 30 h ago, under a policy whose references go stale after 24 h
/// with `on_stale = STOP`. Both sides are refused with the "no rate" sentence and nothing is
/// written. JPY, MANUAL in the same publication, is never stale and is quoted.
#[actix_web::test]
async fn a_stale_line_under_stop_gets_the_no_rate_answer() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    publish_auto(&pool, 30, Some("STOP"), None).await;

    refused_quote(
        &counter,
        "BUY",
        "USD",
        json!(1),
        AMOUNT_OR_CURRENCY,
        "USD BUY is stale under STOP",
    )
    .await;
    refused_quote(
        &counter,
        "SELL",
        "USD",
        json!(100),
        AMOUNT_OR_CURRENCY,
        "USD SELL is stale under STOP",
    )
    .await;

    let jpy = line(&pool, "MAIN", "JPY").await;
    let id = "00000000-0000-4000-8000-00000004b009";
    quote(&counter, id, "BUY", "JPY", json!(1000)).await;
    let (line_id, rate, unit_size, _, thb, _, _) = stored(&counter, id).await;
    assert_eq!(line_id.as_deref(), Some(jpy.id.as_str()));
    assert_eq!((rate, unit_size, thb), (d("23.1"), d("100"), d("231.00")));
    assert_eq!(
        requests(&counter).await,
        1,
        "only the JPY quote was written"
    );
    counter.finish().await;
}

/// R5 with the control case: the same USD line fetched 1 h ago is not stale, and STOP quotes it.
#[actix_web::test]
async fn a_fresh_auto_line_under_stop_is_quoted() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    publish_auto(&pool, 1, Some("STOP"), None).await;
    let usd = line(&pool, "MAIN", "USD").await;

    let id = "00000000-0000-4000-8000-00000004b00a";
    let answer = quote(&counter, id, "BUY", "USD", json!(100)).await;
    assert_eq!(number(&answer["thb"], "thb"), d("3210"), "{answer}");
    let (line_id, rate, _, _, thb, _, _) = stored(&counter, id).await;
    assert_eq!(line_id.as_deref(), Some(usd.id.as_str()));
    assert_eq!((rate, thb), (d("32.1"), d("3210.00")));
    counter.finish().await;
}

/// R5 with `on_stale = LIMIT`, `stale_limit_thb = 1,000.00` (A7): `StalePolicy::admits(side,
/// thb)` is asked with the quote's THB. 31 USD at 32.1 is 995.10 THB, quoted; 32 USD is
/// 1,027.20 THB, refused with the "no rate" sentence, and nothing is written.
#[actix_web::test]
async fn a_stale_line_under_limit_is_quoted_only_up_to_the_limit() {
    let counter = Counter::open(&[]).await;
    let pool = counter.desk.db.pool.clone();
    publish_auto(&pool, 30, Some("LIMIT"), Some("1000.00")).await;

    let id = "00000000-0000-4000-8000-00000004b00b";
    let answer = quote(&counter, id, "BUY", "USD", json!(31)).await;
    assert_eq!(number(&answer["thb"], "thb"), d("995.1"), "{answer}");
    let (_, _, _, _, thb, _, _) = stored(&counter, id).await;
    assert_eq!(thb, d("995.10"), "31 × 32.1");

    refused_quote(
        &counter,
        "BUY",
        "USD",
        json!(32),
        AMOUNT_OR_CURRENCY,
        "32 × 32.1 = 1,027.20 is above the stale limit",
    )
    .await;
    assert_eq!(requests(&counter).await, 1);
    counter.finish().await;
}
