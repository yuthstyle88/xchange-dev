//! Ticket #1516 (VT-3) — value tiers in the draft → evaluate → publish path.
//! Tests written before implementation by the test-writer (gemini/warut-6, ordered by claude/ad05bf8d).
//!
//! Design: `docs/design/blueprint/rates.md` §12.3 (tiers and formula), §12.4 (draft path builds tiers),
//! §12.10 (ticket VT-3 row), and issue #1516 §9 edge cases.
//!
//! §9 Edge Cases covered:
//! 1. ON publishes four lines per currency, with value_tier T1…T4, the version's bands
//!    (T1 lower NULL, T4 upper NULL) and one shared base_buy_rate / base_sell_rate.
//! 2. OFF publishes one 'ALL' line per currency, identical to today's: rates, value_tier = 'ALL',
//!    NULL bands and bases. A version with tier rows but value_tiers_on = false is also OFF.
//! 3. A tier whose adjust + margin lies outside the guard makes the draft out of bounds
//!    (OWNER approval, design §4.5), even though the untiered rate is inside.
//! 4. The evaluation hash moves when one tier margin changes (a new policy version differing
//!    only in that margin), and an OFF draft's hash is unchanged from today's.
//! 5. line_for picks T2 for design §12.3's worked example. USD at BoT 32.4500/32.9000,
//!    buy_adjust −0.5000, sell_adjust +0.4000, and the example's bands and margins: a customer
//!    selling 1,000 USD gets the T2 line, and the quote is 32,320.20 THB. With OFF, the same
//!    customer gets 32,287.75 THB.
//! 6. Each ON line's source_snapshot.tier carries no and both margins (design §12.4's [required] snapshot).

mod rate_engine_common;

use chrono::{DateTime, Utc};
use rate_engine_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::Row;
use xc_auth::domain::access::{self, Actor, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{ExchangeArithmetic, ForeignAmount, Money, Rate, Side};
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::domain::draft::rate_draft::{DraftLine, DraftStatus, LineSide};
use xc_rate::domain::publication::published_rates::{LineSelector, PublishedRates};

// ── Actor helper rule (§7) ──────────────────────────────────────────────────

/// Builds every actor in this test file through this one helper function (§7).
fn test_actor(user: &str, role: &str) -> Actor {
    match role {
        "OWNER" => Actor::new(
            user,
            ORG,
            vec![
                ScopedRole::new(
                    access::Role::Executive,
                    None,
                    DateTime::<Utc>::UNIX_EPOCH,
                    None,
                ),
                ScopedRole::new(access::Role::Admin, None, DateTime::<Utc>::UNIX_EPOCH, None),
            ],
            None,
        ),
        "RATES" => Actor::new(
            user,
            ORG,
            vec![ScopedRole::new(
                access::Role::RateManager,
                Some(BranchId::new(MAIN)),
                DateTime::<Utc>::UNIX_EPOCH,
                None,
            )],
            None,
        ),
        _ => panic!("unsupported role {role} in test_actor"),
    }
}

// ── Test helpers ─────────────────────────────────────────────────────────────

fn default_guard() -> Value {
    json!({
        "USD": {
            "buy": { "min_pct": "-5.0000", "max_pct": "5.0000" },
            "sell": { "min_pct": "-5.0000", "max_pct": "5.0000" }
        }
    })
}

/// Seeds a policy version with the `value_tiers_on` flag and its 4 `rate_policy_value_tiers` rows directly (§9).
async fn seed_policy_with_tiers(
    db: &TestDb,
    branch: &str,
    buy_adjust_pct: &str,
    sell_adjust_pct: &str,
    guards: Value,
    value_tiers_on: bool,
    tiers: &[(u8, Option<&str>, &str, &str)],
) -> String {
    let policy_id: String = sqlx::query_scalar(
        "INSERT INTO rate_policy_versions \
           (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
            valid_from, valid_to, approved_by, reason, value_tiers_on) \
         VALUES (gen_random_uuid(), $1, (SELECT COALESCE(MAX(version), 0) + 1 FROM rate_policy_versions WHERE branch_id = $1), \
                 $2::numeric, $3::numeric, $4, \
                 now() - interval '1 day', NULL, $5, 'seeded by test', $6) \
         RETURNING id::text",
    )
    .bind(branch)
    .bind(buy_adjust_pct)
    .bind(sell_adjust_pct)
    .bind(guards)
    .bind(OWNER_ID)
    .bind(value_tiers_on)
    .fetch_one(&db.pool)
    .await
    .expect("insert rate_policy_versions with value_tiers_on");

    for (tier_no, upper, buy_margin, sell_margin) in tiers {
        sqlx::query(
            "INSERT INTO rate_policy_value_tiers \
               (policy_version_id, tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct) \
             VALUES ($1::uuid, $2, $3::numeric, $4::numeric, $5::numeric)",
        )
        .bind(&policy_id)
        .bind(*tier_no as i16)
        .bind(*upper)
        .bind(*buy_margin)
        .bind(*sell_margin)
        .execute(&db.pool)
        .await
        .expect("insert rate_policy_value_tiers");
    }

    policy_id
}

