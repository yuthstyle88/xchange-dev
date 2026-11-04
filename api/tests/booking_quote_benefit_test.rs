//! Up-front tests for BQ-M3 (design `docs/design/blueprint/marketing-campaigns.md` §10 O-3, answer A, decided on
//! the owner's order 2026-10-19, #564 issuecomment-6262478258). Written by the sub-controller claude/644b3958 on
//! `feat/binding-quote` before the implementation. **Do not edit, delete or weaken.**
//!
//! O-3 A: a BOOKING-channel quote is INDICATIVE and shows its estimated benefit with **no reservation** (no budget
//! reservation, no coupon lock); the completion's DESK quote alone reserves. The eligibility channel is DESK for a
//! booking quote staff issue (`BookingsDesk`; no customer reaches `QuoteService`) and APP for `CustomerSelf` (no
//! caller yet: the customer-side estimate needs its own design). Today `QuoteService` passes `BOOKING` into
//! `EvalContext.channel` (`quote.rs` eval context), so a rule naming DESK is skipped on a booking quote, and it
//! reserves budget for every channel.
//!
//! The two seams (skeleton, `application/workflows/exchange/quote.rs`):
//! - `benefit_channel(channel) -> QuoteChannel`: `Booking` ⇒ `Desk`; every other channel is itself;
//! - `reserves_benefits(channel) -> bool`: `false` for `Booking` only.
//!
//! `QuoteService` uses both: the eligibility channel is `benefit_channel(req.channel)`, and when
//! `reserves_benefits(req.channel)` is false no `benefit_reservations` row is written (budget or coupon) and every
//! applied benefit stays in `benefit_snapshot.applied` with `reservationId` null. A rule's channels are DESK,
//! KIOSK or APP (`marketing-campaigns.md` §4.1 line 258), so no rule names BOOKING.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

mod common;
mod quote_common;

use chrono::{DateTime, Duration, Utc};
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_auth::domain::access::{Role as AccessRole, StandingKind};
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};
use xc_marketing::domain::coupon::code_hasher::CouponCodeHasher;
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xchange::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService, benefit_channel, reserves_benefits,
};
use xchange::application::workflows::save_step::SaveActor;

const ALL_CHANNELS: [QuoteChannel; 4] = [
    QuoteChannel::Desk,
    QuoteChannel::Kiosk,
    QuoteChannel::Booking,
    QuoteChannel::App,
];

fn teller() -> SaveActor {
    SaveActor {
        id: "teller-bqm3".to_owned(),
        username: "user-teller-bqm3".to_owned(),
        employee_id: Some("teller-bqm3".to_owned()),
        standing: StandingKind::Staff(AccessRole::Teller),
    }
}

fn hasher() -> CouponCodeHasher {
    common::install_test_pii_keys();
    let keys = xc_customer::get_pii_keys().expect("test PII keys installed");
    CouponCodeHasher::new(&keys)
}

/// USD 1 at 32.00 / 33.00 on a branch of its own.
async fn branch_with_usd(db: &TestDb, branch: &str) {
    db.branch(branch).await;
    xc_testdb::seed_branch_currencies(&db.pool, branch, &["USD"])
        .await
        .expect("seed branch currencies");
    db.publish(branch, &[Line::manual("USD", "1", "32.00", "33.00")])
        .await;
}

async fn user(db: &TestDb) -> Uuid {
    let id = Uuid::new_v4();
    db.exec(&format!(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ('{id}', '{ORG_1}'::uuid, '{}', 'ACTIVE', 'system')",
        unique("bqm3-user")
    ))
    .await;
    id
}

