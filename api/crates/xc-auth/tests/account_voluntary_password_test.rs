//! Row SP-1 (#331) — an employee's voluntary password change asks for the current
//! password (`docs/design/xchange-v3.md` §5.3 *The staff's own password change*).
//! Written before the code. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in
//! `_test`). Without it they fail; none of them skips.
//!
//! Mutations that must turn this red:
//! - do not ask for `currentPassword`                  -> …_without_the_current_password_…
//! - compare it as text, or skip it when it is wrong   -> a_wrong_current_password_…
//! - ask for it on the first-password path             -> the_first_password_change_…
//! - accept the unchanged password                     -> the_same_password_…
//! - check the length before the current password      -> the_current_password_is_checked_…
//! - do not count, or count on another key             -> voluntary_attempts_count_…
//! - count the first-password path                     -> the_first_password_change_is_not_counted

mod account_common;

use account_common::*;
use serde_json::{Value, json};

const CURRENT_WRONG: &str = "รหัสผ่านปัจจุบันไม่ถูกต้อง";
const SAME_PASSWORD: &str = "รหัสผ่านใหม่ต้องไม่ซ้ำกับรหัสผ่านปัจจุบัน";
const OLD: &str = "second-password-2";
const NEW: &str = "third-password-33";

/// A working employee (`mustChange` false, password `OLD`) and a fresh staff session.
/// Uses two attempts of the employee's name counter: the first sign-in inside
/// `working_employee`, and this one.
async fn signed_in(service: &Service, username: &str) -> String {
    working_employee(service, username, OLD, at(0)).await;
    service
        .login_employee(
            &json!({ "username": username, "password": OLD }),
            &unique("counter"),
            at(1),
        )
        .await
        .expect("sign in with the working password")
        .token
}

async fn refused(service: &Service, token: &str, payload: Value) -> String {
    let error = service
        .change_password(token, &payload, at(2))
        .await
        .expect_err("refused");
    refusal(&error)
}

/// The password that signs `username` in now, among `OLD` and `NEW`; `None` when neither.
async fn password_now(service: &Service, username: &str) -> Option<&'static str> {
    for password in [OLD, NEW] {
        let answer = service
            .login_employee(
                &json!({ "username": username, "password": password }),
                &unique("probe"),
                // the next 15-minute window, so the probe never meets the limit
                at(15 * 60 + 1),
            )
            .await;
        if answer.is_ok() {
            return Some(password);
        }
    }
    None
}