/// Standard four tiers for §12.3 worked example.
fn worked_example_tiers() -> Vec<(u8, Option<&'static str>, &'static str, &'static str)> {
    vec![
        (1, Some("20000.00"), "0.0000", "0.0000"),
        (2, Some("100000.00"), "0.1000", "-0.1000"),
        (3, Some("500000.00"), "0.2000", "-0.2000"),
        (4, None, "0.3000", "-0.3000"),
    ]
}

// ── §9 Cases ────────────────────────────────────────────────────────────────

/// Case 1: ON publishes four lines per currency, with value_tier T1…T4,
/// the version's bands (T1 lower NULL, T4 upper NULL) and one shared base_buy_rate / base_sell_rate.
#[tokio::test]
async fn on_publishes_four_lines_per_currency_with_bands_and_shared_base_rates() {
    let db = TestDb::start().await;
    let e = db.engine();

    // BoT reference: buy 32.4500, sell 32.9000
    db.seed_batch(
        Utc::now() - chrono::Duration::hours(1),
        &[("USD", "1", Some("32.4500"), Some("32.9000"))],
    )
    .await;

    // Policy with value_tiers_on = true and worked example tiers
    seed_policy_with_tiers(
        &db,
        MAIN,
        "-0.5000",
        "0.4000",
        default_guard(),
        true,
        &worked_example_tiers(),
    )
    .await;

    let owner = test_actor(OWNER_ID, "OWNER");
    let lines = vec![DraftLine::new(
        code("USD"),
        LineSide::Auto,
        LineSide::Auto,
        None,
    )];

    let mut tx = db.begin().await;
    let draft = e
        .drafts
        .create(&mut tx, &main_branch(), &owner, lines)
        .await
        .expect("create draft");
    tx.commit().await.expect("commit draft creation");

    let mut tx = db.begin().await;
    let outcome = e
        .drafts
        .submit(&mut tx, &main_branch(), &owner, &draft.id, &key())
        .await
        .expect("submit draft");
    tx.commit().await.expect("commit submit");
    assert_eq!(outcome.status, DraftStatus::Approved);

    let mut tx = db.begin().await;
    let pub_ref = e
        .publisher
        .publish(&mut tx, &main_branch(), &owner, &draft.id, None)
        .await
        .expect("publish draft");
    tx.commit().await.expect("commit publish");

    // Query published_rate_lines for USD
    let rows = sqlx::query(
        "SELECT value_tier, tier_lower_exclusive_thb, tier_upper_inclusive_thb, \
                buy_rate, sell_rate, base_buy_rate, base_sell_rate \
         FROM published_rate_lines \
         WHERE publication_id = $1::uuid AND currency_code = 'USD' \
         ORDER BY value_tier ASC",
    )
    .bind(pub_ref.id.to_string())
    .fetch_all(&db.pool)
    .await
    .expect("fetch published lines");

    assert_eq!(
        rows.len(),
        4,
        "ON must publish exactly 4 tier lines per currency, got {}",
        rows.len()
    );

    let expected_bands: [(
        &str,
        Option<Decimal>,
        Option<Decimal>,
        &str,
        &str,
        &str,
        &str,
    ); 4] = [
        (
            "T1",
            None,
            Some(Decimal::from(20000)),
            "32.28775000",
            "33.03160000",
            "32.28775000",
            "33.03160000",
        ),
        (
            "T2",
            Some(Decimal::from(20000)),
            Some(Decimal::from(100000)),
            "32.32020000",
            "32.99870000",
            "32.28775000",
            "33.03160000",
        ),
        (
            "T3",
            Some(Decimal::from(100000)),
            Some(Decimal::from(500000)),
            "32.35265000",
            "32.96580000",
            "32.28775000",
            "33.03160000",
        ),
        (
            "T4",
            Some(Decimal::from(500000)),
            None,
            "32.38510000",
            "32.93290000",
            "32.28775000",
            "33.03160000",
        ),
    ];

    for (row, expected) in rows.iter().zip(expected_bands.iter()) {
        let vt: String = row.get("value_tier");
        let lower: Option<Decimal> = row.get("tier_lower_exclusive_thb");
        let upper: Option<Decimal> = row.get("tier_upper_inclusive_thb");
        let buy: Decimal = row.get("buy_rate");
        let sell: Decimal = row.get("sell_rate");
        let base_buy: Option<Decimal> = row.get("base_buy_rate");
        let base_sell: Option<Decimal> = row.get("base_sell_rate");

        assert_eq!(vt, expected.0, "value_tier mismatch");
        assert_eq!(
            lower, expected.1,
            "tier_lower_exclusive_thb mismatch for {}",
            vt
        );
        assert_eq!(
            upper, expected.2,
            "tier_upper_inclusive_thb mismatch for {}",
            vt
        );
        assert_eq!(buy.to_string(), expected.3, "buy_rate mismatch for {}", vt);
        assert_eq!(
            sell.to_string(),
            expected.4,
            "sell_rate mismatch for {}",
            vt
        );
        assert_eq!(
            base_buy.map(|d| d.to_string()),
            Some(expected.5.to_string()),
            "base_buy_rate mismatch for {}",
            vt
        );
        assert_eq!(
            base_sell.map(|d| d.to_string()),
            Some(expected.6.to_string()),
            "base_sell_rate mismatch for {}",
            vt
        );
    }
}