/// An APPROVED, running campaign on `branch` with one RATE_IMPROVEMENT rule (`FIXED_RATE` +0.50) of `eligibility`;
/// its rule id.
async fn campaign(db: &TestDb, branch: &str, eligibility: Value, requires_coupon: bool) -> Uuid {
    let by = user(db).await;
    let (campaign, version, rule) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    db.exec(&format!(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
         VALUES ('{campaign}', '{ORG_1}'::uuid, '{}', '{by}', now())",
        unique("bqm3-campaign")
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
         budget_thb, stack_group, priority, version_hash, edited_at, created_by, created_at) \
         VALUES ('{version}', '{ORG_1}'::uuid, '{campaign}', 1, 'APPROVED', now() - interval '1 day', \
         now() + interval '30 days', 5000.00, 'default', 1, 'hash_bqm3', now(), '{by}', now())"
    ))
    .await;
    db.exec(&format!(
        "UPDATE campaigns SET current_version_id = '{version}' WHERE id = '{campaign}'"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ('{version}', '{branch}')"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) VALUES ('{version}', 0, 0)"
    ))
    .await;
    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, \
         caps, requires_coupon, rule_no) \
         VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', $3, $4, '{}'::jsonb, $5, 1)",
    )
    .bind(rule)
    .bind(version)
    .bind(eligibility)
    .bind(json!({ "type": "FIXED_RATE", "delta": "0.50" }))
    .bind(requires_coupon)
    .execute(&db.pool)
    .await
    .expect("insert benefit_rules");
    rule
}

/// An ISSUED coupon `code` for `rule`, valid for a day; its id.
async fn coupon(db: &TestDb, rule: Uuid, code: &str) -> Uuid {
    let id = Uuid::new_v4();
    let by = user(db).await;
    let expires: DateTime<Utc> = Utc::now() + Duration::days(1);
    sqlx::query(
        "INSERT INTO coupons (id, organization_id, rule_id, code_hash, status, customer_id, expires_at, issued_by, \
         issued_at) VALUES ($1, $2::uuid, $3, $4, 'ISSUED', NULL, $5, $6, now())",
    )
    .bind(id)
    .bind(ORG_1)
    .bind(rule)
    .bind(&hasher().hash(code)[..])
    .bind(expires)
    .bind(by)
    .execute(&db.pool)
    .await
    .expect("insert coupon");
    id
}

/// A one-line BUY 100 USD quote on `channel` at `branch`, committed.
async fn quote(
    db: &TestDb,
    branch: &str,
    channel: QuoteChannel,
    coupon_code: Option<&str>,
) -> Quote {
    let req = QuoteRequest {
        branch_id: BranchId::new(branch),
        customer_id: None,
        channel,
        lines: vec![QuoteRequestLine {
            currency: CurrencyCode::parse("USD").expect("USD"),
            shop_side: Side::Buy,
            quantity: ForeignAmount::from_decimal(Decimal::from(100)),
            fee_thb: None,
            denomination_group: GroupCode::all(),
        }],
        payment_method: None,
    };
    let mut tx = db.pool.begin().await.expect("begin");
    let q = QuoteService::new(db.pool.clone())
        .quote_with_coupon_in(&mut tx, req, &teller(), Utc::now(), coupon_code)
        .await
        .unwrap_or_else(|e| panic!("quote on {channel:?}: {e}"));
    tx.commit().await.expect("commit");
    q
}

async fn reservations(db: &TestDb, q: &Quote) -> i64 {
    db.count(&format!(
        "SELECT count(*) FROM benefit_reservations WHERE quote_id = '{}'",
        q.id
    ))
    .await
}