/// The session still names the employee.
async fn still_signed_in(service: &Service, token: &str) -> bool {
    service.status("", token, at(3)).await.unwrap().is_some()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_voluntary_change_without_the_current_password_is_refused_and_changes_nothing() {
    let db = TestDb::start().await;
    let service = db.service();
    let token = signed_in(&service, "somchai").await;
    let before = db.audit().await.len();

    assert_eq!(
        refused(&service, &token, json!({ "password": NEW })).await,
        CURRENT_WRONG
    );
    assert!(still_signed_in(&service, &token).await, "the session ended");
    assert_eq!(db.audit().await.len(), before, "an audit line was written");
    assert_eq!(password_now(&service, "somchai").await, Some(OLD));
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_current_password_is_refused_with_one_sentence() {
    let db = TestDb::start().await;
    let service = db.service();
    let token = signed_in(&service, "somchai").await;

    for current in [
        json!("wrong-password-99"),
        json!(""),
        json!(null),
        json!(1234567890123_i64),
        json!(["second-password-2"]),
        json!({ "password": OLD }),
        // the right text, but not as the field's own value
        json!(format!("{OLD} ")),
        json!(OLD.to_uppercase()),
    ] {
        assert_eq!(
            refused(
                &service,
                &token,
                json!({ "currentPassword": current, "password": NEW })
            )
            .await,
            CURRENT_WRONG,
            "currentPassword {current}"
        );
    }
    assert!(still_signed_in(&service, &token).await, "the session ended");
    assert_eq!(password_now(&service, "somchai").await, Some(OLD));
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_right_current_password_changes_it_and_ends_the_sessions() {
    let db = TestDb::start().await;
    let service = db.service();
    let token = signed_in(&service, "somchai").await;

    service
        .change_password(
            &token,
            &json!({ "currentPassword": OLD, "password": NEW }),
            at(2),
        )
        .await
        .expect("changed");
    assert!(
        !still_signed_in(&service, &token).await,
        "the session goes on"
    );
    assert_eq!(password_now(&service, "somchai").await, Some(NEW));
    let employee = list(&service, at(3))
        .await
        .into_iter()
        .find(|e| e.username == "somchai")
        .expect("listed");
    assert!(
        !employee.must_change,
        "a voluntary change sets no must-change"
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_password_is_refused() {
    let db = TestDb::start().await;
    let service = db.service();
    let token = signed_in(&service, "somchai").await;

    assert_eq!(
        refused(
            &service,
            &token,
            json!({ "currentPassword": OLD, "password": OLD })
        )
        .await,
        SAME_PASSWORD
    );
    assert!(still_signed_in(&service, &token).await, "the session ended");
    assert_eq!(password_now(&service, "somchai").await, Some(OLD));
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_current_password_is_checked_before_the_new_one() {
    let db = TestDb::start().await;
    let service = db.service();
    let token = signed_in(&service, "somchai").await;

    // a wrong current password and a short new one: the current one answers
    assert_eq!(
        refused(
            &service,
            &token,
            json!({ "currentPassword": "wrong-password-99", "password": "short" })
        )
        .await,
        CURRENT_WRONG
    );
    // the right current password and a short new one: the length answers
    for new in [json!("short"), json!("x".repeat(129)), json!(null)] {
        assert_eq!(
            refused(
                &service,
                &token,
                json!({ "currentPassword": OLD, "password": new })
            )
            .await,
            PASSWORD_LENGTH,
            "new password {new}"
        );
    }
    assert!(still_signed_in(&service, &token).await, "the session ended");
    assert_eq!(password_now(&service, "somchai").await, Some(OLD));
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_session_is_still_looked_at_first() {
    let db = TestDb::start().await;
    let service = db.service();
    signed_in(&service, "somchai").await;

    for token in [String::new(), "f".repeat(64)] {
        assert_eq!(
            refused(
                &service,
                &token,
                json!({ "currentPassword": "wrong-password-99", "password": "short" })
            )
            .await,
            AUTH_REQUIRED
        );
    }
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_password_change_asks_for_nothing_more() {
    let db = TestDb::start().await;
    let service = db.service();
    for (username, payload) in [
        ("first.a", json!({ "password": NEW })),
        // a current password that is wrong is not read on this path
        (
            "first.b",
            json!({ "currentPassword": "wrong-password-99", "password": NEW }),
        ),
        // nor is one equal to the new password
        (
            "first.c",
            json!({ "currentPassword": NEW, "password": NEW }),
        ),
    ] {
        create_employee(
            &service,
            &new_employee(username, "FRONT", "MAIN", "the-first-password"),
            at(0),
        )
        .await;
        let login = service
            .login_employee(
                &json!({ "username": username, "password": "the-first-password" }),
                &unique("first"),
                at(1),
            )
            .await
            .expect("first sign-in");
        service
            .change_password(&login.token, &payload, at(2))
            .await
            .unwrap_or_else(|e| panic!("{username}: {e:?}"));
        assert_eq!(
            password_now(&service, username).await,
            Some(NEW),
            "{username}"
        );
    }
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn voluntary_attempts_count_on_the_sign_in_name_counter() {
    let db = TestDb::start().await;
    let service = db.service();
    // attempts 1 and 2 of the window
    let token = signed_in(&service, "somchai").await;

    // attempts 3 to 10: refused for the password, not for the limit
    for n in 3..=10 {
        assert_eq!(
            refused(
                &service,
                &token,
                json!({ "currentPassword": format!("wrong-password-{n:02}"), "password": NEW })
            )
            .await,
            CURRENT_WRONG,
            "attempt {n}"
        );
    }
    // attempt 11: the limit, even with the right password
    assert_eq!(
        refused(
            &service,
            &token,
            json!({ "currentPassword": OLD, "password": NEW })
        )
        .await,
        LIMIT
    );
    assert!(still_signed_in(&service, &token).await, "the session ended");

    // the counter is the one sign-in uses: the staff door is shut for the name too
    let error = service
        .login_employee(
            &json!({ "username": "somchai", "password": OLD }),
            &unique("elsewhere"),
            at(4),
        )
        .await
        .expect_err("refused");
    assert_eq!(refusal(&error), LIMIT);

    // another employee's counter is untouched
    let other = signed_in(&service, "somsri").await;
    service
        .change_password(
            &other,
            &json!({ "currentPassword": OLD, "password": NEW }),
            at(5),
        )
        .await
        .expect("somsri's change is not limited by somchai's attempts");

    // the next window starts again
    service
        .change_password(
            &token,
            &json!({ "currentPassword": OLD, "password": NEW }),
            at(15 * 60),
        )
        .await
        .expect("changed in the next window");
    assert_eq!(password_now(&service, "somchai").await, Some(NEW));
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_password_change_is_not_counted() {
    let db = TestDb::start().await;
    let service = db.service();
    create_employee(
        &service,
        &new_employee("somchai", "FRONT", "MAIN", "the-first-password"),
        at(0),
    )
    .await;
    // attempt 1 of the window
    let login = service
        .login_employee(
            &json!({ "username": "somchai", "password": "the-first-password" }),
            &unique("first"),
            at(1),
        )
        .await
        .expect("first sign-in");
    // twelve refused first-password changes count nothing
    for _ in 0..12 {
        assert_eq!(
            refused(&service, &login.token, json!({ "password": "short" })).await,
            PASSWORD_LENGTH
        );
    }
    service
        .change_password(&login.token, &json!({ "password": OLD }), at(2))
        .await
        .expect("the first password is set");
    // attempt 2: still far from the limit
    service
        .login_employee(
            &json!({ "username": "somchai", "password": OLD }),
            &unique("again"),
            at(3),
        )
        .await
        .expect("the name counter was not used by the first-password path");
    db.drop_schema().await;
}
