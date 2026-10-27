//! XC-V9-PRICING (pricing-tiers.md §15.2, §15.2a, §15.10): the Pg stores over the §15.2 tables, one round trip each.
//! PV-a4a `PgTierSetStore`, PV-a4b `PgBranchPolicyStore`, PV-a4c `PgDisplaySettingsStore`, PV-f2a
//! `PgFollowResultStore`, PV-f2b `PgPricingModeStore`, PV-l1 `PricingSuggestionSettingsStore::{branch_min_spread,
//! save_branch_min_spread}`. Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING
//! skeleton). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test` (see
//! `rate_engine_common`). Every test fails without it; none skips.

mod rate_engine_common;

use chrono::{DateTime, Duration, TimeZone, Utc};
use rate_engine_common::{MAIN, TestDb};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, Rate};
use xc_rate::domain::pricing::branch_policy::{
    BranchPolicyPatch, BranchPolicyRow, BranchPolicyStore, BranchPricingPolicy,
};
use xc_rate::domain::pricing::display_settings::{
    DisplayFrom, DisplaySettings, DisplaySettingsStore,
};
use xc_rate::domain::pricing::follow_plan::{
    FollowReason, FollowResult, FollowResultStatus, FollowResultStore,
};
use xc_rate::domain::pricing::market_quote::MarketSource;
use xc_rate::domain::pricing::pricing_mode::{PricingMode, PricingModeStore};
use xc_rate::domain::pricing::tier_set::{
    MarketAnchor, PricingTierSet, TierRate, TierSetSource, TierSetStore,
};
use xc_rate::infrastructure::pricing::branch_policy_store::PgBranchPolicyStore;
use xc_rate::infrastructure::pricing::display_settings_store::PgDisplaySettingsStore;
use xc_rate::infrastructure::pricing::follow_result_store::PgFollowResultStore;
use xc_rate::infrastructure::pricing::pricing_mode_store::PgPricingModeStore;
use xc_rate::infrastructure::pricing::suggestion_settings_store::PricingSuggestionSettingsStore;
use xc_rate::infrastructure::pricing::tier_set_store::PgTierSetStore;

const B: &str = "XC-V9-B";

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a currency")
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).expect("a rate")
}

fn dec(text: &str) -> Decimal {
    text.parse().expect("a decimal")
}

fn at(minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 27, 3, minute, 0).unwrap()
}

/// PV-1's tiers (`H:` §4.4), per unit size 1.
fn pv1_tiers() -> [TierRate; 4] {
    [
        ("33.23", "33.87"),
        ("33.24", "33.86"),
        ("33.26", "33.84"),
        ("33.29", "33.80"),
    ]
    .map(|(b, s)| TierRate {
        buy: rate(b),
        sell: rate(s),
    })
}

fn tier_set(
    branch: &str,
    currency: &str,
    source: TierSetSource,
    request_id: &str,
    approved_at: DateTime<Utc>,
) -> PricingTierSet {
    PricingTierSet {
        id: Uuid::new_v4(),
        request_id: request_id.to_string(),
        branch: branch.to_string(),
        currency: code(currency),
        source,
        base_tier_set_id: None,
        tiers: pv1_tiers(),
        anchor: Some(MarketAnchor {
            source: MarketSource::Tt,
            buy: rate("33.20"),
            sell: rate("33.90"),
            fetched_at: approved_at - Duration::minutes(2),
            batch_id: Some(Uuid::new_v4()),
        }),
        flags: vec!["MinSpread".to_string()],
        draft_id: None,
        approved_by: "owner-1".to_string(),
        approved_at,
    }
}

async fn db_with_branch() -> TestDb {
    let db = TestDb::start().await;
    db.seed_branch(B).await;
    db
}

// ------------------------------------------------------------------ PgTierSetStore (PV-a4a)

