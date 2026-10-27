//! XC-BE-GAPS-A row 5: the desk save's `KIOSK_CUSTOMER_DIFFERS` line (D13) and the passport skip's
//! placeholder (D20), at the kiosk service.
//! Written before the implementation by the ticket author (claude/491239fc, XC-BE-GAPS-A row 5, 2026-10-27).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Decided, and pinned here:
//! - D13 (`docs/design/blueprint/kiosk-shared-phone.md:229`, behaviour row B16 `:391`): the desk saves on the
//!   passport's customer A; when A is not the request's member B, the request keeps B and one
//!   `KIOSK_CUSTOMER_DIFFERS` line `{savedCustomerId: A, requestCustomerId: B}` is written beside `KIOSK_COMPLETED`,
//!   after it, by the saver, on the request, at the same time, on the save's own transaction.
//! - D20 (`:236`, class table `:325`): the desk's skip (`web/src/screens/desk/steps/customer/step.tsx` `onSkip`,
//!   `step.test.tsx:402`) sends passport `WALKIN` / nationality `XXX`; `WalkInPlaceholder::is` is the one place that
//!   recognises it, whatever the ASCII case and surrounding spaces. A save on it reaches the service as
//!   `saved_customer = None` and never counts as a different customer.
//! - No line when A = B, when the request has no member (a kiosk request), or on a repeat save (the request is
//!   already completed by this submission and nothing moves, as today).
//! - `complete_in` stays `complete_saved_in(…, None, …)`: it never writes the line.
//!
//! The service tests need PostgreSQL: see `kiosk_service_common/mod.rs`.

mod kiosk_service_common;

use kiosk_service_common::*;
use serde_json::{Value, json};
use xc_kiosk::domain::kiosk::value_objects::walk_in_placeholder::WalkInPlaceholder;

// ── WalkInPlaceholder (D20) ─────────────────────────────────────────────────

/// The desk's skip values (`step.test.tsx:402-403`), recognised whatever the ASCII case and surrounding spaces.
#[test]
fn gapsa5_the_placeholder_is_recognised_whatever_the_case_and_spaces() {
    assert_eq!(WalkInPlaceholder::PASSPORT_NO, "WALKIN");
    assert_eq!(WalkInPlaceholder::NATIONALITY, "XXX");
    for (passport_no, nationality) in [
        ("WALKIN", "XXX"),
        (" walkin ", "xxx"),
        ("WalkIn", "xXx"),
        ("\tWALKIN\n", "  XXX  "),
        (
            WalkInPlaceholder::PASSPORT_NO,
            WalkInPlaceholder::NATIONALITY,
        ),
    ] {
        assert!(
            WalkInPlaceholder::is(passport_no, nationality),
            "({passport_no:?}, {nationality:?}) is the desk's skip"
        );
    }
}

#[test]
fn gapsa5_anything_short_of_both_values_is_not_the_placeholder() {
    for (passport_no, nationality) in [
        ("WALKIN", "THA"),
        ("AA1234567", "XXX"),
        ("AA1234567", "THA"),
        ("WALKIN", ""),
        ("", "XXX"),
        ("", ""),
        ("XXX", "WALKIN"),
        ("WALKIN1", "XXX"),
        ("WALKIN", "XXXX"),
    ] {
        assert!(
            !WalkInPlaceholder::is(passport_no, nationality),
            "({passport_no:?}, {nationality:?}) is a real passport, not the skip"
        );
    }
}

// ── complete_saved_in (D13) ─────────────────────────────────────────────────

const A: &str = "customer-A";
const B: &str = "member-B";

/// Makes `customer` the member of request `id`, as `set_customer_in` does for a web request.
async fn member(db: &TestDb, id: &str, customer: &str) {
    let done = sqlx::query(
        "UPDATE kiosk_requests SET customer_id = $1 WHERE branch_id = 'MAIN' AND id = $2",
    )
    .bind(customer)
    .bind(id)
    .execute(&db.pool)
    .await
    .expect("set the request's member");
    assert_eq!(done.rows_affected(), 1, "request {id} exists");
}

async fn member_of(db: &TestDb, id: &str) -> Option<String> {
    sqlx::query_scalar(
        "SELECT customer_id FROM kiosk_requests WHERE branch_id = 'MAIN' AND id = $1",
    )
    .bind(id)
    .fetch_one(&db.pool)
    .await
    .expect("read the request's member")
}

