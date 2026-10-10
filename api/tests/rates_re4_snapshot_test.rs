//! Ticket #757 (RE-4c) — `adminSnapshot.rates[]` and `adminDeskConfig.rates` read the branch's
//! current publication, per 1 unit; the answer of the direct save (`adminSaveRate`) shows the
//! publication, so a direct save is no longer visible in it.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md`
//! - §4.8 (`adminSnapshot` reads `current`; v2 shape kept: `rates[]` from the lines,
//!   `updated` = `published_at`);
//! - §4.9 R3 (the v2-compat wire stays per 1: `rate ÷ unit_size`, exact, normalised, through
//!   `CompatRate::per_one`) and R10 (`updated` = `published_at`; `requestId` `""`;
//!   `partnerName`/`partnerCost` from `source_snapshot.partner`, else `""`/null;
//!   `botReference` `""`; `rateHistory` stays on `rates` until RE-7; `adminSaveRate`'s answer
//!   (`write_answer(snapshot)`) shows the publication);
//! - D-3 (the current rate is the line of `branches.current_publication_id`);
//! - §9 RE-4c: "`adminSnapshot.rates[]` is the publication per 1 (R10)".
//!
//! The builder is driven directly, as `snapshot_test.rs` drives it (the harness of
//! `snapshot_common`). Publications are made through the rate engine
//! (`rates_re4_common::publish`); the direct save through `RateService::save_rate`, which
//! `adminSaveRate` calls, or as the row it appends. Numbers are compared as the `V2Json` the
//! builder prints, so `0.231` is held to its exact text.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.
//!
//! Signatures this test assumes:
//! - [required] `adminSnapshot.rates[]` and `adminDeskConfig.rates` keep v2's shape: one entry per
//!   register code but `THB`, in register order, `{code, name, symbol?, buy, sell, updated,
//!   requestId, partnerName, partnerCost, botReference}`; a currency the publication does not
//!   list keeps today's unset `buy`/`sell` (`null`);
//! - [required] a published entry is R10's: `buy`/`sell` = the line's rate ÷ `unit_size`, exact
//!   and normalised (R3); `updated` = `published_at` as `v2_json::time_json` prints it;
//!   `requestId` `""`; `partnerName`/`partnerCost` from `source_snapshot.partner`;
//!   `botReference` `""`;
//! - the merged signatures of `SnapshotBuilder::{new, admin_snapshot, desk_config}`,
//!   `write_answer`, `time_json` and `RateService::save_rate`, as on the merge base `d6ce058`.

mod compat_common;
mod snapshot_common;

use chrono::{DateTime, Utc};
use snapshot_common::rates_re4c_common::{line, publish};
use snapshot_common::{MAIN, Shop, branch, time};
use sqlx::PgPool;
use xc_kernel::money::CurrencyCode;
use xc_rate::domain::rate::commands::save_rate::SaveRate;
use xc_rate::domain::rate::entities::rate_source::ReferenceRates;
use xchange::application::compat::snapshot::write_answer;
use xchange::application::compat::v2_json::{V2Json, time_json};

const AT: &str = "2026-10-06T06:00:00.000Z";

/// The keys of a published entry that R10 decides, in v2's order.
const DECIDED: [&str; 7] = [
    "buy",
    "sell",
    "updated",
    "requestId",
    "partnerName",
    "partnerCost",
    "botReference",
];

fn number(text: &str) -> V2Json {
    V2Json::Number(text.to_owned())
}

fn text(text: &str) -> V2Json {
    V2Json::Text(text.to_owned())
}

/// The value of `key` in a printed object.
fn field<'a>(object: &'a V2Json, key: &str) -> &'a V2Json {
    let V2Json::Object(pairs) = object else {
        panic!("not an object: {}", object.to_text());
    };
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
        .unwrap_or_else(|| panic!("no `{key}` in {}", object.to_text()))
}

/// The entries of `rates` in a snapshot or a desk config.
fn rates(answer: &V2Json) -> &[V2Json] {
    match field(answer, "rates") {
        V2Json::Array(entries) => entries,
        other => panic!("rates is not a list: {}", other.to_text()),
    }
}

/// The `rates` entry of `code`.
fn entry<'a>(answer: &'a V2Json, code: &str) -> &'a V2Json {
    rates(answer)
        .iter()
        .find(|e| field(e, "code") == &text(code))
        .unwrap_or_else(|| panic!("no rate of {code}"))
}

/// The R10 fields of `code`'s entry, in order.
fn decided(answer: &V2Json, code: &str) -> Vec<(&'static str, V2Json)> {
    let e = entry(answer, code);
    DECIDED.iter().map(|k| (*k, field(e, k).clone())).collect()
}

/// What R10 says a published entry shows.
fn published(
    buy: &str,
    sell: &str,
    at: &V2Json,
    partner: (&str, Option<&str>),
) -> Vec<(&'static str, V2Json)> {
    vec![
        ("buy", number(buy)),
        ("sell", number(sell)),
        ("updated", at.clone()),
        ("requestId", text("")),
        ("partnerName", text(partner.0)),
        ("partnerCost", partner.1.map_or(V2Json::Null, number)),
        ("botReference", text("")),
    ]
}