/// PV-a4a: every field written comes back, including a 5-decimal price (B2) and the anchor (§15.3 step 3).
#[tokio::test]
async fn a_tier_set_reads_back_exactly_as_written() {
    let db = db_with_branch().await;
    let store = PgTierSetStore;
    let mut set = tier_set(MAIN, "VND", TierSetSource::Manual, "req-vnd", at(0));
    set.tiers = [
        ("0.00121", "0.00133"),
        ("0.00122", "0.00132"),
        ("0.00123", "0.00131"),
        ("0.00124", "0.00130"),
    ]
    .map(|(b, s)| TierRate {
        buy: rate(b),
        sell: rate(s),
    });
    let mut conn = db.pool.acquire().await.expect("a connection");
    assert!(store.insert_in(&mut conn, &set).await.expect("insert"));

    let got = store
        .latest_in(&mut conn, MAIN, code("VND"))
        .await
        .expect("read")
        .expect("the row");
    assert_eq!(got, set);
    assert_eq!(
        got.tiers[0].buy.value(),
        dec("0.00121"),
        "B2: 5 decimals kept"
    );
    assert_eq!(
        store.get_in(&mut conn, set.id).await.expect("get"),
        Some(set.clone())
    );
    assert_eq!(
        store
            .by_request_in(&mut conn, "req-vnd")
            .await
            .expect("by request"),
        Some(set)
    );
}

/// PV-a4a: the same `request_id` twice writes one row and answers `false` the second time (§15.4 retry, §15.5).
#[tokio::test]
async fn a_second_insert_with_the_same_request_id_writes_nothing() {
    let db = db_with_branch().await;
    let store = PgTierSetStore;
    let mut conn = db.pool.acquire().await.expect("a connection");
    let first = tier_set(MAIN, "USD", TierSetSource::Manual, "req-1", at(0));
    let again = tier_set(MAIN, "USD", TierSetSource::Manual, "req-1", at(1));
    assert!(store.insert_in(&mut conn, &first).await.expect("insert"));
    assert!(
        !store
            .insert_in(&mut conn, &again)
            .await
            .expect("insert again")
    );
    assert_eq!(db.count("pricing_tier_sets").await, 1);
    assert_eq!(
        store
            .latest_in(&mut conn, MAIN, code("USD"))
            .await
            .expect("read"),
        Some(first)
    );
}

/// PV-a4a: latest = newest `approved_at` for (branch, currency); other branches and currencies never leak in; the
/// latest FOLLOW set ignores a newer MANUAL one (§15.7 `MISMATCH`).
#[tokio::test]
async fn latest_is_per_branch_and_currency_and_latest_follow_skips_manual_rows() {
    let db = db_with_branch().await;
    let store = PgTierSetStore;
    let mut conn = db.pool.acquire().await.expect("a connection");
    let follow = tier_set(
        MAIN,
        "USD",
        TierSetSource::Follow,
        "follow:MAIN:USD:b1",
        at(0),
    );
    let manual = tier_set(MAIN, "USD", TierSetSource::Manual, "req-2", at(5));
    let mut pushed = tier_set(B, "USD", TierSetSource::MainPush, "req-2:XC-V9-B", at(6));
    pushed.base_tier_set_id = Some(manual.id);
    let eur = tier_set(MAIN, "EUR", TierSetSource::Manual, "req-3", at(7));
    for set in [&follow, &manual, &pushed, &eur] {
        assert!(store.insert_in(&mut conn, set).await.expect("insert"));
    }

    assert_eq!(
        store
            .latest_in(&mut conn, MAIN, code("USD"))
            .await
            .expect("read"),
        Some(manual.clone())
    );
    assert_eq!(
        store
            .latest_follow_in(&mut conn, MAIN, code("USD"))
            .await
            .expect("read"),
        Some(follow)
    );
    assert_eq!(
        store
            .latest_in(&mut conn, B, code("USD"))
            .await
            .expect("read"),
        Some(pushed)
    );
    assert_eq!(
        store
            .latest_follow_in(&mut conn, B, code("USD"))
            .await
            .expect("read"),
        None
    );
    assert_eq!(
        store
            .latest_in(&mut conn, B, code("EUR"))
            .await
            .expect("read"),
        None
    );
}

