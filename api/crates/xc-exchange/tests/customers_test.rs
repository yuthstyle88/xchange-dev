//! Written before the implementation (ticket T6a). Do not edit, delete or weaken.
//!
//! Specification of `xc_exchange::application::customer::service::CustomerService`
//! and of `CustomerDetails::parse` (design §3 "Customer identity", §4 `customers`,
//! `counters`).
//!
//! Part 1 replays what the v2 engine answered, step by step
//! (`tests/fixtures/v2_customer_cases.json`, recorded by
//! `api/tools/v2-fixtures/gen_customers.mjs`). Part 2 is what a recording of a
//! single-threaded engine cannot show, and the two operations the save of an
//! exchange (ticket T8) will call inside its own transaction.
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use futures::stream::{self, StreamExt};
use pos_error::AppResult;
use tokio::sync::Barrier;
use xc_kernel::ids::BranchId;

fn details(cmd: &ReserveCustomer) -> CustomerDetails {
    CustomerDetails::parse(cmd).expect("well-formed details")
}

/// Run `confirm_in` in a transaction of `branch_id`.
async fn confirm(
    db: &TestDb,
    branch_id: &str,
    customer_id: &str,
    d: &CustomerDetails,
) -> AppResult<()> {
    let (b, id, d) = (branch(branch_id), customer_id.to_owned(), d.clone());
    let inner = b.clone();
    db.branch_tx()
        .run(&b, move |tx| {
            Box::pin(async move { CustomerService::confirm_in(tx, &inner, &id, &d).await })
        })
        .await
}

/// Run `record_exchange_in` in a transaction of `branch_id`.
async fn record(
    db: &TestDb,
    branch_id: &str,
    customer_id: &str,
    d: &CustomerDetails,
    document_ref: &str,
) -> AppResult<()> {
    let (b, id, d, r) = (
        branch(branch_id),
        customer_id.to_owned(),
        d.clone(),
        document_ref.to_owned(),
    );
    let inner = b.clone();
    db.branch_tx()
        .run(&b, move |tx| {
            Box::pin(
                async move { CustomerService::record_exchange_in(tx, &inner, &id, &d, &r).await },
            )
        })
        .await
}

async fn reserve(db: &TestDb, branch_id: &str, cmd: ReserveCustomer, seconds: i64) -> Reservation {
    db.customers()
        .reserve(&branch(branch_id), cmd, at(seconds))
        .await
        .expect("reserve")
}

async fn found(db: &TestDb, branch_id: &str, id: &str) -> Customer {
    db.customers()
        .find(&branch(branch_id), id)
        .await
        .expect("find")
        .unwrap_or_else(|| panic!("customer {id} is not in branch {branch_id}"))
}

// ── Part 1: the v2 recording ────────────────────────────────────────────────

