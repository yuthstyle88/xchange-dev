//! PT-1 (pricing-tiers.md §3, §3.1, §6, §10.1): `PolicyVersionWriter::next`, the one writer of policy versions.
//! Written before the implementation by the ticket author (claude/647e1967, PT-0). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! The override rows go through `CurrencyTierOverrideStore` (PT-3), so the last two tests pass only once PT-3 is on
//! `feat/pricing-tiers`. Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database ending in `_test`.

mod rate_engine_common;

use rate_engine_common::{MAIN, TestDb};
use rust_decimal::Decimal;
use serde_json::json;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, RatePercent};
use xc_rate::application::policy::policy_version_writer::{
    PolicyVersionChange, PolicyVersionWriter,
};
use xc_rate::domain::policy::anchor_source::AnchorSource;
use xc_rate::domain::policy::currency_tier_override::{CurrencyTierOverride, TierMargin};
use xc_rate::domain::policy::policy_tiers::PolicyTier;
use xc_rate::infrastructure::policy::currency_tier_override_store::CurrencyTierOverrideStore;

fn pct(text: &str) -> RatePercent {
    RatePercent::parse(text).expect("a percent")
}

fn dec(text: &str) -> Decimal {
    text.parse().expect("decimal")
}

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a currency")
}

fn value_tiers() -> Vec<PolicyTier> {
    let m = |t: &str| Some(Money::parse(t).expect("money"));
    vec![
        PolicyTier::new(1, m("10000"), pct("-1.0000"), pct("1.0000")),
        PolicyTier::new(2, m("50000"), pct("-0.8000"), pct("0.8000")),
        PolicyTier::new(3, m("200000"), pct("-0.6000"), pct("0.6000")),
        PolicyTier::new(4, None, pct("-0.4000"), pct("0.4000")),
    ]
}

fn change(
    anchor: Option<AnchorSource>,
    overrides: Option<Vec<CurrencyTierOverride>>,
) -> PolicyVersionChange {
    PolicyVersionChange {
        buy_adjust_pct: dec("-0.5000"),
        sell_adjust_pct: dec("0.5000"),
        guards: json!({"USD": {"buy": {"min": -5, "max": 5}, "sell": {"min": -5, "max": 5}}}),
        max_reference_age_hours: Some(6),
        on_stale: Some("STOP".into()),
        stale_limit_thb: None,
        value_tiers_on: true,
        sharp_move_alert_pct: Some(dec("2.0000")),
        value_tiers: value_tiers(),
        anchor_source: anchor,
        overrides,
        approved_by: "owner-1".into(),
        reason: "PT-1 test".into(),
    }
}

fn margins(base: &str) -> [TierMargin; 4] {
    let b = dec(base);
    [1u8, 2, 3, 4].map(|n| TierMargin {
        tier_no: n,
        buy_margin_pct: pct(&format!("{:.4}", -b)),
        sell_margin_pct: pct(&format!("{:.4}", b)),
    })
}

fn over(currency: &str, base: Option<&str>, anchor: Option<AnchorSource>) -> CurrencyTierOverride {
    CurrencyTierOverride {
        currency: code(currency),
        tiers: base.map(margins),
        min_spread_pct: None,
        anchor_source: anchor,
    }
}

#[derive(Debug)]
struct Row {
    version: i32,
    buy_adjust_pct: Decimal,
    sell_adjust_pct: Decimal,
    max_reference_age_hours: Option<i32>,
    on_stale: Option<String>,
    value_tiers_on: bool,
    sharp_move_alert_pct: Option<Decimal>,
    approved_by: String,
    reason: String,
    source_version: Option<i32>,
    anchor_source: String,
    open: bool,
}

type RowTuple = (
    i32,
    Decimal,
    Decimal,
    Option<i32>,
    Option<String>,
    bool,
    Option<Decimal>,
    String,
    String,
    Option<i32>,
    String,
    bool,
);

