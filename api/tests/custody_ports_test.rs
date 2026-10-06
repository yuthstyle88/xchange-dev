//! Specification tests for T10b: `PgCustodyPorts`, what the custody service
//! learns from the other contexts.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! `xc_shift`'s `CustodyPorts` asks four things the shift context may not look
//! up itself: the state of the rates and of the exchanges of a branch, whether a
//! customer is known, whether files are the caller's evidence, what role an
//! employee has. `PgCustodyPorts` answers them from the tables of the other
//! contexts, reading through the transaction it is handed.
//!
//! The rules are the old server's (its custody code read them off the admin
//! snapshot); each test says which.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose
//! name ends in `_test`). Without it they FAIL; none skips.

// the shop of the save tests: its helpers build the rows these tests read
#[allow(unused_imports)]
mod save_common;

use std::sync::Arc;

use save_common::*;
use xc_shift::domain::custody::repositories::custody_ports::{CustodyPorts, OutsideFacts};
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xchange::infrastructure::custody::custody_ports::PgCustodyPorts;

const B01: &str = "XC-B01";

/// The ports as the custody service holds them.
fn ports() -> Arc<dyn CustodyPorts> {
    Arc::new(PgCustodyPorts)
}

async fn outside(shop: &Shop, branch_id: &str, shift: Option<&str>) -> OutsideFacts {
    let mut tx = shop.pool.begin().await.expect("begin");
    let facts = ports()
        .outside_facts(&mut tx, &branch(branch_id), shift)
        .await
        .expect("outside_facts");
    tx.rollback().await.expect("rollback");
    facts
}

async fn known(shop: &Shop, branch_id: &str, customer_id: &str) -> bool {
    let mut tx = shop.pool.begin().await.expect("begin");
    let got = ports()
        .customer_known(&mut tx, &branch(branch_id), customer_id)
        .await
        .expect("customer_known");
    tx.rollback().await.expect("rollback");
    got
}

async fn owned(shop: &Shop, branch_id: &str, owner: &str, keys: &[&str]) -> bool {
    let keys: Vec<String> = keys.iter().map(|k| (*k).to_owned()).collect();
    let mut tx = shop.pool.begin().await.expect("begin");
    let got = ports()
        .evidence_owned(&mut tx, &branch(branch_id), owner, &keys)
        .await
        .expect("evidence_owned");
    tx.rollback().await.expect("rollback");
    got
}

async fn role(shop: &Shop, branch_id: &str, employee_id: &str) -> Option<CustodyRole> {
    let mut tx = shop.pool.begin().await.expect("begin");
    let got = ports()
        .employee_role(&mut tx, &branch(branch_id), employee_id)
        .await
        .expect("employee_role");
    tx.rollback().await.expect("rollback");
    got
}

/// A rate row written at a given time (the table is ticket T5a's).
async fn rate_at(shop: &Shop, branch_id: &str, currency: &str, updated_at: &str) {
    sqlx::query(
        "INSERT INTO rates (branch_id, code, buy, sell, updated_at, updated_by, request_id) \
         VALUES ($1, $2, 32.1, 33, $3::timestamptz, 'test', $4)",
    )
    .bind(branch_id)
    .bind(currency)
    .bind(updated_at)
    .bind(unique("rate"))
    .execute(&shop.pool)
    .await
    .expect("insert into rates");
}

/// An employee (the table is ticket T4b's).
async fn employee(
    shop: &Shop,
    id: &str,
    role: &str,
    branch_id: &str,
    active: bool,
    must_change: bool,
) {
    sqlx::query(
        "INSERT INTO employees (id, username, name, role, branch_id, password_hash, active, must_change, created_at) \
         VALUES ($1, $2, $3, $4, $5, 'x', $6, $7, now())",
    )
    .bind(id)
    .bind(format!("user-{id}"))
    .bind(format!("Name {id}"))
    .bind(role)
    .bind(branch_id)
    .bind(active)
    .bind(must_change)
    .execute(&shop.pool)
    .await
    .expect("insert into employees");
}

// ---------------------------------------------------------------- the rates