/// Case 2: OFF publishes one 'ALL' line per currency, identical to today's: rates,
/// value_tier = 'ALL', NULL bands and bases. A version with tier rows but value_tiers_on = false is also OFF.
#[tokio::test]
async fn off_publishes_one_all_line_even_with_tier_rows_present() {
    let db = TestDb::start().await;
    let e = db.engine();

    db.seed_batch(
        Utc::now() - chrono::Duration::hours(1),
        &[("USD", "1", Some("32.4500"), Some("32.9000"))],
    )
    .await;

    // Seed policy with value_tiers_on = false, but tier rows ARE present in rate_policy_value_tiers
    seed_policy_with_tiers(
        &db,
        MAIN,
        "-0.5000",
        "0.4000",
        default_guard(),
        false,
        &worked_example_tiers(),
    )
    .await;

    let owner = test_actor(OWNER_ID, "OWNER");
    let lines = vec![DraftLine::new(
        code("USD"),
        LineSide::Auto,
        LineSide::Auto,
        None,
    )];

    let mut tx = db.begin().await;
    let draft = e
        .drafts
        .create(&mut tx, &main_branch(), &owner, lines)
        .await
        .expect("create draft");
    tx.commit().await.expect("commit draft creation");

    let mut tx = db.begin().await;
    let outcome = e
        .drafts
        .submit(&mut tx, &main_branch(), &owner, &draft.id, &key())
        .await
        .expect("submit draft");
    tx.commit().await.expect("commit submit");
    assert_eq!(outcome.status, DraftStatus::Approved);

    let mut tx = db.begin().await;
    let pub_ref = e
        .publisher
        .publish(&mut tx, &main_branch(), &owner, &draft.id, None)
        .await
        .expect("publish draft");
    tx.commit().await.expect("commit publish");

    let rows = sqlx::query(
        "SELECT value_tier, tier_lower_exclusive_thb, tier_upper_inclusive_thb, \
                buy_rate, sell_rate, base_buy_rate, base_sell_rate \
         FROM published_rate_lines \
         WHERE publication_id = $1::uuid AND currency_code = 'USD'",
    )
    .bind(pub_ref.id.to_string())
    .fetch_all(&db.pool)
    .await
    .expect("fetch published lines");

    assert_eq!(
        rows.len(),
        1,
        "OFF must publish exactly 1 'ALL' line per currency, got {}",
        rows.len()
    );

    let row = &rows[0];
    let vt: String = row.get("value_tier");
    let lower: Option<Decimal> = row.get("tier_lower_exclusive_thb");
    let upper: Option<Decimal> = row.get("tier_upper_inclusive_thb");
    let buy: Decimal = row.get("buy_rate");
    let sell: Decimal = row.get("sell_rate");
    let base_buy: Option<Decimal> = row.get("base_buy_rate");
    let base_sell: Option<Decimal> = row.get("base_sell_rate");

    assert_eq!(vt, "ALL");
    assert!(lower.is_none(), "OFF tier_lower_exclusive_thb must be NULL");
    assert!(upper.is_none(), "OFF tier_upper_inclusive_thb must be NULL");
    assert!(base_buy.is_none(), "OFF base_buy_rate must be NULL");
    assert!(base_sell.is_none(), "OFF base_sell_rate must be NULL");
    assert_eq!(buy.to_string(), "32.28775000");
    assert_eq!(sell.to_string(), "33.03160000");
}