// ------------------------------------------------------------------ PgBranchPolicyStore (PV-a4b)

/// PV-a4b: a branch with no rows reads empty; a save upserts; a patch leaves the columns it does not name.
#[tokio::test]
async fn branch_policy_rows_upsert_and_a_patch_keeps_what_it_does_not_name() {
    let db = db_with_branch().await;
    let store = PgBranchPolicyStore;
    let mut conn = db.pool.acquire().await.expect("a connection");
    assert_eq!(
        store.for_branch_in(&mut conn, B).await.expect("read"),
        BranchPricingPolicy::default()
    );

    let both = BranchPolicyPatch {
        buy_bp: Some(5),
        sell_bp: Some(-5),
        own: None,
        follow: None,
    };
    store
        .save_in(&mut conn, B, "*", both, "owner-1", at(0))
        .await
        .expect("save *");
    store
        .save_in(
            &mut conn,
            B,
            "USD",
            BranchPolicyPatch {
                follow: Some(true),
                ..BranchPolicyPatch::default()
            },
            "owner-1",
            at(1),
        )
        .await
        .expect("save USD");
    store
        .save_in(
            &mut conn,
            B,
            "USD",
            BranchPolicyPatch {
                buy_bp: Some(12),
                ..BranchPolicyPatch::default()
            },
            "owner-1",
            at(2),
        )
        .await
        .expect("save USD again");

    let mut rows = store.for_branch_in(&mut conn, B).await.expect("read").rows;
    rows.sort_by(|a, b| a.currency.cmp(&b.currency));
    assert_eq!(
        rows,
        vec![
            BranchPolicyRow {
                currency: "*".to_string(),
                buy_bp: Some(5),
                sell_bp: Some(-5),
                own: false,
                follow: false,
            },
            BranchPolicyRow {
                currency: "USD".to_string(),
                buy_bp: Some(12),
                sell_bp: None,
                own: false,
                follow: true,
            },
        ]
    );
    assert!(
        store
            .for_branch_in(&mut conn, MAIN)
            .await
            .expect("read MAIN")
            .rows
            .is_empty(),
        "another branch's rows never leak in"
    );
}

// ------------------------------------------------------------------ PgDisplaySettingsStore (PV-a4c)

fn display(featured: &[&str], tiers: &[u8]) -> DisplaySettings {
    DisplaySettings {
        tiers: tiers.to_vec(),
        featured: featured.iter().map(|c| c.to_string()).collect(),
        highlight: false,
        all_market: false,
    }
}

/// PV-a4c, PV-17: no row anywhere ⇒ V.9's default (`H:` §8); MAIN's row ⇒ B reads MAIN's; B's own row wins; clear
/// returns B to MAIN's.
#[tokio::test]
async fn display_settings_fall_back_from_the_branch_to_main_to_the_default() {
    let db = db_with_branch().await;
    let store = PgDisplaySettingsStore;
    let mut conn = db.pool.acquire().await.expect("a connection");

    let default = DisplaySettings {
        tiers: vec![1, 2, 3, 4],
        featured: [
            "MYR", "CNY", "INR", "RUB", "KRW", "JPY", "GBP", "USD", "TWD", "EUR",
        ]
        .iter()
        .map(|c| c.to_string())
        .collect(),
        highlight: true,
        all_market: true,
    };
    assert_eq!(
        store.effective_in(&mut conn, B).await.expect("read"),
        (default, DisplayFrom::Default)
    );
    assert_eq!(store.newest_in(&mut conn).await.expect("newest"), None);

    let main_row = display(&["USD", "EUR"], &[1, 4]);
    store
        .save_in(&mut conn, MAIN, &main_row, "owner-1", at(0))
        .await
        .expect("save MAIN");
    assert_eq!(
        store.effective_in(&mut conn, B).await.expect("read"),
        (main_row.clone(), DisplayFrom::Main)
    );
    assert_eq!(
        store
            .effective_in(&mut conn, MAIN)
            .await
            .expect("read MAIN"),
        (main_row.clone(), DisplayFrom::Branch),
        "MAIN's own row is its branch row"
    );

    let b_row = display(&["JPY"], &[2]);
    store
        .save_in(&mut conn, B, &b_row, "owner-1", at(3))
        .await
        .expect("save B");
    assert_eq!(
        store.effective_in(&mut conn, B).await.expect("read"),
        (b_row, DisplayFrom::Branch)
    );
    assert_eq!(
        store.newest_in(&mut conn).await.expect("newest"),
        Some(at(3))
    );

    store.clear_in(&mut conn, B).await.expect("clear");
    assert_eq!(
        store.effective_in(&mut conn, B).await.expect("read"),
        (main_row, DisplayFrom::Main)
    );
}

