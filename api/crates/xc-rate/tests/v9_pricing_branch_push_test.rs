//! XC-V9-PRICING PV-d1 (#5162): `BranchPush::push` (pricing-tiers.md §15.4, §15.5 "pushes to branches that have
//! `follow = true`", §15.8a `branches[].state`; behaviour PV-7, PV-8, PV-9, PV-14 of §15.11). After MAIN's set, every
//! other active branch is adjusted in basis points (`H:` §7), written as its own tier set and published, each in its
//! own transaction; `own` branches are skipped, a failing branch is reported and undoes nothing, a retry is
//! `ALREADY_DONE`, and FOLLOW reaches only branches whose currency row enrols them.
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! `H:` is the owner's handoff (`PRICING-DESIGN.md`, kept out of the repo). MAIN's tiers are PV-1's (`H:` §4.4);
//! TT is 33.20/33.90.
//!
//! Readings:
//! - reading: a pushed branch's publication carries `source_snapshot.pricing.source` = the branch row's stored
//!   source (`"MAIN_PUSH"` for a manual push, `"FOLLOW"` for a follow push); the projection reads `MAIN_PUSH` as
//!   `MANUAL` (§15.7), the snapshot does not.
//! - reading: D (+300/+300) fails in `BranchPricingPolicy::adjust`'s `TierCheck` (tier 1 more than 3 % from the TT
//!   anchor), reported as `validationFailed` (`branch_push.rs` doc), the same reading as
//!   `api/tests/v9_pricing_push_test.rs`.
//! - reading: a retried push still lists every branch: the one already written is `ALREADY_DONE`, an `own` one is
//!   `SKIPPED_OWN` again, and a branch that failed is tried again (nothing of it was written) and fails again.
//! - reading: in a FOLLOW push, a branch whose currency row has `follow = true` and `own = true` is never published;
//!   whether it is listed (`SKIPPED_OWN`) or left out is not pinned here.
//!
//! Derived numbers (from `H:` §7's formula, `buy = floor_d(main × (1 + buyBp/10000))`,
//! `sell = ceil_d(main × (1 + sellBp/10000))`, d = 2 from the base):
//! - B at +5/−5: T1 33.23 × 1.0005 = 33.246615 → 33.24; 33.87 × 0.9995 = 33.853065 → 33.86 (`H:` §7's own example);
//!   T2 33.256620 → 33.25 / 33.843070 → 33.85; T3 33.276630 → 33.27 / 33.823080 → 33.83;
//!   T4 33.306645 → 33.30 / 33.783100 → 33.79. Monotonic already.
//! - D at +300/+300: T1 33.23 × 1.03 = 34.2269 → 34.22; 33.87 × 1.03 = 34.8861 → 34.89. 34.22 / 33.20 − 1 = +3.07 %
//!   on the buy side, beyond 3 × farFromAnchorPct (1 %) = 3 %.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test` (see
//! `rate_engine_common`). Every test fails without it; none skips.

mod rate_engine_common;

use std::sync::Arc;

use chrono::{DateTime, Duration, TimeZone, Utc};
use rate_engine_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_rate::application::pricing::branch_push::{
    BranchPush, BranchPushState, BranchResult, PushKind,
};
use xc_rate::application::pricing::tier_publish::TierSetPublisher;
use xc_rate::domain::pricing::branch_policy::{BranchPolicyPatch, BranchPolicyStore};
use xc_rate::domain::pricing::market_quote::MarketSource;
use xc_rate::domain::pricing::tier_set::{
    MarketAnchor, PricingTierSet, TierRate, TierSetSource, TierSetStore,
};
use xc_rate::infrastructure::pricing::branch_policy_store::PgBranchPolicyStore;
use xc_rate::infrastructure::pricing::tier_set_store::PgTierSetStore;

const B: &str = "XC-PV-B";
const C: &str = "XC-PV-C";
const D: &str = "XC-PV-D";
const E: &str = "XC-PV-E";

/// PV-1's tiers (`H:` §4.4), per unit size 1.
const PV1: [(&str, &str); 4] = [
    ("33.23", "33.87"),
    ("33.24", "33.86"),
    ("33.26", "33.84"),
    ("33.29", "33.80"),
];

/// MAIN's PV-1 tiers at B with buyBp +5 / sellBp −5 (derived from H: §7's formula; see the header).
const B_TIERS: [(&str, &str); 4] = [
    ("33.24", "33.86"),
    ("33.25", "33.85"),
    ("33.27", "33.83"),
    ("33.30", "33.79"),
];

fn dec(text: &str) -> Decimal {
    text.parse()
        .unwrap_or_else(|e| panic!("decimal {text}: {e}"))
}

