//! KWQ-C2 — `KioskRepository::open_web_request_in` against a real PostgreSQL (design
//! `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D12, §4.4 B4): the member's open web
//! request in a branch — `QUOTED` or `WAITING` with that `customer_id` — and nothing else.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod kiosk_service_common;

use kiosk_service_common::*;
use sqlx::AssertSqlSafe;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};
use xc_kiosk::domain::kiosk::repositories::kiosk_repository::KioskRepository;
use xc_kiosk::domain::kiosk::value_objects::payment_method::PaymentMethod;
use xc_kiosk::infrastructure::kiosk::kiosk_repository::PgKioskRepository;

fn quoted(id: &str) -> KioskRequest {
    KioskRequest {
        id: id.to_owned(),
        status: KioskStatus::Quoted,
        created: KioskTime::from_epoch_millis(T0),
        staff: None,
        quote: KioskQuote {
            side: Side::Buy,
            currency: CurrencyCode::parse("USD").unwrap(),
            rate: Rate::parse("35.00").unwrap(),
            unit_size: UnitSize::parse("1").unwrap(),
            line_id: None,
            foreign: Money::from_minor(10_000).unwrap(),
            thb: Money::from_minor(350_000).unwrap(),
            input_amount: Money::from_minor(10_000).unwrap(),
            payment_method: PaymentMethod::Cash,
        },
        transaction: None,
    }
}

/// A request `id` of `branch_id` in `status`, of `customer` (or a kiosk request).
async fn add(db: &TestDb, repo: &PgKioskRepository, branch_id: &str, id: &str, status: &str, customer: Option<&str>) {
    let mut tx = db.begin().await;
    repo.insert_in(&mut tx, &branch(branch_id), &quoted(id), "10.0.0.1")
        .await
        .expect("insert");
    tx.commit().await.expect("commit");
    let customer = customer.map_or("NULL".to_owned(), |c| format!("'{c}'"));
    let cancel = if status == "CANCELLED" {
        ", cancelled_at = now(), cancelled_by = 'test', cancel_reason = 'CUSTOMER_LEFT'"
    } else {
        ""
    };
    let staff = if matches!(status, "ACCEPTED" | "ID_CHECKED" | "COMPLETED") {
        ", staff = 'front1'"
    } else {
        ""
    };
    sqlx::query(AssertSqlSafe(format!(
        "UPDATE kiosk_requests SET status = '{status}', customer_id = {customer}{cancel}{staff} \
         WHERE branch_id = '{branch_id}' AND id = '{id}'"
    )))
    .execute(&db.pool)
    .await
    .expect("put the request in its state");
}

async fn open(db: &TestDb, repo: &PgKioskRepository, branch_id: &str, customer: &str) -> Option<String> {
    let mut tx = db.begin().await;
    repo.open_web_request_in(&mut tx, &branch(branch_id), customer)
        .await
        .expect("lookup")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_quoted_or_waiting_request_of_the_member_is_open() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, MAIN, "q", "QUOTED", Some("cust-1")).await;
    assert_eq!(open(&db, &repo, MAIN, "cust-1").await.as_deref(), Some("q"));

    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, MAIN, "w", "WAITING", Some("cust-1")).await;
    assert_eq!(open(&db, &repo, MAIN, "cust-1").await.as_deref(), Some("w"));
}

#[tokio::test(flavor = "multi_thread")]
async fn finished_requests_other_members_other_branches_and_kiosk_requests_are_not() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, MAIN, "done", "COMPLETED", Some("cust-1")).await;
    add(&db, &repo, MAIN, "gone", "CANCELLED", Some("cust-1")).await;
    add(&db, &repo, MAIN, "desk", "ACCEPTED", Some("cust-1")).await;
    add(&db, &repo, MAIN, "theirs", "QUOTED", Some("cust-2")).await;
    add(&db, &repo, B2, "elsewhere", "QUOTED", Some("cust-1")).await;
    add(&db, &repo, MAIN, "kiosk", "QUOTED", None).await;

    assert_eq!(open(&db, &repo, MAIN, "cust-1").await, None);
    assert_eq!(open(&db, &repo, B2, "cust-1").await.as_deref(), Some("elsewhere"));
    assert_eq!(open(&db, &repo, MAIN, "cust-2").await.as_deref(), Some("theirs"));
    assert_eq!(open(&db, &repo, MAIN, "").await, None);
}