#[tokio::test]
async fn the_rates_are_live_once_the_branch_has_one_and_dated_by_the_last_written() {
    // v2: a live rate is any currency with a buy and a sell rate; "updated" is
    // the time of the last rate row written in the branch, as the screens print it
    let shop = Shop::start().await;
    shop.seed_branch(B01, "Second").await;

    let none = outside(&shop, MAIN, None).await;
    assert!(!none.has_live_rate);
    assert_eq!(none.rates_updated, "", "no rate yet: the empty text");

    // a rate of another branch is not this branch's
    rate_at(&shop, B01, "USD", "2026-10-06T04:00:00.000Z").await;
    let still = outside(&shop, MAIN, None).await;
    assert_eq!(
        (still.has_live_rate, still.rates_updated.as_str()),
        (false, "")
    );
    let other = outside(&shop, B01, None).await;
    assert_eq!(
        (other.has_live_rate, other.rates_updated.as_str()),
        (true, "2026-10-06T04:00:00.000Z")
    );

    rate_at(&shop, MAIN, "USD", "2026-10-06T05:00:00.123Z").await;
    let one = outside(&shop, MAIN, None).await;
    assert!(one.has_live_rate);
    assert_eq!(
        one.rates_updated, "2026-10-06T05:00:00.123Z",
        "milliseconds and Z, as v2 prints a time"
    );

    // the last row written counts, whatever its currency — and even when its
    // time is earlier than another row's
    rate_at(&shop, MAIN, "JPY", "2026-10-06T12:30:00.5+07:00").await;
    assert_eq!(
        outside(&shop, MAIN, None).await.rates_updated,
        "2026-10-06T05:30:00.500Z"
    );
    rate_at(&shop, MAIN, "USD", "2026-10-06T01:02:03.004Z").await;
    assert_eq!(
        outside(&shop, MAIN, None).await.rates_updated,
        "2026-10-06T01:02:03.004Z"
    );
    // the other branch did not move
    assert_eq!(
        outside(&shop, B01, None).await.rates_updated,
        "2026-10-06T04:00:00.000Z"
    );
    shop.finish().await;
}

// ------------------------------------------------------------ the exchanges

#[tokio::test]
async fn the_last_exchange_is_the_last_live_one_of_the_shift_asked_about() {
    // v2: the last of the transactions whose shift is the open one; `null` when there is none
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let first_shift = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;
    let demo_person = ReserveCustomer {
        mode: "DEMO".to_owned(),
        ..person()
    };
    let demo_customer = shop.reserve(MAIN, demo_person.clone()).await;

    // no shift asked about; a shift with no exchange yet
    assert_eq!(outside(&shop, MAIN, None).await.last_transaction, None);
    assert_eq!(
        outside(&shop, MAIN, Some(&first_shift))
            .await
            .last_transaction,
        None
    );

    let one = shop
        .save(
            MAIN,
            form("last-0001-aaaaaaaaaaa", &customer, &passport),
            at(0),
        )
        .await
        .unwrap();
    assert_eq!(
        outside(&shop, MAIN, Some(&first_shift))
            .await
            .last_transaction,
        Some(one.document_ref.clone())
    );
    let mut second = form("last-0002-aaaaaaaaaaa", &customer, &passport);
    second.amount = "1".to_owned();
    let two = shop.save(MAIN, second, at(1)).await.unwrap();
    assert_eq!(
        (one.document_ref.as_str(), two.document_ref.as_str()),
        ("XC-20261006-000001", "XC-20261006-000002")
    );
    assert_eq!(
        outside(&shop, MAIN, Some(&first_shift))
            .await
            .last_transaction,
        Some("XC-20261006-000002".to_owned())
    );

    // a demo exchange belongs to no shift and is never the last one
    let mut demo = form("last-0003-aaaaaaaaaaa", &demo_customer, &passport);
    demo.customer = demo_person;
    let saved = shop.save(MAIN, demo, at(2)).await.unwrap();
    assert_eq!(saved.document_ref, "TEST-XC-20261006-000001");
    assert_eq!(
        outside(&shop, MAIN, Some(&first_shift))
            .await
            .last_transaction,
        Some("XC-20261006-000002".to_owned())
    );
    // without a shift there is no last exchange, whatever was saved
    assert_eq!(outside(&shop, MAIN, None).await.last_transaction, None);

    // a new shift starts without one; the closed shift keeps its own
    shop.close_shift(MAIN).await;
    let second_shift = shop
        .open_shift(MAIN, &[("THB", "100000"), ("USD", "1000")])
        .await;
    assert_ne!(second_shift, first_shift);
    assert_eq!(
        outside(&shop, MAIN, Some(&second_shift))
            .await
            .last_transaction,
        None
    );
    let mut third = form("last-0004-aaaaaaaaaaa", &customer, &passport);
    third.amount = "2".to_owned();
    let three = shop.save(MAIN, third, at(3)).await.unwrap();
    assert_eq!(
        outside(&shop, MAIN, Some(&second_shift))
            .await
            .last_transaction,
        Some(three.document_ref.clone())
    );
    assert_eq!(
        outside(&shop, MAIN, Some(&first_shift))
            .await
            .last_transaction,
        Some("XC-20261006-000002".to_owned())
    );

    // a shift is asked about in its own branch
    shop.seed_branch(B01, "Second").await;
    assert_eq!(
        outside(&shop, B01, Some(&second_shift))
            .await
            .last_transaction,
        None
    );
    assert_eq!(
        outside(&shop, MAIN, Some("SH-no-such-shift"))
            .await
            .last_transaction,
        None
    );
    shop.finish().await;
}