async fn row(db: &TestDb, id: &Uuid) -> Row {
    let t: RowTuple = sqlx::query_as(
        "SELECT version, buy_adjust_pct, sell_adjust_pct, max_reference_age_hours, on_stale, value_tiers_on, \
                sharp_move_alert_pct, approved_by, reason, source_version, anchor_source, valid_to IS NULL AS open \
         FROM rate_policy_versions WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("the written version");
    Row {
        version: t.0,
        buy_adjust_pct: t.1,
        sell_adjust_pct: t.2,
        max_reference_age_hours: t.3,
        on_stale: t.4,
        value_tiers_on: t.5,
        sharp_move_alert_pct: t.6,
        approved_by: t.7,
        reason: t.8,
        source_version: t.9,
        anchor_source: t.10,
        open: t.11,
    }
}

async fn write(db: &TestDb, c: PolicyVersionChange) -> Uuid {
    let mut tx = db.begin().await;
    let written = PolicyVersionWriter::next(&mut tx, &BranchId::new(MAIN), c)
        .await
        .expect("next");
    tx.commit().await.expect("commit");
    written.id
}

#[tokio::test]
async fn the_first_version_of_a_branch_is_version_1_anchored_on_bot_with_its_value_tiers() {
    let db = TestDb::start().await;
    db.seed_branch(MAIN).await;
    let mut tx = db.begin().await;
    let written = PolicyVersionWriter::next(&mut tx, &BranchId::new(MAIN), change(None, None))
        .await
        .expect("next");
    tx.commit().await.expect("commit");
    assert_eq!(written.version, 1);

    let r = row(&db, &written.id).await;
    assert_eq!(r.version, 1);
    assert_eq!(r.source_version, None);
    assert_eq!(
        r.anchor_source, "BOT",
        "§3.1: no anchor given and no previous version ⇒ BOT"
    );
    assert_eq!(
        (r.buy_adjust_pct, r.sell_adjust_pct),
        (dec("-0.5000"), dec("0.5000"))
    );
    assert_eq!(r.max_reference_age_hours, Some(6));
    assert_eq!(r.on_stale.as_deref(), Some("STOP"));
    assert!(r.value_tiers_on);
    assert_eq!(r.sharp_move_alert_pct, Some(dec("2.0000")));
    assert_eq!(
        (r.approved_by.as_str(), r.reason.as_str()),
        ("owner-1", "PT-1 test")
    );
    assert!(r.open);

    let tiers: Vec<(i16, Option<Decimal>, Decimal, Decimal)> = sqlx::query_as(
        "SELECT tier_no, upper_inclusive_thb, buy_margin_pct, sell_margin_pct FROM rate_policy_value_tiers \
         WHERE policy_version_id = $1 ORDER BY tier_no",
    )
    .bind(written.id)
    .fetch_all(&db.pool)
    .await
    .expect("value tiers");
    assert_eq!(
        tiers,
        vec![
            (1, Some(dec("10000.00")), dec("-1.0000"), dec("1.0000")),
            (2, Some(dec("50000.00")), dec("-0.8000"), dec("0.8000")),
            (3, Some(dec("200000.00")), dec("-0.6000"), dec("0.6000")),
            (4, None, dec("-0.4000"), dec("0.4000")),
        ]
    );
}

#[tokio::test]
async fn the_next_version_closes_the_current_one_and_names_it_as_its_source() {
    let db = TestDb::start().await;
    db.seed_branch(MAIN).await;
    let v1 = write(&db, change(None, None)).await;
    let v2 = write(&db, change(None, None)).await;

    let (r1, r2) = (row(&db, &v1).await, row(&db, &v2).await);
    assert_eq!(r2.version, 2);
    assert_eq!(r2.source_version, Some(1));
    assert!(!r1.open, "the previous version's valid_to is set");
    assert!(r2.open);
}

#[tokio::test]
async fn no_anchor_in_the_change_keeps_the_current_versions_anchor() {
    let db = TestDb::start().await;
    db.seed_branch(MAIN).await;
    let v1 = write(&db, change(Some(AnchorSource::MarketTt), None)).await;
    assert_eq!(row(&db, &v1).await.anchor_source, "MARKET_TT");

    let v2 = write(&db, change(None, None)).await;
    assert_eq!(
        row(&db, &v2).await.anchor_source,
        "MARKET_TT",
        "§11: an adjust-only save keeps the anchor"
    );

    let v3 = write(&db, change(Some(AnchorSource::MarketSr), None)).await;
    assert_eq!(row(&db, &v3).await.anchor_source, "MARKET_SR");
}

#[tokio::test]
async fn no_overrides_in_the_change_carries_every_override_forward() {
    let db = TestDb::start().await;
    db.seed_branch(MAIN).await;
    let set = vec![
        over("EUR", Some("0.3"), None),
        over("USD", Some("0.5"), Some(AnchorSource::MarketTt)),
    ];
    let v1 = write(&db, change(None, Some(set.clone()))).await;
    let v2 = write(&db, change(None, None)).await;

    let mut tx = db.begin().await;
    assert_eq!(
        CurrencyTierOverrideStore::for_version(&mut tx, &v1)
            .await
            .expect("v1"),
        set
    );
    assert_eq!(
        CurrencyTierOverrideStore::for_version(&mut tx, &v2)
            .await
            .expect("v2"),
        set,
        "§11: `ratePolicySave` changing only adjust_pct carries every currency's overrides"
    );
}

#[tokio::test]
async fn overrides_in_the_change_replace_those_currencies_and_carry_the_others() {
    let db = TestDb::start().await;
    db.seed_branch(MAIN).await;
    write(
        &db,
        change(
            None,
            Some(vec![
                over("EUR", Some("0.3"), None),
                over("JPY", Some("0.2"), None),
                over("USD", Some("0.5"), None),
            ]),
        ),
    )
    .await;
    // USD replaced; EUR given with no tiers, no spread and no anchor ⇒ removed; JPY not named ⇒ carried.
    let v2 = write(
        &db,
        change(
            None,
            Some(vec![
                over("USD", Some("0.7"), None),
                over("EUR", None, None),
            ]),
        ),
    )
    .await;

    let mut tx = db.begin().await;
    assert_eq!(
        CurrencyTierOverrideStore::for_version(&mut tx, &v2)
            .await
            .expect("v2"),
        vec![
            over("JPY", Some("0.2"), None),
            over("USD", Some("0.7"), None)
        ]
    );
}