/// Case 3: A tier whose adjust + margin lies outside the guard makes the draft out of bounds
/// (OWNER approval, design §4.5), even though the untiered rate is inside.
#[tokio::test]
async fn tier_outside_guard_makes_draft_out_of_bounds_requiring_owner_approval() {
    let db = TestDb::start().await;
    let e = db.engine();

    // Reference: buy 32.0, sell 33.0
    db.seed_batch(
        Utc::now() - chrono::Duration::hours(1),
        &[("USD", "1", Some("32.0000"), Some("33.0000"))],
    )
    .await;

    // Guard on USD: [-1.0000, 1.0000]
    let guards = json!({
        "USD": {
            "buy": { "min_pct": "-1.0000", "max_pct": "1.0000" },
            "sell": { "min_pct": "-1.0000", "max_pct": "1.0000" }
        }
    });

    // Policy adjust is 0.5000 (inside guard [-1.0, 1.0]).
    // T4 has buy_margin_pct = +0.6000, so effective adjust is 0.5 + 0.6 = +1.1000 (> 1.0000, outside guard!).
    let tiers_with_out_of_bound = vec![
        (1, Some("20000.00"), "0.0000", "0.0000"),
        (2, Some("100000.00"), "0.2000", "-0.2000"),
        (3, Some("500000.00"), "0.4000", "-0.4000"),
        (4, None, "0.6000", "-0.4000"), // 0.5000 + 0.6000 = 1.1000 > 1.0000!
    ];

    seed_policy_with_tiers(
        &db,
        MAIN,
        "0.5000",
        "0.5000",
        guards,
        true,
        &tiers_with_out_of_bound,
    )
    .await;

    // RATES staff submits the draft.
    // If evaluated untiered, adjust 0.5000 is inside [-1.0, 1.0], which would auto-approve.
    // But because T4 is outside the guard, the draft is out-of-bound and MUST require OWNER approval!
    let rates = test_actor(RATES_ID, "RATES");
    let lines = vec![DraftLine::new(
        code("USD"),
        LineSide::Auto,
        LineSide::Auto,
        None,
    )];

    let mut tx = db.begin().await;
    let draft = e
        .drafts
        .create(&mut tx, &main_branch(), &rates, lines)
        .await
        .expect("create draft");
    tx.commit().await.expect("commit draft creation");

    let mut tx = db.begin().await;
    let outcome = e
        .drafts
        .submit(&mut tx, &main_branch(), &rates, &draft.id, &key())
        .await
        .expect("submit draft");
    tx.commit().await.expect("commit submit");

    assert_eq!(
        outcome.status,
        DraftStatus::PendingApproval,
        "a tier whose adjust+margin lies outside the guard must make the draft out-of-bounds"
    );

    let requests = db.requests_of(&draft.id.to_string()).await;
    assert_eq!(
        requests.len(),
        1,
        "exactly one approval request must be opened"
    );
    assert_eq!(requests[0].1, "PENDING", "approval request must be pending");
}

