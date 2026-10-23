//! KWQ-A1 — the web request's persistence against a real PostgreSQL (design
//! `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D2, D5, D9; §4.6): the lookup by request
//! token (`find_by_token_in`, one hash compare in one branch), the member
//! (`set_customer_in` / `customer_in`, the migration `kiosk_requests.customer_id`) and the
//! branch's timezone for the printed ticket (`branch_timezone_in`).
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod kiosk_service_common;

use kiosk_service_common::*;
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

/// A QUOTED request `id` of `branch_id`, with the hash of `token` stored as its request token.
async fn add(
    db: &TestDb,
    repo: &PgKioskRepository,
    branch_id: &str,
    id: &str,
    token: Option<&str>,
) {
    let mut tx = db.begin().await;
    let b = branch(branch_id);
    repo.insert_in(&mut tx, &b, &quoted(id), "10.0.0.1")
        .await
        .expect("insert the request");
    if let Some(token) = token {
        repo.set_cancel_token_in(&mut tx, &b, id, token)
            .await
            .expect("store the token's hash");
    }
    tx.commit().await.expect("commit");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_finds_its_own_request_in_its_own_branch_only() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, MAIN, "web-1", Some("tok-main")).await;
    add(&db, &repo, B2, "web-2", Some("tok-b2")).await;
    add(&db, &repo, MAIN, "kiosk-1", None).await;

    let mut tx = db.begin().await;
    let found = repo
        .find_by_token_in(&mut tx, &branch(MAIN), "tok-main")
        .await
        .expect("lookup");
    assert_eq!(found.map(|r| r.id), Some("web-1".to_owned()));

    // another branch's token, and an unknown one, find nothing
    let other = repo
        .find_by_token_in(&mut tx, &branch(MAIN), "tok-b2")
        .await
        .expect("lookup");
    assert_eq!(other, None);
    let unknown = repo
        .find_by_token_in(&mut tx, &branch(MAIN), "never-minted")
        .await
        .expect("lookup");
    assert_eq!(unknown, None);
    // a request with no token is never found, not even by an empty token
    let empty = repo
        .find_by_token_in(&mut tx, &branch(MAIN), "")
        .await
        .expect("lookup");
    assert_eq!(empty, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replaced_token_no_longer_finds_the_request() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, MAIN, "web-1", Some("tok-old")).await;

    let mut tx = db.begin().await;
    repo.set_cancel_token_in(&mut tx, &branch(MAIN), "web-1", "tok-new")
        .await
        .expect("replace the token");
    let old = repo
        .find_by_token_in(&mut tx, &branch(MAIN), "tok-old")
        .await
        .expect("lookup");
    assert_eq!(old, None);
    let new = repo
        .find_by_token_in(&mut tx, &branch(MAIN), "tok-new")
        .await
        .expect("lookup");
    assert_eq!(new.map(|r| r.id), Some("web-1".to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_member_is_stored_and_read_back_and_a_kiosk_request_has_none() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, MAIN, "web-1", Some("tok")).await;
    add(&db, &repo, MAIN, "kiosk-1", None).await;

    let mut tx = db.begin().await;
    repo.set_customer_in(&mut tx, &branch(MAIN), "web-1", "cust-1")
        .await
        .expect("store the member");
    tx.commit().await.expect("commit");

    let stored: Option<String> =
        sqlx::query_scalar("SELECT customer_id FROM kiosk_requests WHERE id = 'web-1'")
            .fetch_one(&db.pool)
            .await
            .expect("read the column");
    assert_eq!(stored.as_deref(), Some("cust-1"));

    let mut tx = db.begin().await;
    let b = branch(MAIN);
    assert_eq!(
        repo.customer_in(&mut tx, &b, "web-1").await.expect("read"),
        Some("cust-1".to_owned())
    );
    assert_eq!(
        repo.customer_in(&mut tx, &b, "kiosk-1")
            .await
            .expect("read"),
        None
    );
    assert_eq!(
        repo.customer_in(&mut tx, &b, "no-such")
            .await
            .expect("read"),
        None
    );
    // another branch's id reads nothing
    assert_eq!(
        repo.customer_in(&mut tx, &branch(B2), "web-1")
            .await
            .expect("read"),
        None
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_branch_timezone_is_the_branch_row_and_none_for_no_branch() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    sqlx::query("UPDATE branches SET timezone = 'Asia/Tokyo' WHERE id = 'B2'")
        .execute(&db.pool)
        .await
        .expect("give B2 its own timezone");
    let main_tz: String = sqlx::query_scalar("SELECT timezone FROM branches WHERE id = 'MAIN'")
        .fetch_one(&db.pool)
        .await
        .expect("MAIN's timezone");

    let mut tx = db.begin().await;
    assert_eq!(
        repo.branch_timezone_in(&mut tx, &branch(MAIN))
            .await
            .expect("read"),
        Some(main_tz)
    );
    assert_eq!(
        repo.branch_timezone_in(&mut tx, &branch(B2))
            .await
            .expect("read"),
        Some("Asia/Tokyo".to_owned())
    );
    assert_eq!(
        repo.branch_timezone_in(&mut tx, &branch("NOPE"))
            .await
            .expect("read"),
        None
    );
}