fn decs(prices: [(&str, &str); 4]) -> Vec<(Decimal, Decimal)> {
    prices.iter().map(|(b, s)| (dec(b), dec(s))).collect()
}

fn v9_bands() -> Vec<(Option<Decimal>, Option<Decimal>)> {
    vec![
        (None, Some(dec("10000"))),
        (Some(dec("10000")), Some(dec("50000"))),
        (Some(dec("50000")), Some(dec("200000"))),
        (Some(dec("200000")), None),
    ]
}

fn whole_second_now() -> DateTime<Utc> {
    Utc.timestamp_opt(Utc::now().timestamp(), 0)
        .single()
        .expect("now")
}

fn guards() -> Value {
    let g = json!({ "buy": { "min_pct": "-5.0000", "max_pct": "5.0000" },
                    "sell": { "min_pct": "-5.0000", "max_pct": "5.0000" } });
    json!({ "USD": g })
}

/// MAIN and `branches`, each on policy version 1 (adjust −0.5 / +0.4, guards ±5 %, value tiers off); BoT USD
/// 32.45/32.90; a fresh TT USD 33.20/33.90 with its `market_feed_status`; and at each branch a publication of USD
/// AUTO by today's path. Answers the TT `(batch id, fetched_at)`.
async fn push_room(db: &TestDb, branches: &[&str]) -> (Uuid, DateTime<Utc>) {
    db.seed_policy(MAIN, "-0.5", "0.4", guards(), None, None, None)
        .await;
    for b in branches {
        db.seed_branch(b).await;
        db.seed_policy(b, "-0.5", "0.4", guards(), None, None, None)
            .await;
    }
    db.seed_batch(
        Utc::now() - Duration::hours(1),
        &[("USD", "1", Some("32.4500"), Some("32.9000"))],
    )
    .await;
    let at = whole_second_now();
    let batch: Uuid = sqlx::query_scalar(
        "INSERT INTO rate_reference_batches (id, provider, source_date, fetched_at, raw_checksum, status) \
         VALUES (gen_random_uuid(), 'TT', CURRENT_DATE, $1, $2, 'COMPLETE') RETURNING id",
    )
    .bind(at)
    .bind(unique("chk"))
    .fetch_one(&db.pool)
    .await
    .expect("insert the TT batch");
    sqlx::query(
        "INSERT INTO rate_reference_lines (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
         VALUES (gen_random_uuid(), $1, 'USD', 'MARKET_TT', 1, 33.20, 33.90, '{}'::jsonb)",
    )
    .bind(batch)
    .execute(&db.pool)
    .await
    .expect("insert the TT line");
    sqlx::query(
        "INSERT INTO market_feed_status (source, last_attempt_at, last_error) VALUES ('TT', $1, NULL) \
         ON CONFLICT (source) DO UPDATE SET last_attempt_at = EXCLUDED.last_attempt_at, last_error = NULL",
    )
    .bind(at)
    .execute(&db.pool)
    .await
    .expect("upsert market_feed_status");

    let e = db.engine();
    for b in branches {
        let branch = BranchId::new(*b);
        let mut tx = db.begin().await;
        let draft = e
            .drafts
            .create(&mut tx, &branch, &owner_user(), vec![auto_line("USD")])
            .await
            .expect("create");
        e.drafts
            .submit(&mut tx, &branch, &owner_user(), &draft.id, &key())
            .await
            .expect("submit");
        e.publisher
            .publish(&mut tx, &branch, &owner_user(), &draft.id, None)
            .await
            .expect("publish");
        tx.commit()
            .await
            .expect("commit the branch's first publication");
    }
    (batch, at)
}

/// MAIN's set of USD (PV-1's tiers, anchored on TT), written and committed. A MANUAL set's `request_id` is
/// `<approve key>:USD`; a FOLLOW set's is `follow:MAIN:USD:<TT batch>` (§15.5).
async fn main_set(db: &TestDb, source: TierSetSource, tt: (Uuid, DateTime<Utc>)) -> PricingTierSet {
    let request_id = match source {
        TierSetSource::Follow => format!("follow:MAIN:USD:{}", tt.0),
        _ => format!("{}:USD", unique("approve")),
    };
    let set = PricingTierSet {
        id: Uuid::new_v4(),
        request_id,
        branch: MAIN.to_owned(),
        currency: code("USD"),
        source,
        base_tier_set_id: None,
        tiers: PV1.map(|(b, s)| TierRate {
            buy: rate(b),
            sell: rate(s),
        }),
        anchor: Some(MarketAnchor {
            source: MarketSource::Tt,
            buy: rate("33.20"),
            sell: rate("33.90"),
            fetched_at: tt.1,
            batch_id: Some(tt.0),
        }),
        flags: Vec::new(),
        draft_id: None,
        approved_by: OWNER_ID.to_owned(),
        approved_at: Utc::now(),
    };
    let mut tx = db.begin().await;
    assert!(
        PgTierSetStore
            .insert_in(&mut tx, &set)
            .await
            .expect("insert MAIN's set")
    );
    tx.commit().await.expect("commit MAIN's set");
    set
}