/// Case 3 (Sell side): A tier whose SELL adjust + margin lies outside the guard
/// (while every buy margin stays inside) makes the draft out of bounds, requiring OWNER approval.
#[tokio::test]
async fn tier_sell_margin_outside_guard_makes_draft_out_of_bounds_requiring_owner_approval() {
    let db = TestDb::start().await;
    let e = db.engine();

    // Reference: buy 32.0, sell 33.0
    db.seed_batch(
        Utc::now() - chrono::Duration::hours(1),
        &[("USD", "1", Some("32.0000"), Some("33.0000"))],
    )
    .await;

    // Guard on USD: [-1.0000, 1.0000]
    let guards = json!({
        "USD": {
            "buy": { "min_pct": "-1.0000", "max_pct": "1.0000" },
            "sell": { "min_pct": "-1.0000", "max_pct": "1.0000" }
        }
    });

    // Policy adjust is buy: 0.5000, sell: 0.5000 (both inside guard [-1.0, 1.0]).
    // Every buy margin stays inside:
    //   T1 buy: 0.5 + 0.0 = 0.5000
    //   T2 buy: 0.5 - 0.2 = 0.3000
    //   T3 buy: 0.5 - 0.4 = 0.1000
    //   T4 buy: 0.5 - 0.4 = 0.1000  (all inside [-1.0, 1.0])
    // But T4 has sell_margin_pct = +0.6000, so effective sell adjust is:
    //   0.5000 + 0.6000 = +1.1000 (> 1.0000, outside guard!).
    let tiers_with_out_of_bound_sell = vec![
        (1, Some("20000.00"), "0.0000", "0.0000"),
        (2, Some("100000.00"), "-0.2000", "0.2000"),
        (3, Some("500000.00"), "-0.4000", "0.4000"),
        (4, None, "-0.4000", "0.6000"), // sell: 0.5000 + 0.6000 = 1.1000 > 1.0000!
    ];

    seed_policy_with_tiers(
        &db,
        MAIN,
        "0.5000",
        "0.5000",
        guards,
        true,
        &tiers_with_out_of_bound_sell,
    )
    .await;

    // RATES staff submits the draft.
    // If evaluated untiered, sell adjust 0.5000 is inside [-1.0, 1.0].
    // Every tier buy margin is also inside the guard.
    // But because T4's sell margin is outside the guard, the draft MUST require OWNER approval!
    let rates = test_actor(RATES_ID, "RATES");
    let lines = vec![DraftLine::new(
        code("USD"),
        LineSide::Auto,
        LineSide::Auto,
        None,
    )];

    let mut tx = db.begin().await;
    let draft = e
        .drafts
        .create(&mut tx, &main_branch(), &rates, lines)
        .await
        .expect("create draft");
    tx.commit().await.expect("commit draft creation");

    let mut tx = db.begin().await;
    let outcome = e
        .drafts
        .submit(&mut tx, &main_branch(), &rates, &draft.id, &key())
        .await
        .expect("submit draft");
    tx.commit().await.expect("commit submit");

    assert_eq!(
        outcome.status,
        DraftStatus::PendingApproval,
        "a tier whose sell margin lies outside the guard must make the draft out-of-bounds"
    );

    let requests = db.requests_of(&draft.id.to_string()).await;
    assert_eq!(
        requests.len(),
        1,
        "exactly one approval request must be opened"
    );
    assert_eq!(requests[0].1, "PENDING", "approval request must be pending");
}

