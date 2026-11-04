// Written before the implementation by the ticket author claude/e2926a49 (sub-controller
// XC-BOOKING-ESTIMATE under the xchange BE lane controller claude/553782). Do not edit, delete or weaken.

//! Up-front tests for BE-5, the typed quote answer (design `marketing-campaigns.md` §6.4.3 `QuoteDto`, class
//! diagram §6.4.4 `QuoteBenefitDto`, behaviour table §6.4.5). `QuoteDto.benefits` becomes `QuoteBenefitDto[]`, built
//! from the snapshot's `applied` (`AppliedBenefit`, `xc-marketing/src/domain/engine/snapshot.rs`) field for field, and
//! `QuoteDto.benefitBasis` is `"INDICATIVE"` when the channel does not reserve (`reserves_benefits`), else
//! `"RESERVED"`, empty list or not.
//!
//! Rows of §6.4.5 covered here:
//! - 17: a `BOOKING` quote with an applied benefit ⇒ `benefitBasis = "INDICATIVE"`, `benefits[0].reservationId` is
//!   `null`; a `DESK` quote ⇒ `"RESERVED"`, `reservationId` is the reservation's id. Every item has exactly the
//!   twelve keys of `QuoteBenefitDto`.
//! - 18: a `DESK` quote with no benefit ⇒ `benefits = []`, `benefitBasis = "RESERVED"` (and a `BOOKING` one
//!   `"INDICATIVE"`).
//! - 19: a stored snapshot whose `applied` entry lacks `amountThb` ⇒ `QuoteDto::from_domain` is
//!   `AppError::Internal` naming the quote (the commands answer it as a 500 `serverError`); nothing drops the entry.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

mod common;
mod quote_common;

use chrono::Utc;
use pos_error::AppError;
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_auth::domain::access::{Role as AccessRole, StandingKind};
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xchange::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use xchange::application::workflows::save_step::SaveActor;
use xchange::presentation::dto::v3::quote::QuoteDto;

/// The keys of one `QuoteBenefitDto` (§6.4.3), sorted.
const BENEFIT_KEYS: [&str; 12] = [
    "amountThb",
    "benefitClass",
    "benefitType",
    "couponId",
    "feeAfter",
    "feeBefore",
    "floorCapped",
    "rateAfter",
    "rateBefore",
    "reservationId",
    "ruleId",
    "versionId",
];