// ------------------------------------------------------------------ PgFollowResultStore (PV-f2a)

/// PV-f2a: one row per currency, the latest plan overwrites the previous one; reasons keep their order.
#[tokio::test]
async fn a_follow_result_is_the_latest_plan_per_currency() {
    let db = db_with_branch().await;
    let tier_sets = PgTierSetStore;
    let store = PgFollowResultStore;
    let mut conn = db.pool.acquire().await.expect("a connection");
    assert_eq!(
        store.latest_in(&mut conn, code("USD")).await.expect("read"),
        None
    );

    let blocked = FollowResult {
        currency: code("USD"),
        status: FollowResultStatus::Blocked,
        reasons: vec![FollowReason::StaleSource, FollowReason::LargeMove],
        anchor_batch_id: Some(Uuid::new_v4()),
        tier_set_id: None,
        planned_at: at(0),
    };
    store.record_in(&mut conn, &blocked).await.expect("record");
    assert_eq!(
        store.latest_in(&mut conn, code("USD")).await.expect("read"),
        Some(blocked)
    );

    let set = tier_set(
        MAIN,
        "USD",
        TierSetSource::Follow,
        "follow:MAIN:USD:b2",
        at(5),
    );
    assert!(tier_sets.insert_in(&mut conn, &set).await.expect("insert"));
    let published = FollowResult {
        currency: code("USD"),
        status: FollowResultStatus::Published,
        reasons: vec![],
        anchor_batch_id: set.anchor.as_ref().and_then(|a| a.batch_id),
        tier_set_id: Some(set.id),
        planned_at: at(5),
    };
    store
        .record_in(&mut conn, &published)
        .await
        .expect("record again");
    assert_eq!(
        store.latest_in(&mut conn, code("USD")).await.expect("read"),
        Some(published)
    );
    assert_eq!(db.count("pricing_follow_results").await, 1);
    assert_eq!(
        store.newest_in(&mut conn).await.expect("newest"),
        Some(at(5))
    );
}

// ------------------------------------------------------------------ PgPricingModeStore (PV-f2b)