/// `pricingBranchPolicySave`'s store write for (branch, scope).
async fn save_policy(db: &TestDb, branch: &str, scope: &str, patch: BranchPolicyPatch) {
    let mut tx = db.begin().await;
    PgBranchPolicyStore
        .save_in(&mut tx, branch, scope, patch, OWNER_ID, Utc::now())
        .await
        .expect("save the branch policy");
    tx.commit().await.expect("commit the branch policy");
}

fn bp(buy: i32, sell: i32) -> BranchPolicyPatch {
    BranchPolicyPatch {
        buy_bp: Some(buy),
        sell_bp: Some(sell),
        ..BranchPolicyPatch::default()
    }
}

fn pusher(db: &TestDb) -> BranchPush {
    let e = db.engine();
    BranchPush::new(
        db.pool.clone(),
        Arc::new(TierSetPublisher::new(e.drafts, e.publisher)),
        Arc::new(PgTierSetStore),
        Arc::new(PgBranchPolicyStore),
    )
}

fn states(results: &[BranchResult]) -> Vec<(&str, BranchPushState)> {
    results
        .iter()
        .map(|r| (r.branch.as_str(), r.state))
        .collect()
}

/// One `pricing_tier_sets` row of a branch.
#[derive(Debug, Clone)]
struct Row {
    id: Uuid,
    request_id: String,
    source: String,
    base_tier_set_id: Option<Uuid>,
    tiers: Value,
}

impl Row {
    fn prices(&self) -> Vec<(Decimal, Decimal)> {
        let mut tiers: Vec<&Value> = self
            .tiers
            .as_array()
            .unwrap_or_else(|| panic!("tiers is an array: {}", self.tiers))
            .iter()
            .collect();
        tiers.sort_by_key(|t| t["tier"].as_i64().unwrap_or(0));
        tiers
            .iter()
            .map(|t| {
                (
                    dec(t["buy"].as_str().expect("a decimal string")),
                    dec(t["sell"].as_str().expect("a decimal string")),
                )
            })
            .collect()
    }
}

/// Every USD tier set of `branch`.
async fn rows_of(db: &TestDb, branch: &str) -> Vec<Row> {
    let rows: Vec<(Uuid, String, String, Option<Uuid>, Value)> = sqlx::query_as(
        "SELECT id, request_id, source, base_tier_set_id, tiers FROM pricing_tier_sets \
         WHERE branch_id = $1 AND currency_code = 'USD' ORDER BY approved_at, id",
    )
    .bind(branch)
    .fetch_all(&db.pool)
    .await
    .expect("read pricing_tier_sets");
    rows.into_iter()
        .map(|(id, request_id, source, base_tier_set_id, tiers)| Row {
            id,
            request_id,
            source,
            base_tier_set_id,
            tiers,
        })
        .collect()
}

type PubLineRow = (
    String,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Value,
);

/// USD's lines in `branch`'s current publication, lowest band first.
async fn usd_lines(db: &TestDb, branch: &str) -> Vec<PubLineRow> {
    sqlx::query_as(
        "SELECT l.value_tier, l.tier_lower_exclusive_thb, l.tier_upper_inclusive_thb, l.buy_rate, l.sell_rate, \
                l.source_snapshot \
         FROM published_rate_lines l JOIN branches b ON b.current_publication_id = l.publication_id \
         WHERE b.id = $1 AND l.currency_code = 'USD' AND l.denomination_group = 'ALL' \
         ORDER BY l.tier_lower_exclusive_thb NULLS FIRST, l.value_tier",
    )
    .bind(branch)
    .fetch_all(&db.pool)
    .await
    .expect("read published_rate_lines")
}

