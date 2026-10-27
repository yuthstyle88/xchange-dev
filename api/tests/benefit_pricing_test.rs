// Written before the implementation by the ticket author claude/e2926a49 (sub-controller
// XC-BOOKING-ESTIMATE under the xchange BE lane controller claude/bc8fd29a). Do not edit, delete or weaken.

//! Up-front tests for BE-1, `BenefitPricing::evaluate_in` (design `marketing-campaigns.md` §6.4, decision E-3,
//! class diagram §6.4.4, behaviour table §6.4.5). Owner decision OD-1 B (2026-10-27): the estimate keeps only
//! rules whose `channels` also admit DESK, through `BenefitPricingInput.also_admitted_on = Some(Desk)`.
//!
//! Rows of §6.4.5 covered here:
//! - 16: `also_admitted_on = Some(Desk)` and an APP-only rule ⇒ the rule is not in `evaluation.snapshot.applied`;
//!   a lower-priority rule admitting DESK in the same stack group applies instead. Control: with
//!   `also_admitted_on = None` the higher-priority APP-only rule applies.
//!
//! Row 15 (`price_in` unchanged before/after the extraction) is not written here yet: it waits for another branch.
//!
//! Each evaluation runs in a transaction the test rolls back. Needs `TEST_DATABASE_URL` naming a database whose
//! name ends in `_test`; nothing here skips.

mod common;
mod quote_common;

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_exchange::domain::quote::quote::QuoteChannel;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Side};
use xc_marketing::infrastructure::pg_campaign_repository::PgCampaignRepository;
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::domain::publication::published_rates::{LineSelector, PublishedRates};
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;
use xchange::application::workflows::exchange::benefit_pricing::{
    BenefitPricing, BenefitPricingInput, PricingLine,
};

/// An APPROVED running campaign on `branch` in stack group `default` at `priority`, with one PRICE
/// `RATE_IMPROVEMENT` rule of `eligibility` and `delta`; its rule id.
async fn campaign(
    db: &TestDb,
    branch: &str,
    user: Uuid,
    now: DateTime<Utc>,
    priority: i32,
    eligibility: Value,
    delta: &str,
) -> Uuid {
    let (campaign, version, rule) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, $4, now())",
    )
    .bind(campaign)
    .bind(ORG_1)
    .bind(unique("be1-campaign"))
    .bind(user)
    .execute(&db.pool)
    .await
    .expect("insert campaign");
    sqlx::query(
        "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
             budget_thb, stack_group, priority, version_hash, edited_at, created_by, created_at) \
         VALUES ($1, $2::uuid, $3, 1, 'APPROVED', $4, $5, 5000.00, 'default', $6, 'hash-be1', now(), $7, now())",
    )
    .bind(version)
    .bind(ORG_1)
    .bind(campaign)
    .bind(now - Duration::days(1))
    .bind(now + Duration::days(30))
    .bind(priority)
    .bind(user)
    .execute(&db.pool)
    .await
    .expect("insert campaign version");
    sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
        .bind(version)
        .bind(campaign)
        .execute(&db.pool)
        .await
        .expect("set current version");
    sqlx::query("INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2)")
        .bind(version)
        .bind(branch)
        .execute(&db.pool)
        .await
        .expect("insert campaign branch");
    sqlx::query(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) VALUES ($1, 0, 0)",
    )
    .bind(version)
    .execute(&db.pool)
    .await
    .expect("insert campaign budget");
    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, \
             caps, requires_coupon, rule_no) \
         VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', $3, $4, '{}'::jsonb, false, 1)",
    )
    .bind(rule)
    .bind(version)
    .bind(eligibility)
    .bind(json!({ "delta": delta }))
    .execute(&db.pool)
    .await
    .expect("insert benefit rule");
    rule
}

/// The rule ids `evaluate_in` applied, judged on APP for an anonymous caller buying-side 100 USD, fee 0.
async fn applied_on_app(
    db: &TestDb,
    branch: &str,
    now: DateTime<Utc>,
    also_admitted_on: Option<QuoteChannel>,
) -> Vec<Uuid> {
    let branch_id = BranchId::new(branch);
    let currency = CurrencyCode::parse("USD").expect("USD");
    let quantity = ForeignAmount::from_decimal(Decimal::from(100));
    let mut tx = db.pool.begin().await.expect("begin");
    let published = PgPublishedRates::new(db.pool.clone())
        .line_for(
            &mut tx,
            &branch_id,
            LineSelector {
                code: currency,
                side: Side::Buy,
                group: GroupCode::all(),
                quantity: Some(quantity),
            },
        )
        .await
        .expect("read the published line")
        .expect("USD BUY is published");
    let priced = BenefitPricing::new(Arc::new(PgCampaignRepository::new()))
        .evaluate_in(
            &mut tx,
            BenefitPricingInput {
                organization_id: Uuid::from_str(ORG_1).expect("ORG_1 is a uuid"),
                branch: &branch_id,
                eligibility_channel: QuoteChannel::App,
                also_admitted_on,
                customer_id: None,
                coupon: None,
                coupon_code: None,
                line: PricingLine {
                    currency,
                    shop_side: Side::Buy,
                    quantity,
                    fee_thb: Money::ZERO,
                    published,
                },
                now,
            },
        )
        .await
        .unwrap_or_else(|e| panic!("evaluate_in with also_admitted_on {also_admitted_on:?}: {e}"));
    tx.rollback().await.expect("rollback");
    priced
        .evaluation
        .snapshot
        .applied
        .iter()
        .map(|a| a.rule_id)
        .collect()
}

#[tokio::test]
async fn be1_row16_also_admitted_on_desk_drops_an_app_only_rule() {
    let db = TestDb::start().await;
    let branch = "B-BE1-R16";
    db.branch(branch).await;
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
    let user = Uuid::new_v4();
    db.exec(&format!(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ('{user}', '{ORG_1}'::uuid, '{}', 'ACTIVE', 'system')",
        unique("be1-user")
    ))
    .await;
    let now = whole_second_now();
    // Same stack group: the APP-only rule wins on priority unless it is filtered out first.
    let app_only = campaign(
        &db,
        branch,
        user,
        now,
        2,
        json!({ "currencies": ["USD"], "channels": ["APP"] }),
        "0.50",
    )
    .await;
    let app_and_desk = campaign(
        &db,
        branch,
        user,
        now,
        1,
        json!({ "currencies": ["USD"], "channels": ["APP", "DESK"] }),
        "0.20",
    )
    .await;

    let filtered = applied_on_app(&db, branch, now, Some(QuoteChannel::Desk)).await;
    assert!(
        !filtered.contains(&app_only),
        "also_admitted_on = Some(Desk) drops the APP-only rule {app_only}; applied {filtered:?}"
    );
    assert_eq!(
        filtered,
        vec![app_and_desk],
        "the lower-priority rule admitting DESK applies instead; applied {filtered:?}"
    );

    let unfiltered = applied_on_app(&db, branch, now, None).await;
    assert_eq!(
        unfiltered,
        vec![app_only],
        "control: with also_admitted_on = None the higher-priority APP-only rule applies; applied {unfiltered:?}"
    );
    db.finish().await;
}
