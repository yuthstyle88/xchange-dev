//! KC-D — the kiosk cancel's persistence against a real PostgreSQL: `mark_cancelled_in` (one
//! guarded `UPDATE`), reading `CANCELLED` back, the customer's cancel token stored as a hash
//! (`set_cancel_token_in`, `matches_cancel_token_in`), and the queue filters.
//! Written before the implementation by the ticket author (claude/studio-claude-10). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/kiosk-cancel.md` §3.1, §3.3, §3.4 (views), as amended by
//! PR 4154. Needs the KC-M migration, and `TEST_DATABASE_URL` naming an existing database whose
//! name ends in `_test`.

mod kiosk_service_common;

use chrono::{TimeZone, Utc};
use kiosk_service_common::*;
use sqlx::AssertSqlSafe;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};
use xc_kiosk::domain::kiosk::entities::kiosk_request::KioskRequest;
use xc_kiosk::domain::kiosk::repositories::kiosk_repository::KioskRepository;
use xc_kiosk::domain::kiosk::value_objects::cancel_reason::{CancelReason, Cancellation};
use xc_kiosk::domain::kiosk::value_objects::kiosk_quote::KioskQuote;
use xc_kiosk::domain::kiosk::value_objects::kiosk_status::KioskStatus;
use xc_kiosk::domain::kiosk::value_objects::kiosk_time::KioskTime;
use xc_kiosk::domain::kiosk::value_objects::payment_method::PaymentMethod;
use xc_kiosk::domain::kiosk::value_objects::queue_filter::QueueFilter;
use xc_kiosk::infrastructure::kiosk::kiosk_repository::{PgKioskRepository, queue_statuses};

fn main_branch() -> BranchId {
    BranchId::new(MAIN)
}

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

/// A request of MAIN stored in `status`, held by `staff`.
async fn add(db: &TestDb, repo: &PgKioskRepository, id: &str, status: &str, staff: Option<&str>) {
    let mut tx = db.begin().await;
    repo.insert_in(&mut tx, &main_branch(), &quoted(id), "10.0.0.1")
        .await
        .expect("insert the request");
    tx.commit().await.expect("commit the insert");
    let staff = staff.map_or("NULL".to_owned(), |s| format!("'{s}'"));
    sqlx::query(AssertSqlSafe(format!(
        "UPDATE kiosk_requests SET status = '{status}', staff = {staff} \
         WHERE branch_id = 'MAIN' AND id = '{id}'"
    )))
    .execute(&db.pool)
    .await
    .expect("put the request in its status");
}

fn cancellation(by: &str, reason: CancelReason) -> Cancellation {
    Cancellation {
        at: Utc.with_ymd_and_hms(2026, 10, 21, 4, 0, 0).unwrap(),
        by: by.to_owned(),
        reason,
    }
}

/// `status | staff | cancelled_at | cancelled_by | cancel_reason | cancel_note` of one row.
async fn row(db: &TestDb, id: &str) -> String {
    sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT concat_ws(' | ', status, coalesce(staff, '-'), \
                coalesce(to_char(cancelled_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI'), '-'), \
                coalesce(cancelled_by, '-'), coalesce(cancel_reason, '-'), coalesce(cancel_note, '-')) \
         FROM kiosk_requests WHERE branch_id = 'MAIN' AND id = '{id}'"
    )))
    .fetch_one(&db.pool)
    .await
    .expect("read the row")
}

async fn mark(
    db: &TestDb,
    repo: &PgKioskRepository,
    id: &str,
    from: KioskStatus,
    cancel: &Cancellation,
) -> bool {
    let mut tx = db.begin().await;
    let moved = repo
        .mark_cancelled_in(&mut tx, &main_branch(), id, from, cancel)
        .await
        .expect("mark_cancelled_in answers");
    tx.commit().await.expect("commit");
    moved
}

#[tokio::test]
async fn a_waiting_request_is_stored_cancelled_with_when_who_why_and_note() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, "KR-1", "WAITING", None).await;

    let moved = mark(
        &db,
        &repo,
        "KR-1",
        KioskStatus::Waiting,
        &cancellation(
            "teller-1",
            CancelReason::Other {
                note: "ลูกค้าโทรมายกเลิก".to_owned(),
            },
        ),
    )
    .await;

    assert!(moved);
    assert_eq!(
        row(&db, "KR-1").await,
        "CANCELLED | - | 2026-10-21 04:00 | teller-1 | OTHER | ลูกค้าโทรมายกเลิก"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_held_request_keeps_its_holder_and_a_code_without_note_stores_none() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, "KR-1", "ID_CHECKED", Some("teller-1")).await;

    let moved = mark(
        &db,
        &repo,
        "KR-1",
        KioskStatus::IdChecked,
        &cancellation("manager-1", CancelReason::Duplicate { note: None }),
    )
    .await;

    assert!(moved);
    assert_eq!(
        row(&db, "KR-1").await,
        "CANCELLED | teller-1 | 2026-10-21 04:00 | manager-1 | DUPLICATE | -"
    );
    db.finish().await;
}