/// The branch's one pushed row: `source`, based on MAIN's set, `request_id = <MAIN's>:<branch>`, at `prices`; and the
/// branch's current publication has those 4 tiers in V.9's bands, naming the row.
async fn assert_pushed(
    db: &TestDb,
    branch: &str,
    main: &PricingTierSet,
    source: &str,
    prices: [(&str, &str); 4],
    result: &BranchResult,
) {
    let rows = rows_of(db, branch).await;
    assert_eq!(rows.len(), 1, "one tier set at {branch}: {rows:?}");
    let row = &rows[0];
    assert_eq!(row.source, source, "{branch}'s row source");
    assert_eq!(
        row.base_tier_set_id,
        Some(main.id),
        "{branch}'s row is based on MAIN's set"
    );
    assert_eq!(
        row.request_id,
        format!("{}:{branch}", main.request_id),
        "§15.4: request_id = <MAIN request>:<branch>"
    );
    assert_eq!(row.prices(), decs(prices), "{branch}'s adjusted tiers");
    assert_eq!(
        result.tier_set_id,
        Some(row.id),
        "the answer names {branch}'s row"
    );
    assert_eq!(result.error, None);

    let lines = usd_lines(db, branch).await;
    assert_eq!(
        lines.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(),
        vec!["T1", "T2", "T3", "T4"],
        "{branch} publishes 4 tier lines: {lines:#?}"
    );
    assert_eq!(
        lines.iter().map(|l| (l.1, l.2)).collect::<Vec<_>>(),
        v9_bands()
    );
    assert_eq!(
        lines
            .iter()
            .map(|l| (l.3.expect("buy"), l.4.expect("sell")))
            .collect::<Vec<_>>(),
        decs(prices),
        "{branch}'s published tiers"
    );
    for l in &lines {
        assert_eq!(
            l.5["pricing"],
            json!({ "tier_set_id": row.id.to_string(), "source": source }),
            "{branch}'s source_snapshot.pricing on {}",
            l.0
        );
    }
}

/// `(current publication id, its snapshot)` of `branch`.
async fn publication_of(db: &TestDb, branch: &str) -> (String, (String, String)) {
    let id = db
        .current_publication(branch)
        .await
        .unwrap_or_else(|| panic!("{branch} has a publication"));
    let snapshot = db.publication_snapshot(&id).await;
    (id, snapshot)
}

/// PV-7, PV-8, PV-9 (§15.4, `H:` §5, §7): a manual push answers, in branch id order, B `PUBLISHED` (tier 1
/// 33.24/33.86, a `MAIN_PUSH` row based on MAIN's), C `SKIPPED_OWN` and D `FAILED` `validationFailed`; MAIN is not in
/// the answer. B publishes the 4 adjusted tiers; C's and D's publications are untouched.
#[tokio::test]
async fn a_manual_push_publishes_b_skips_own_c_and_reports_d_failed() {
    let db = TestDb::start().await;
    let tt = push_room(&db, &[B, C, D]).await;
    save_policy(&db, B, "USD", bp(5, -5)).await;
    save_policy(
        &db,
        C,
        "USD",
        BranchPolicyPatch {
            own: Some(true),
            ..BranchPolicyPatch::default()
        },
    )
    .await;
    save_policy(&db, D, "USD", bp(300, 300)).await;
    let main = main_set(&db, TierSetSource::Manual, tt).await;
    let c_before = publication_of(&db, C).await;
    let d_before = publication_of(&db, D).await;

    let results = pusher(&db)
        .push(&owner_user(), &main, PushKind::Manual)
        .await;

    assert_eq!(
        states(&results),
        vec![
            (B, BranchPushState::Published),
            (C, BranchPushState::SkippedOwn),
            (D, BranchPushState::Failed),
        ],
        "§15.8a: each other active branch, in id order: {results:?}"
    );
    assert_pushed(&db, B, &main, "MAIN_PUSH", B_TIERS, &results[0]).await;
    assert_eq!(results[1].error, None);
    assert_eq!(
        results[2].error.as_deref(),
        Some("validationFailed"),
        "PV-9: D's error"
    );
    assert!(rows_of(&db, C).await.is_empty(), "PV-8: C is not written");
    assert!(
        rows_of(&db, D).await.is_empty(),
        "PV-9: D's failure rolls back D only"
    );
    assert_eq!(publication_of(&db, C).await, c_before, "PV-8: C unchanged");
    assert_eq!(publication_of(&db, D).await, d_before, "PV-9: D unchanged");
    db.finish().await;
}

