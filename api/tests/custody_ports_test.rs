//! Specification tests for T10b: `PgCustodyPorts`, what the custody service
//! learns from the other contexts.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! Re-expected for RE-7 #604 by the ticket author: `rates_updated` is the `published_at` of the
//! branch's current publication (rates.md §8 migration 6 drops `rates`, whose last row it was);
//! the rate tests publish instead of writing `rates` rows, and keep their scenarios.
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
mod shift_holder_common;

use std::sync::Arc;

use save_common::*;
use shift_holder_common::user_of;
use xc_shift::domain::custody::repositories::custody_ports::{CustodyPorts, OutsideFacts};
use xchange::infrastructure::custody::custody_ports::PgCustodyPorts;

async fn owner_user(pool: &sqlx::PgPool, branch_id: &str) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT u.id::text FROM users u \
         JOIN branches b ON b.id = $1 AND b.organization_id = u.organization_id \
         WHERE EXISTS (SELECT 1 FROM user_roles r \
                       WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
         ORDER BY u.created_at, u.id LIMIT 1",
    )
    .bind(branch_id)
    .fetch_one(pool)
    .await
    .expect("read branch owner user id")
}

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

/// A publication of `currency` in `branch` dated `published_at`: published through the rate
/// engine (over the branch's current publication), then its `published_at` set to the instant the
/// test names, so each instant is the test's, to the millisecond.
///
/// Re-expected for RE-7 #604 by the ticket author: this was a `rates` row written at a given time;
/// RE-7 drops `rates` (rates.md §8 migration 6) and `rates_updated` becomes the current
/// publication's `published_at`.
async fn publish_at(shop: &Shop, branch_id: &str, currency: &str, published_at: &str) {
    rates_re4c_common::set_counter_rate(&shop.pool, branch_id, currency, "32.1", "33").await;
    date_current(&shop.pool, branch_id, published_at).await;
}

/// Set `published_at` of `branch_id`'s current publication, through `executor`.
async fn date_current<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    branch_id: &str,
    published_at: &str,
) {
    sqlx::query(
        "UPDATE rate_publications p SET published_at = $2::timestamptz \
           FROM branches b WHERE b.id = $1 AND p.id = b.current_publication_id",
    )
    .bind(branch_id)
    .bind(published_at)
    .execute(executor)
    .await
    .expect("date the current publication");
}

// ---------------------------------------------------------------- the rates