/// Case 4: The evaluation hash moves when one tier margin changes (a new policy version
/// differing only in that margin), and an OFF draft's hash is unchanged from today's.
#[tokio::test]
async fn evaluation_hash_moves_when_tier_margin_changes_and_unchanged_when_off() {
    let db = TestDb::start().await;
    let e = db.engine();

    db.seed_batch(
        Utc::now() - chrono::Duration::hours(1),
        &[("USD", "1", Some("32.4500"), Some("32.9000"))],
    )
    .await;

    let owner = test_actor(OWNER_ID, "OWNER");

    // Part A: ON.
    // Changing a tier margin moves the evaluation hash.
    let tiers_v1 = vec![
        (1, Some("20000.00"), "0.0000", "0.0000"),
        (2, Some("100000.00"), "0.1000", "-0.1000"),
        (3, Some("500000.00"), "0.2000", "-0.2000"),
        (4, None, "0.3000", "-0.3000"),
    ];
    let pol_on_id =
        seed_policy_with_tiers(&db, MAIN, "0", "0", default_guard(), true, &tiers_v1).await;

    let lines1 = vec![DraftLine::new(
        code("USD"),
        LineSide::Auto,
        LineSide::Auto,
        None,
    )];
    let mut tx = db.begin().await;
    let draft1 = e
        .drafts
        .create(&mut tx, &main_branch(), &owner, lines1)
        .await
        .expect("create draft 1");
    tx.commit().await.expect("commit draft 1");

    let mut tx = db.begin().await;
    e.drafts
        .submit(&mut tx, &main_branch(), &owner, &draft1.id, &key())
        .await
        .expect("submit draft 1");
    tx.commit().await.expect("commit submit 1");
    let hash1 = db
        .evaluation_hash(&draft1.id.to_string())
        .await
        .expect("hash 1 exists");

    // Modify T2 buy margin on the active policy
    sqlx::query(
        "UPDATE rate_policy_value_tiers SET buy_margin_pct = 0.1500 \
         WHERE policy_version_id = $1::uuid AND tier_no = 2",
    )
    .bind(&pol_on_id)
    .execute(&db.pool)
    .await
    .expect("update tier margin");

    // Reset draft to DRAFT status and re-submit to re-evaluate with the updated tier margin
    sqlx::query("UPDATE rate_drafts SET status = 'DRAFT' WHERE id = $1::uuid")
        .bind(draft1.id.as_uuid())
        .execute(&db.pool)
        .await
        .expect("reset draft status");

    let mut tx = db.begin().await;
    e.drafts
        .submit(&mut tx, &main_branch(), &owner, &draft1.id, &key())
        .await
        .expect("re-submit draft 1");
    tx.commit().await.expect("commit re-submit 1");
    let hash2 = db
        .evaluation_hash(&draft1.id.to_string())
        .await
        .expect("hash 2 exists");

    assert_ne!(
        hash1, hash2,
        "evaluation hash must move when a tier margin changes on an ON policy"
    );

    // Part B: OFF.
    // An OFF draft's hash is unchanged from today's untiered hash.
    sqlx::query("UPDATE rate_policy_versions SET valid_to = now() WHERE branch_id = $1")
        .bind(MAIN)
        .execute(&db.pool)
        .await
        .expect("expire policy");

    let pol_off_id =
        seed_policy_with_tiers(&db, MAIN, "0", "0", default_guard(), false, &tiers_v1).await;
    let lines_off = vec![DraftLine::new(
        code("USD"),
        LineSide::Auto,
        LineSide::Auto,
        None,
    )];
    let mut tx = db.begin().await;
    let draft_off = e
        .drafts
        .create(&mut tx, &main_branch(), &owner, lines_off)
        .await
        .expect("create draft off");
    tx.commit().await.expect("commit draft off");

    let mut tx = db.begin().await;
    e.drafts
        .submit(&mut tx, &main_branch(), &owner, &draft_off.id, &key())
        .await
        .expect("submit draft off");
    tx.commit().await.expect("commit submit off");
    let hash_off1 = db
        .evaluation_hash(&draft_off.id.to_string())
        .await
        .expect("hash off1 exists");

    // Changing tier margin on an OFF policy does not move evaluation hash
    sqlx::query(
        "UPDATE rate_policy_value_tiers SET buy_margin_pct = 0.2500 \
         WHERE policy_version_id = $1::uuid AND tier_no = 2",
    )
    .bind(&pol_off_id)
    .execute(&db.pool)
    .await
    .expect("update inactive tier margin");

    sqlx::query("UPDATE rate_drafts SET status = 'DRAFT' WHERE id = $1::uuid")
        .bind(draft_off.id.as_uuid())
        .execute(&db.pool)
        .await
        .expect("reset draft status");

    let mut tx = db.begin().await;
    e.drafts
        .submit(&mut tx, &main_branch(), &owner, &draft_off.id, &key())
        .await
        .expect("re-submit draft off");
    tx.commit().await.expect("commit re-submit off");
    let hash_off2 = db
        .evaluation_hash(&draft_off.id.to_string())
        .await
        .expect("hash off2 exists");

    assert_eq!(
        hash_off1, hash_off2,
        "an OFF draft's hash must be unchanged when inactive tier margin changes"
    );
}

