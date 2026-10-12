//! Ticket #769 — one reader of the stale policy a published line was published under:
//! `PublishedRates::stale_policy_in`, implemented by `PgPublishedRates`.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.8 "Stale at read time" (a line is judged by the
//! stale settings of **the policy version it was published under**: `on_stale` ∈ {STOP, LIMIT,
//! NULL} and `stale_limit_thb`), §4.9 R5 (*a stale line whose policy cannot be found must be an
//! error*). Ticket #769 §7 (the [required] signature), §8 (the answers), §9 (the edge cases).
//!
//! The stale lines here are published the way the owner publishes them, as
//! `published_rates_test.rs` does: a BOT reference fetched 30 h ago and a policy version whose
//! references go stale after 24 h are seeded (`rate_engine_common`), and the draft goes through
//! the rate engine. `PublishService` writes `fetched_at`, `max_reference_age_hours` and
//! `policy_version_id` into `source_snapshot` together (`publish_service.rs:330-342`), so no
//! route makes a stale line whose policy cannot be found: those lines are made by rewriting the
//! published line's `source_snapshot` with SQL after a real publish (the data error #769 §2
//! describes). `stale_limit_thb` that is not a `Money` (a negative one) is seeded the same way, in
//! the policy row, because the policy routes never write one.
//!
//! Needs `TEST_DATABASE_URL` (see `rate_engine_common/mod.rs`); nothing here skips.
//!
//! Signatures this test assumes (besides `published_rates_test.rs`):
//! - [required] `PublishedRates::stale_policy_in<'a>(&'a self, tx: &'a mut Transaction<'_,
//!   Postgres>, line: &'a PublishedSide) -> BoxFuture<'a, AppResult<StalePolicy>>`, implemented by
//!   `PgPublishedRates`: `Ok(StalePolicy { on_stale, stale_limit_thb })` from the
//!   `rate_policy_versions` row named by the line's `source_snapshot.policy_version_id`;
//!   `AppError::Internal` when the snapshot has no `policy_version_id`, when no row has that id,
//!   or when `stale_limit_thb` is not a `Money`.
//! - `StalePolicy { on_stale: Option<OnStale>, stale_limit_thb: Option<Money> }` with public
//!   fields, unchanged (`xc-rate/src/domain/policy/stale_policy.rs`).
//!
//! OR-7.1 #1020 (design `org-roles-branches.md` §3.3, §4.3), by #1020's ticket author: `RateDraftService::submit`
//! takes `access::Actor`, so its callers here pass the B1-mapped user (`rate_engine_common::owner_user`: EXECUTIVE
//! + ADMIN organisation-wide). Every assertion is unchanged.

mod rate_engine_common;

use chrono::{Duration, Utc};
use pos_error::{AppError, AppResult};
use rate_engine_common::*;
use serde_json::{Value, json};
use xc_kernel::money::{Money, Side};
use xc_rate::domain::draft::rate_draft::DraftStatus;
use xc_rate::domain::policy::stale_policy::{OnStale, StalePolicy};
use xc_rate::domain::publication::published_rates::{PublishedRates, PublishedSide};

fn money(text: &str) -> Money {
    Money::parse(text).expect("a valid amount")
}

fn guard() -> Value {
    json!({ "min_pct": "-5.0000", "max_pct": "5.0000" })
}

/// MAIN publishes USD AUTO against a reference fetched 30 h ago, under the one policy version
/// of MAIN, whose references go stale after 24 h, with `on_stale` and `stale_limit_thb` as given.
/// Answers the policy version's id.
async fn publish_stale_usd(
    db: &TestDb,
    e: &Engine,
    on_stale: Option<&str>,
    stale_limit_thb: Option<&str>,
) -> String {
    db.seed_batch(
        Utc::now() - Duration::hours(30),
        &[("USD", "1", Some("32.1"), Some("32.45"))],
    )
    .await;
    let policy = db
        .seed_policy(
            MAIN,
            "0",
            "0",
            json!({ "USD": { "buy": guard(), "sell": guard() } }),
            Some(24),
            on_stale,
            stale_limit_thb,
        )
        .await;

    let mut tx = db.begin().await;
    let d = e
        .drafts
        .create(
            &mut tx,
            &main_branch(),
            &owner_user(),
            vec![auto_line("USD")],
        )
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
        .publish(&mut tx, &main_branch(), &owner_user(), &d.id, None)
        .await
        .expect("publish");
    tx.commit().await.expect("commit");

    let stored: Option<String> = sqlx::query_scalar(
        "SELECT l.source_snapshot ->> 'policy_version_id' FROM branches b \
         JOIN published_rate_lines l ON l.publication_id = b.current_publication_id \
         WHERE b.id = $1 AND l.currency_code = 'USD'",
    )
    .bind(MAIN)
    .fetch_one(&db.pool)
    .await
    .expect("read the USD line's snapshot");
    assert_eq!(
        stored.as_deref(),
        Some(policy.as_str()),
        "the setup: the line names the policy it was published under"
    );
    policy
}