#[test]
fn the_fixture_is_the_one_this_file_was_written_for() {
    let fixture = fixture();
    assert_eq!(fixture.scenarios.len(), 62);
    let reserves = fixture
        .scenarios
        .iter()
        .flat_map(|s| &s.steps)
        .filter(|s| s.op == "reserve")
        .count();
    assert_eq!(reserves, 75);
    let mut known = vec![
        NO_SUCH_BRANCH,
        BAD_TEXT,
        BAD_DATE_FORMAT,
        BAD_DATE,
        BAD_MODE,
        COUNTER_EXHAUSTED,
    ];
    known.sort_unstable();
    assert_eq!(
        fixture.sentences, known,
        "the sentences v2 used are the six constants"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_scenario_answers_as_v2_did() {
    let fixture = fixture();
    let played = fixture.scenarios.len();
    let wrong: Vec<String> = stream::iter(&fixture.scenarios)
        .map(play)
        .buffer_unordered(6)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect();
    assert!(
        wrong.is_empty(),
        "{} difference(s) from v2 in {played} scenario(s):\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

// ── Part 2a: the details of a customer, judged without a database ───────────

#[test]
fn details_are_normalised_as_they_will_be_stored() {
    let d = details(&ReserveCustomer {
        name: "  somchai Jaidee ".to_owned(),
        passport_no: " aa1234567 ".to_owned(),
        nationality: " tha ".to_owned(),
        birth_date: " 1990-01-31 ".to_owned(),
        passport_expiry: String::new(),
        mode: "DEMO".to_owned(),
    });
    assert_eq!(d.name, "somchai Jaidee", "trimmed, case kept");
    assert_eq!(d.passport_no, "AA1234567", "trimmed, upper case");
    assert_eq!(d.nationality, "THA", "trimmed, upper case");
    assert_eq!(d.birth_date, Some(day("1990-01-31")));
    assert_eq!(d.passport_expiry, None);
    assert_eq!(d.mode, Mode::Demo);
}

#[test]
fn details_are_refused_in_v2s_order_name_passport_nationality_birth_expiry_mode() {
    let good = person(1);
    let with = |f: fn(&mut ReserveCustomer)| {
        let mut c = person(1);
        f(&mut c);
        c
    };
    assert!(CustomerDetails::parse(&good).is_ok());
    let cases: Vec<(ReserveCustomer, &str)> = vec![
        (with(|c| c.name = " ".to_owned()), BAD_TEXT),
        (with(|c| c.passport_no = "A".repeat(31)), BAD_TEXT),
        (with(|c| c.nationality = String::new()), BAD_TEXT),
        (
            with(|c| c.birth_date = "31/01/1990".to_owned()),
            BAD_DATE_FORMAT,
        ),
        (with(|c| c.birth_date = "1990-02-30".to_owned()), BAD_DATE),
        (with(|c| c.birth_date = "1990-01-311".to_owned()), BAD_TEXT),
        (
            with(|c| c.passport_expiry = "1920-04-01".to_owned()),
            BAD_DATE,
        ),
        (with(|c| c.mode = "live".to_owned()), BAD_MODE),
        // two things wrong: the earlier field is the answer
        (
            with(|c| {
                c.name = String::new();
                c.mode = String::new();
            }),
            BAD_TEXT,
        ),
        (
            with(|c| {
                c.nationality = String::new();
                c.birth_date = "x".to_owned();
            }),
            BAD_TEXT,
        ),
        (
            with(|c| {
                c.birth_date = "x".to_owned();
                c.passport_expiry = "1990-02-30".to_owned();
            }),
            BAD_DATE_FORMAT,
        ),
        (
            with(|c| {
                c.passport_expiry = "1990-02-30".to_owned();
                c.mode = "X".to_owned();
            }),
            BAD_DATE,
        ),
    ];
    for (cmd, sentence) in cases {
        let shown = format!("{cmd:?}");
        assert_eq!(refusal(CustomerDetails::parse(&cmd)), sentence, "{shown}");
    }
}

#[test]
fn a_mode_prints_as_the_screens_send_it() {
    assert_eq!(Mode::Live.as_str(), "LIVE");
    assert_eq!(Mode::Demo.as_str(), "DEMO");
}

// ── Part 2b: reserving ──────────────────────────────────────────────────────

#[tokio::test]
async fn a_fresh_database_has_no_customer_and_no_counter() {
    let db = TestDb::start().await;
    assert_eq!(db.count("customers").await, 0);
    assert_eq!(db.count("counters").await, 0);
    assert!(
        db.customers()
            .list_live(&branch(MAIN))
            .await
            .expect("list_live")
            .is_empty()
    );
    db.finish().await;
}

#[tokio::test]
async fn a_new_customer_is_stored_whole_with_the_time_cut_to_the_millisecond() {
    let db = TestDb::start().await;
    let mut cmd = person(1);
    cmd.birth_date = "1990-01-31".to_owned();
    cmd.passport_expiry = "2031-12-31".to_owned();
    let now = at(0);
    assert_ne!(now, millis(now), "the test's clock has microseconds");

    let r = db
        .customers()
        .reserve(&branch(MAIN), cmd, now)
        .await
        .expect("reserve");

    assert_eq!(
        r,
        Reservation {
            customer_id: "CX-00000001".to_owned(),
            existing: false
        }
    );
    let c = found(&db, MAIN, "CX-00000001").await;
    assert_eq!(
        c,
        Customer {
            id: "CX-00000001".to_owned(),
            branch_id: branch(MAIN),
            name: "Customer 1".to_owned(),
            passport_no: "P0000001".to_owned(),
            nationality: "THA".to_owned(),
            birth_date: Some(day("1990-01-31")),
            passport_expiry: Some(day("2031-12-31")),
            mode: Mode::Live,
            created_at: millis(now),
            last_document_ref: None,
        }
    );
    assert_eq!(db.counter("customer-LIVE").await, Some(1));
    assert_eq!(db.counter("customer-DEMO").await, None);
    db.finish().await;
}

#[tokio::test]
async fn a_customer_is_found_only_in_its_own_branch() {
    let db = TestDb::start().await;
    db.seed_branch("BOTHER", "Other").await;
    reserve(&db, MAIN, person(1), 0).await;
    let customers = db.customers();
    assert!(
        customers
            .find(&branch(MAIN), "CX-00000001")
            .await
            .expect("find")
            .is_some()
    );
    assert!(
        customers
            .find(&branch("BOTHER"), "CX-00000001")
            .await
            .expect("find")
            .is_none()
    );
    assert!(
        customers
            .find(&branch(MAIN), "CX-00000002")
            .await
            .expect("find")
            .is_none()
    );
    assert!(
        customers
            .find(&branch("BNOPE"), "CX-00000001")
            .await
            .expect("find")
            .is_none()
    );
    assert_eq!(
        refusal(customers.list_live(&branch("BNOPE")).await),
        NO_SUCH_BRANCH
    );
    db.finish().await;
}

#[tokio::test]
async fn a_refused_reservation_spends_no_number() {
    let db = TestDb::start().await;
    let customers = db.customers();
    let mut bad = person(1);
    bad.mode = "X".to_owned();
    assert_eq!(
        refusal(customers.reserve(&branch(MAIN), bad, at(0)).await),
        BAD_MODE
    );
    assert_eq!(
        refusal(customers.reserve(&branch("BNOPE"), person(1), at(0)).await),
        NO_SUCH_BRANCH
    );
    assert_eq!(db.counter("customer-LIVE").await, None);

    assert_eq!(
        reserve(&db, MAIN, person(1), 1).await.customer_id,
        "CX-00000001"
    );
    // The same customer again spends none either.
    assert!(reserve(&db, MAIN, person(1), 2).await.existing);
    assert_eq!(db.counter("customer-LIVE").await, Some(1));
    assert_eq!(
        reserve(&db, MAIN, person(2), 3).await.customer_id,
        "CX-00000002"
    );
    db.finish().await;
}

#[tokio::test]
async fn an_exhausted_counter_stays_where_it_is() {
    let db = TestDb::start().await;
    db.seed_counter("customer-LIVE", 999_999_999).await;
    assert_eq!(
        refusal(
            db.customers()
                .reserve(&branch(MAIN), person(1), at(0))
                .await
        ),
        COUNTER_EXHAUSTED
    );
    assert_eq!(db.counter("customer-LIVE").await, Some(999_999_999));
    assert_eq!(db.count("customers").await, 0);
    // The demo numbering is another counter and still works.
    let mut demo = person(1);
    demo.mode = "DEMO".to_owned();
    assert_eq!(
        reserve(&db, MAIN, demo, 1).await.customer_id,
        "TEST-CX-00000001"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_new_customer_raises_the_version_of_its_branch_only() {
    let db = TestDb::start().await;
    db.seed_branch("BOTHER", "Other").await;
    let tx = db.branch_tx();
    let before = (
        tx.version(&branch(MAIN)).await.unwrap(),
        tx.version(&branch("BOTHER")).await.unwrap(),
    );
    reserve(&db, MAIN, person(1), 0).await;
    assert!(tx.version(&branch(MAIN)).await.unwrap() > before.0);
    assert_eq!(tx.version(&branch("BOTHER")).await.unwrap(), before.1);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_passport_reserved_from_many_desks_at_once_is_one_customer() {
    const DESKS: usize = 8;
    let db = TestDb::start().await;
    let customers = Arc::new(db.customers());
    let barrier = Arc::new(Barrier::new(DESKS));
    let mut tasks = Vec::new();
    for n in 0..DESKS {
        let (customers, barrier) = (customers.clone(), barrier.clone());
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            customers
                .reserve(&branch(MAIN), person(7), at(n as i64))
                .await
        }));
    }
    let mut fresh = 0;
    for task in tasks {
        let r = within(MUST_FINISH, "a concurrent reservation", task)
            .await
            .unwrap()
            .expect("every desk gets the customer");
        assert_eq!(r.customer_id, "CX-00000001");
        if !r.existing {
            fresh += 1;
        }
    }
    assert_eq!(fresh, 1, "exactly one desk created the customer");
    assert_eq!(db.count("customers").await, 1);
    assert_eq!(
        db.counter("customer-LIVE").await,
        Some(1),
        "the losers spent no number"
    );
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn customers_reserved_at_once_in_two_branches_share_one_numbering_without_gaps() {
    const DESKS: usize = 8;
    let db = TestDb::start().await;
    db.seed_branch("BOTHER", "Other").await;
    let customers = Arc::new(db.customers());
    let barrier = Arc::new(Barrier::new(DESKS));
    let mut tasks = Vec::new();
    for n in 0..DESKS {
        let (customers, barrier) = (customers.clone(), barrier.clone());
        tasks.push(tokio::spawn(async move {
            let b: BranchId = branch(if n % 2 == 0 { MAIN } else { "BOTHER" });
            barrier.wait().await;
            customers.reserve(&b, person(n as u32), at(n as i64)).await
        }));
    }
    let mut ids = BTreeSet::new();
    for task in tasks {
        let r = within(MUST_FINISH, "a concurrent reservation", task)
            .await
            .unwrap()
            .expect("reserve");
        assert!(!r.existing);
        ids.insert(r.customer_id);
    }
    let want: BTreeSet<String> = (1..=DESKS).map(|n| format!("CX-{n:08}")).collect();
    assert_eq!(
        ids, want,
        "eight customers, eight consecutive numbers, none twice"
    );
    assert_eq!(db.counter("customer-LIVE").await, Some(DESKS as i64));
    db.finish().await;
}

// ── Part 2c: what the save of an exchange calls inside its transaction ──────

#[tokio::test]
async fn confirm_in_accepts_the_customer_that_passport_was_reserved_as() {
    let db = TestDb::start().await;
    db.seed_branch("BOTHER", "Other").await;
    let id = reserve(&db, MAIN, person(1), 0).await.customer_id;
    let mut demo = person(1);
    demo.mode = "DEMO".to_owned();
    let demo_id = reserve(&db, MAIN, demo.clone(), 1).await.customer_id;
    let elsewhere = reserve(&db, "BOTHER", person(1), 2).await.customer_id;

    confirm(&db, MAIN, &id, &details(&person(1)))
        .await
        .expect("the reserved customer");
    confirm(&db, MAIN, &demo_id, &details(&demo))
        .await
        .expect("the demo customer");
    // The name on the form may differ from the stored one; the save will overwrite it.
    let mut renamed = person(1);
    renamed.name = "Married Name".to_owned();
    confirm(&db, MAIN, &id, &details(&renamed))
        .await
        .expect("the name is not part of the identity");
    // Lower case on the form is the same passport.
    let mut lower = person(1);
    lower.passport_no = "p0000001".to_owned();
    lower.nationality = "tha".to_owned();
    confirm(&db, MAIN, &id, &details(&lower))
        .await
        .expect("details are compared as stored");

    let mut other_passport = person(1);
    other_passport.passport_no = "P0000002".to_owned();
    let mut other_nationality = person(1);
    other_nationality.nationality = "LAO".to_owned();
    for (what, branch_id, customer_id, d) in [
        (
            "an id that does not exist",
            MAIN,
            "CX-00000099",
            details(&person(1)),
        ),
        (
            "another passport number",
            MAIN,
            id.as_str(),
            details(&other_passport),
        ),
        (
            "another nationality",
            MAIN,
            id.as_str(),
            details(&other_nationality),
        ),
        (
            "the live id with demo details",
            MAIN,
            id.as_str(),
            details(&demo),
        ),
        (
            "the demo id with live details",
            MAIN,
            demo_id.as_str(),
            details(&person(1)),
        ),
        (
            "a customer of another branch",
            MAIN,
            elsewhere.as_str(),
            details(&person(1)),
        ),
        (
            "this branch's customer asked in another branch",
            "BOTHER",
            id.as_str(),
            details(&person(1)),
        ),
    ] {
        assert_eq!(
            refusal(confirm(&db, branch_id, customer_id, &d).await),
            NOT_THIS_PASSPORT,
            "{what}"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn record_exchange_in_overwrites_name_and_dates_and_notes_the_document() {
    let db = TestDb::start().await;
    let mut first = person(1);
    first.birth_date = "1990-01-31".to_owned();
    first.passport_expiry = "2031-12-31".to_owned();
    let id = reserve(&db, MAIN, first, 0).await.customer_id;
    let other = reserve(&db, MAIN, person(2), 1).await.customer_id;
    let before_other = found(&db, MAIN, &other).await;

    // The form of the exchange carries a new name, a new expiry and no birth date.
    let mut form = person(1);
    form.name = "  Married Name ".to_owned();
    form.passport_expiry = "2036-06-30".to_owned();
    record(&db, MAIN, &id, &details(&form), "XC-20261006-000001")
        .await
        .expect("record");

    let c = found(&db, MAIN, &id).await;
    assert_eq!(c.name, "Married Name");
    assert_eq!(
        c.birth_date, None,
        "a date the form left empty is cleared, as v2 does"
    );
    assert_eq!(c.passport_expiry, Some(day("2036-06-30")));
    assert_eq!(c.last_document_ref.as_deref(), Some("XC-20261006-000001"));
    assert_eq!(c.passport_no, "P0000001");
    assert_eq!(
        c.created_at,
        millis(at(0)),
        "the creation time is not touched"
    );
    assert_eq!(
        found(&db, MAIN, &other).await,
        before_other,
        "no other customer is touched"
    );

    record(&db, MAIN, &id, &details(&person(1)), "XC-20261006-000002")
        .await
        .expect("a later exchange");
    let c = found(&db, MAIN, &id).await;
    assert_eq!(
        (c.name.as_str(), c.last_document_ref.as_deref()),
        ("Customer 1", Some("XC-20261006-000002"))
    );
    db.finish().await;
}

#[tokio::test]
async fn record_exchange_in_refuses_a_customer_that_is_not_that_passport_and_changes_nothing() {
    let db = TestDb::start().await;
    let id = reserve(&db, MAIN, person(1), 0).await.customer_id;
    let before = found(&db, MAIN, &id).await;
    let mut other = person(1);
    other.passport_no = "P0000002".to_owned();
    other.name = "Someone Else".to_owned();

    assert_eq!(
        refusal(record(&db, MAIN, &id, &details(&other), "XC-1").await),
        NOT_THIS_PASSPORT
    );
    assert_eq!(
        refusal(record(&db, MAIN, "CX-00000099", &details(&person(1)), "XC-1").await),
        NOT_THIS_PASSPORT
    );
    assert_eq!(found(&db, MAIN, &id).await, before);
    db.finish().await;
}