/// The number of the last audit line written so far.
async fn last_line(db: &TestDb) -> i64 {
    db.audit_since(0).await.1
}

/// The save of request `id` as the desk makes it, on `saved` (None = the passport skip), committed.
async fn save(
    db: &TestDb,
    service: &KioskService,
    id: &str,
    submission: &str,
    document: &str,
    saved: Option<&str>,
    ms: i64,
) {
    let main = branch(MAIN);
    let mut tx = db.begin().await;
    service
        .check_save_in(&mut tx, &main, id, &save100(submission), &front1())
        .await
        .expect("ready to save");
    service
        .complete_saved_in(
            &mut tx,
            &main,
            id,
            &link(submission, document),
            saved,
            &front1(),
            at(ms),
        )
        .await
        .expect("complete");
    tx.commit().await.expect("the save commits");
}

fn completed(id: &str, document: &str) -> Value {
    json!({ "branch": "MAIN", "actor": "front1", "kind": "KIOSK_COMPLETED", "subject": id,
            "documentRef": document })
}

fn differs(id: &str, saved: &str, request: &str) -> Value {
    json!({ "branch": "MAIN", "actor": "front1", "kind": "KIOSK_CUSTOMER_DIFFERS", "subject": id,
            "savedCustomerId": saved, "requestCustomerId": request })
}

/// B16: saved on A, the request's member is B ⇒ `KIOSK_COMPLETED`, then one `KIOSK_CUSTOMER_DIFFERS`, by the
/// saver, on the request, at the same (millisecond) time; the request keeps B. Both lines live and die with the
/// save's transaction.
#[tokio::test(flavor = "multi_thread")]
async fn gapsa5_a_save_on_another_customer_writes_customer_differs_after_completed() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    bring(&service, MAIN, "q1", "checked").await;
    member(&db, "q1", B).await;
    let before = last_line(&db).await;

    // a save that rolls back leaves neither line
    let mut tx = db.begin().await;
    service
        .complete_saved_in(
            &mut tx,
            &main,
            "q1",
            &link("SUB-1", "XC-20261006-000001"),
            Some(A),
            &front1(),
            at(7000),
        )
        .await
        .expect("complete, then roll back");
    tx.rollback().await.expect("the save fails after the hook");
    assert_eq!(db.audit_since(before).await.0, json!([]));

    save(
        &db,
        &service,
        "q1",
        "SUB-1",
        "XC-20261006-000001",
        Some(A),
        7000,
    )
    .await;
    let (lines, _, times) = db.audit_since(before).await;
    assert_eq!(
        lines,
        json!([completed("q1", "XC-20261006-000001"), differs("q1", A, B),])
    );
    assert_eq!(
        times,
        [exact(7000), exact(7000)],
        "the same time, cut to ms"
    );
    assert_eq!(db.status(MAIN, "q1").await.as_deref(), Some("COMPLETED"));
    assert_eq!(
        member_of(&db, "q1").await.as_deref(),
        Some(B),
        "the member stays on the request"
    );
    db.finish().await;
}

/// A = B: the save is on the request's own member, so only `KIOSK_COMPLETED`. Beside it, a request whose member
/// differs gets its line, so the rule is seen to run.
#[tokio::test(flavor = "multi_thread")]
async fn gapsa5_a_save_on_the_requests_member_writes_no_customer_differs() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    bring(&service, MAIN, "q1", "checked").await;
    bring(&service, MAIN, "q2", "checked").await;
    member(&db, "q1", A).await;
    member(&db, "q2", B).await;
    let before = last_line(&db).await;

    save(&db, &service, "q1", "SUB-1", "XC-1", Some(A), 7000).await;
    let (lines, after_q1, _) = db.audit_since(before).await;
    assert_eq!(lines, json!([completed("q1", "XC-1")]));

    save(&db, &service, "q2", "SUB-2", "XC-2", Some(A), 8000).await;
    let (lines, _, _) = db.audit_since(after_q1).await;
    assert_eq!(lines, json!([completed("q2", "XC-2"), differs("q2", A, B)]));
    assert_eq!(member_of(&db, "q1").await.as_deref(), Some(A));
    assert_eq!(member_of(&db, "q2").await.as_deref(), Some(B));
    db.finish().await;
}