/// The USD line of MAIN's current publication for `side`, which must read stale.
async fn stale_side(e: &Engine, db: &TestDb, side: Side) -> PublishedSide {
    let mut tx = db.begin().await;
    let got = e
        .published
        .line_in(&mut tx, &main_branch(), code("USD"), side)
        .await
        .expect("read the published line")
        .unwrap_or_else(|| panic!("USD {side:?} is published"));
    tx.rollback().await.expect("roll back");
    assert!(
        got.stale,
        "the setup: USD {side:?} was fetched 30 h ago, limit 24 h"
    );
    got
}

/// `stale_policy_in` for `line`, in a transaction of its own.
async fn policy_of(e: &Engine, db: &TestDb, line: &PublishedSide) -> AppResult<StalePolicy> {
    let mut tx = db.begin().await;
    let answer = e.published.stale_policy_in(&mut tx, line).await;
    tx.rollback().await.expect("roll back");
    answer
}

fn assert_internal(answer: AppResult<StalePolicy>, what: &str) {
    match answer {
        Err(AppError::Internal(_)) => {}
        Err(other) => panic!("{what}: AppError::Internal expected, got the error {other}"),
        Ok(policy) => panic!(
            "{what}: AppError::Internal expected, got the policy on_stale={:?} limit={:?}",
            policy.on_stale.map(|s| s.as_str()),
            policy.stale_limit_thb.map(|m| m.minor())
        ),
    }
}

/// Rewrite the `source_snapshot` of MAIN's current USD line with `update` (an SQL expression of
/// `source_snapshot`).
async fn rewrite_snapshot(db: &TestDb, update: &str) {
    let changed = sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE published_rate_lines l SET source_snapshot = {update} \
         FROM branches b WHERE b.id = '{MAIN}' AND l.publication_id = b.current_publication_id \
           AND l.currency_code = 'USD'"
    )))
    .execute(&db.pool)
    .await
    .expect("rewrite the line's snapshot")
    .rows_affected();
    assert_eq!(changed, 1, "the setup: one USD line rewritten");
}

/// §8: STOP is read from the row the line names, on both sides.
#[tokio::test]
async fn a_stale_line_published_under_stop_reads_stop() {
    let db = TestDb::start().await;
    let e = db.engine();
    publish_stale_usd(&db, &e, Some("STOP"), None).await;
    for side in [Side::Buy, Side::Sell] {
        let line = stale_side(&e, &db, side).await;
        let policy = policy_of(&e, &db, &line)
            .await
            .unwrap_or_else(|err| panic!("USD {side:?} under STOP: {err}"));
        assert_eq!(policy.on_stale, Some(OnStale::Stop), "USD {side:?}");
        assert_eq!(policy.stale_limit_thb, None, "USD {side:?}: no limit");
        assert!(
            policy.admits(&line, money("0.01")).is_err(),
            "USD {side:?}: STOP refuses the stale line"
        );
    }
    db.finish().await;
}

/// §8: LIMIT is read with its limit; `admits` then admits exactly the limit and refuses one
/// satang over (§9).
#[tokio::test]
async fn a_stale_line_published_under_limit_reads_limit_and_its_limit() {
    let db = TestDb::start().await;
    let e = db.engine();
    publish_stale_usd(&db, &e, Some("LIMIT"), Some("1027.20")).await;
    let line = stale_side(&e, &db, Side::Buy).await;
    let policy = policy_of(&e, &db, &line)
        .await
        .unwrap_or_else(|err| panic!("USD under LIMIT: {err}"));
    assert_eq!(policy.on_stale, Some(OnStale::Limit));
    assert_eq!(policy.stale_limit_thb, Some(money("1027.20")));
    assert!(
        policy.admits(&line, money("1027.20")).is_ok(),
        "= the limit"
    );
    assert!(
        policy.admits(&line, money("1027.21")).is_err(),
        "one satang over the limit"
    );
    db.finish().await;
}

/// §8, §9 "Stale, `on_stale` NULL": the row's NULLs are read as `None`, and `admits` admits.
#[tokio::test]
async fn a_stale_line_published_under_a_null_policy_reads_none_and_is_admitted() {
    let db = TestDb::start().await;
    let e = db.engine();
    publish_stale_usd(&db, &e, None, None).await;
    let line = stale_side(&e, &db, Side::Sell).await;
    let policy = policy_of(&e, &db, &line)
        .await
        .unwrap_or_else(|err| panic!("USD under NULL: {err}"));
    assert_eq!(policy.on_stale, None);
    assert_eq!(policy.stale_limit_thb, None);
    assert!(
        policy.admits(&line, money("1000000.00")).is_ok(),
        "NULL admits"
    );
    db.finish().await;
}

