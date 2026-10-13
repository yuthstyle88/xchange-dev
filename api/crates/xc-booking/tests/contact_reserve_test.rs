//! BK-3 #1380 — `ContactReserve` integration tests (design §5, B6, §9.1).
//!
//! "DESK with a new phone creates one customer with an unverified PHONE identity, and the same
//! phone again finds it"

mod booking_common;

use chrono::Utc;
use uuid::Uuid;

use booking_common::{ORG_1, TestDb};

#[tokio::test]
async fn new_phone_at_desk_creates_customer_with_unverified_phone_identity() {
    let db = TestDb::start().await;
    let contact_reserve = db.contact_reserve();
    let org_id = Uuid::parse_str(ORG_1).unwrap();
    let now = Utc::now();
    let phone = "+66812345678";
    let name = "Somchai Desk";

    let mut tx = db.pool.begin().await.expect("begin tx");
    let reservation = contact_reserve
        .reserve_in(&mut tx, org_id, name, phone, now)
        .await
        .expect("reserve customer");
    tx.commit().await.expect("commit tx");

    assert!(
        !reservation.existing,
        "first reservation creates new customer"
    );
    assert!(
        reservation.customer_id.starts_with("CX-"),
        "customer id starts with CX-"
    );

    // Verify customer and identity row in database
    let cust_count = db
        .count(&format!(
            "SELECT count(*) FROM customers WHERE id = '{}' AND organization_id = '{ORG_1}'",
            reservation.customer_id
        ))
        .await;
    assert_eq!(cust_count, 1, "customer created");

    let ident_count = db
        .count(&format!(
            "SELECT count(*) FROM customer_identities WHERE customer_id = '{}' AND type = 'PHONE' \
             AND verified_at IS NULL AND status = 'ACTIVE'",
            reservation.customer_id
        ))
        .await;
    assert_eq!(ident_count, 1, "unverified PHONE identity created");

    db.finish().await;
}

#[tokio::test]
async fn same_phone_twice_at_desk_finds_same_customer() {
    let db = TestDb::start().await;
    let contact_reserve = db.contact_reserve();
    let org_id = Uuid::parse_str(ORG_1).unwrap();
    let now = Utc::now();
    let phone = "+66898765432";
    let name1 = "First Teller Caller";
    let name2 = "Second Teller Caller";

    let mut tx1 = db.pool.begin().await.expect("begin tx 1");
    let res1 = contact_reserve
        .reserve_in(&mut tx1, org_id, name1, phone, now)
        .await
        .expect("first reserve");
    tx1.commit().await.expect("commit tx 1");

    let mut tx2 = db.pool.begin().await.expect("begin tx 2");
    let res2 = contact_reserve
        .reserve_in(&mut tx2, org_id, name2, phone, now)
        .await
        .expect("second reserve");
    tx2.commit().await.expect("commit tx 2");

    assert!(!res1.existing);
    assert!(res2.existing, "second call finds existing customer");
    assert_eq!(
        res1.customer_id, res2.customer_id,
        "both calls share same customer id"
    );

    db.finish().await;
}