#[tokio::test]
async fn no_exchange_is_ever_pending() {
    // v2 kept an exchange PENDING while its files were being stored. Here a save
    // is one transaction: an exchange exists whole or not at all, so the list
    // of pending ones is always empty.
    let shop = Shop::start().await;
    assert!(
        outside(&shop, MAIN, None)
            .await
            .pending_transactions
            .is_empty()
    );
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let shift = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;
    shop.save(
        MAIN,
        form("pend-0001-aaaaaaaaaaa", &customer, &passport),
        at(0),
    )
    .await
    .unwrap();
    let facts = outside(&shop, MAIN, Some(&shift)).await;
    assert!(facts.pending_transactions.is_empty(), "{facts:?}");
    assert_eq!(
        facts,
        OutsideFacts {
            has_live_rate: true,
            rates_updated: facts.rates_updated.clone(),
            pending_transactions: vec![],
            last_transaction: Some("XC-20261006-000001".to_owned()),
        }
    );
    shop.finish().await;
}

// ------------------------------------------------------------- the customer

#[tokio::test]
async fn a_customer_is_known_when_live_in_that_branch() {
    // v2: the customers of the snapshot — those of the branch, live ones only
    let shop = Shop::start().await;
    shop.seed_branch(B01, "Second").await;
    let live = shop.reserve(MAIN, person()).await;
    let demo = shop
        .reserve(
            MAIN,
            ReserveCustomer {
                mode: "DEMO".to_owned(),
                ..person()
            },
        )
        .await;
    let elsewhere = shop
        .reserve(
            B01,
            ReserveCustomer {
                passport_no: "BB7654321".to_owned(),
                ..person()
            },
        )
        .await;
    assert_eq!(
        (live.as_str(), demo.as_str(), elsewhere.as_str()),
        ("CX-00000001", "TEST-CX-00000001", "CX-00000002")
    );

    assert!(known(&shop, MAIN, &live).await);
    assert!(
        !known(&shop, MAIN, &demo).await,
        "a demo customer is not a customer"
    );
    assert!(
        !known(&shop, MAIN, &elsewhere).await,
        "a customer of another branch"
    );
    assert!(known(&shop, B01, &elsewhere).await);
    assert!(!known(&shop, B01, &live).await);
    for unknown in [
        "CX-99999999",
        "",
        "cx-00000001",
        " CX-00000001",
        "CX-00000001 ",
        "%",
    ] {
        assert!(!known(&shop, MAIN, unknown).await, "{unknown:?}");
    }
    shop.finish().await;
}

// ------------------------------------------------------------- the evidence

