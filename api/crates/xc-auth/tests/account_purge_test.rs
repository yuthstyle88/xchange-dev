//! `AccountService::purge_expired` (ticket #84): the sign-in tables stop
//! growing for ever.
//!
//! What is deleted, and nothing else:
//! - a session of the owner or of an employee whose `expires` is before `now`
//!   (`now` cut to the millisecond, as every time the service stores);
//! - a row of `auth_attempts` whose 15-minute window ended at or before the
//!   start of the window `now` falls in.
//!
//! `now` is an argument: the service reads no machine clock. The three deletes
//! are one transaction, and the answer says how many rows of each kind went.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose
//! name ends in `_test`). Without it they FAIL; none skips.

mod account_common;

use account_common::{LIMIT, Service, TestDb, WRONG, at, refusal, working_employee};
use chrono::{DateTime, Duration, Utc};
use pos_error::AppError;
use serde_json::json;
use sqlx::AssertSqlSafe;
use xc_auth::domain::account::entities::session::SignedIn;
use xc_auth::domain::account::repositories::account_repository::Purged;

const OWNER_PASSWORD: &str = "owner password 01";
const STAFF_PASSWORD: &str = "staff password 01";
const HALF_A_DAY: i64 = 12 * 3600;

async fn count_where(db: &TestDb, table: &str, condition: &str, now: DateTime<Utc>) -> i64 {
    sqlx::query_scalar::<_, i64>(AssertSqlSafe(format!(
        "SELECT count(*) FROM {table} WHERE {condition}"
    )))
    .bind(now)
    .fetch_one(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("count {table} where {condition}: {e}"))
}

async fn staff_token(service: &Service, username: &str, ip: &str, now: DateTime<Utc>) -> String {
    service
        .login_employee(
            &json!({ "username": username, "password": STAFF_PASSWORD }),
            ip,
            now,
        )
        .await
        .unwrap_or_else(|e| panic!("{username} signs in: {e:?}"))
        .token
}

async fn owner_token(service: &Service, ip: &str, now: DateTime<Utc>) -> String {
    service
        .login(
            &json!({ "username": "boss", "password": OWNER_PASSWORD }),
            ip,
            now,
        )
        .await
        .unwrap_or_else(|e| panic!("the owner signs in: {e:?}"))
        .token
}

/// The owner `boss` and the employee `anna`, each signed in at `at(0)` and
/// again an hour later.
struct Signed {
    owner_early: String,
    owner_late: String,
    staff_early: String,
    staff_late: String,
}

