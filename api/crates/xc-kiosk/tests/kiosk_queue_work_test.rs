//! Row Q1 (`docs/design/xchange-v3.md` §5.5.3, §5.5.7): queue work in the kiosk context —
//! `WorkTimes`, `QueueFilter`, `QueuePage`, `QueueItem`, `AcceptOutcome`,
//! `Notifications.working`, and the service's `list`, `accept` and `notifications` as the
//! 10-06 bundle's `queue-work.mjs` and `server.mjs:8011-8055` answer them. Written before the
//! implementation. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The names and signatures used here are the ticket's [required] ones. Until they exist
//! this file does not compile; `kiosk_work_times_test.rs` holds what can fail on results
//! today.
//!
//! Roles are not judged here: who may `list` and `accept` is `Policy`'s (row POL). Every
//! caller below is a desk role.
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

mod kiosk_service_common;

use kiosk_service_common::{
    FakePorts, KioskStatus, KioskTime, MAIN, T0, TestDb, at, branch, bring, exact, front1, front2,
    link, owner,
};
use serde_json::{Value, json};
use xc_kiosk::domain::kiosk::value_objects::accept_outcome::AcceptOutcome;
use xc_kiosk::domain::kiosk::value_objects::notifications::WorkingRequest;
use xc_kiosk::domain::kiosk::value_objects::queue_filter::QueueFilter;
use xc_kiosk::domain::kiosk::value_objects::queue_page::QueuePage;
use xc_kiosk::domain::kiosk::value_objects::work_times::WorkTimes;

const PAGE_INVALID: &str = "หน้าข้อมูลไม่ถูกต้อง";
const FILTER_INVALID: &str = "ตัวกรองไม่ถูกต้อง";

fn ms(n: i64) -> KioskTime {
    KioskTime::from_epoch_millis(T0 + n)
}

fn times(started: Option<i64>, completed: Option<i64>) -> WorkTimes {
    WorkTimes {
        started_at: started.map(ms),
        completed_at: completed.map(ms),
    }
}

// ── the value objects ────────────────────────────────────────────────────────

/// `queue-work.mjs:3`: whole seconds from start to completion, rounded down, never below
/// zero; `None` unless both times exist.
#[test]
fn the_duration_is_whole_seconds_between_two_known_times() {
    let cases: [(Option<i64>, Option<i64>, Option<i64>); 8] = [
        (Some(0), Some(151_749), Some(151)),
        (Some(250), Some(151_999), Some(151)),
        (Some(0), Some(999), Some(0)),
        (Some(1000), Some(1000), Some(0)),
        (Some(5000), Some(4000), Some(0)),
        (Some(0), Some(86_400_000), Some(86_400)),
        (None, Some(1000), None),
        (Some(1000), None, None),
    ];
    for (started, completed, want) in cases {
        assert_eq!(
            times(started, completed).duration_seconds(),
            want,
            "{started:?} → {completed:?}"
        );
    }
    assert_eq!(WorkTimes::default(), times(None, None));
}

/// `queue-work.mjs:16`: `groups[p.filter || 'all']` — a missing or falsy filter is all; one
/// of the four names, exactly; anything else is refused.
#[test]
fn the_filter_is_read_as_v2_reads_it() {
    let accepted = [
        (None, QueueFilter::All),
        (Some(json!(null)), QueueFilter::All),
        (Some(json!("")), QueueFilter::All),
        (Some(json!(false)), QueueFilter::All),
        (Some(json!(0)), QueueFilter::All),
        (Some(json!("all")), QueueFilter::All),
        (Some(json!("waiting")), QueueFilter::Waiting),
        (Some(json!("working")), QueueFilter::Working),
        (Some(json!("completed")), QueueFilter::Completed),
    ];
    for (value, want) in accepted {
        assert_eq!(
            QueueFilter::parse(value.as_ref()).map_err(|e| e.to_string()),
            Ok(want),
            "{value:?}"
        );
    }
    for value in [
        json!("Waiting"),
        json!("WAITING"),
        json!("x"),
        json!(" all"),
        json!(1),
        json!(true),
        json!({}),
    ] {
        assert_eq!(
            QueueFilter::parse(Some(&value)).map_err(|e| e.to_string()),
            Err(FILTER_INVALID.to_owned()),
            "{value}"
        );
    }
}

/// `queue-work.mjs:15`: `Number(p.page || 0)`, a safe integer from 0 to 100,000.
#[test]
fn the_page_is_read_as_v2_reads_it() {
    let accepted: [(Option<Value>, u32); 12] = [
        (None, 0),
        (Some(json!(null)), 0),
        (Some(json!(0)), 0),
        (Some(json!("")), 0),
        (Some(json!(false)), 0),
        (Some(json!(2)), 2),
        (Some(json!("2")), 2),
        (Some(json!(" 3 ")), 3),
        (Some(json!("1e3")), 1000),
        (Some(json!(true)), 1),
        (Some(json!(2.0)), 2),
        (Some(json!(100_000)), 100_000),
    ];
    for (value, want) in accepted {
        assert_eq!(
            QueuePage::parse(value.as_ref()).map_err(|e| e.to_string()),
            Ok(want),
            "{value:?}"
        );
    }
    for value in [
        json!(-1),
        json!(1.5),
        json!(-0.5),
        json!(100_001),
        json!("x"),
        json!("2px"),
        json!({}),
        json!(1e300),
    ] {
        assert_eq!(
            QueuePage::parse(Some(&value)).map_err(|e| e.to_string()),
            Err(PAGE_INVALID.to_owned()),
            "{value}"
        );
    }
}