/// §8: the policy the line was published under, not one in force later.
#[tokio::test]
async fn the_policy_read_is_the_one_the_line_names_not_a_later_one() {
    let db = TestDb::start().await;
    let e = db.engine();
    publish_stale_usd(&db, &e, Some("STOP"), None).await;
    // a later version of MAIN, which would admit the line
    sqlx::query(
        "INSERT INTO rate_policy_versions \
           (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
            max_reference_age_hours, on_stale, stale_limit_thb, valid_from, valid_to, \
            approved_by, reason, source_version) \
         VALUES (gen_random_uuid(), $1, 2, 0, 0, '{}'::jsonb, 24, NULL, NULL, now(), NULL, \
                 $2, 'a later version', 1)",
    )
    .bind(MAIN)
    .bind(OWNER_ID)
    .execute(&db.pool)
    .await
    .expect("insert a later policy version");
    let line = stale_side(&e, &db, Side::Buy).await;
    let policy = policy_of(&e, &db, &line)
        .await
        .unwrap_or_else(|err| panic!("USD: {err}"));
    assert_eq!(
        policy.on_stale,
        Some(OnStale::Stop),
        "version 1, which the line names"
    );
    db.finish().await;
}

/// §8, §9 "Stale, snapshot without `policy_version_id`": the key absent, and the key null, are
/// both an error, never "no policy".
#[tokio::test]
async fn a_stale_line_whose_snapshot_names_no_policy_is_internal() {
    let db = TestDb::start().await;
    let e = db.engine();
    publish_stale_usd(&db, &e, Some("STOP"), None).await;

    rewrite_snapshot(&db, "source_snapshot - 'policy_version_id'").await;
    for side in [Side::Buy, Side::Sell] {
        let line = stale_side(&e, &db, side).await;
        assert_internal(
            policy_of(&e, &db, &line).await,
            &format!("USD {side:?}: no policy_version_id key"),
        );
    }

    rewrite_snapshot(
        &db,
        "jsonb_set(source_snapshot, '{policy_version_id}', 'null'::jsonb, true)",
    )
    .await;
    let line = stale_side(&e, &db, Side::Buy).await;
    assert_internal(
        policy_of(&e, &db, &line).await,
        "USD BUY: policy_version_id is null",
    );
    db.finish().await;
}

/// §8, §9 "Stale, policy row missing": the id the line names has no `rate_policy_versions` row.
#[tokio::test]
async fn a_stale_line_whose_policy_row_is_missing_is_internal() {
    let db = TestDb::start().await;
    let e = db.engine();
    publish_stale_usd(&db, &e, Some("STOP"), None).await;
    let unknown = uuid::Uuid::new_v4().to_string();
    rewrite_snapshot(
        &db,
        &format!("jsonb_set(source_snapshot, '{{policy_version_id}}', '\"{unknown}\"'::jsonb)"),
    )
    .await;
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_policy_versions WHERE id::text = $1")
            .bind(&unknown)
            .fetch_one(&db.pool)
            .await
            .expect("count the policy rows");
    assert_eq!(rows, 0, "the setup: no row has that id");
    for side in [Side::Buy, Side::Sell] {
        let line = stale_side(&e, &db, side).await;
        assert_internal(
            policy_of(&e, &db, &line).await,
            &format!("USD {side:?}: the policy row is missing"),
        );
    }
    db.finish().await;
}

/// §8: a `stale_limit_thb` that is not a `Money` is an error, never "no limit" (today the
/// copies read it with `Money::parse(..).ok()`).
#[tokio::test]
async fn a_stale_limit_that_is_not_money_is_internal() {
    let db = TestDb::start().await;
    let e = db.engine();
    publish_stale_usd(&db, &e, Some("LIMIT"), Some("1000.00")).await;
    let changed =
        sqlx::query("UPDATE rate_policy_versions SET stale_limit_thb = -1.00 WHERE branch_id = $1")
            .bind(MAIN)
            .execute(&db.pool)
            .await
            .expect("make the limit negative")
            .rows_affected();
    assert_eq!(changed, 1, "the setup: MAIN's one policy version");
    let line = stale_side(&e, &db, Side::Buy).await;
    assert_internal(
        policy_of(&e, &db, &line).await,
        "USD BUY: stale_limit_thb -1.00",
    );
    db.finish().await;
}