/// `published_at` of `branch`'s current publication, as `time_json` prints it.
async fn published_at(pool: &PgPool, branch: &str) -> V2Json {
    let at: DateTime<Utc> = sqlx::query_scalar(
        "SELECT p.published_at FROM branches b JOIN rate_publications p \
           ON p.id = b.current_publication_id WHERE b.id = $1",
    )
    .bind(branch)
    .fetch_one(pool)
    .await
    .expect("the branch has a publication");
    time_json(at)
}

/// A row of the direct save (`rates`, per 1), as `adminSaveRate` appends it, with a partner
/// `(name, cost)` or none.
async fn direct_save(
    pool: &PgPool,
    code: &str,
    buy: &str,
    sell: &str,
    partner: Option<(&str, &str)>,
) {
    let (name, cost) = partner.map_or(("", None), |(n, c)| (n, Some(c)));
    sqlx::query(
        "INSERT INTO rates (branch_id, code, buy, sell, partner_name, partner_cost, updated_at, \
         updated_by, request_id) \
         VALUES ($1, $2, $3::numeric, $4::numeric, $5, $6::numeric, now(), 'owner', \
         gen_random_uuid()::text)",
    )
    .bind(MAIN)
    .bind(code)
    .bind(buy)
    .bind(sell)
    .bind(name)
    .bind(cost)
    .execute(pool)
    .await
    .expect("insert into rates");
}

async fn snapshot(shop: &Shop) -> V2Json {
    shop.builder()
        .admin_snapshot(&branch(MAIN), "owner", time(AT))
        .await
        .expect("the snapshot")
}

/// The register codes but `THB`, in register order: v2's 152 entries.
fn register_but_thb() -> Vec<V2Json> {
    CurrencyCode::all()
        .iter()
        .filter(|c| !c.is_thb())
        .map(|c| text(c.as_str()))
        .collect()
}

/// §9 RE-4c / R10 / R3: `adminSnapshot.rates[]` is the publication, per 1. JPY at 23.1/23.5 per
/// 100 shows 0.231/0.235; KRW at 25.12345678 per 1000 shows 0.02512345678, exact; USD shows the
/// partner the publication copied. A direct save after the publication shows nowhere in
/// `rates[]`, but `rateHistory` still lists it (RE-7 drops it). A currency the publication does
/// not list is unset. The shape is v2's: 152 entries in register order.
#[tokio::test]
async fn admin_snapshot_rates_are_the_current_publication_per_1() {
    let shop = Shop::open().await;
    let pool = &shop.db.pool;
    direct_save(pool, "USD", "32", "33", Some(("Partner Co", "31.9"))).await;
    publish(
        pool,
        MAIN,
        &[
            ("USD", "32.1", "33"),
            ("JPY", "23.1", "23.5"),
            ("KRW", "25.12345678", "26"),
        ],
    )
    .await;
    let at = published_at(pool, MAIN).await;
    direct_save(pool, "USD", "35", "36", Some(("Other", "34"))).await;
    direct_save(pool, "EUR", "35.5", "36.25", None).await;

    let answer = snapshot(&shop).await;

    assert_eq!(
        rates(&answer)
            .iter()
            .map(|e| field(e, "code").clone())
            .collect::<Vec<_>>(),
        register_but_thb(),
        "v2's 152 entries, in register order"
    );
    assert_eq!(
        decided(&answer, "JPY"),
        published("0.231", "0.235", &at, ("", None))
    );
    assert_eq!(
        decided(&answer, "KRW"),
        published("0.02512345678", "0.026", &at, ("", None)),
        "25.12345678 ÷ 1000, exact"
    );
    assert_eq!(
        decided(&answer, "USD"),
        published("32.1", "33", &at, ("Partner Co", Some("31.9"))),
        "the publication's partner, not the direct save's after it"
    );
    let eur = entry(&answer, "EUR");
    assert_eq!(
        (field(eur, "buy"), field(eur, "sell")),
        (&V2Json::Null, &V2Json::Null),
        "a direct save the publication does not list is no counter rate"
    );
    let V2Json::Array(history) = field(&answer, "rateHistory") else {
        panic!("rateHistory is a list");
    };
    assert_eq!(
        history.len(),
        3,
        "rateHistory still lists the rows of `rates`"
    );
    assert_eq!(field(&history[0], "code"), &text("EUR"));
    assert_eq!(field(&history[1], "buy"), &number("35"));
    shop.finish().await;
}

