//! Ticket LY-1 (#1381) up-front tests — `LoyaltyEvents::completed_in` / `reversed_in` write the two loyalty
//! outbox rows through OB-1's `OutboxWriter::enqueue_in` (design `membership-points.md` D-4, §4.4, §9.3 row
//! LY-1). Written before the implementation by the ticket author (claude/29df2ec5). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Narrowed by the lane controller's ruling on #1381 (claude/ad05bf8d, 2026-10-12): "a customer without a
//! verified phone enqueues nothing" is LY-3's (`LoyaltyEffect`, D-5), not tested here.
//!
//! Red on `main` by result: the scaffold's `completed_in` / `reversed_in` answer an error and its event
//! kinds are empty.

mod common;

use std::str::FromStr;

use chrono::{NaiveDate, TimeZone, Utc};
use common::*;
use rust_decimal::Decimal;
use serde_json::json;
use uuid::Uuid;
use xc_kernel::Mode;
use xc_kernel::money::Side;
use xc_loyalty::application::events::LoyaltyEvents;
use xc_loyalty::domain::events::{LOYALTY_COMPLETED, LOYALTY_REVERSED};
use xc_loyalty::domain::facts::{CompletedFacts, CompletedLine, ReversedFacts};

fn org() -> Uuid {
    Uuid::parse_str(ORG).expect("ORG is a uuid")
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal literal")
}

/// A LIVE completed exchange of a customer, with a fresh transaction id.
fn completed(mode: Mode, customer: Option<&str>) -> CompletedFacts {
    CompletedFacts {
        organization_id: org(),
        transaction_id: Uuid::new_v4(),
        document_ref: unique("XC"),
        customer_id: customer.map(str::to_owned),
        mode,
        branch_id: "XC-B01".to_owned(),
        completed_at: Utc.with_ymd_and_hms(2026, 10, 1, 3, 4, 5).unwrap(),
        business_date: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        reversible_until: Utc.with_ymd_and_hms(2026, 10, 1, 16, 59, 59).unwrap(),
        lines: vec![CompletedLine {
            currency: "USD".to_owned(),
            shop_side: Side::Buy,
            quantity: dec("100"),
            unit_size: dec("1"),
            thb_amount: dec("3400"),
            settle_thb: dec("3400"),
            realized_margin_thb: None,
        }],
    }
}

fn reversed(of: &CompletedFacts) -> ReversedFacts {
    ReversedFacts {
        organization_id: of.organization_id,
        reversal_id: Uuid::new_v4(),
        transaction_id: of.transaction_id,
        customer_id: of.customer_id.clone(),
        posted_at: Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap(),
        amount_thb: dec("3400"),
        full: true,
    }
}

async fn complete_and_commit(db: &TestDb, facts: &CompletedFacts) {
    let mut tx = db.begin().await;
    LoyaltyEvents::completed_in(&mut tx, facts)
        .await
        .expect("completed_in answers Ok");
    tx.commit().await.expect("commit");
}

async fn reverse_and_commit(db: &TestDb, facts: &ReversedFacts) {
    let mut tx = db.begin().await;
    LoyaltyEvents::reversed_in(&mut tx, facts)
        .await
        .expect("reversed_in answers Ok");
    tx.commit().await.expect("commit");
}

#[test]
fn the_two_event_kinds_are_the_designs_and_share_one_destination() {
    assert_eq!(LOYALTY_COMPLETED.event_type, "loyalty.exchange_completed");
    assert_eq!(LOYALTY_REVERSED.event_type, "loyalty.exchange_reversed");
    assert_eq!(LOYALTY_COMPLETED.aggregate_type, "transaction");
    assert_eq!(LOYALTY_REVERSED.aggregate_type, "transaction");
    // OB-1's FIFO is per (aggregate, destination): a reversal can wait behind its earn only if both
    // go to the same destination.
    assert!(
        !LOYALTY_COMPLETED.destination.is_empty(),
        "a destination is named"
    );
    assert_eq!(LOYALTY_COMPLETED.destination, LOYALTY_REVERSED.destination);
}