/// Case 5: line_for picks T2 for design §12.3's worked example.
/// USD at BoT 32.4500/32.9000, buy_adjust −0.5000, sell_adjust +0.4000, and the example's bands and margins:
/// a customer selling 1,000 USD gets the T2 line, and the quote is 32,320.20 THB.
/// With OFF, the same customer gets 32,287.75 THB.
#[tokio::test]
async fn line_for_picks_t2_for_worked_example_and_quotes_correct_thb() {
    let db = TestDb::start().await;
    let e = db.engine();

    db.seed_batch(
        Utc::now() - chrono::Duration::hours(1),
        &[("USD", "1", Some("32.4500"), Some("32.9000"))],
    )
    .await;

    let owner = test_actor(OWNER_ID, "OWNER");

    // 1. Publish with ON
    seed_policy_with_tiers(
        &db,
        MAIN,
        "-0.5000",
        "0.4000",
        default_guard(),
        true,
        &worked_example_tiers(),
    )
    .await;

    let lines = vec![DraftLine::new(
        code("USD"),
        LineSide::Auto,
        LineSide::Auto,
        None,
    )];
    let mut tx = db.begin().await;
    let draft_on = e
        .drafts
        .create(&mut tx, &main_branch(), &owner, lines)
        .await
        .expect("create draft on");
    tx.commit().await.expect("commit draft on");

    let mut tx = db.begin().await;
    e.drafts
        .submit(&mut tx, &main_branch(), &owner, &draft_on.id, &key())
        .await
        .expect("submit draft on");
    tx.commit().await.expect("commit submit on");

    let mut tx = db.begin().await;
    let pub_on = e
        .publisher
        .publish(&mut tx, &main_branch(), &owner, &draft_on.id, None)
        .await
        .expect("publish draft on");
    tx.commit().await.expect("commit publish on");

    // Customer selling 1,000 USD (shop BUY):
    // base_buy = 32.4500 * (1 - 0.005) = 32.28775000
    // base_value = 1,000 * 32.28775000 = 32,287.75 THB -> falls in T2 (20,000, 100,000]
    // T2 buy rate = 32.4500 * (1 - 0.005 + 0.001) = 32.4500 * 0.996 = 32.32020000
    // quote = 1,000 * 32.32020000 = 32,320.20 THB
    let sel = LineSelector {
        code: code("USD"),
        side: Side::Buy,
        group: GroupCode::all(),
        quantity: Some(ForeignAmount::from_decimal(Decimal::from(1000))),
    };

    let mut tx = db.begin().await;
    let line_on = e
        .published
        .line_for(&mut tx, &main_branch(), sel.clone())
        .await
        .expect("line_for call")
        .expect("published line for 1,000 USD on ON");
    tx.rollback().await.expect("rollback");

    assert_eq!(
        line_on.value_tier.as_str(),
        "T2",
        "customer must receive T2 line"
    );
    assert_eq!(line_on.rate, Rate::parse("32.32020000").unwrap());
    let quote_on = ExchangeArithmetic::thb(
        ForeignAmount::from_decimal(Decimal::from(1000)),
        line_on.rate,
        line_on.unit_size,
    );
    assert_eq!(
        quote_on,
        Money::parse("32320.20").unwrap(),
        "ON quote for 1,000 USD must be 32,320.20 THB"
    );

    // 2. Publish with OFF: same customer gets 32,287.75 THB
    sqlx::query("UPDATE rate_policy_versions SET valid_to = now() WHERE branch_id = $1")
        .bind(MAIN)
        .execute(&db.pool)
        .await
        .expect("expire policy");

    seed_policy_with_tiers(
        &db,
        MAIN,
        "-0.5000",
        "0.4000",
        default_guard(),
        false,
        &worked_example_tiers(),
    )
    .await;

    let lines_off = vec![DraftLine::new(
        code("USD"),
        LineSide::Auto,
        LineSide::Auto,
        None,
    )];
    let mut tx = db.begin().await;
    let draft_off = e
        .drafts
        .create(&mut tx, &main_branch(), &owner, lines_off)
        .await
        .expect("create draft off");
    tx.commit().await.expect("commit draft off");

    let mut tx = db.begin().await;
    e.drafts
        .submit(&mut tx, &main_branch(), &owner, &draft_off.id, &key())
        .await
        .expect("submit draft off");
    tx.commit().await.expect("commit submit off");

    let mut tx = db.begin().await;
    e.publisher
        .publish(
            &mut tx,
            &main_branch(),
            &owner,
            &draft_off.id,
            Some(pub_on.version),
        )
        .await
        .expect("publish draft off");
    tx.commit().await.expect("commit publish off");

    let mut tx = db.begin().await;
    let line_off = e
        .published
        .line_for(&mut tx, &main_branch(), sel)
        .await
        .expect("line_for call")
        .expect("published line for 1,000 USD on OFF");
    tx.rollback().await.expect("rollback");

    assert_eq!(line_off.value_tier.as_str(), "ALL");
    assert_eq!(line_off.rate, Rate::parse("32.28775000").unwrap());
    let quote_off = ExchangeArithmetic::thb(
        ForeignAmount::from_decimal(Decimal::from(1000)),
        line_off.rate,
        line_off.unit_size,
    );
    assert_eq!(
        quote_off,
        Money::parse("32287.75").unwrap(),
        "OFF quote for 1,000 USD must be 32,287.75 THB"
    );
}