fn applied(q: &Quote) -> Vec<Value> {
    q.benefit_snapshot
        .get("applied")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

#[test]
fn m1_a_booking_quote_is_judged_on_desk_and_every_other_channel_on_itself() {
    assert_eq!(benefit_channel(QuoteChannel::Booking), QuoteChannel::Desk);
    for c in [QuoteChannel::Desk, QuoteChannel::Kiosk, QuoteChannel::App] {
        assert_eq!(benefit_channel(c), c, "{c:?} is judged on itself");
    }
}

#[test]
fn m2_only_a_booking_quote_reserves_nothing() {
    for c in ALL_CHANNELS {
        assert_eq!(
            reserves_benefits(c),
            c != QuoteChannel::Booking,
            "{c:?} reserves benefits"
        );
    }
}

#[tokio::test]
async fn m3_a_booking_quote_shows_a_desk_rule_s_benefit_and_reserves_nothing() {
    let db = TestDb::start().await;
    let branch = "B-BQM3-3";
    branch_with_usd(&db, branch).await;
    campaign(
        &db,
        branch,
        json!({ "currencies": ["USD"], "channels": ["DESK"] }),
        false,
    )
    .await;

    let q = quote(&db, branch, QuoteChannel::Booking, None).await;

    assert_eq!(
        q.quote_type.as_str(),
        "INDICATIVE",
        "a booking quote is indicative"
    );
    assert_ne!(
        q.lines[0].final_rate, q.lines[0].base_rate,
        "the DESK rule applies to a staff booking quote (estimated benefit shown)"
    );
    let applied = applied(&q);
    assert_eq!(applied.len(), 1, "one applied benefit: {applied:?}");
    assert!(
        applied[0]["reservationId"].is_null(),
        "the applied benefit names no reservation: {}",
        applied[0]
    );
    assert_eq!(
        reservations(&db, &q).await,
        0,
        "no benefit_reservations row"
    );
    let stored: Value = sqlx::query_scalar("SELECT benefit_snapshot FROM quotes WHERE id = $1")
        .bind(q.id)
        .fetch_one(&db.pool)
        .await
        .expect("read quotes");
    assert_eq!(
        stored, q.benefit_snapshot,
        "the stored snapshot is the answered one"
    );
    db.finish().await;
}

#[tokio::test]
async fn m4_a_desk_quote_still_reserves_the_same_rule() {
    let db = TestDb::start().await;
    let branch = "B-BQM3-4";
    branch_with_usd(&db, branch).await;
    campaign(
        &db,
        branch,
        json!({ "currencies": ["USD"], "channels": ["DESK"] }),
        false,
    )
    .await;

    let q = quote(&db, branch, QuoteChannel::Desk, None).await;

    assert_ne!(q.lines[0].final_rate, q.lines[0].base_rate, "applied");
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM benefit_reservations WHERE quote_id = '{}' AND status = 'RESERVED'",
            q.id
        ))
        .await,
        1,
        "a DESK quote reserves as today"
    );
    assert!(
        !applied(&q)[0]["reservationId"].is_null(),
        "its applied benefit names the reservation"
    );
    db.finish().await;
}

#[tokio::test]
async fn m5_an_app_only_rule_does_not_apply_to_a_staff_booking_quote() {
    let db = TestDb::start().await;
    let branch = "B-BQM3-5";
    branch_with_usd(&db, branch).await;
    campaign(
        &db,
        branch,
        json!({ "currencies": ["USD"], "channels": ["APP"] }),
        false,
    )
    .await;

    let q = quote(&db, branch, QuoteChannel::Booking, None).await;

    assert_eq!(
        q.lines[0].final_rate, q.lines[0].base_rate,
        "a staff booking quote is judged on DESK, so an APP-only rule is skipped"
    );
    assert!(applied(&q).is_empty(), "nothing applied");
    assert_eq!(reservations(&db, &q).await, 0);
    db.finish().await;
}

#[tokio::test]
async fn m6_a_coupon_on_a_booking_quote_is_shown_but_not_locked() {
    let db = TestDb::start().await;
    let branch = "B-BQM3-6";
    branch_with_usd(&db, branch).await;
    let rule = campaign(
        &db,
        branch,
        json!({ "currencies": ["USD"], "requiresCoupon": true }),
        true,
    )
    .await;
    let code = unique("BQM3-CPN").to_uppercase();
    let coupon_id = coupon(&db, rule, &code).await;

    let q = quote(&db, branch, QuoteChannel::Booking, Some(&code)).await;

    assert_ne!(
        q.lines[0].final_rate, q.lines[0].base_rate,
        "the coupon's benefit is shown"
    );
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM benefit_reservations WHERE coupon_id = '{coupon_id}'"
        ))
        .await,
        0,
        "no coupon lock"
    );
    assert_eq!(reservations(&db, &q).await, 0, "no reservation at all");
    assert_eq!(
        db.text(&format!(
            "SELECT status FROM coupons WHERE id = '{coupon_id}'"
        ))
        .await
        .as_deref(),
        Some("ISSUED"),
        "the coupon stays ISSUED"
    );

    // The completion's DESK quote then locks it, as today.
    let desk = quote(&db, branch, QuoteChannel::Desk, Some(&code)).await;
    assert_eq!(
        db.count(&format!(
            "SELECT count(*) FROM benefit_reservations WHERE coupon_id = '{coupon_id}' AND quote_id = '{}'",
            desk.id
        ))
        .await,
        1,
        "the DESK quote locks the coupon"
    );
    db.finish().await;
}