/// §15.4 "Retrying with the same request id skips branches already written": a second push of the same base answers
/// B `ALREADY_DONE` and writes no tier set and no publication.
#[tokio::test]
async fn a_second_push_of_the_same_base_is_already_done_and_writes_nothing() {
    let db = TestDb::start().await;
    let tt = push_room(&db, &[B, C, D]).await;
    save_policy(&db, B, "USD", bp(5, -5)).await;
    save_policy(
        &db,
        C,
        "USD",
        BranchPolicyPatch {
            own: Some(true),
            ..BranchPolicyPatch::default()
        },
    )
    .await;
    save_policy(&db, D, "USD", bp(300, 300)).await;
    let main = main_set(&db, TierSetSource::Manual, tt).await;
    let push = pusher(&db);
    let first = push.push(&owner_user(), &main, PushKind::Manual).await;
    assert_eq!(first[0].state, BranchPushState::Published, "{first:?}");
    let sets = db.count("pricing_tier_sets").await;
    let publications = db.count("rate_publications").await;
    let b_before = publication_of(&db, B).await;

    let again = push.push(&owner_user(), &main, PushKind::Manual).await;

    assert_eq!(
        states(&again),
        vec![
            (B, BranchPushState::AlreadyDone),
            (C, BranchPushState::SkippedOwn),
            (D, BranchPushState::Failed),
        ],
        "the retry: {again:?}"
    );
    assert_eq!(db.count("pricing_tier_sets").await, sets, "no new tier set");
    assert_eq!(
        db.count("rate_publications").await,
        publications,
        "no new publication"
    );
    assert_eq!(publication_of(&db, B).await, b_before, "B unchanged");
    db.finish().await;
}

/// PV-14 (§15.5, `H:` §7 "a branch's default never enrols it", `H:` §10.6, §10.7): a FOLLOW push reaches only B,
/// whose USD row has `follow = true`, adjusted by its bp, as a `FOLLOW` row. D (follow only on its `'*'` row) and E
/// (no row) are not in the answer and keep their publications; C (`follow` and `own`) is never published.
#[tokio::test]
async fn a_follow_push_reaches_only_branches_whose_currency_row_follows() {
    let db = TestDb::start().await;
    let tt = push_room(&db, &[B, C, D, E]).await;
    save_policy(
        &db,
        B,
        "USD",
        BranchPolicyPatch {
            follow: Some(true),
            ..bp(5, -5)
        },
    )
    .await;
    save_policy(
        &db,
        C,
        "USD",
        BranchPolicyPatch {
            own: Some(true),
            follow: Some(true),
            ..BranchPolicyPatch::default()
        },
    )
    .await;
    save_policy(
        &db,
        D,
        "*",
        BranchPolicyPatch {
            follow: Some(true),
            ..bp(5, -5)
        },
    )
    .await;
    let main = main_set(&db, TierSetSource::Follow, tt).await;
    let c_before = publication_of(&db, C).await;
    let d_before = publication_of(&db, D).await;
    let e_before = publication_of(&db, E).await;

    let results = pusher(&db)
        .push(&owner_user(), &main, PushKind::Follow)
        .await;

    let published: Vec<_> = states(&results)
        .into_iter()
        .filter(|(b, _)| *b != C)
        .collect();
    assert_eq!(
        published,
        vec![(B, BranchPushState::Published)],
        "only the enrolled branch: {results:?}"
    );
    if let Some(c) = results.iter().find(|r| r.branch == C) {
        assert_eq!(c.state, BranchPushState::SkippedOwn, "C is own: {c:?}");
    }
    assert_pushed(&db, B, &main, "FOLLOW", B_TIERS, &results[0]).await;
    for (branch, before) in [(C, c_before), (D, d_before), (E, e_before)] {
        assert!(
            rows_of(&db, branch).await.is_empty(),
            "{branch} gets no tier set"
        );
        assert_eq!(
            publication_of(&db, branch).await,
            before,
            "{branch} unchanged"
        );
    }
    db.finish().await;
}

/// §15.4 "one step per other **active** branch": an INACTIVE branch is never pushed nor listed. B (active, no
/// policy rows: 0/0) publishes MAIN's tiers as they are.
#[tokio::test]
async fn an_inactive_branch_is_never_pushed() {
    let db = TestDb::start().await;
    let tt = push_room(&db, &[B, E]).await;
    sqlx::query("UPDATE branches SET status = 'INACTIVE' WHERE id = $1")
        .bind(E)
        .execute(&db.pool)
        .await
        .expect("deactivate E");
    let main = main_set(&db, TierSetSource::Manual, tt).await;
    let e_before = publication_of(&db, E).await;

    let results = pusher(&db)
        .push(&owner_user(), &main, PushKind::Manual)
        .await;

    assert_eq!(
        states(&results),
        vec![(B, BranchPushState::Published)],
        "E is inactive: {results:?}"
    );
    assert_pushed(&db, B, &main, "MAIN_PUSH", PV1, &results[0]).await;
    assert!(rows_of(&db, E).await.is_empty(), "E gets no tier set");
    assert_eq!(publication_of(&db, E).await, e_before, "E unchanged");
    db.finish().await;
}
