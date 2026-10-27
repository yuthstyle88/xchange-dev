//! XC-V9-PRICING PV-b (#5155): draft lines that carry four tier prices (pricing-tiers.md §15.2 `rate_draft_line_tiers`,
//! §15.3 steps 4–6, §15.1 "copy"; behaviour PV-2 and PV-4 of §15.11), through `RateDraftService` and
//! `PublishService` directly, without `pricingApprove`. A `DraftLine` with `tiers` is stored as 4
//! `rate_draft_line_tiers` rows and read back; it is in bound at submit with no BoT guard and no reference needed (B3,
//! gap 7); it publishes as 4 tier lines at exactly its prices, in V.9's fixed THB bands, with
//! `source_snapshot.pricing = {tier_set_id, source}`; `copy` keeps the tiers. Other lines keep today's path.
//! Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! `H:` is the owner's handoff (`PRICING-DESIGN.md`, kept out of the repo). PV-1's tiers are `H:` §4.4's (TT
//! 33.20/33.90, d = 2, defaults).
//!
//! Readings (§15 leaves them open; each matches `H:`):
//! - reading: "in bound" at submit is today's in-bound path (`rate_draft_service.rs` submit): one `RATE_PUBLICATION`
//!   request, decided by the policy itself (`policy:<id>`, `in_bound`), so no person approves and nothing waits in
//!   `PENDING_APPROVAL` (B3). The request row itself still exists.
//! - reading: the 4 published tier lines are coded `T1`..`T4`, the codes `PolicyTiers` publishes (the same reading as
//!   `api/tests/v9_pricing_approve_test.rs`).
//! - reading: V.9's `tierFor` bounds (`H:` §4.1) land in `tier_lower_exclusive_thb` / `tier_upper_inclusive_thb`:
//!   T1 (—, 10 000], T2 (10 000, 50 000], T3 (50 000, 200 000], T4 (200 000, —).
//! - reading: `source_snapshot.pricing.source` is the tier set's stored source (`"MANUAL"` here).
//! - reading: a copy's republish is pinned on prices and bands only. Which tier set its `source_snapshot.pricing`
//!   names is PV-e's (the copied draft is not the set's `draft_id`).
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test` (see
//! `rate_engine_common`). Every test fails without it; none skips.

mod rate_engine_common;

use chrono::{DateTime, Duration, TimeZone, Utc};
use rate_engine_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_rate::domain::draft::rate_draft::{DraftId, DraftLine, DraftStatus};
use xc_rate::domain::pricing::market_quote::MarketSource;
use xc_rate::domain::pricing::tier_set::{
    MarketAnchor, PricingTierSet, TierRate, TierSetSource, TierSetStore,
};
use xc_rate::infrastructure::pricing::tier_set_store::PgTierSetStore;

/// PV-1's tiers (`H:` §4.4), per unit size 1.
const PV1: [(&str, &str); 4] = [
    ("33.23", "33.87"),
    ("33.24", "33.86"),
    ("33.26", "33.84"),
    ("33.29", "33.80"),
];

fn dec(text: &str) -> Decimal {
    text.parse()
        .unwrap_or_else(|e| panic!("decimal {text}: {e}"))
}

fn tiers_of(prices: [(&str, &str); 4]) -> [TierRate; 4] {
    prices.map(|(b, s)| TierRate {
        buy: rate(b),
        sell: rate(s),
    })
}

fn decs(prices: [(&str, &str); 4]) -> Vec<(Decimal, Decimal)> {
    prices.iter().map(|(b, s)| (dec(b), dec(s))).collect()
}

/// A MANUAL line at tier 1 carrying the four tiers (§15.3 step 4).
fn tier_line(currency: &str, prices: [(&str, &str); 4]) -> DraftLine {
    let mut line = DraftLine::new(
        code(currency),
        manual(prices[0].0),
        manual(prices[0].1),
        Some("V.9 approved tiers".to_owned()),
    );
    line.tiers = Some(tiers_of(prices));
    line
}

/// V.9's fixed bands (`H:` §4.1 `tierFor`), as `(lower exclusive, upper inclusive)` THB.
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

fn usd_guards() -> Value {
    let g = json!({ "buy": { "min_pct": "-5.0000", "max_pct": "5.0000" },
                    "sell": { "min_pct": "-5.0000", "max_pct": "5.0000" } });
    json!({ "USD": g.clone(), "EUR": g.clone(), "GBP": g })
}

/// Policy version 1 of `branch`: adjust −0.5 / +0.4, guards ±5 %, and `value_tiers_on` with the policy's own bands
/// 20 000 / 100 000 / 500 000 THB (margins T1 0/0 … T4 +0.3/−0.3). Those bands are deliberately not V.9's, so a test
/// can tell which table a tier line used (§15.3: `rate_policy_value_tiers` stay for AUTO currencies only).
async fn policy(db: &TestDb, branch: &str, value_tiers_on: bool) -> String {
    let id: String = sqlx::query_scalar(
        "INSERT INTO rate_policy_versions \
           (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, valid_from, valid_to, \
            approved_by, reason, value_tiers_on) \
         VALUES (gen_random_uuid(), $1, 1, -0.5, 0.4, $2, now() - interval '1 day', NULL, $3, 'seeded', $4) \
         RETURNING id::text",
    )
    .bind(branch)
    .bind(usd_guards())
    .bind(OWNER_ID)
    .bind(value_tiers_on)
    .fetch_one(&db.pool)
    .await
    .expect("insert rate_policy_versions");
    for (no, upper, buy, sell) in [
        (1i16, Some("20000.00"), "0.0000", "0.0000"),
        (2, Some("100000.00"), "0.1000", "-0.1000"),
        (3, Some("500000.00"), "0.2000", "-0.2000"),
        (4, None, "0.3000", "-0.3000"),
    ] {
        sqlx::query(
            "INSERT INTO rate_policy_value_tiers \
               (policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct) \
             VALUES ($1::uuid, $2, $3::numeric, $4::numeric, $5::numeric)",
        )
        .bind(&id)
        .bind(no)
        .bind(upper)
        .bind(buy)
        .bind(sell)
        .execute(&db.pool)
        .await
        .expect("insert rate_policy_value_tiers");
    }
    id
}

/// A TT batch (`MARKET_TT`, per one unit) fetched now, and TT's `market_feed_status` row saying it was the last
/// attempt with no error. Answers `(batch id, fetched_at)`.
async fn seed_tt(db: &TestDb, lines: &[(&str, &str, &str)]) -> (Uuid, DateTime<Utc>) {
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
    for (currency, buy, sell) in lines {
        sqlx::query(
            "INSERT INTO rate_reference_lines (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw) \
             VALUES (gen_random_uuid(), $1, $2, 'MARKET_TT', 1, $3::numeric, $4::numeric, '{}'::jsonb)",
        )
        .bind(batch)
        .bind(*currency)
        .bind(*buy)
        .bind(*sell)
        .execute(&db.pool)
        .await
        .expect("insert a TT line");
    }
    sqlx::query(
        "INSERT INTO market_feed_status (source, last_attempt_at, last_error) VALUES ('TT', $1, NULL) \
         ON CONFLICT (source) DO UPDATE SET last_attempt_at = EXCLUDED.last_attempt_at, last_error = NULL",
    )
    .bind(at)
    .execute(&db.pool)
    .await
    .expect("upsert market_feed_status");
    (batch, at)
}

/// A BoT batch fetched an hour ago, unit 1.
async fn seed_bot(db: &TestDb, lines: &[(&str, &str, &str)]) {
    let rows: Vec<(&str, &str, Option<&str>, Option<&str>)> = lines
        .iter()
        .map(|(c, b, s)| (*c, "1", Some(*b), Some(*s)))
        .collect();
    db.seed_batch(Utc::now() - Duration::hours(1), &rows).await;
}

/// A MANUAL tier set of `currency` at `branch` (§15.2), anchored on the TT batch.
fn manual_set(
    branch: &str,
    currency: &str,
    prices: [(&str, &str); 4],
    tt: (Uuid, DateTime<Utc>),
) -> PricingTierSet {
    PricingTierSet {
        id: Uuid::new_v4(),
        request_id: format!("{}:{currency}", unique("approve")),
        branch: branch.to_owned(),
        currency: code(currency),
        source: TierSetSource::Manual,
        base_tier_set_id: None,
        tiers: tiers_of(prices),
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
    }
}

async fn create(db: &TestDb, branch: &str, lines: Vec<DraftLine>) -> DraftId {
    let e = db.engine();
    let mut tx = db.begin().await;
    let draft = e
        .drafts
        .create(&mut tx, &BranchId::new(branch), &owner_user(), lines)
        .await
        .expect("create the draft");
    tx.commit().await.expect("commit the draft");
    draft.id
}

/// Writes `set` and points it at `draft` (§15.3 steps 3–4: the row first, then the draft that publishes it).
async fn attach(db: &TestDb, set: &PricingTierSet, draft: &DraftId) {
    let store = PgTierSetStore;
    let mut tx = db.begin().await;
    assert!(
        store
            .insert_in(&mut tx, set)
            .await
            .expect("insert the tier set"),
        "a new request_id is written"
    );
    store
        .set_draft_in(&mut tx, set.id, draft.as_uuid())
        .await
        .expect("record the publishing draft");
    tx.commit().await.expect("commit the tier set");
}

async fn submit_as(
    db: &TestDb,
    branch: &str,
    who: &xc_auth::domain::access::Actor,
    draft: &DraftId,
) -> DraftStatus {
    let e = db.engine();
    let mut tx = db.begin().await;
    let out = e
        .drafts
        .submit(&mut tx, &BranchId::new(branch), who, draft, &key())
        .await
        .expect("submit the draft");
    tx.commit().await.expect("commit the submit");
    out.status
}

async fn publish(db: &TestDb, branch: &str, draft: &DraftId, expected: Option<i32>) -> Uuid {
    let e = db.engine();
    let mut tx = db.begin().await;
    let out = e
        .publisher
        .publish(
            &mut tx,
            &BranchId::new(branch),
            &owner_user(),
            draft,
            expected,
        )
        .await
        .expect("publish the draft");
    tx.commit().await.expect("commit the publish");
    out.id.as_uuid()
}

/// One published line of a publication.
#[derive(Debug, Clone, PartialEq)]
struct PubLine {
    value_tier: String,
    lower: Option<Decimal>,
    upper: Option<Decimal>,
    buy: Option<Decimal>,
    sell: Option<Decimal>,
    base_buy: Option<Decimal>,
    base_sell: Option<Decimal>,
    snapshot: Value,
}

type PubLineRow = (
    String,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Value,
);

/// `currency`'s lines of one publication, lowest band first.
async fn lines_of(db: &TestDb, publication: Uuid, currency: &str) -> Vec<PubLine> {
    let rows: Vec<PubLineRow> = sqlx::query_as(
        "SELECT value_tier, tier_lower_exclusive_thb, tier_upper_inclusive_thb, buy_rate, sell_rate, \
                base_buy_rate, base_sell_rate, source_snapshot \
         FROM published_rate_lines \
         WHERE publication_id = $1 AND currency_code = $2 AND denomination_group = 'ALL' \
         ORDER BY tier_lower_exclusive_thb NULLS FIRST, value_tier",
    )
    .bind(publication)
    .bind(currency)
    .fetch_all(&db.pool)
    .await
    .expect("read published_rate_lines");
    rows.into_iter()
        .map(
            |(value_tier, lower, upper, buy, sell, base_buy, base_sell, snapshot)| PubLine {
                value_tier,
                lower,
                upper,
                buy,
                sell,
                base_buy,
                base_sell,
                snapshot,
            },
        )
        .collect()
}

/// `(currency, tier_no, buy, sell)` of every `rate_draft_line_tiers` row of a draft.
async fn tier_rows(db: &TestDb, draft: &DraftId) -> Vec<(String, i16, Decimal, Decimal)> {
    let rows: Vec<(String, i16, Decimal, Decimal)> = sqlx::query_as(
        "SELECT l.currency_code::text, t.tier_no, t.buy, t.sell \
         FROM rate_draft_line_tiers t JOIN rate_draft_lines l ON l.id = t.draft_line_id \
         WHERE l.draft_id = $1 ORDER BY l.currency_code, t.tier_no",
    )
    .bind(draft.as_uuid())
    .fetch_all(&db.pool)
    .await
    .expect("read rate_draft_line_tiers");
    rows.into_iter()
        .map(|(c, n, b, s)| (c.trim().to_owned(), n, b, s))
        .collect()
}

fn expected_rows(
    currency: &str,
    prices: [(&str, &str); 4],
) -> Vec<(String, i16, Decimal, Decimal)> {
    prices
        .iter()
        .enumerate()
        .map(|(i, (b, s))| (currency.to_owned(), i as i16 + 1, dec(b), dec(s)))
        .collect()
}

/// Asserts `lines` are 4 tier lines at exactly `prices`, in V.9's bands, naming `set_id` / `source`.
fn assert_tier_lines(
    lines: &[PubLine],
    prices: [(&str, &str); 4],
    set_id: Option<Uuid>,
    source: &str,
) {
    assert_eq!(
        lines
            .iter()
            .map(|l| l.value_tier.as_str())
            .collect::<Vec<_>>(),
        vec!["T1", "T2", "T3", "T4"],
        "§15.3 step 6: exactly 4 tier lines, no ALL line: {lines:#?}"
    );
    assert_eq!(
        lines.iter().map(|l| (l.lower, l.upper)).collect::<Vec<_>>(),
        v9_bands(),
        "§15.3 step 6: V.9's fixed bands (H: §4.1), not the policy's"
    );
    assert_eq!(
        lines
            .iter()
            .map(|l| (
                l.buy.expect("a published buy"),
                l.sell.expect("a published sell")
            ))
            .collect::<Vec<_>>(),
        decs(prices),
        "§15.3 step 6: the prices are the tier rows, exactly"
    );
    for l in lines {
        assert_eq!(
            l.snapshot["pricing"]["source"],
            json!(source),
            "§15.3 step 6: source_snapshot.pricing.source on {}: {}",
            l.value_tier,
            l.snapshot
        );
        if let Some(id) = set_id {
            assert_eq!(
                l.snapshot["pricing"]["tier_set_id"],
                json!(id.to_string()),
                "§15.3 step 6: source_snapshot.pricing.tier_set_id on {}: {}",
                l.value_tier,
                l.snapshot
            );
        }
    }
}

// ------------------------------------------------------------ storage

/// PV-2 (§15.2, §15.3 step 4): a line with `tiers` is stored as 4 `rate_draft_line_tiers` rows, and `get` reads the
/// line back with its tiers and its MANUAL tier-1 sides.
#[tokio::test]
async fn a_line_with_tiers_is_stored_as_four_tier_rows_and_read_back() {
    let db = TestDb::start().await;
    policy(&db, MAIN, false).await;
    let draft = create(&db, MAIN, vec![tier_line("USD", PV1)]).await;

    assert_eq!(
        tier_rows(&db, &draft).await,
        expected_rows("USD", PV1),
        "§15.3 step 4: 4 rate_draft_line_tiers rows"
    );

    let got = db
        .engine()
        .drafts
        .get(&main_branch(), &draft)
        .await
        .expect("get the draft")
        .expect("the draft exists");
    assert_eq!(got.lines.len(), 1);
    let usd = &got.lines[0];
    assert_eq!(usd.buy, manual("33.23"), "tier 1 is the MANUAL buy");
    assert_eq!(usd.sell, manual("33.87"), "tier 1 is the MANUAL sell");
    assert_eq!(usd.tiers, Some(tiers_of(PV1)), "get returns the tiers");
    db.finish().await;
}

/// PV-b (§15.2): a line without tiers never gets a `rate_draft_line_tiers` row, beside one that has them; and an
/// edit that drops the tiers drops the rows.
#[tokio::test]
async fn no_tier_row_is_written_for_a_line_without_tiers() {
    let db = TestDb::start().await;
    policy(&db, MAIN, false).await;
    let draft = create(
        &db,
        MAIN,
        vec![
            tier_line("USD", PV1),
            auto_line("EUR"),
            manual_line("GBP", "42.00", "43.00"),
        ],
    )
    .await;
    assert_eq!(
        tier_rows(&db, &draft).await,
        expected_rows("USD", PV1),
        "only USD's line has tier rows"
    );

    let e = db.engine();
    let mut tx = db.begin().await;
    let edited = e
        .drafts
        .edit(
            &mut tx,
            &main_branch(),
            &owner_user(),
            &draft,
            1,
            vec![manual_line("USD", "33.23", "33.87"), auto_line("EUR")],
        )
        .await
        .expect("edit the draft");
    tx.commit().await.expect("commit the edit");
    assert_eq!(edited.version, 2);
    assert!(
        tier_rows(&db, &draft).await.is_empty(),
        "the edited lines have no tiers ⇒ no tier rows"
    );
    let got = e
        .drafts
        .get(&main_branch(), &draft)
        .await
        .expect("get")
        .expect("exists");
    assert!(
        got.lines.iter().all(|l| l.tiers.is_none()),
        "no line reads back with tiers: {got:?}"
    );
    db.finish().await;
}

// ------------------------------------------------------------ submit (B3, gap 7)

/// Asserts the draft was approved by the policy itself: APPROVED, one request, decided `policy:<id>` `in_bound`.
async fn assert_in_bound(db: &TestDb, draft: &DraftId, status: DraftStatus, policy_id: &str) {
    assert_eq!(
        status,
        DraftStatus::Approved,
        "B3: a line with tier rows is in bound; no EXECUTIVE step"
    );
    assert_eq!(db.draft_status(&draft.to_string()).await, "APPROVED");
    let requests = db.requests_of(&draft.to_string()).await;
    assert_eq!(requests.len(), 1, "one request: {requests:?}");
    assert_eq!(requests[0].1, "APPROVED", "nothing waits: {requests:?}");
    assert_eq!(
        db.decisions_of(&requests[0].0).await,
        vec![(
            format!("policy:{policy_id}"),
            "APPROVED".to_owned(),
            "in_bound".to_owned()
        )],
        "B3: the policy decides, no person"
    );
}

/// PV-2 (§15.3 step 5, B3): a line with tier rows is in bound with no BoT guard, even when BoT (31.00/31.50) is far
/// from tier 1 (33.23, +7.2 % beyond the ±5 % guard). A RATE_MANAGER's submit is approved by the policy.
#[tokio::test]
async fn a_line_with_tiers_is_in_bound_far_from_the_bot_reference() {
    let db = TestDb::start().await;
    let policy_id = policy(&db, MAIN, false).await;
    seed_bot(&db, &[("USD", "31.0000", "31.5000")]).await;
    seed_tt(&db, &[("USD", "33.20", "33.90")]).await;
    let draft = create(&db, MAIN, vec![tier_line("USD", PV1)]).await;

    let e = db.engine();
    let mut tx = db.begin().await;
    let evaluated = e
        .drafts
        .evaluate(&mut tx, &main_branch(), &draft)
        .await
        .expect("evaluate");
    tx.rollback().await.expect("roll back");
    assert!(
        evaluated.evaluation.is_in_bound(),
        "§15.3 step 5: in bound, no BoT guard: {:?}",
        evaluated.evaluation.out_of_bound()
    );

    let status = submit_as(&db, MAIN, &rates_user(), &draft).await;
    assert_in_bound(&db, &draft, status, &policy_id).await;
    db.finish().await;
}

/// PV-4 (§15.3 step 5, gap 7): a line with tier rows for a currency with no BoT line and no starting price (TT
/// present) is in bound and approved by the policy.
#[tokio::test]
async fn a_line_with_tiers_is_in_bound_with_no_bot_or_start_reference() {
    let db = TestDb::start().await;
    let policy_id = policy(&db, MAIN, false).await;
    seed_tt(&db, &[("USD", "33.20", "33.90")]).await;
    let draft = create(&db, MAIN, vec![tier_line("USD", PV1)]).await;

    let status = submit_as(&db, MAIN, &rates_user(), &draft).await;
    assert_in_bound(&db, &draft, status, &policy_id).await;
    db.finish().await;
}

// ------------------------------------------------------------ publish

async fn publishes_four_v9_tier_lines(value_tiers_on: bool) {
    let db = TestDb::start().await;
    policy(&db, MAIN, value_tiers_on).await;
    seed_bot(&db, &[("USD", "32.4500", "32.9000")]).await;
    let tt = seed_tt(&db, &[("USD", "33.20", "33.90")]).await;
    let draft = create(&db, MAIN, vec![tier_line("USD", PV1)]).await;
    let set = manual_set(MAIN, "USD", PV1, tt);
    attach(&db, &set, &draft).await;

    let status = submit_as(&db, MAIN, &rates_user(), &draft).await;
    assert_eq!(status, DraftStatus::Approved, "B3");
    let publication = publish(&db, MAIN, &draft, None).await;

    let lines = lines_of(&db, publication, "USD").await;
    assert_tier_lines(&lines, PV1, Some(set.id), "MANUAL");
    assert_eq!(
        db.count("published_rate_lines").await,
        4,
        "the publication has USD's 4 tier lines and nothing else"
    );
    db.finish().await;
}

/// PV-2 (§15.3 step 6): a line with tier rows publishes exactly 4 tier lines at 33.23/33.87 … 33.29/33.80, in V.9's
/// bands (not the policy's 20 000 / 100 000 / 500 000), with `source_snapshot.pricing = {tier_set_id, "MANUAL"}`
/// from the tier set whose `draft_id` is the draft. The branch has `value_tiers_on`.
#[tokio::test]
async fn publish_writes_four_v9_tier_lines_with_value_tiers_on() {
    publishes_four_v9_tier_lines(true).await;
}

/// PV-2 (§15.3 step 6, "gap 1 needs no switch"): the same with `value_tiers_on` off: still 4 tier lines.
#[tokio::test]
async fn publish_writes_four_v9_tier_lines_with_value_tiers_off() {
    publishes_four_v9_tier_lines(false).await;
}

/// PV-b (§15.3 step 5 "Other lines in the same draft keep today's evaluation"): an AUTO EUR line beside USD's tier
/// line publishes exactly what a draft of EUR alone publishes (its policy tiers, its rates, no `pricing`).
/// Keep-behaviour guard: its EUR assertions hold today; the test fails today only where it writes the tier set.
#[tokio::test]
async fn an_auto_line_beside_a_tier_line_publishes_as_today() {
    let db = TestDb::start().await;
    policy(&db, MAIN, true).await;
    seed_bot(
        &db,
        &[("USD", "32.4500", "32.9000"), ("EUR", "38.0000", "38.6000")],
    )
    .await;
    let tt = seed_tt(&db, &[("USD", "33.20", "33.90")]).await;

    // Today's path: EUR alone.
    let control = create(&db, MAIN, vec![auto_line("EUR")]).await;
    assert_eq!(
        submit_as(&db, MAIN, &rates_user(), &control).await,
        DraftStatus::Approved
    );
    let first = publish(&db, MAIN, &control, None).await;
    let eur_alone = lines_of(&db, first, "EUR").await;
    assert_eq!(
        eur_alone.len(),
        4,
        "value tiers on: EUR has the policy's 4 tiers: {eur_alone:#?}"
    );

    let draft = create(&db, MAIN, vec![tier_line("USD", PV1), auto_line("EUR")]).await;
    let set = manual_set(MAIN, "USD", PV1, tt);
    attach(&db, &set, &draft).await;
    assert_eq!(
        submit_as(&db, MAIN, &rates_user(), &draft).await,
        DraftStatus::Approved
    );
    let second = publish(&db, MAIN, &draft, Some(1)).await;

    assert_eq!(
        lines_of(&db, second, "EUR").await,
        eur_alone,
        "the AUTO line publishes as today"
    );
    for l in &eur_alone {
        assert_eq!(
            l.snapshot["pricing"],
            Value::Null,
            "an AUTO line has no pricing: {}",
            l.snapshot
        );
    }
    assert_tier_lines(
        &lines_of(&db, second, "USD").await,
        PV1,
        Some(set.id),
        "MANUAL",
    );
    db.finish().await;
}

// ------------------------------------------------------------ copy

/// PV-2 (§15.1, §15.3 step 4): `copy` of a draft that published a tier line keeps the tiers on the copied line, and
/// publishing the copy publishes the same 4 tier lines.
#[tokio::test]
async fn copy_keeps_the_tiers_and_the_copy_republishes_them() {
    let db = TestDb::start().await;
    policy(&db, MAIN, true).await;
    seed_bot(&db, &[("USD", "32.4500", "32.9000")]).await;
    let tt = seed_tt(&db, &[("USD", "33.20", "33.90")]).await;
    let draft = create(&db, MAIN, vec![tier_line("USD", PV1)]).await;
    let set = manual_set(MAIN, "USD", PV1, tt);
    attach(&db, &set, &draft).await;
    assert_eq!(
        submit_as(&db, MAIN, &rates_user(), &draft).await,
        DraftStatus::Approved
    );
    publish(&db, MAIN, &draft, None).await;

    let e = db.engine();
    let mut tx = db.begin().await;
    let copied = e
        .drafts
        .copy(&mut tx, &main_branch(), &owner_user(), &draft)
        .await
        .expect("copy the published draft");
    tx.commit().await.expect("commit the copy");

    assert_eq!(
        tier_rows(&db, &copied.id).await,
        expected_rows("USD", PV1),
        "the copied line has the 4 tier rows"
    );
    let got = e
        .drafts
        .get(&main_branch(), &copied.id)
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(
        got.lines.iter().map(|l| l.tiers).collect::<Vec<_>>(),
        vec![Some(tiers_of(PV1))],
        "copy keeps the tiers"
    );

    assert_eq!(
        submit_as(&db, MAIN, &rates_user(), &copied.id).await,
        DraftStatus::Approved
    );
    let again = publish(&db, MAIN, &copied.id, Some(1)).await;
    let lines = lines_of(&db, again, "USD").await;
    assert_eq!(
        lines
            .iter()
            .map(|l| l.value_tier.as_str())
            .collect::<Vec<_>>(),
        vec!["T1", "T2", "T3", "T4"],
        "the copy republishes 4 tier lines: {lines:#?}"
    );
    assert_eq!(
        lines.iter().map(|l| (l.lower, l.upper)).collect::<Vec<_>>(),
        v9_bands()
    );
    assert_eq!(
        lines
            .iter()
            .map(|l| (l.buy.expect("buy"), l.sell.expect("sell")))
            .collect::<Vec<_>>(),
        decs(PV1),
        "the same 4 prices"
    );
    db.finish().await;
}
