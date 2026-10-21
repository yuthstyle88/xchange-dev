//! KC-M — the kiosk-cancel migration against a real PostgreSQL: the stored `CANCELLED` status,
//! the five nullable cancel columns and the five named CHECKs of
//! `docs/design/blueprint/kiosk-cancel.md` §3.1 item 1, with the sixth code `DAY_CLOSED` (KC-5, PR 4154).
//! Written before the migration by the ticket author (claude/studio-claude-10). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Every assertion is plain SQL against `kiosk_requests`, so it needs no Rust code beyond the
//! migration. A CHECK refusal is SQLSTATE `23514`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`.

mod kiosk_service_common;

use kiosk_service_common::*;
use sqlx::AssertSqlSafe;

const CHECK_VIOLATION: &str = "23514";

/// The SQLSTATE `sql` fails with, or `"ok"`.
async fn outcome(db: &TestDb, sql: &str) -> String {
    match sqlx::query(AssertSqlSafe(sql.to_owned()))
        .execute(&db.pool)
        .await
    {
        Ok(_) => "ok".to_owned(),
        Err(e) => match e.as_database_error().and_then(|d| d.code()) {
            Some(code) => code.into_owned(),
            None => panic!("{sql}: not a database error: {e}"),
        },
    }
}

async fn exec(db: &TestDb, sql: &str) {
    assert_eq!(outcome(db, sql).await, "ok", "{sql}");
}

/// A request of branch MAIN in `status`, held by `staff` (`NULL` for none).
async fn add_request(db: &TestDb, id: &str, status: &str, staff: &str) {
    exec(
        db,
        &format!(
            "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, \
                 rate, foreign_amount, thb, input_amount, payment_method) \
             VALUES ('MAIN', '{id}', '{status}', now(), {staff}, 'BUY', 'USD', 35.0000, 100.00, \
                 3500.00, 100.00, 'Cash')"
        ),
    )
    .await;
}

/// `UPDATE` one request with the `SET` list `set`; the outcome.
async fn update(db: &TestDb, id: &str, set: &str) -> String {
    outcome(
        db,
        &format!("UPDATE kiosk_requests SET {set} WHERE branch_id = 'MAIN' AND id = '{id}'"),
    )
    .await
}

/// The `SET` list of a cancel by `by` for `reason`.
fn cancel_by(by: &str, reason: &str) -> String {
    format!(
        "status = 'CANCELLED', cancelled_at = now(), cancelled_by = '{by}', cancel_reason = '{reason}'"
    )
}

fn cancel(reason: &str) -> String {
    cancel_by("teller-1", reason)
}

#[tokio::test]
async fn the_constraints_carry_the_names_the_design_gives_them() {
    let db = TestDb::start().await;

    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT conname::text FROM pg_constraint \
         WHERE conrelid = 'kiosk_requests'::regclass AND contype = 'c' ORDER BY conname",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read the CHECKs");
    let names: Vec<String> = rows.into_iter().map(|(n,)| n).collect();

    for name in [
        "kiosk_requests_status_check",
        "kiosk_requests_staff_check",
        "kiosk_requests_cancel_reason_check",
        "kiosk_requests_cancelled_check",
        "kiosk_requests_cancel_note_check",
        "kiosk_requests_check",
    ] {
        assert!(names.iter().any(|n| n == name), "{name} in {names:?}");
    }
    assert!(
        !names.iter().any(|n| n == "kiosk_requests_check1"),
        "the unnamed staff CHECK is dropped and replaced by kiosk_requests_staff_check: {names:?}"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_five_cancel_columns_are_nullable_text_and_timestamptz() {
    let db = TestDb::start().await;

    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT column_name::text, data_type::text, is_nullable::text \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'kiosk_requests' \
           AND column_name IN ('cancelled_at', 'cancelled_by', 'cancel_reason', 'cancel_note', \
                               'cancel_token_hash') \
         ORDER BY column_name",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read the columns");

    assert_eq!(
        rows,
        [
            ("cancel_note", "text", "YES"),
            ("cancel_reason", "text", "YES"),
            ("cancel_token_hash", "text", "YES"),
            ("cancelled_at", "timestamp with time zone", "YES"),
            ("cancelled_by", "text", "YES"),
        ]
        .map(|(c, t, n)| (c.to_owned(), t.to_owned(), n.to_owned()))
    );
    db.finish().await;
}

#[tokio::test]
async fn a_row_written_as_today_gets_no_cancel_values_and_still_inserts() {
    let db = TestDb::start().await;
    add_request(&db, "KR-OLD", "WAITING", "NULL").await;

    let row: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        bool,
    ) = sqlx::query_as(
        "SELECT cancelled_by, cancel_reason, cancel_note, cancel_token_hash, \
                    cancelled_at IS NULL \
             FROM kiosk_requests WHERE branch_id = 'MAIN' AND id = 'KR-OLD'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("read the row");
    assert_eq!(row, (None, None, None, None, true));
    db.finish().await;
}

#[tokio::test]
async fn a_waiting_request_with_no_holder_can_be_stored_cancelled() {
    let db = TestDb::start().await;
    add_request(&db, "KR-1", "WAITING", "NULL").await;

    assert_eq!(update(&db, "KR-1", &cancel("CUSTOMER_LEFT")).await, "ok");
    assert_eq!(
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT staff FROM kiosk_requests WHERE branch_id = 'MAIN' AND id = 'KR-1'"
        )
        .fetch_one(&db.pool)
        .await
        .expect("read the holder"),
        None,
        "staff keeps meaning who held it: a request cancelled from WAITING has none"
    );
    db.finish().await;
}