async fn two_sign_ins_each(service: &Service) -> Signed {
    service
        .create_owner("boss", OWNER_PASSWORD, at(0))
        .await
        .expect("the owner");
    working_employee(service, "anna", STAFF_PASSWORD, at(0)).await;
    Signed {
        owner_early: owner_token(service, "10.0.0.1", at(0)).await,
        staff_early: staff_token(service, "anna", "10.0.0.1", at(0)).await,
        owner_late: owner_token(service, "10.0.0.2", at(3600)).await,
        staff_late: staff_token(service, "anna", "10.0.0.2", at(3600)).await,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_sessions_go_and_the_others_still_sign_their_holder_in() {
    let db = TestDb::start().await;
    let service = db.service();
    let signed = two_sign_ins_each(&service).await;

    // One millisecond after the sessions of at(0) ran out.
    let now = at(HALF_A_DAY) + Duration::milliseconds(1);
    let dead_owner = count_where(&db, "owner_sessions", "expires < $1", now).await;
    let dead_staff = count_where(&db, "employee_sessions", "expires < $1", now).await;
    let live_owner = count_where(&db, "owner_sessions", "expires >= $1", now).await;
    let live_staff = count_where(&db, "employee_sessions", "expires >= $1", now).await;
    assert_eq!((dead_owner, live_owner), (1, 1));
    // `working_employee` signed in once itself, also at at(0)
    assert_eq!((dead_staff, live_staff), (2, 1));

    let purged = service.purge_expired(now).await.expect("the purge");
    assert_eq!(
        (purged.owner_sessions, purged.employee_sessions),
        (dead_owner as u64, dead_staff as u64),
        "the answer counts what went"
    );
    assert_eq!(db.count("owner_sessions").await, live_owner);
    assert_eq!(db.count("employee_sessions").await, live_staff);
    assert_eq!(
        count_where(&db, "owner_sessions", "expires < $1", now).await
            + count_where(&db, "employee_sessions", "expires < $1", now).await,
        0
    );

    // the sessions of an hour later still are their holders
    let who = service
        .status(&signed.owner_late, "", now)
        .await
        .expect("status");
    assert!(matches!(who, Some(SignedIn::Owner(_))), "{who:?}");
    let anna = service
        .employee_session(&signed.staff_late, false, now)
        .await
        .expect("the later staff session");
    assert_eq!(anna.username, "anna");
    // and the early ones are nobody, as they were before the purge
    assert!(matches!(
        service.status(&signed.owner_early, "", now).await,
        Ok(None)
    ));
    assert!(matches!(
        service
            .employee_session(&signed.staff_early, false, now)
            .await,
        Err(AppError::Unauthenticated)
    ));

    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_goes_only_once_its_last_millisecond_is_over() {
    let db = TestDb::start().await;
    let service = db.service();
    two_sign_ins_each(&service).await;
    let sessions = (
        db.count("owner_sessions").await,
        db.count("employee_sessions").await,
    );

    // `expires` of the first sign-ins is exactly at(12 h). Before it, at it,
    // and within the same millisecond, nothing goes: expired means `expires < now`
    // with `now` cut to the millisecond.
    for now in [
        at(0),
        at(HALF_A_DAY) - Duration::milliseconds(1),
        at(HALF_A_DAY),
        at(HALF_A_DAY) + Duration::microseconds(999),
    ] {
        let purged = service.purge_expired(now).await.expect("the purge");
        assert_eq!(
            (purged.owner_sessions, purged.employee_sessions),
            (0, 0),
            "at {now}"
        );
        assert_eq!(
            (
                db.count("owner_sessions").await,
                db.count("employee_sessions").await
            ),
            sessions,
            "at {now}"
        );
    }

    let purged = service
        .purge_expired(at(HALF_A_DAY) + Duration::milliseconds(1))
        .await
        .expect("the purge");
    assert_eq!((purged.owner_sessions, purged.employee_sessions), (1, 2));

    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn past_attempt_windows_go_and_the_current_window_still_counts() {
    let db = TestDb::start().await;
    let service = db.service();
    working_employee(&service, "anna", STAFF_PASSWORD, at(-3600)).await;
    let wrong = json!({ "username": "anna", "password": "not the password" });
    let attempt = |now: DateTime<Utc>| {
        let (service, wrong) = (service.clone(), wrong.clone());
        async move {
            let got = service.login_employee(&wrong, "10.0.0.9", now).await;
            refusal(&got.expect_err("a wrong password is refused"))
        }
    };

    // the window before at(0): two attempts, one of them in its last millisecond
    assert_eq!(attempt(at(-900)).await, WRONG);
    assert_eq!(attempt(at(0) - Duration::milliseconds(1)).await, WRONG);
    // the window that starts at at(0): ten attempts, the name is at its limit
    for second in 0..10 {
        assert_eq!(attempt(at(second)).await, WRONG);
    }
    assert_eq!(attempt(at(10)).await, LIMIT);

    let now = at(899) + Duration::milliseconds(999); // the last millisecond of that window
    let past = count_where(&db, "auth_attempts", "window_start < $1", at(0)).await;
    let current = count_where(&db, "auth_attempts", "window_start >= $1", at(0)).await;
    assert!(past >= 2 && current == 2, "past {past}, current {current}");

    let purged = service.purge_expired(now).await.expect("the purge");
    assert_eq!(purged.attempt_windows, past as u64);
    assert_eq!(db.count("auth_attempts").await, current);
    assert_eq!(
        count_where(&db, "auth_attempts", "window_start < $1", at(0)).await,
        0
    );
    // the limit of the current window is what it was
    assert_eq!(attempt(now).await, LIMIT);

    // the window is over at at(900): now its rows are a past window too
    let purged = service.purge_expired(at(900)).await.expect("the purge");
    assert_eq!(purged.attempt_windows, current as u64);
    assert_eq!(db.count("auth_attempts").await, 0);
    assert_eq!(attempt(at(900)).await, WRONG);

    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_purge_removes_nothing_and_nothing_else_is_ever_deleted() {
    let db = TestDb::start().await;
    let service = db.service();
    two_sign_ins_each(&service).await;
    // a disabled employee and the audit trail are not the purge's business
    let anna = account_common::list(&service, at(0))
        .await
        .into_iter()
        .find(|e| e.username == "anna")
        .expect("anna");
    working_employee(&service, "gone", STAFF_PASSWORD, at(0)).await;
    let gone = account_common::list(&service, at(0))
        .await
        .into_iter()
        .find(|e| e.username == "gone")
        .expect("gone");
    service
        .employees("disable", &json!({ "id": gone.id }), "owner", at(1))
        .await
        .expect("disable");
    let kept = (
        db.count("owners").await,
        db.count("employees").await,
        db.count("employee_audit").await,
    );

    let now = at(10 * 24 * 3600);
    let first = service.purge_expired(now).await.expect("the purge");
    assert!(
        first.owner_sessions == 2 && first.employee_sessions >= 3 && first.attempt_windows >= 2,
        "{first:?}"
    );
    assert_eq!(
        (
            db.count("owner_sessions").await,
            db.count("employee_sessions").await,
            db.count("auth_attempts").await
        ),
        (0, 0, 0)
    );

    let second = service.purge_expired(now).await.expect("the second purge");
    assert_eq!(second, Purged::default());

    assert_eq!(
        (
            db.count("owners").await,
            db.count("employees").await,
            db.count("employee_audit").await
        ),
        kept
    );
    let listed = account_common::list(&service, now).await;
    assert!(listed.iter().any(|e| e.id == anna.id && e.active));
    assert!(listed.iter().any(|e| e.id == gone.id && !e.active));

    db.drop_schema().await;
}

/// The three deletes are one transaction: when any of them fails, none of
/// them happened. Each table in turn is made unusable, so the order the
/// deletes run in does not decide whether this test can see it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_purge_that_fails_part_way_removes_nothing() {
    for broken in ["owner_sessions", "employee_sessions", "auth_attempts"] {
        let db = TestDb::start().await;
        let service = db.service();
        two_sign_ins_each(&service).await;
        let now = at(10 * 24 * 3600);

        sqlx::query(AssertSqlSafe(format!(
            "ALTER TABLE {broken} RENAME TO {broken}_away"
        )))
        .execute(&db.pool)
        .await
        .expect("take a table away");

        let got = service.purge_expired(now).await;
        assert!(
            matches!(got, Err(AppError::Database(_))),
            "without {broken}: {got:?}"
        );
        for table in ["owner_sessions", "employee_sessions", "auth_attempts"] {
            if table == broken {
                continue;
            }
            assert!(
                db.count(table).await > 0,
                "without {broken}: the purge failed and still emptied {table}"
            );
        }

        db.drop_schema().await;
    }
}