/// PV-f2b: each save bumps the currency's revision from 1; `following_in` lists FOLLOW codes only, by code.
#[tokio::test]
async fn a_mode_save_bumps_the_revision_and_following_lists_follow_codes() {
    let db = db_with_branch().await;
    let store = PgPricingModeStore;
    let mut conn = db.pool.acquire().await.expect("a connection");
    assert_eq!(
        store.get_in(&mut conn, code("USD")).await.expect("read"),
        None
    );
    assert_eq!(
        store.newest_revision_in(&mut conn).await.expect("newest"),
        None
    );

    assert_eq!(
        store
            .save_in(
                &mut conn,
                code("USD"),
                PricingMode::Follow,
                "owner-1",
                at(0)
            )
            .await
            .expect("save"),
        1
    );
    assert_eq!(
        store
            .save_in(
                &mut conn,
                code("USD"),
                PricingMode::Manual,
                "owner-1",
                at(1)
            )
            .await
            .expect("save again"),
        2
    );
    store
        .save_in(
            &mut conn,
            code("JPY"),
            PricingMode::Follow,
            "owner-1",
            at(2),
        )
        .await
        .expect("save JPY");
    store
        .save_in(
            &mut conn,
            code("EUR"),
            PricingMode::Follow,
            "owner-1",
            at(3),
        )
        .await
        .expect("save EUR");

    let usd = store
        .get_in(&mut conn, code("USD"))
        .await
        .expect("read")
        .expect("the row");
    assert_eq!(usd.mode, PricingMode::Manual);
    assert_eq!(usd.revision, 2);
    assert_eq!(usd.updated_at, at(1));
    assert_eq!(
        store.following_in(&mut conn).await.expect("following"),
        vec![code("EUR"), code("JPY")]
    );
    assert_eq!(
        store.newest_revision_in(&mut conn).await.expect("newest"),
        Some(2)
    );
}

// ------------------------------------------------------------------ min spread '*' (PV-l1)

/// PV-l1 (gap 10): the branch `'*'` min spread reads back; `None` clears it and keeps the `'*'` row's `better`; a
/// branch with no `'*'` row gets one with V.9's default `better`.
#[tokio::test]
async fn the_branch_min_spread_is_saved_on_the_star_row() {
    let db = db_with_branch().await;
    let mut conn = db.pool.acquire().await.expect("a connection");
    assert_eq!(
        PricingSuggestionSettingsStore::branch_min_spread(&mut conn, B)
            .await
            .expect("read"),
        None
    );

    PricingSuggestionSettingsStore::save_branch_min_spread(
        &mut conn,
        B,
        Some(dec("0.5")),
        "owner-1",
        at(0),
    )
    .await
    .expect("save");
    assert_eq!(
        PricingSuggestionSettingsStore::branch_min_spread(&mut conn, B)
            .await
            .expect("read"),
        Some(dec("0.5"))
    );
    let settings = PricingSuggestionSettingsStore::for_branch(&mut conn, B)
        .await
        .expect("settings");
    assert_eq!(
        settings.branch_default,
        Some([dec("0.1"), dec("0.14"), dec("0.2"), dec("0.3")]),
        "a new '*' row carries V.9's default better"
    );

    PricingSuggestionSettingsStore::save_branch_min_spread(&mut conn, B, None, "owner-1", at(1))
        .await
        .expect("clear");
    assert_eq!(
        PricingSuggestionSettingsStore::branch_min_spread(&mut conn, B)
            .await
            .expect("read"),
        None
    );
    assert!(
        PricingSuggestionSettingsStore::for_branch(&mut conn, B)
            .await
            .expect("settings")
            .branch_default
            .is_some(),
        "clearing the min spread keeps the '*' row's better"
    );
}

/// PV-a4a: `set_draft_in` records the publishing draft on the row (§15.3 step 4; `PublishService` reads it back).
#[tokio::test]
async fn set_draft_records_the_publishing_draft() {
    let db = db_with_branch().await;
    let store = PgTierSetStore;
    let mut conn = db.pool.acquire().await.expect("a connection");
    let set = tier_set(MAIN, "USD", TierSetSource::Manual, "req-draft", at(0));
    assert!(store.insert_in(&mut conn, &set).await.expect("insert"));
    let draft = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO rate_drafts (id, branch_id, status, created_by, version) \
         VALUES ($1, 'MAIN', 'DRAFT', 'owner-1', 1)",
    )
    .bind(draft)
    .execute(&db.pool)
    .await
    .expect("a draft");
    store
        .set_draft_in(&mut conn, set.id, draft)
        .await
        .expect("set draft");
    assert_eq!(
        store
            .get_in(&mut conn, set.id)
            .await
            .expect("get")
            .and_then(|s| s.draft_id),
        Some(draft)
    );
}