#[tokio::test]
async fn an_accepted_or_id_checked_request_keeps_its_holder_when_cancelled() {
    let db = TestDb::start().await;
    add_request(&db, "KR-A", "ACCEPTED", "'teller-1'").await;
    add_request(&db, "KR-I", "ID_CHECKED", "'teller-1'").await;

    assert_eq!(update(&db, "KR-A", &cancel("DUPLICATE")).await, "ok");
    assert_eq!(
        update(&db, "KR-I", &cancel_by("manager-1", "CUSTOMER_LEFT")).await,
        "ok"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_status_list_takes_cancelled_and_nothing_new_besides() {
    let db = TestDb::start().await;
    add_request(&db, "KR-1", "WAITING", "NULL").await;

    assert_eq!(
        update(&db, "KR-1", "status = 'EXPIRED'").await,
        CHECK_VIOLATION,
        "EXPIRED is computed when read, never stored"
    );
    assert_eq!(
        update(&db, "KR-1", "status = 'CANCELED'").await,
        CHECK_VIOLATION
    );
    assert_eq!(
        update(&db, "KR-1", "status = 'NO_SHOW'").await,
        CHECK_VIOLATION
    );
    db.finish().await;
}

#[tokio::test]
async fn a_held_status_still_needs_a_holder() {
    let db = TestDb::start().await;
    add_request(&db, "KR-1", "WAITING", "NULL").await;

    for held in ["ACCEPTED", "ID_CHECKED", "COMPLETED"] {
        assert_eq!(
            update(&db, "KR-1", &format!("status = '{held}'")).await,
            CHECK_VIOLATION,
            "{held} with no staff"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn cancelled_needs_when_who_and_why_together() {
    let db = TestDb::start().await;
    add_request(&db, "KR-1", "WAITING", "NULL").await;

    for missing in [
        "status = 'CANCELLED', cancelled_by = 'teller-1', cancel_reason = 'CUSTOMER_LEFT'",
        "status = 'CANCELLED', cancelled_at = now(), cancel_reason = 'CUSTOMER_LEFT'",
        "status = 'CANCELLED', cancelled_at = now(), cancelled_by = 'teller-1'",
        "status = 'CANCELLED'",
    ] {
        assert_eq!(
            update(&db, "KR-1", missing).await,
            CHECK_VIOLATION,
            "{missing}"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn a_request_that_is_not_cancelled_carries_no_cancel_record() {
    let db = TestDb::start().await;
    add_request(&db, "KR-1", "WAITING", "NULL").await;

    assert_eq!(
        update(
            &db,
            "KR-1",
            "cancelled_at = now(), cancelled_by = 'teller-1', cancel_reason = 'CUSTOMER_LEFT'"
        )
        .await,
        CHECK_VIOLATION,
        "a full cancel record on a WAITING request (kiosk_requests_cancelled_check)"
    );
    assert_eq!(
        update(&db, "KR-1", "cancel_token_hash = 'ab12'").await,
        "ok",
        "the token hash is written at confirm, while WAITING"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_reason_is_one_of_the_six_codes() {
    let db = TestDb::start().await;
    for (n, code) in [
        "CUSTOMER_LEFT",
        "CUSTOMER_CANCELLED",
        "CUSTOMER_CHANGED_MIND",
        "DUPLICATE",
        "DAY_CLOSED",
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("KR-{n}");
        add_request(&db, &id, "WAITING", "NULL").await;
        assert_eq!(update(&db, &id, &cancel(code)).await, "ok", "{code}");
    }
    add_request(&db, "KR-BAD", "WAITING", "NULL").await;
    for code in ["", "customer_left", "LEFT", "OTHERS", "DAY_CLOSE"] {
        assert_eq!(
            update(&db, "KR-BAD", &cancel(code)).await,
            CHECK_VIOLATION,
            "{code:?}"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn other_needs_a_note_of_one_to_two_hundred_characters() {
    let db = TestDb::start().await;
    add_request(&db, "KR-1", "WAITING", "NULL").await;
    let other = cancel("OTHER");

    for refused in [
        format!("{other}, cancel_note = NULL"),
        format!("{other}, cancel_note = ''"),
        format!("{other}, cancel_note = '   '"),
        format!("{other}, cancel_note = '{}'", "ก".repeat(201)),
    ] {
        assert_eq!(
            update(&db, "KR-1", &refused).await,
            CHECK_VIOLATION,
            "{}",
            &refused[..refused.len().min(120)]
        );
    }
    assert_eq!(
        update(&db, "KR-1", &format!("{other}, cancel_note = 'x'")).await,
        "ok"
    );
    add_request(&db, "KR-2", "WAITING", "NULL").await;
    assert_eq!(
        update(
            &db,
            "KR-2",
            &format!("{other}, cancel_note = '{}'", "ก".repeat(200))
        )
        .await,
        "ok",
        "200 characters, not bytes"
    );
    add_request(&db, "KR-3", "WAITING", "NULL").await;
    assert_eq!(
        update(
            &db,
            "KR-3",
            &format!("{}, cancel_note = NULL", cancel("DUPLICATE"))
        )
        .await,
        "ok",
        "a code other than OTHER needs no note"
    );
    db.finish().await;
}
