//! Ticket #598 (RE-1+RE-3) — stale published lines and `StalePolicy::admits` (RE-1).
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.3 (`max_reference_age_hours`, `on_stale` ∈
//! {STOP, LIMIT, NULL}, `stale_limit_thb`; A3, A7), §4.8 "Stale at read time" (`PublishedSide`
//! carries `stale` and `source_date`; an AUTO line is stale when `now − source_snapshot.fetched_at
//! > max_reference_age_hours` of the policy version it was published under; a MANUAL line with no
//! reference is never stale; `StalePolicy::admits(side, thb) -> Result<(), RateUnavailable>`:
//! STOP refuses, LIMIT refuses a THB amount above `stale_limit_thb`, NULL admits), §9 RE-1.
//!
//! Needs `TEST_DATABASE_URL` (see `rate_engine_common/mod.rs`); nothing here skips. The clock is
//! the wall clock: the references are fetched 30 h and 1 h before the test against a 24 h limit,
//! so no test depends on timing.
//!
//! Signatures this test assumes (besides `rate_draft_flow_test.rs`):
//! - `PublishedSide` has public `stale: bool` and `source_date: Option<chrono::NaiveDate>` (the
//!   reference batch's `source_date`; `None` for a line with no reference).
//! - `xc_rate::domain::policy::stale_policy::{StalePolicy, OnStale, RateUnavailable}`:
//!   `enum OnStale { Stop, Limit }`; `StalePolicy::new(on_stale: Option<OnStale>,
//!   stale_limit_thb: Option<Money>) -> StalePolicy`; `StalePolicy::admits(&self,
//!   side: &PublishedSide, thb: Money) -> Result<(), RateUnavailable>`; `RateUnavailable: Debug`.
//! - `line_in` decides `stale` with the wall clock (`Utc::now()`).
//!
//! OR-7.1 #1020 (design `org-roles-branches.md` §3.3, §4.3), by #1020's ticket author: `RateDraftService::submit`
//! takes `access::Actor`, so its callers here pass the B1-mapped user (`rate_engine_common::owner_user`: EXECUTIVE
//! + ADMIN organisation-wide). Every assertion is unchanged.

mod rate_engine_common;

use chrono::{Duration, Utc};
use rate_engine_common::*;
use serde_json::{Value, json};
use xc_kernel::money::{Money, Side};
use xc_rate::domain::draft::rate_draft::{DraftLine, DraftStatus, LineSide};
use xc_rate::domain::policy::stale_policy::{OnStale, StalePolicy};
use xc_rate::domain::publication::published_rates::{PublishedRates, PublishedSide};

fn money(text: &str) -> Money {
    Money::parse(text).expect("a valid amount")
}

fn guard() -> Value {
    json!({ "min_pct": "-5.0000", "max_pct": "5.0000" })
}

/// MAIN publishes USD AUTO (against a reference fetched `age_hours` ago) and EUR MANUAL (no
/// reference), under a policy whose references go stale after 24 h. Answers the reference's
/// source date.
async fn publish_with_reference_aged(db: &TestDb, e: &Engine, age_hours: i64) -> chrono::NaiveDate {
    let fetched_at = Utc::now() - Duration::hours(age_hours);
    db.seed_batch(fetched_at, &[("USD", "1", Some("32.1"), Some("32.45"))])
        .await;
    db.seed_policy(
        MAIN,
        "0",
        "0",
        json!({ "USD": { "buy": guard(), "sell": guard() } }),
        Some(24),
        Some("STOP"),
        None,
    )
    .await;

    let lines = vec![
        DraftLine::new(code("USD"), LineSide::Auto, LineSide::Auto, None),
        manual_line("EUR", "37.4", "38.0"),
    ];
    let mut tx = db.begin().await;
    let d = e
        .drafts
        .create(&mut tx, &main_branch(), &owner(), lines)
        .await
        .expect("create");
    tx.commit().await.expect("commit");
    let mut tx = db.begin().await;
    let outcome = e
        .drafts
        .submit(&mut tx, &main_branch(), &owner_user(), &d.id, &key())
        .await
        .expect("submit");
    tx.commit().await.expect("commit");
    assert_eq!(
        outcome.status,
        DraftStatus::Approved,
        "OWNER's draft is approved at submit"
    );
    let mut tx = db.begin().await;
    e.publisher
        .publish(&mut tx, &main_branch(), &owner(), &d.id, None)
        .await
        .expect("publish");
    tx.commit().await.expect("commit");
    fetched_at.date_naive()
}

async fn side(e: &Engine, db: &TestDb, currency: &str, s: Side) -> PublishedSide {
    let mut tx = db.begin().await;
    let got = e
        .published
        .line_in(&mut tx, &main_branch(), code(currency), s)
        .await
        .expect("read the published line")
        .unwrap_or_else(|| panic!("{currency} {s:?} is published"));
    tx.rollback().await.expect("roll back");
    got
}

#[tokio::test]
async fn an_auto_line_older_than_max_reference_age_hours_reads_stale_and_a_manual_one_never_does() {
    let db = TestDb::start().await;
    let e = db.engine();
    let source_date = publish_with_reference_aged(&db, &e, 30).await;

    for s in [Side::Buy, Side::Sell] {
        let usd = side(&e, &db, "USD", s).await;
        assert!(usd.stale, "USD {s:?}: fetched 30 h ago, limit 24 h");
        assert_eq!(
            usd.source_date,
            Some(source_date),
            "USD {s:?}: the batch's source date"
        );
        let eur = side(&e, &db, "EUR", s).await;
        assert!(
            !eur.stale,
            "EUR {s:?}: MANUAL with no reference is never stale"
        );
        assert_eq!(
            eur.source_date, None,
            "EUR {s:?}: no reference, no source date"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn an_auto_line_younger_than_max_reference_age_hours_is_not_stale() {
    let db = TestDb::start().await;
    let e = db.engine();
    publish_with_reference_aged(&db, &e, 1).await;
    for s in [Side::Buy, Side::Sell] {
        assert!(
            !side(&e, &db, "USD", s).await.stale,
            "USD {s:?}: fetched 1 h ago"
        );
    }
    // and STOP admits a side that is not stale
    let usd = side(&e, &db, "USD", Side::Sell).await;
    assert!(
        StalePolicy::new(Some(OnStale::Stop), None)
            .admits(&usd, money("1000000.00"))
            .is_ok()
    );
    db.finish().await;
}

#[tokio::test]
async fn stale_policy_stop_refuses_limit_caps_at_the_amount_and_null_admits() {
    let db = TestDb::start().await;
    let e = db.engine();
    publish_with_reference_aged(&db, &e, 30).await;
    let stale = side(&e, &db, "USD", Side::Sell).await;
    let fresh = side(&e, &db, "EUR", Side::Sell).await;
    assert!(stale.stale && !fresh.stale, "the setup");

    // STOP: a stale side is refused whatever the amount
    let stop = StalePolicy::new(Some(OnStale::Stop), None);
    assert!(stop.admits(&stale, money("0.01")).is_err(), "STOP, 0.01");
    assert!(
        stop.admits(&fresh, money("0.01")).is_ok(),
        "STOP, a side that is not stale"
    );

    // LIMIT 1,000.00: exactly the limit is admitted, one satang more is refused
    let limit = StalePolicy::new(Some(OnStale::Limit), Some(money("1000.00")));
    assert!(
        limit.admits(&stale, money("1000.00")).is_ok(),
        "LIMIT, = the limit"
    );
    assert!(
        limit.admits(&stale, money("999.99")).is_ok(),
        "LIMIT, below the limit"
    );
    assert!(
        limit.admits(&stale, money("1000.01")).is_err(),
        "LIMIT, one satang more"
    );
    assert!(
        limit.admits(&fresh, money("1000000.00")).is_ok(),
        "LIMIT caps only a stale side"
    );

    // NULL: quotes go on (the screens show the stale badge)
    let none = StalePolicy::new(None, None);
    assert!(none.admits(&stale, money("1000000.00")).is_ok(), "NULL");
    db.finish().await;
}