#[tokio::test]
async fn each_reason_is_stored_under_its_code() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    for (n, (reason, code)) in [
        (
            CancelReason::CustomerLeft {
                note: Some("เดินออกไปแล้ว".to_owned()),
            },
            "CUSTOMER_LEFT",
        ),
        (CancelReason::CustomerCancelled, "CUSTOMER_CANCELLED"),
        (
            CancelReason::CustomerChangedMind { note: None },
            "CUSTOMER_CHANGED_MIND",
        ),
        (CancelReason::DayClosed, "DAY_CLOSED"),
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("KR-{n}");
        add(&db, &repo, &id, "WAITING", None).await;
        assert!(
            mark(
                &db,
                &repo,
                &id,
                KioskStatus::Waiting,
                &cancellation("CUSTOMER-KIOSK", reason.clone())
            )
            .await,
            "{code}"
        );
        let stored: (String, Option<String>) = sqlx::query_as(AssertSqlSafe(format!(
            "SELECT cancel_reason, cancel_note FROM kiosk_requests WHERE branch_id = 'MAIN' AND id = '{id}'"
        )))
        .fetch_one(&db.pool)
        .await
        .expect("read the reason");
        assert_eq!(stored, (code.to_owned(), reason.note().map(str::to_owned)));
    }
    db.finish().await;
}

#[tokio::test]
async fn a_request_no_longer_in_the_status_read_is_left_alone_and_answers_false() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, "KR-1", "ACCEPTED", Some("teller-2")).await;
    let before = row(&db, "KR-1").await;

    let moved = mark(
        &db,
        &repo,
        "KR-1",
        KioskStatus::Waiting,
        &cancellation("teller-1", CancelReason::CustomerLeft { note: None }),
    )
    .await;

    assert!(!moved, "the guard is the status the caller read");
    assert_eq!(row(&db, "KR-1").await, before);

    let unknown = mark(
        &db,
        &repo,
        "KR-NONE",
        KioskStatus::Waiting,
        &cancellation("teller-1", CancelReason::CustomerLeft { note: None }),
    )
    .await;
    assert!(!unknown, "no such request");
    db.finish().await;
}

#[tokio::test]
async fn only_the_named_branch_and_request_move() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, "KR-1", "WAITING", None).await;
    add(&db, &repo, "KR-2", "WAITING", None).await;

    let mut tx = db.begin().await;
    let elsewhere = repo
        .mark_cancelled_in(
            &mut tx,
            &BranchId::new(B2),
            "KR-1",
            KioskStatus::Waiting,
            &cancellation("teller-1", CancelReason::Duplicate { note: None }),
        )
        .await
        .expect("mark_cancelled_in answers");
    tx.commit().await.expect("commit");
    assert!(!elsewhere, "KR-1 is MAIN's, not B2's");

    assert!(
        mark(
            &db,
            &repo,
            "KR-1",
            KioskStatus::Waiting,
            &cancellation("teller-1", CancelReason::Duplicate { note: None })
        )
        .await
    );
    assert!(
        row(&db, "KR-2")
            .await
            .starts_with("WAITING | - | - | - | - | -")
    );
    db.finish().await;
}

#[tokio::test]
async fn a_cancelled_request_reads_back_as_cancelled() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, "KR-1", "WAITING", None).await;
    mark(
        &db,
        &repo,
        "KR-1",
        KioskStatus::Waiting,
        &cancellation("teller-1", CancelReason::CustomerLeft { note: None }),
    )
    .await;

    let found = repo
        .find(&main_branch(), "KR-1")
        .await
        .expect("a CANCELLED row is readable")
        .expect("the request");
    assert_eq!(found.status, KioskStatus::Cancelled);
    assert_eq!(found.staff, None);

    let mut tx = db.begin().await;
    let in_tx = repo
        .find_in(&mut tx, &main_branch(), "KR-1")
        .await
        .expect("readable inside a transaction")
        .expect("the request");
    assert_eq!(in_tx.status, KioskStatus::Cancelled);
    drop(tx);
    db.finish().await;
}

#[tokio::test]
async fn the_cancel_token_is_stored_only_as_its_sha256_hex() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, "KR-1", "WAITING", None).await;
    let token = "3f2c1a9e-7b44-4c1e-9d55-0a1b2c3d4e5f";

    let mut tx = db.begin().await;
    repo.set_cancel_token_in(&mut tx, &main_branch(), "KR-1", token)
        .await
        .expect("store the token");
    tx.commit().await.expect("commit");

    let stored: Option<String> = sqlx::query_scalar(
        "SELECT cancel_token_hash FROM kiosk_requests WHERE branch_id = 'MAIN' AND id = 'KR-1'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("read the hash");
    let expected: String =
        sqlx::query_scalar("SELECT encode(sha256(convert_to($1, 'UTF8')), 'hex')")
            .bind(token)
            .fetch_one(&db.pool)
            .await
            .expect("hash in SQL");
    assert_eq!(stored.as_deref(), Some(expected.as_str()));
    assert_ne!(stored.as_deref(), Some(token), "never the token itself");
    db.finish().await;
}