// ── accept ───────────────────────────────────────────────────────────────────

/// `queue-work.mjs:6-13`: the first claim of a waiting request is `Claimed` with its start;
/// the holder again (ACCEPTED or ID_CHECKED) is `AlreadyMine` with the same start; anybody
/// else, and the holder of a completed request, is `HeldBy` the holder's id; nothing to
/// claim is `Unavailable`. Only the first writes anything.
#[tokio::test(flavor = "multi_thread")]
async fn accept_answers_the_four_outcomes() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    bring(&service, MAIN, "q1", "waiting").await;
    bring(&service, MAIN, "q2", "quoted").await;

    let claimed = service
        .accept(&main, &front1(), "q1", at(5000))
        .await
        .expect("the claim");
    assert_eq!(claimed, AcceptOutcome::Claimed(times(Some(5000), None)));
    let lines = db.count("test_audit").await;

    for (who, want) in [
        (
            front1(),
            AcceptOutcome::AlreadyMine(times(Some(5000), None)),
        ),
        (front2(), AcceptOutcome::HeldBy("front1".to_owned())),
        (owner(), AcceptOutcome::HeldBy("front1".to_owned())),
    ] {
        assert_eq!(
            service
                .accept(&main, &who, "q1", at(6000))
                .await
                .expect("an outcome"),
            want,
            "{who:?}"
        );
    }
    for id in ["q2", "nope", ""] {
        assert_eq!(
            service
                .accept(&main, &front1(), id, at(6000))
                .await
                .expect("an outcome"),
            AcceptOutcome::Unavailable,
            "{id:?}"
        );
    }
    assert_eq!(
        service
            .accept(&branch("B2"), &front2(), "q1", at(6000))
            .await
            .expect("an outcome"),
        AcceptOutcome::Unavailable,
        "a request of another branch"
    );
    assert_eq!(db.count("test_audit").await, lines, "only the claim wrote");

    service
        .identity_checked(&main, &front1(), "q1", true, at(7000))
        .await
        .expect("checked");
    assert_eq!(
        service
            .accept(&main, &front1(), "q1", at(8000))
            .await
            .expect("an outcome"),
        AcceptOutcome::AlreadyMine(times(Some(5000), None))
    );

    let mut tx = db.begin().await;
    service
        .complete_in(
            &mut tx,
            &main,
            "q1",
            &link("S-1", "XC-1"),
            &front1(),
            at(9500),
        )
        .await
        .expect("complete");
    tx.commit().await.expect("commit");
    assert_eq!(
        service
            .accept(&main, &front1(), "q1", at(10_000))
            .await
            .expect("an outcome"),
        AcceptOutcome::HeldBy("front1".to_owned()),
        "the holder of a completed request is named, as v2 does"
    );
    db.finish().await;
}

// ── list ─────────────────────────────────────────────────────────────────────

fn ids(page: &QueuePage) -> Vec<&str> {
    page.items.iter().map(|i| i.request.id.as_str()).collect()
}

/// `queue-work.mjs:14-20`: per filter, newest first then id descending, a quote nobody
/// confirmed never; each item with its identity and its times.
#[tokio::test(flavor = "multi_thread")]
async fn list_filters_and_orders_the_queue() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    for id in ["q1", "q2", "q3"] {
        bring(&service, MAIN, id, "waiting").await;
    }
    bring(&service, MAIN, "q0", "quoted").await;
    service
        .accept(&main, &front1(), "q2", at(5000))
        .await
        .expect("q2 claimed");

    let list = |filter: QueueFilter| {
        let service = service.clone();
        let main = main.clone();
        async move {
            service
                .list(&main, &front1(), filter, 0, at(9000))
                .await
                .expect("a page")
        }
    };
    let all = list(QueueFilter::All).await;
    assert_eq!(
        ids(&all),
        ["q3", "q2", "q1"],
        "same quote time: id descending"
    );
    assert_eq!((all.page, all.has_more), (0, false));
    assert_eq!(ids(&list(QueueFilter::Waiting).await), ["q3", "q1"]);
    assert_eq!(ids(&list(QueueFilter::Working).await), ["q2"]);
    assert!(list(QueueFilter::Completed).await.items.is_empty());

    let q2 = &all.items[1];
    assert_eq!(q2.times, times(Some(5000), None));
    assert_eq!(q2.identity.queue, "K-Q2");
    assert_eq!(q2.identity.status, KioskStatus::Accepted);
    assert_eq!(q2.request.staff.as_deref(), Some("front1"));
    assert_eq!(all.items[0].times, WorkTimes::default());
    db.finish().await;
}