fn teller() -> SaveActor {
    SaveActor {
        id: "teller-be5".to_owned(),
        username: "user-teller-be5".to_owned(),
        employee_id: Some("teller-be5".to_owned()),
        standing: StandingKind::Staff(AccessRole::Teller),
    }
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

/// An APPROVED, running campaign on `branch` with one PRICE `RATE_IMPROVEMENT` rule (`FIXED_RATE` +0.50) that
/// admits DESK (so a BOOKING quote, judged on DESK, shows it too); its rule id.
async fn desk_campaign(db: &TestDb, branch: &str) -> Uuid {
    let by = Uuid::new_v4();
    db.exec(&format!(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ('{by}', '{ORG_1}'::uuid, '{}', 'ACTIVE', 'system')",
        unique("be5-user")
    ))
    .await;
    let (campaign, version, rule) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    db.exec(&format!(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
         VALUES ('{campaign}', '{ORG_1}'::uuid, '{}', '{by}', now())",
        unique("be5-campaign")
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
         budget_thb, stack_group, priority, version_hash, edited_at, created_by, created_at) \
         VALUES ('{version}', '{ORG_1}'::uuid, '{campaign}', 1, 'APPROVED', now() - interval '1 day', \
         now() + interval '30 days', 5000.00, 'default', 1, 'hash_be5', now(), '{by}', now())"
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
         VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', $3, $4, '{}'::jsonb, false, 1)",
    )
    .bind(rule)
    .bind(version)
    .bind(json!({ "currencies": ["USD"], "channels": ["DESK"] }))
    .bind(json!({ "type": "FIXED_RATE", "delta": "0.50" }))
    .execute(&db.pool)
    .await
    .expect("insert benefit_rules");
    rule
}

/// A one-line BUY 100 USD quote on `channel` at `branch`, committed.
async fn quote(db: &TestDb, branch: &str, channel: QuoteChannel) -> Quote {
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
        .quote_in(&mut tx, req, &teller(), Utc::now())
        .await
        .unwrap_or_else(|e| panic!("quote on {channel:?}: {e}"));
    tx.commit().await.expect("commit");
    q
}

/// The quote's answer as the wire carries it.
fn answer(q: Quote) -> Value {
    let dto = QuoteDto::from_domain(q, 1).expect("the quote's answer");
    serde_json::to_value(dto).expect("QuoteDto serialises")
}

fn benefit_keys(item: &Value) -> Vec<String> {
    let mut keys: Vec<String> = item
        .as_object()
        .unwrap_or_else(|| panic!("a benefit is an object: {item}"))
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

#[tokio::test]
async fn be5_row17_a_booking_quote_is_indicative_and_a_desk_quote_reserved() {
    let db = TestDb::start().await;
    let branch = "B-BE5-17";
    branch_with_usd(&db, branch).await;
    let rule = desk_campaign(&db, branch).await;

    let booking = answer(quote(&db, branch, QuoteChannel::Booking).await);
    assert_eq!(
        booking["benefitBasis"],
        json!("INDICATIVE"),
        "a BOOKING quote reserves nothing: {booking}"
    );
    let item = &booking["benefits"][0];
    assert_eq!(
        benefit_keys(item),
        BENEFIT_KEYS,
        "a QuoteBenefitDto: {item}"
    );
    assert_eq!(item["ruleId"], json!(rule.to_string()));
    assert_eq!(item["reservationId"], Value::Null, "no reservation: {item}");
    assert_eq!(item["couponId"], Value::Null);
    assert_eq!(item["benefitClass"], json!("PRICE"));
    assert_eq!(item["amountThb"], json!("50.00"));
    assert_eq!(item["floorCapped"], json!(false));

    let desk_quote = quote(&db, branch, QuoteChannel::Desk).await;
    let reservation = db
        .text(&format!(
            "SELECT id::text FROM benefit_reservations WHERE quote_id = '{}'",
            desk_quote.id
        ))
        .await
        .expect("the DESK quote reserved its benefit");
    let desk = answer(desk_quote);
    assert_eq!(
        desk["benefitBasis"],
        json!("RESERVED"),
        "a DESK quote reserves: {desk}"
    );
    let item = &desk["benefits"][0];
    assert_eq!(
        benefit_keys(item),
        BENEFIT_KEYS,
        "a QuoteBenefitDto: {item}"
    );
    assert_eq!(
        item["reservationId"],
        json!(reservation),
        "the reservation's id: {item}"
    );
    db.finish().await;
}

#[tokio::test]
async fn be5_row18_a_quote_with_no_benefit_still_carries_its_basis() {
    let db = TestDb::start().await;
    let branch = "B-BE5-18";
    branch_with_usd(&db, branch).await;

    let desk = answer(quote(&db, branch, QuoteChannel::Desk).await);
    assert_eq!(desk["benefits"], json!([]), "{desk}");
    assert_eq!(desk["benefitBasis"], json!("RESERVED"), "{desk}");

    let booking = answer(quote(&db, branch, QuoteChannel::Booking).await);
    assert_eq!(booking["benefits"], json!([]), "{booking}");
    assert_eq!(booking["benefitBasis"], json!("INDICATIVE"), "{booking}");
    db.finish().await;
}

#[tokio::test]
async fn be5_row19_an_applied_entry_that_does_not_decode_is_a_server_error_naming_the_quote() {
    let db = TestDb::start().await;
    let branch = "B-BE5-19";
    branch_with_usd(&db, branch).await;
    desk_campaign(&db, branch).await;

    let mut q = quote(&db, branch, QuoteChannel::Booking).await;
    let id = q.id;
    let entry = q
        .benefit_snapshot
        .get_mut("applied")
        .and_then(Value::as_array_mut)
        .and_then(|a| a.first_mut())
        .and_then(Value::as_object_mut)
        .expect("the quote has an applied benefit");
    assert!(
        entry.remove("amountThb").is_some(),
        "the entry had amountThb"
    );

    match QuoteDto::from_domain(q, 1) {
        Err(AppError::Internal(message)) => assert!(
            message.contains(&id.to_string()),
            "the error names the quote {id}: {message}"
        ),
        Err(other) => panic!("expected AppError::Internal naming the quote, got {other:?}"),
        Ok(dto) => panic!(
            "an entry without amountThb must not be answered; got benefits {}",
            serde_json::to_string(&dto.benefits).expect("json")
        ),
    }
    db.finish().await;
}