/// Case 6: Each ON line's source_snapshot.tier carries no and both margins (design §12.4's [required] snapshot).
#[tokio::test]
async fn each_on_line_source_snapshot_tier_carries_no_and_both_margins() {
    let db = TestDb::start().await;
    let e = db.engine();

    db.seed_batch(
        Utc::now() - chrono::Duration::hours(1),
        &[("USD", "1", Some("32.4500"), Some("32.9000"))],
    )
    .await;

    seed_policy_with_tiers(
        &db,
        MAIN,
        "-0.5000",
        "0.4000",
        default_guard(),
        true,
        &worked_example_tiers(),
    )
    .await;

    let owner = test_actor(OWNER_ID, "OWNER");
    let lines = vec![DraftLine::new(
        code("USD"),
        LineSide::Auto,
        LineSide::Auto,
        None,
    )];

    let mut tx = db.begin().await;
    let draft = e
        .drafts
        .create(&mut tx, &main_branch(), &owner, lines)
        .await
        .expect("create draft");
    tx.commit().await.expect("commit draft creation");

    let mut tx = db.begin().await;
    e.drafts
        .submit(&mut tx, &main_branch(), &owner, &draft.id, &key())
        .await
        .expect("submit draft");
    tx.commit().await.expect("commit submit");

    let mut tx = db.begin().await;
    let pub_ref = e
        .publisher
        .publish(&mut tx, &main_branch(), &owner, &draft.id, None)
        .await
        .expect("publish draft");
    tx.commit().await.expect("commit publish");

    let rows = sqlx::query(
        "SELECT value_tier, source_snapshot \
         FROM published_rate_lines \
         WHERE publication_id = $1::uuid AND currency_code = 'USD' \
         ORDER BY value_tier ASC",
    )
    .bind(pub_ref.id.to_string())
    .fetch_all(&db.pool)
    .await
    .expect("fetch published lines");

    assert_eq!(rows.len(), 4, "ON must publish 4 lines");

    let expected_tier_meta: [(i64, &str, &str); 4] = [
        (1, "0.0000", "0.0000"),
        (2, "0.1000", "-0.1000"),
        (3, "0.2000", "-0.2000"),
        (4, "0.3000", "-0.3000"),
    ];

    for (row, expected) in rows.iter().zip(expected_tier_meta.iter()) {
        let vt: String = row.get("value_tier");
        let snapshot: Value = row.get("source_snapshot");

        let tier_obj = snapshot.get("tier").unwrap_or_else(|| {
            panic!("tier {vt} source_snapshot must contain 'tier' key: {snapshot}")
        });
        assert!(
            tier_obj.is_object(),
            "source_snapshot.tier must be an object"
        );

        let no = tier_obj
            .get("no")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| {
                panic!("tier {vt} source_snapshot.tier must contain integer 'no': {tier_obj}")
            });
        assert_eq!(no, expected.0, "tier no mismatch for {vt}");

        let buy_margin = tier_obj
            .get("buy_margin_pct")
            .unwrap_or_else(|| panic!("tier {vt} must have buy_margin_pct: {tier_obj}"))
            .to_string();
        let sell_margin = tier_obj
            .get("sell_margin_pct")
            .unwrap_or_else(|| panic!("tier {vt} must have sell_margin_pct: {tier_obj}"))
            .to_string();

        assert_eq!(
            buy_margin.trim_matches('"'),
            expected.1,
            "buy_margin_pct mismatch for {vt}"
        );
        assert_eq!(
            sell_margin.trim_matches('"'),
            expected.2,
            "sell_margin_pct mismatch for {vt}"
        );
    }
}
