//! Ticket #757 (RE-4c) — `adminSnapshot.rates[]` and `adminDeskConfig.rates` read the branch's
//! current publication, per 1 unit; the answer of the direct save (`adminSaveRate`) shows the
//! publication, so a direct save is no longer visible in it.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Re-expected for RE-7 #604 by the ticket author (rates.md §8 migration 6, §9 RE-7): no test
//! writes a direct-save `rates` row any more; `rateHistory` is expected `[]` (R10 after RE-7);
//! the test of `adminSaveRate`'s answer is retired with the direct save (note at its place).
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
//! (`rates_re4_common::publish`). Numbers are compared as the `V2Json` the builder prints, so
//! `0.231` is held to its exact text.
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
//! - the merged signatures of `SnapshotBuilder::{new, admin_snapshot, desk_config}`
//!   and `time_json`, as on the merge base `d6ce058`.

mod compat_common;
mod snapshot_common;

use chrono::{DateTime, Utc};
use snapshot_common::rates_re4c_common::publish;
use snapshot_common::{MAIN, Shop, branch, time};
use sqlx::PgPool;
use xc_kernel::money::CurrencyCode;
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
/// 100 shows 0.231/0.235; KRW at 25.12345678 per 1000 shows 0.02512345678, exact; with no
/// partner line USD shows no partner. A currency the publication does not list is unset.
/// `rateHistory` is `[]` (R10: `rates` is dropped by RE-7). The shape is v2's: 152 entries in
/// register order.
///
/// Re-expected for RE-7 #604 by the ticket author: the direct-save rows (the partner USD row
/// before the publication, the USD and EUR rows after it) are gone with `rates`; so is the
/// partner they carried, which RE-7 moves to `'PARTNER'` reference lines (rates.md §4.1). How
/// `partnerName`/`partnerCost` show such a line is not pinned here (see the RE-7 report).
/// `rateHistory` is re-expected from "lists the rows of `rates`" to `[]`.
#[tokio::test]
async fn admin_snapshot_rates_are_the_current_publication_per_1() {
    let shop = Shop::open().await;
    let pool = &shop.db.pool;
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
        published("32.1", "33", &at, ("", None)),
        "no partner line: no partner"
    );
    let eur = entry(&answer, "EUR");
    assert_eq!(
        (field(eur, "buy"), field(eur, "sell")),
        (&V2Json::Null, &V2Json::Null),
        "a currency the publication does not list is no counter rate"
    );
    assert_eq!(
        field(&answer, "rateHistory"),
        &V2Json::Array(Vec::new()),
        "R10: `rateHistory` is empty once RE-7 drops `rates`"
    );
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

// Retired for RE-7 #604 by the ticket author: `the_answer_of_a_direct_save_shows_the_publication_
// not_the_save` drove `RateService::save_rate` (`adminSaveRate`), which RE-7 removes (rates.md §9
// RE-7, the "Provided tests that change" row "the direct save → publications"). That
// `adminSaveRate` is gone is pinned by `rates_re7_retire_test.rs`; that the snapshot shows the
// publication is pinned by the tests above and below.

/// D-3: before the branch's first publication there is no counter rate. (Before RE-7 a
/// direct-save row sat in `rates` here as a decoy; RE-7 #604 drops the table.)
#[tokio::test]
async fn before_the_first_publication_no_rate_is_set() {
    let shop = Shop::open().await;

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