#[tokio::test]
async fn evidence_is_owned_when_every_key_is_the_callers_upload_in_that_branch() {
    // v2: for each key, a file record with that key, that owner and that branch
    let shop = Shop::start().await;
    shop.seed_branch(B01, "Second").await;
    let staff = "employee-7";
    let a = shop.upload(MAIN, staff, "image/jpeg", JPEG).await;
    let b = shop.upload(MAIN, staff, "application/pdf", PDF).await;
    let others = shop.upload(MAIN, OWNER, "image/jpeg", JPEG).await;
    let elsewhere = shop.upload(B01, staff, "image/jpeg", JPEG).await;
    let never = "0f8fad5b-d9cb-469f-a165-70867728950e";

    assert!(owned(&shop, MAIN, staff, &[&a]).await);
    assert!(owned(&shop, MAIN, staff, &[&a, &b]).await);
    assert!(
        owned(&shop, MAIN, staff, &[&b, &a, &a]).await,
        "a key named twice is still owned"
    );
    assert!(
        owned(&shop, MAIN, staff, &[]).await,
        "nothing to own: how many are needed is the service's rule"
    );
    assert!(owned(&shop, B01, staff, &[&elsewhere]).await);
    assert!(owned(&shop, MAIN, OWNER, &[&others]).await);

    // one key that is not theirs spoils the lot, wherever it stands
    for bad in [others.as_str(), elsewhere.as_str(), never, "", "x", "%"] {
        assert!(!owned(&shop, MAIN, staff, &[bad]).await, "{bad:?} alone");
        assert!(!owned(&shop, MAIN, staff, &[&a, bad]).await, "{bad:?} last");
        assert!(
            !owned(&shop, MAIN, staff, &[bad, &a, &b]).await,
            "{bad:?} first"
        );
    }
    assert!(
        !owned(&shop, MAIN, "Employee-7", &[&a]).await,
        "owners are compared exactly"
    );
    assert!(!owned(&shop, B01, staff, &[&a]).await);

    // v2 asks only for the record: bytes gone from the disk do not change the answer
    std::fs::remove_file(shop.dir.join(&a)).expect("remove an uploaded file");
    assert!(owned(&shop, MAIN, staff, &[&a]).await);
    shop.finish().await;
}

// -------------------------------------------------------------- the employee

#[tokio::test]
async fn an_employees_role_is_told_for_an_active_employee_of_that_branch() {
    // v2: SELECT id, role FROM employees WHERE id = ? AND branch_id = ? AND active
    let shop = Shop::start().await;
    shop.seed_branch(B01, "Second").await;
    let table = [
        ("e-front", "FRONT", Some(CustodyRole::Front)),
        ("e-finance", "FINANCE", Some(CustodyRole::Finance)),
        ("e-monitor", "MONITOR", Some(CustodyRole::Monitor)),
        ("e-manager", "MANAGER", Some(CustodyRole::Manager)),
        ("e-backend", "BACKEND", Some(CustodyRole::Backend)),
        // an employee whose role has no part in custody is still an employee
        ("e-rates", "RATES", Some(CustodyRole::Other)),
        ("e-system", "SYSTEM", Some(CustodyRole::Other)),
        ("e-documents", "DOCUMENTS", Some(CustodyRole::Other)),
        ("e-review", "DOCUMENT_REVIEW", Some(CustodyRole::Other)),
        // an employee is never the owner, whatever the row says
        ("e-owner", "OWNER", Some(CustodyRole::Other)),
        ("e-lower", "front", Some(CustodyRole::Other)),
    ];
    for (id, role_text, _) in table {
        employee(&shop, id, role_text, MAIN, true, false).await;
    }
    employee(&shop, "e-inactive", "FRONT", MAIN, false, false).await;
    employee(&shop, "e-elsewhere", "FRONT", B01, true, false).await;
    // not yet through the first password change: still an employee with a role
    employee(&shop, "e-new", "MANAGER", MAIN, true, true).await;

    for (id, _, want) in table {
        assert_eq!(role(&shop, MAIN, id).await, want, "{id}");
        assert_eq!(
            role(&shop, B01, id).await,
            None,
            "{id} is not of the other branch"
        );
    }
    assert_eq!(role(&shop, MAIN, "e-inactive").await, None);
    assert_eq!(role(&shop, MAIN, "e-elsewhere").await, None);
    assert_eq!(
        role(&shop, B01, "e-elsewhere").await,
        Some(CustodyRole::Front)
    );
    assert_eq!(role(&shop, MAIN, "e-new").await, Some(CustodyRole::Manager));
    for unknown in ["e-nobody", "", "E-FRONT", "e-front ", "%"] {
        assert_eq!(role(&shop, MAIN, unknown).await, None, "{unknown:?}");
    }
    shop.finish().await;
}