/// D20: the passport skip reaches the service as `None` and never counts as a different customer, through
/// `complete_saved_in(None)` and through `complete_in`; the request keeps its member. Beside them, the same
/// request shape saved on A gets its line.
#[tokio::test(flavor = "multi_thread")]
async fn gapsa5_a_walk_in_save_writes_no_customer_differs() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    bring(&service, MAIN, "q1", "checked").await;
    bring(&service, MAIN, "q2", "checked").await;
    bring(&service, MAIN, "q3", "checked").await;
    for id in ["q1", "q2", "q3"] {
        member(&db, id, B).await;
    }
    let before = last_line(&db).await;

    save(&db, &service, "q1", "SUB-1", "XC-1", None, 7000).await;
    let (lines, after_q1, _) = db.audit_since(before).await;
    assert_eq!(lines, json!([completed("q1", "XC-1")]), "the skip: no line");

    let mut tx = db.begin().await;
    service
        .complete_in(
            &mut tx,
            &main,
            "q2",
            &link("SUB-2", "XC-2"),
            &front1(),
            at(8000),
        )
        .await
        .expect("complete_in");
    tx.commit().await.expect("commit");
    let (lines, after_q2, _) = db.audit_since(after_q1).await;
    assert_eq!(
        lines,
        json!([completed("q2", "XC-2")]),
        "complete_in is complete_saved_in with None"
    );

    save(&db, &service, "q3", "SUB-3", "XC-3", Some(A), 9000).await;
    let (lines, _, _) = db.audit_since(after_q2).await;
    assert_eq!(lines, json!([completed("q3", "XC-3"), differs("q3", A, B)]));
    for id in ["q1", "q2", "q3"] {
        assert_eq!(member_of(&db, id).await.as_deref(), Some(B), "{id}");
    }
    db.finish().await;
}

/// A request with no member (a kiosk request, `customer_in` = None): nothing to differ from, so no line. Beside
/// it, a request with a member gets its line.
#[tokio::test(flavor = "multi_thread")]
async fn gapsa5_a_request_with_no_member_writes_no_customer_differs() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    bring(&service, MAIN, "q1", "checked").await;
    bring(&service, MAIN, "q2", "checked").await;
    member(&db, "q2", B).await;
    let before = last_line(&db).await;

    save(&db, &service, "q1", "SUB-1", "XC-1", Some(A), 7000).await;
    let (lines, after_q1, _) = db.audit_since(before).await;
    assert_eq!(lines, json!([completed("q1", "XC-1")]));
    assert_eq!(
        member_of(&db, "q1").await,
        None,
        "the save does not give the request a member"
    );

    save(&db, &service, "q2", "SUB-2", "XC-2", Some(A), 8000).await;
    let (lines, _, _) = db.audit_since(after_q1).await;
    assert_eq!(lines, json!([completed("q2", "XC-2"), differs("q2", A, B)]));
    db.finish().await;
}

/// A repeat of the same save finds the request already completed by this submission: nothing moves, so no line
/// at all, not even `KIOSK_CUSTOMER_DIFFERS` a second time.
#[tokio::test(flavor = "multi_thread")]
async fn gapsa5_a_repeat_save_writes_no_line_at_all() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    bring(&service, MAIN, "q1", "checked").await;
    member(&db, "q1", B).await;
    let before = last_line(&db).await;

    save(&db, &service, "q1", "SUB-1", "XC-1", Some(A), 7000).await;
    let (lines, after_first, _) = db.audit_since(before).await;
    assert_eq!(lines, json!([completed("q1", "XC-1"), differs("q1", A, B)]));

    let mut tx = db.begin().await;
    service
        .complete_saved_in(
            &mut tx,
            &main,
            "q1",
            &link("SUB-1", "XC-1"),
            Some(A),
            &front1(),
            at(9000),
        )
        .await
        .expect("the same save again");
    tx.commit().await.expect("commit");
    assert_eq!(db.audit_since(after_first).await.0, json!([]));
    assert_eq!(member_of(&db, "q1").await.as_deref(), Some(B));
    db.finish().await;
}