/// Re-expected for RE-7 #604 by the ticket author (rates.md §8 migration 6, D-3): "the last rate
/// row written" becomes the branch's current publication. The scenario is v2's, step by step:
/// none yet; another branch's is not this one's; the time prints with milliseconds and `Z`; a
/// newer publication moves it, whatever its currency, and also when its time is earlier.
#[tokio::test]
async fn the_rates_are_live_once_the_branch_has_one_and_dated_by_the_current_publication() {
    // v2: a live rate is any currency with a buy and a sell rate; "updated" is
    // the time of the branch's rates, as the screens print it — since RE-7 the
    // `published_at` of the branch's current publication
    let shop = Shop::start().await;
    shop.seed_branch(B01, "Second").await;

    let none = outside(&shop, MAIN, None).await;
    assert!(!none.has_live_rate);
    assert_eq!(none.rates_updated, "", "no rate yet: the empty text");

    // a publication of another branch is not this branch's
    publish_at(&shop, B01, "USD", "2026-10-06T04:00:00.000Z").await;
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

    publish_at(&shop, MAIN, "USD", "2026-10-06T05:00:00.123Z").await;
    let one = outside(&shop, MAIN, None).await;
    assert!(one.has_live_rate);
    assert_eq!(
        one.rates_updated, "2026-10-06T05:00:00.123Z",
        "milliseconds and Z, as v2 prints a time"
    );

    // the current publication counts, whatever currency it moved — and even
    // when its time is earlier than the one before
    publish_at(&shop, MAIN, "JPY", "2026-10-06T12:30:00.5+07:00").await;
    assert_eq!(
        outside(&shop, MAIN, None).await.rates_updated,
        "2026-10-06T05:30:00.500Z"
    );
    publish_at(&shop, MAIN, "USD", "2026-10-06T01:02:03.004Z").await;
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
    let first_shift = shop.open_shift_id(MAIN).await;
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
    let shift = shop.open_shift_id(MAIN).await;
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
        // CI-4 (#651): DEMO shares LIVE's customer of a passport (owner, 2026-10-09).
        ("CX-00000001", "CX-00000001", "CX-00000002")
    );

    assert!(known(&shop, MAIN, &live).await);
    // CI-4 (#651): the DEMO reserve answered the LIVE customer (owner, 2026-10-09: DEMO shares LIVE).
    assert!(
        known(&shop, MAIN, &demo).await,
        "the DEMO reserve's customer is the LIVE customer"
    );
    // CI-4 (#651): a customer of the organisation is known at every branch (#554 K1).
    assert!(
        known(&shop, MAIN, &elsewhere).await,
        "a customer reserved at another branch is the organisation's customer"
    );
    assert!(known(&shop, B01, &elsewhere).await);
    // CI-4 (#651): K1, the MAIN customer is the organisation's at B01 too.
    assert!(known(&shop, B01, &live).await);
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

    let staff_id = user_of(&shop.pool, staff).await;
    let owner_id = owner_user(&shop.pool, MAIN).await;

    // OR-7.m G1b: rule 2 (#3797)
    assert!(owned(&shop, MAIN, &staff_id, &[&a]).await);
    // OR-7.m G1b: rule 2 (#3797)
    assert!(owned(&shop, MAIN, &staff_id, &[&a, &b]).await);
    // OR-7.m G1b: rule 2 (#3797)
    assert!(
        owned(&shop, MAIN, &staff_id, &[&b, &a, &a]).await,
        "a key named twice is still owned"
    );
    // OR-7.m G1b: rule 2 (#3797)
    assert!(
        owned(&shop, MAIN, &staff_id, &[]).await,
        "nothing to own: how many are needed is the service's rule"
    );
    // OR-7.m G1b: rule 2 (#3797)
    assert!(owned(&shop, B01, &staff_id, &[&elsewhere]).await);
    // OR-7.m G1b: rule 2 (#3797)
    assert!(owned(&shop, MAIN, &owner_id, &[&others]).await);

    // one key that is not theirs spoils the lot, wherever it stands
    for bad in [others.as_str(), elsewhere.as_str(), never, "", "x", "%"] {
        // OR-7.m G1b: rule 2 (#3797)
        assert!(
            !owned(&shop, MAIN, &staff_id, &[bad]).await,
            "{bad:?} alone"
        );
        // OR-7.m G1b: rule 2 (#3797)
        assert!(
            !owned(&shop, MAIN, &staff_id, &[&a, bad]).await,
            "{bad:?} last"
        );
        // OR-7.m G1b: rule 2 (#3797)
        assert!(
            !owned(&shop, MAIN, &staff_id, &[bad, &a, &b]).await,
            "{bad:?} first"
        );
    }
    assert!(
        !owned(&shop, MAIN, "Employee-7", &[&a]).await,
        "owners are compared exactly"
    );
    // OR-7.m G1b: rule 2 (#3797)
    assert!(!owned(&shop, B01, &staff_id, &[&a]).await);

    // v2 asks only for the record: bytes gone from the disk do not change the answer
    std::fs::remove_file(shop.dir.join(&a)).expect("remove an uploaded file");
    // OR-7.m G1b: rule 2 (#3797)
    assert!(owned(&shop, MAIN, &staff_id, &[&a]).await);
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
    let owner_id = owner_user(&shop.pool, MAIN).await;
    let shift = shop.open_shift_id(MAIN).await;
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

    // RE-7 #604 (re-expected by the ticket author): B01's rates are its current publication,
    // committed before the transaction at 08:00; inside it only its date moves, so the port
    // answering the new date read through the transaction, and after the rollback the old one
    publish_at(&shop, B01, "USD", "2026-10-06T08:00:00.000Z").await;
    let mut tx = shop.pool.begin().await.expect("begin");
    // a rate
    date_current(&mut *tx, B01, "2026-10-06T09:09:09.009Z").await;
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
    // OR-7.m G1b: rule 2 (#3797)
    assert!(
        !ports
            .evidence_owned(&mut tx, &main, &owner_id, std::slice::from_ref(&passport))
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
    sqlx::query("DELETE FROM transaction_files")
        .execute(&mut *tx)
        .await
        .expect("delete the files of the exchange");
    // TX-2 (#670, design §3.3–§3.5): a save also writes a line, a payment and a receipt, which
    // refer to the exchange. Added by #670's ticket author (claude/f03345fe), lane controller's
    // ruling of 2026-10-10; what this test asserts is unchanged.
    for children in ["receipts", "transaction_payments", "transaction_lines"] {
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {children}")))
            .execute(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("delete the exchange's {children}: {e}"));
    }
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
    assert_eq!(
        outside(&shop, B01, None).await.rates_updated,
        "2026-10-06T08:00:00.000Z",
        "RE-7 #604: the publication's committed date, not the one moved in the transaction"
    );
    assert!(known(&shop, MAIN, &customer).await);
    // OR-7.m G1b: rule 2 (#3797)
    assert!(owned(&shop, MAIN, &owner_id, &[&passport]).await);
    assert_eq!(
        outside(&shop, MAIN, Some(&shift)).await.last_transaction,
        Some("XC-20261006-000001".to_owned())
    );
    shop.finish().await;
}