/// R3/R10: `adminDeskConfig.rates` is the snapshot's: the publication per 1.
#[tokio::test]
async fn desk_config_rates_are_the_snapshots() {
    let shop = Shop::open().await;
    let pool = &shop.db.pool;
    publish(
        pool,
        MAIN,
        &[("USD", "32.1", "33"), ("JPY", "21.95123456", "22.95")],
    )
    .await;
    let at = published_at(pool, MAIN).await;
    direct_save(pool, "USD", "40", "41", None).await;

    let desk = shop
        .builder()
        .desk_config(&branch(MAIN))
        .await
        .expect("the desk config");
    assert_eq!(
        decided(&desk, "JPY"),
        published("0.2195123456", "0.2295", &at, ("", None)),
        "21.95123456 ÷ 100, exact"
    );
    assert_eq!(
        decided(&desk, "USD"),
        published("32.1", "33", &at, ("", None))
    );
    assert_eq!(
        field(&desk, "rates"),
        field(&snapshot(&shop).await, "rates"),
        "one rate list"
    );
    shop.finish().await;
}

/// D-3 / R10: `adminSaveRate` still writes `rates`, but its answer (`write_answer` of the
/// snapshot) shows the publication: the rate just saved is not in it, and neither is a
/// currency only the direct save knows.
#[tokio::test]
async fn the_answer_of_a_direct_save_shows_the_publication_not_the_save() {
    let shop = Shop::open().await;
    let pool = &shop.db.pool;
    publish(pool, MAIN, &[("USD", "32.1", "33")]).await;
    let usd = line(pool, MAIN, "USD").await;
    let at = published_at(pool, MAIN).await;

    for (n, code, buy, sell) in [(1, "USD", "35", "36"), (2, "EUR", "35.5", "36.25")] {
        shop.source.answer_next(ReferenceRates {
            source: "https://www.bot.or.th/th/statistics/exchange-rate.html".to_owned(),
            fetched_at: "2026-10-06T05:00:00.000Z".to_owned(),
            kind: "DAILY_COMMERCIAL_BANK_AVERAGE".to_owned(),
            rows: Vec::new(),
        });
        shop.rates
            .save_rate(
                &branch(MAIN),
                SaveRate {
                    request_id: format!("00000000-0000-4000-8000-0000000c{n:04}"),
                    code: code.to_owned(),
                    buy: buy.to_owned(),
                    sell: sell.to_owned(),
                    partner_name: String::new(),
                    partner_cost: None,
                    previous_updated: String::new(),
                },
                "owner",
                time(AT),
            )
            .await
            .expect("the direct save is still accepted");
    }
    let answer = write_answer(&snapshot(&shop).await);

    assert_eq!(
        decided(&answer, "USD"),
        published("32.1", "33", &at, ("", None)),
        "the published USD, not the 35/36 just saved"
    );
    let eur = entry(&answer, "EUR");
    assert_eq!(
        (field(eur, "buy"), field(eur, "sell")),
        (&V2Json::Null, &V2Json::Null)
    );
    assert_eq!(
        line(pool, MAIN, "USD").await,
        usd,
        "the direct save published nothing"
    );
    let V2Json::Array(history) = field(&answer, "rateHistory") else {
        panic!("rateHistory is a list");
    };
    assert_eq!(
        field(&history[0], "code"),
        &text("EUR"),
        "the save is in rateHistory"
    );
    shop.finish().await;
}

/// D-3: before the branch's first publication there is no counter rate, whatever `rates` holds.
#[tokio::test]
async fn before_the_first_publication_no_rate_is_set() {
    let shop = Shop::open().await;
    let pool = &shop.db.pool;
    direct_save(pool, "USD", "32.1", "33", Some(("Partner Co", "31.9"))).await;

    let answer = snapshot(&shop).await;
    assert_eq!(rates(&answer).len(), 152);
    for e in rates(&answer) {
        assert_eq!(
            (field(e, "buy"), field(e, "sell")),
            (&V2Json::Null, &V2Json::Null),
            "{}",
            e.to_text()
        );
    }
    shop.finish().await;
}

/// §4.8: the snapshot follows the branch's current publication. Version 2 moves USD and drops
/// EUR: USD shows version 2's rate and time, EUR is unset; another branch's publication is not
/// this branch's.
#[tokio::test]
async fn a_newer_publication_replaces_the_rates_and_their_time() {
    let shop = Shop::open().await;
    let pool = &shop.db.pool;
    shop.db.seed_branch("XC-B01", "Airport").await;
    publish(
        pool,
        MAIN,
        &[("USD", "32.1", "33"), ("EUR", "35.5", "36.25")],
    )
    .await;
    publish(pool, MAIN, &[("USD", "32.2", "33.1")]).await;
    let second = published_at(pool, MAIN).await;
    publish(pool, "XC-B01", &[("USD", "30", "31")]).await;

    let answer = snapshot(&shop).await;
    assert_eq!(
        decided(&answer, "USD"),
        published("32.2", "33.1", &second, ("", None))
    );
    let eur = entry(&answer, "EUR");
    assert_eq!(
        (field(eur, "buy"), field(eur, "sell")),
        (&V2Json::Null, &V2Json::Null),
        "version 2 lists no EUR"
    );
    shop.finish().await;
}
