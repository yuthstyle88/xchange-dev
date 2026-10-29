//! pricing-formula wave 3 (docs/design/blueprint/pricing-formula.md §3a; B21, B22): the FOLLOW loop prices through
//! MAIN's resolved formula and records which formula priced the tier set.
//!
//! Written before the implementation. Do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.
//!
//! Contract pinned here: `MarketPricingLoop::plan_currency` resolves MAIN's formula
//! (`FormulaResolver::resolve(tx, MAIN, <MAIN's current policy version>)`), plans with `FollowPlan::plan_with`,
//! takes the tier set's flags from `PricingSuggestion::of_with`, and records the source on the tier set it inserts
//! (`FormulaRecord::tier_set`, in the same transaction).

mod v9_follow_common;

use uuid::Uuid;
use v9_follow_common::*;
use xc_rate::domain::pricing::follow_plan::FollowResultStatus;

async fn plan(room: &Room, now: chrono::DateTime<chrono::Utc>) -> Vec<xc_rate::domain::pricing::follow_plan::FollowResult> {
    pricing_loop(&room.pool)
        .plan_once(now)
        .await
        .unwrap_or_else(|e| panic!("plan_once: {}", e.code()))
}

async fn source_of(room: &Room, set: Uuid) -> (Option<String>, Option<Uuid>) {
    sqlx::query_as("SELECT formula_source, formula_version_id FROM pricing_tier_sets WHERE id = $1")
        .bind(set)
        .fetch_one(&room.pool)
        .await
        .expect("the tier set")
}

#[tokio::test]
async fn b21_with_no_formula_rows_follow_publishes_todays_tiers_and_records_default() {
    let room = Room::start().await;
    let now = whole_second_now();
    follow_usd_at_main(&room, now, 2).await;
    let results = plan(&room, now).await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, FollowResultStatus::Published, "{results:?}");
    let sets = room.tier_sets(MAIN, "USD").await;
    assert_eq!(sets[0].tiers, pairs(&PV1_TIERS), "today's PV-1 tiers to the digit");
    assert_eq!(room.published_tiers(MAIN, "USD").await, pairs(&PV1_TIERS));
    assert_eq!(source_of(&room, sets[0].id).await, (Some("DEFAULT".into()), None));
    room.finish().await;
}

#[tokio::test]
async fn b22_an_organisation_formula_prices_follow_then_v9s_steps_apply() {
    let room = Room::start().await;
    let now = whole_second_now();
    follow_usd_at_main(&room, now, 2).await;
    let v1: Uuid = sqlx::query_scalar(
        "INSERT INTO pricing_formula_versions \
         (id, organization_id, version, buy_formula, sell_formula, valid_from, approved_by, reason) \
         VALUES (gen_random_uuid(), $1::uuid, 1, 'ref_mid - 0.05', 'ref_mid + 0.05', now() - interval '1 hour', 'owner-1', 'B22') \
         RETURNING id",
    )
    .bind(ORG)
    .fetch_one(&room.pool)
    .await
    .expect("an organisation formula");
    let results = plan(&room, now).await;
    assert_eq!(results[0].status, FollowResultStatus::Published, "{results:?}");
    // TT 33.20 / 33.90, mid 33.55: 33.50 / 33.60 at every tier, under the 0.3 % minimum spread (0.10065), widened
    // to floor(33.499675) / ceil(33.600325)
    let want = pairs(&[("33.49", "33.61"); 4]);
    let sets = room.tier_sets(MAIN, "USD").await;
    assert_eq!(sets[0].tiers, want);
    assert_eq!(room.published_tiers(MAIN, "USD").await, want);
    assert_eq!(source_of(&room, sets[0].id).await, (Some("ORGANISATION".into()), Some(v1)));
    room.finish().await;
}