#[tokio::test]
async fn the_token_matches_only_itself_and_only_on_its_own_request() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, "KR-1", "WAITING", None).await;
    add(&db, &repo, "KR-2", "WAITING", None).await;
    let token = "3f2c1a9e-7b44-4c1e-9d55-0a1b2c3d4e5f";

    let mut tx = db.begin().await;
    repo.set_cancel_token_in(&mut tx, &main_branch(), "KR-1", token)
        .await
        .expect("store the token");
    let right = repo
        .matches_cancel_token_in(&mut tx, &main_branch(), "KR-1", token)
        .await
        .expect("compare");
    let mut wrong = Vec::new();
    for other in [
        "",
        "3F2C1A9E-7B44-4C1E-9D55-0A1B2C3D4E5F",
        "3f2c1a9e-7b44-4c1e-9d55-0a1b2c3d4e5",
        " 3f2c1a9e-7b44-4c1e-9d55-0a1b2c3d4e5f",
    ] {
        wrong.push(
            repo.matches_cancel_token_in(&mut tx, &main_branch(), "KR-1", other)
                .await
                .expect("compare"),
        );
    }
    let on_other_request = repo
        .matches_cancel_token_in(&mut tx, &main_branch(), "KR-2", token)
        .await
        .expect("compare");
    let on_other_branch = repo
        .matches_cancel_token_in(&mut tx, &BranchId::new(B2), "KR-1", token)
        .await
        .expect("compare");
    let unknown = repo
        .matches_cancel_token_in(&mut tx, &main_branch(), "KR-NONE", token)
        .await
        .expect("compare");

    assert!(right);
    assert_eq!(wrong, [false, false, false, false]);
    assert!(!on_other_request, "KR-2 has no token at all");
    assert!(!on_other_branch);
    assert!(!unknown);
    drop(tx);
    db.finish().await;
}

#[tokio::test]
async fn a_token_with_a_backslash_or_thai_is_hashed_as_text_not_read_as_bytea() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, "KR-1", "WAITING", None).await;

    let mut tx = db.begin().await;
    for odd in [r"\x41", r"a\b", "ยกเลิก"] {
        let answer = repo
            .matches_cancel_token_in(&mut tx, &main_branch(), "KR-1", odd)
            .await;
        assert!(
            matches!(answer, Ok(false)),
            "{odd:?} is a wrong token, never a database error: {answer:?}"
        );
    }
    repo.set_cancel_token_in(&mut tx, &main_branch(), "KR-1", r"a\b")
        .await
        .expect("a backslash stores");
    assert!(
        repo.matches_cancel_token_in(&mut tx, &main_branch(), "KR-1", r"a\b")
            .await
            .expect("compare")
    );
    drop(tx);
    db.finish().await;
}

#[test]
fn the_all_filter_lists_cancelled_and_the_panel_filters_do_not() {
    assert_eq!(
        queue_statuses(QueueFilter::All),
        [
            "WAITING",
            "ACCEPTED",
            "ID_CHECKED",
            "COMPLETED",
            "CANCELLED"
        ],
        "All is every status but QUOTED (kiosk_repository.rs:99)"
    );
    for filter in [
        QueueFilter::Waiting,
        QueueFilter::Working,
        QueueFilter::Completed,
    ] {
        assert!(
            !queue_statuses(filter).contains(&"CANCELLED"),
            "{filter:?}: the kiosk queue panel reads these, and a cancelled request leaves it"
        );
    }
}

#[tokio::test]
async fn the_waiting_list_no_longer_counts_a_cancelled_request() {
    let db = TestDb::start().await;
    let repo = PgKioskRepository::new(db.pool.clone());
    add(&db, &repo, "KR-1", "WAITING", None).await;
    add(&db, &repo, "KR-2", "WAITING", None).await;
    mark(
        &db,
        &repo,
        "KR-1",
        KioskStatus::Waiting,
        &cancellation("teller-1", CancelReason::CustomerLeft { note: None }),
    )
    .await;

    let (count, _) = repo.waiting(&main_branch()).await.expect("waiting");
    assert_eq!(count, 1);
    let page = repo
        .queue_page(&main_branch(), QueueFilter::Waiting, 0)
        .await
        .expect("the waiting page");
    assert_eq!(
        page.iter().map(|(r, _)| r.id.as_str()).collect::<Vec<_>>(),
        ["KR-2"]
    );
    let all = repo
        .queue_page(&main_branch(), QueueFilter::All, 0)
        .await
        .expect("the all page");
    assert!(
        all.iter()
            .any(|(r, _)| r.id == "KR-1" && r.status == KioskStatus::Cancelled),
        "the v2 list's All filter shows it"
    );
    db.finish().await;
}