/// Fifty to a page; `has_more` when a 51st exists; a page past the end is empty; a newer
/// quote time comes first whatever its id.
#[tokio::test(flavor = "multi_thread")]
async fn list_pages_by_fifty() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    for n in 0..53i64 {
        sqlx::query(
            "INSERT INTO kiosk_requests (branch_id, id, status, created_at, side, currency, \
             rate, foreign_amount, thb, input_amount, payment_method) \
             VALUES ('MAIN', $1, 'WAITING', $2, 'BUY', 'USD', 32.1, 100, 3210, 100, 'Cash')",
        )
        .bind(format!("r{:02}", 52 - n))
        .bind(exact(n * 1000))
        .execute(&db.pool)
        .await
        .expect("a waiting request");
    }
    let page = |n: u32| {
        let service = service.clone();
        let main = main.clone();
        async move {
            service
                .list(&main, &front1(), QueueFilter::Waiting, n, at(90_000))
                .await
                .expect("a page")
        }
    };
    let first = page(0).await;
    assert_eq!(first.items.len(), 50);
    assert!(first.has_more);
    assert_eq!(ids(&first)[0], "r00", "the newest quote first");
    assert_eq!(ids(&first)[49], "r49");
    let second = page(1).await;
    assert_eq!(ids(&second), ["r50", "r51", "r52"]);
    assert_eq!((second.page, second.has_more), (1, false));
    let third = page(2).await;
    assert!(third.items.is_empty());
    assert!(!third.has_more);
    db.finish().await;
}

// ── notifications ────────────────────────────────────────────────────────────

async fn working_row(db: &TestDb, id: &str, staff: &str, status: &str, started: Option<i64>) {
    sqlx::query(
        "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, \
         rate, foreign_amount, thb, input_amount, payment_method, started_at) \
         VALUES ('MAIN', $1, $2, $3, $4, 'BUY', 'USD', 32.1, 100, 3210, 100, 'Cash', $5)",
    )
    .bind(id)
    .bind(status)
    .bind(exact(0))
    .bind(staff)
    .bind(started.map(exact))
    .execute(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("{id}: {e}"));
}

/// `server.mjs:8015-8018`: every ACCEPTED / ID_CHECKED request is counted; up to ten are
/// listed, by start time with no time last, then by id.
#[tokio::test(flavor = "multi_thread")]
async fn notifications_name_who_works_since_when() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    let main = branch(MAIN);
    bring(&service, MAIN, "w1", "waiting").await;

    let none = service
        .notifications(&main, &front1())
        .await
        .expect("notifications");
    assert_eq!((none.count, none.working_count), (1, 0));
    assert!(none.working.is_empty());

    working_row(&db, "a-old", "emp-9", "ACCEPTED", None).await;
    working_row(&db, "b-late", "emp-2", "ID_CHECKED", Some(9000)).await;
    working_row(&db, "c-tie", "emp-3", "ACCEPTED", Some(3000)).await;
    working_row(&db, "a-tie", "emp-4", "ACCEPTED", Some(3000)).await;
    for n in 0..8i64 {
        working_row(&db, &format!("m{n}"), "emp-5", "ACCEPTED", Some(4000 + n)).await;
    }
    working_row(&db, "done", "emp-6", "COMPLETED", Some(1000)).await;

    let seen = service
        .notifications(&main, &front1())
        .await
        .expect("notifications");
    assert_eq!(seen.count, 1, "one waits");
    assert_eq!(seen.working_count, 12, "every request at work is counted");
    let listed: Vec<&str> = seen.working.iter().map(|w| w.id.as_str()).collect();
    assert_eq!(
        listed,
        [
            "a-tie", "c-tie", "m0", "m1", "m2", "m3", "m4", "m5", "m6", "m7"
        ],
        "ten, by start then id; the later start and the unknown one fall off"
    );
    assert_eq!(
        seen.working[0],
        WorkingRequest {
            id: "a-tie".to_owned(),
            staff: "emp-4".to_owned(),
            started_at: Some(ms(3000)),
            status: KioskStatus::Accepted,
        }
    );
    db.finish().await;
}

/// The unknown start sorts after every known one (`NULLS LAST`), also when fewer than ten
/// work; an ID_CHECKED request is listed with its status.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_start_is_listed_last() {
    let db = TestDb::start().await;
    let service = db.service(FakePorts::standard());
    working_row(&db, "a-old", "emp-9", "ACCEPTED", None).await;
    working_row(&db, "b-late", "emp-2", "ID_CHECKED", Some(9000)).await;
    let seen = service
        .notifications(&branch(MAIN), &front1())
        .await
        .expect("notifications");
    assert_eq!(
        seen.working,
        vec![
            WorkingRequest {
                id: "b-late".to_owned(),
                staff: "emp-2".to_owned(),
                started_at: Some(ms(9000)),
                status: KioskStatus::IdChecked,
            },
            WorkingRequest {
                id: "a-old".to_owned(),
                staff: "emp-9".to_owned(),
                started_at: None,
                status: KioskStatus::Accepted,
            },
        ]
    );
    db.finish().await;
}