// ------------------------------------------- through the transaction handed in

#[tokio::test]
async fn every_answer_is_read_through_the_transaction_it_was_handed() {
    // The custody service asks inside its own transaction and must see what
    // that transaction has written and not yet committed. Each fact is written
    // here inside a transaction, asked for through it, and then rolled back.
    let shop = Shop::start().await;
    let (customer, passport) = shop.ready(&[("THB", "100000"), ("USD", "1000")]).await;
    let shift = shop.shifts.active(&branch(MAIN)).await.unwrap().unwrap().id;
    shop.save(
        MAIN,
        form("intx-0001-aaaaaaaaaaa", &customer, &passport),
        at(0),
    )
    .await
    .unwrap();
    shop.seed_branch(B01, "Second").await;
    let ports = ports();
    let b01 = branch(B01);
    let main = branch(MAIN);

    let mut tx = shop.pool.begin().await.expect("begin");
    // a rate
    sqlx::query(
        "INSERT INTO rates (branch_id, code, buy, sell, updated_at, updated_by, request_id) \
         VALUES ($1, 'USD', 1, 2, '2026-10-06T09:09:09.009Z', 'test', 'in-the-tx')",
    )
    .bind(B01)
    .execute(&mut *tx)
    .await
    .expect("insert a rate");
    let facts = ports.outside_facts(&mut tx, &b01, None).await.unwrap();
    assert_eq!(
        (facts.has_live_rate, facts.rates_updated.as_str()),
        (true, "2026-10-06T09:09:09.009Z")
    );
    // the exchange of MAIN, moved out of its shift
    sqlx::query("UPDATE customers SET mode = 'DEMO' WHERE id = $1")
        .bind(&customer)
        .execute(&mut *tx)
        .await
        .expect("update a customer");
    assert!(
        !ports
            .customer_known(&mut tx, &main, &customer)
            .await
            .unwrap()
    );
    sqlx::query("UPDATE files SET owner_id = 'someone-else' WHERE key = $1")
        .bind(&passport)
        .execute(&mut *tx)
        .await
        .expect("update a file");
    assert!(
        !ports
            .evidence_owned(&mut tx, &main, OWNER, std::slice::from_ref(&passport))
            .await
            .unwrap()
    );
    assert!(
        ports
            .evidence_owned(
                &mut tx,
                &main,
                "someone-else",
                std::slice::from_ref(&passport)
            )
            .await
            .unwrap()
    );
    sqlx::query(
        "INSERT INTO employees (id, username, name, role, branch_id, password_hash, active, must_change, created_at) \
         VALUES ('e-tx', 'user-e-tx', 'In The Tx', 'FINANCE', $1, 'x', true, false, now())",
    )
    .bind(MAIN)
    .execute(&mut *tx)
    .await
    .expect("insert an employee");
    assert_eq!(
        ports.employee_role(&mut tx, &main, "e-tx").await.unwrap(),
        Some(CustodyRole::Finance)
    );
    sqlx::query("DELETE FROM transaction_files")
        .execute(&mut *tx)
        .await
        .expect("delete the files of the exchange");
    sqlx::query("DELETE FROM transactions")
        .execute(&mut *tx)
        .await
        .expect("delete the exchange");
    assert_eq!(
        ports
            .outside_facts(&mut tx, &main, Some(&shift))
            .await
            .unwrap()
            .last_transaction,
        None
    );
    tx.rollback().await.expect("rollback");

    // nothing of it was committed, and asked afresh the answers are the old ones
    assert!(!outside(&shop, B01, None).await.has_live_rate);
    assert!(known(&shop, MAIN, &customer).await);
    assert!(owned(&shop, MAIN, OWNER, &[&passport]).await);
    assert_eq!(role(&shop, MAIN, "e-tx").await, None);
    assert_eq!(
        outside(&shop, MAIN, Some(&shift)).await.last_transaction,
        Some("XC-20261006-000001".to_owned())
    );
    shop.finish().await;
}