#[tokio::test]
async fn a_live_completion_with_a_customer_enqueues_one_row_with_references_only() {
    let db = TestDb::start().await;
    let facts = completed(Mode::Live, Some(&unique("cust")));
    complete_and_commit(&db, &facts).await;

    let rows = db.rows().await;
    assert_eq!(rows.len(), 1, "exactly one row: {rows:?}");
    let row = &rows[0];
    let tx_id = facts.transaction_id.to_string();
    assert_eq!(row.organization_id, ORG);
    assert_eq!(row.event_type, "loyalty.exchange_completed");
    assert_eq!(row.destination, LOYALTY_COMPLETED.destination);
    assert_eq!(row.aggregate_type, "transaction");
    assert_eq!(row.aggregate_id, tx_id);
    assert_eq!(row.idempotency_key, format!("exchange:{tx_id}"));
    assert_eq!(row.status, "PENDING");
    assert_eq!(
        row.payload_redacted,
        json!({ "transactionId": tx_id }),
        "the payload is the transaction reference and nothing else"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_completion_whose_transaction_rolls_back_leaves_no_row() {
    let db = TestDb::start().await;
    let facts = completed(Mode::Live, Some(&unique("cust")));
    let mut tx = db.begin().await;
    LoyaltyEvents::completed_in(&mut tx, &facts)
        .await
        .expect("completed_in answers Ok");
    tx.rollback().await.expect("rollback");
    assert_eq!(
        db.count().await,
        0,
        "the row is in the caller's transaction"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_same_completion_twice_is_one_row() {
    let db = TestDb::start().await;
    let facts = completed(Mode::Live, Some(&unique("cust")));
    complete_and_commit(&db, &facts).await;
    complete_and_commit(&db, &facts).await;
    assert_eq!(db.count().await, 1, "a second call is a no-op");
    db.finish().await;
}

#[tokio::test]
async fn a_demo_completion_enqueues_nothing() {
    let db = TestDb::start().await;
    let facts = completed(Mode::Demo, Some(&unique("cust")));
    complete_and_commit(&db, &facts).await;
    assert_eq!(db.count().await, 0, "test data never earns");
    db.finish().await;
}

#[tokio::test]
async fn a_completion_without_a_customer_enqueues_nothing() {
    let db = TestDb::start().await;
    let facts = completed(Mode::Live, None);
    complete_and_commit(&db, &facts).await;
    assert_eq!(db.count().await, 0, "nobody to credit");
    db.finish().await;
}

#[tokio::test]
async fn a_reversal_is_keyed_by_the_reversal_and_aggregated_on_the_original_transaction() {
    let db = TestDb::start().await;
    let earn = completed(Mode::Live, Some(&unique("cust")));
    complete_and_commit(&db, &earn).await;
    let rev = reversed(&earn);
    reverse_and_commit(&db, &rev).await;

    let rows = db.rows().await;
    assert_eq!(rows.len(), 2, "the earn and the reversal: {rows:?}");
    let row = rows
        .iter()
        .find(|r| r.event_type == "loyalty.exchange_reversed")
        .expect("a loyalty.exchange_reversed row");
    let original = earn.transaction_id.to_string();
    let rev_id = rev.reversal_id.to_string();
    assert_eq!(row.organization_id, ORG);
    assert_eq!(row.aggregate_type, "transaction");
    assert_eq!(
        row.aggregate_id, original,
        "the original transaction, so FIFO delivers it after the earn"
    );
    assert_eq!(row.idempotency_key, format!("reversal:{rev_id}"));
    assert_eq!(row.destination, LOYALTY_REVERSED.destination);
    assert_eq!(row.payload_redacted, json!({ "reversalId": rev_id }));
    let earn_row = rows
        .iter()
        .find(|r| r.event_type == "loyalty.exchange_completed")
        .expect("the earn row");
    assert_eq!(
        (
            earn_row.aggregate_type.as_str(),
            earn_row.aggregate_id.as_str(),
            earn_row.destination.as_str()
        ),
        (
            row.aggregate_type.as_str(),
            row.aggregate_id.as_str(),
            row.destination.as_str()
        ),
        "the earn and its reversal share one FIFO lane"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_same_reversal_twice_is_one_row_and_a_rolled_back_one_is_none() {
    let db = TestDb::start().await;
    let earn = completed(Mode::Live, Some(&unique("cust")));
    let rev = reversed(&earn);
    reverse_and_commit(&db, &rev).await;
    reverse_and_commit(&db, &rev).await;
    assert_eq!(db.count().await, 1, "a second call is a no-op");

    let other = reversed(&earn);
    let mut tx = db.begin().await;
    LoyaltyEvents::reversed_in(&mut tx, &other)
        .await
        .expect("reversed_in answers Ok");
    tx.rollback().await.expect("rollback");
    assert_eq!(
        db.count().await,
        1,
        "the rolled-back reversal wrote nothing"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_stored_rows_hold_no_personal_data() {
    let db = TestDb::start().await;
    let customer = unique("cust");
    let earn = completed(Mode::Live, Some(&customer));
    complete_and_commit(&db, &earn).await;
    reverse_and_commit(&db, &reversed(&earn)).await;

    let rows = db.rows().await;
    assert_eq!(rows.len(), 2);
    for row in &rows {
        let text = db.row_text(&row.id).await;
        for word in [
            "phone",
            "passport",
            "name",
            "lookup",
            "nationality",
            "customer",
            "+66",
            customer.as_str(),
        ] {
            assert!(
                !text.to_lowercase().contains(&word.to_lowercase()),
                "stored row {} mentions {word:?}: {text}",
                row.id
            );
        }
    }
    db.finish().await;
}
