//! Ticket T4b — accounts, passwords, sessions and the sign-in attempt limit.
//! Provided with the ticket — do not edit, delete or weaken; add tests in a file of your own.
//!
//! These tests need a real PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in
//! `_test`). Without it they fail; none of them skips.

mod account_common;

use account_common::*;
use pos_error::AppError;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::task::JoinSet;
use xc_auth::application::account::service::EmployeesAnswer;
use xc_auth::domain::account::entities::session::SignedIn;
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;

const OWNER_PASSWORD: &str = "owner-password-01";

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ── v2's recorded behaviour ──────────────────────────────────────────────

#[test]
fn the_fixture_is_the_one_recorded_from_v2() {
    let fixture = fixture();
    assert_eq!(
        fixture["counts"],
        json!({ "stories": 3, "steps": 285, "accepted": 71, "refused": 211 })
    );
    let steps: usize = fixture["stories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["steps"].as_array().unwrap().len())
        .sum();
    assert_eq!(steps, 285, "no step was removed from the fixture");
}

#[tokio::test(flavor = "multi_thread")]
async fn v2_story_accounts() {
    Replay::run(&story(&fixture(), "accounts")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn v2_story_owner() {
    Replay::run(&story(&fixture(), "owner")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn v2_story_limits() {
    Replay::run(&story(&fixture(), "limits")).await;
}

// ── PasswordHasher ───────────────────────────────────────────────────────

#[test]
fn the_hasher_writes_argon2id_phc_strings_with_a_salt_of_their_own() {
    let hasher = PasswordHasher::with_cost(8, 1, 1);
    let one = hasher.hash("correct horse battery").expect("hash");
    let two = hasher.hash("correct horse battery").expect("hash");
    for stored in [&one, &two] {
        assert!(
            stored.starts_with("$argon2id$v=19$m=8,t=1,p=1$"),
            "Argon2id, version 0x13, with the cost that was asked for: {stored}"
        );
        assert!(
            !stored.contains("correct horse battery"),
            "the password is not in the stored string"
        );
    }
    assert_ne!(one, two, "the same password hashed twice has two salts");
}

#[test]
fn the_hasher_for_real_accounts_is_not_cheaper_than_the_floor() {
    // The cost itself is the implementer's proposal; the floor (19 MiB, 2 passes) is the least
    // that is accepted for Argon2id, so that the cheap test cost can never be what ships.
    let stored = PasswordHasher::new().hash("correct horse battery").unwrap();
    let params = stored
        .strip_prefix("$argon2id$v=19$")
        .and_then(|rest| rest.split('$').next())
        .unwrap_or_else(|| panic!("an Argon2id PHC string: {stored}"));
    let number = |key: &str| -> u32 {
        params
            .split(',')
            .find_map(|part| part.strip_prefix(key))
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("{key} in {params}"))
    };
    assert!(
        number("m=") >= 19_456,
        "memory of at least 19 MiB: {params}"
    );
    assert!(number("t=") >= 2, "at least two passes: {params}");
    assert!(number("p=") >= 1);
    assert!(PasswordHasher::new().verify("correct horse battery", &stored));
}

#[test]
fn the_hasher_verifies_the_right_password_only() {
    let hasher = PasswordHasher::with_cost(8, 1, 1);
    let stored = hasher.hash("correct horse battery").unwrap();
    assert!(hasher.verify("correct horse battery", &stored));
    assert!(!hasher.verify("correct horse batterY", &stored));
    assert!(!hasher.verify("correct horse battery ", &stored));
    assert!(!hasher.verify("", &stored));
    // The cost is read from the stored string, not from the hasher that verifies.
    assert!(PasswordHasher::with_cost(16, 2, 1).verify("correct horse battery", &stored));
    assert!(PasswordHasher::new().verify("correct horse battery", &stored));
}

#[test]
fn the_hasher_answers_false_for_a_stored_value_it_cannot_read() {
    let hasher = PasswordHasher::with_cost(8, 1, 1);
    for stored in [
        "",
        "not a hash",
        "$argon2id$",
        "$argon2id$v=19$m=8,t=1,p=1$",
        // v2's scrypt format (`salt_hex:key_hex`) is never loaded and never matches.
        "00000000000000000000000000000000:0000000000000000000000000000000000000000000000000000000000000000",
    ] {
        assert!(
            !hasher.verify("correct horse battery", stored),
            "stored = {stored:?}"
        );
    }
}

#[test]
fn the_hasher_takes_12_to_128_utf16_units() {
    let hasher = PasswordHasher::with_cost(8, 1, 1);
    let refused = |password: &str| match hasher.hash(password) {
        Ok(stored) => panic!(
            "a password of {} units was hashed: {stored}",
            password.encode_utf16().count()
        ),
        Err(e) => assert_eq!(refusal(&e), PASSWORD_LENGTH),
    };
    refused("");
    refused(&"a".repeat(11));
    refused(&"a".repeat(129));
    refused(&"ก".repeat(11));
    refused(&"😀".repeat(5)); // 10 units
    refused(&"😀".repeat(65)); // 130 units
    for accepted in [
        "a".repeat(12),
        "a".repeat(128),
        "ก".repeat(12),
        "😀".repeat(6),
        "😀".repeat(64),
    ] {
        let stored = hasher
            .hash(&accepted)
            .expect("12 to 128 units are accepted");
        assert!(hasher.verify(&accepted, &stored));
    }
    // v2 never checks a password longer than 128 (`server.mjs:7299`): even a stored hash that
    // really is of such a password (made here with the argon2 crate itself) does not verify.
    let long = "a".repeat(129);
    let params = argon2::Params::new(8, 1, 1, None).unwrap();
    let salt = argon2::password_hash::SaltString::encode_b64(b"0123456789abcdef").unwrap();
    let stored = argon2::password_hash::PasswordHasher::hash_password(
        &argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params),
        long.as_bytes(),
        &salt,
    )
    .unwrap()
    .to_string();
    assert!(stored.starts_with("$argon2id$v=19$m=8,t=1,p=1$"));
    assert!(
        !hasher.verify(&long, &stored),
        "129 units are never checked"
    );
    assert!(hasher.verify(&long[..128], &hasher.hash(&long[..128]).unwrap()));
}

// ── what is stored ───────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn passwords_are_stored_as_argon2id_and_never_in_clear() {
    let db = TestDb::start().await;
    let service = db.service();
    service
        .create_owner("boss", OWNER_PASSWORD, at(0))
        .await
        .unwrap();
    let employee = create_employee(
        &service,
        &new_employee("somchai", "FRONT", "MAIN", "first-password-1"),
        at(0),
    )
    .await;

    let stored = |table: &'static str| {
        let pool = db.pool.clone();
        async move {
            sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(format!(
                "SELECT password_hash FROM {table}"
            )))
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let owner_hash = stored("owners").await;
    let created = stored("employees").await;
    for (hash, password) in [
        (&owner_hash, OWNER_PASSWORD),
        (&created, "first-password-1"),
    ] {
        assert!(hash.starts_with("$argon2id$v=19$"), "stored: {hash}");
        assert!(!hash.contains(password));
        assert!(cheap_hasher().verify(password, hash));
    }

    let login = service
        .login(
            &json!({ "username": "somchai", "password": "first-password-1" }),
            "10.0.0.1",
            at(1),
        )
        .await
        .unwrap();
    service
        .change_password(
            &login.token,
            &json!({ "password": "second-password-2" }),
            at(2),
        )
        .await
        .unwrap();
    let changed = stored("employees").await;
    assert_ne!(changed, created);
    assert!(changed.starts_with("$argon2id$v=19$"));
    assert!(cheap_hasher().verify("second-password-2", &changed));
    assert!(!cheap_hasher().verify("first-password-1", &changed));

    service
        .employees(
            "reset",
            &json!({ "id": employee.id, "password": "third-password-33" }),
            "owner",
            at(3),
        )
        .await
        .unwrap();
    let reset = stored("employees").await;
    assert_ne!(reset, changed);
    assert!(cheap_hasher().verify("third-password-33", &reset));
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_sha256_of_a_session_token_is_stored() {
    let db = TestDb::start().await;
    let service = db.service();
    service
        .create_owner("boss", OWNER_PASSWORD, at(0))
        .await
        .unwrap();
    working_employee(&service, "somchai", "second-password-2", at(0)).await;
    let sessions_before = db.count("employee_sessions").await;

    let staff = service
        .login(
            &json!({ "username": "somchai", "password": "second-password-2" }),
            "10.0.0.1",
            at(1),
        )
        .await
        .unwrap();
    let owner = service
        .login(
            &json!({ "username": "boss", "password": OWNER_PASSWORD }),
            "10.0.0.1",
            at(1),
        )
        .await
        .unwrap();
    assert!(matches!(staff.who, SignedIn::Employee(_)));
    assert!(matches!(owner.who, SignedIn::Owner(_)));
    assert_ne!(staff.token, owner.token);

    for (table, token) in [
        ("employee_sessions", &staff.token),
        ("owner_sessions", &owner.token),
    ] {
        let with = |value: String| {
            let pool = db.pool.clone();
            async move {
                sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
                    "SELECT count(*) FROM {table} WHERE hash = $1"
                )))
                .bind(value)
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        assert_eq!(
            with(token.clone()).await,
            0,
            "{table}: the token itself is not stored"
        );
        assert_eq!(
            with(sha256_hex(token)).await,
            1,
            "{table}: its SHA-256, in hex, is"
        );
    }
    assert_eq!(db.count("employee_sessions").await, sessions_before + 1);
    assert_eq!(db.count("owner_sessions").await, 1);

    // Sign-out removes the row; a refused sign-in makes none.
    service.logout(&owner.token, &staff.token).await.unwrap();
    assert_eq!(db.count("employee_sessions").await, sessions_before);
    assert_eq!(db.count("owner_sessions").await, 0);
    let audit_before = db.count("employee_audit").await;
    for payload in [
        json!({ "username": "somchai", "password": "wrong-password-1" }),
        json!({ "username": "boss", "password": "wrong-password-1" }),
        json!({ "username": "ghost", "password": "second-password-2" }),
    ] {
        let error = service
            .login(&payload, "10.0.0.1", at(2))
            .await
            .expect_err("refused");
        assert_eq!(refusal(&error), WRONG);
    }
    assert_eq!(db.count("employee_sessions").await, sessions_before);
    assert_eq!(db.count("owner_sessions").await, 0);
    assert_eq!(db.count("employee_audit").await, audit_before);
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_employee_made_inactive_in_the_table_alone_is_locked_out() {
    // `disable` also moves the version, which by itself ends the sessions. The account's
    // `active` flag must lock it out on its own account too.
    let db = TestDb::start().await;
    let service = db.service();
    let employee = working_employee(&service, "somchai", "second-password-2", at(0)).await;
    let login = service
        .login(
            &json!({ "username": "somchai", "password": "second-password-2" }),
            "10.0.0.1",
            at(1),
        )
        .await
        .unwrap();
    assert!(
        service
            .employee_session(&login.token, false, at(2))
            .await
            .is_ok()
    );

    let changed = sqlx::query("UPDATE employees SET active = false WHERE id = $1")
        .bind(&employee.id)
        .execute(&db.pool)
        .await
        .unwrap();
    assert_eq!(changed.rows_affected(), 1);

    let error = service
        .employee_session(&login.token, false, at(3))
        .await
        .expect_err("refused");
    assert_eq!(refusal(&error), AUTH_REQUIRED);
    let error = service
        .employee_session(&login.token, true, at(3))
        .await
        .expect_err("refused");
    assert_eq!(refusal(&error), AUTH_REQUIRED);
    assert!(
        service
            .status("", &login.token, at(3))
            .await
            .unwrap()
            .is_none()
    );
    let error = service
        .login(
            &json!({ "username": "somchai", "password": "second-password-2" }),
            "10.0.0.1",
            at(3),
        )
        .await
        .expect_err("refused");
    assert_eq!(refusal(&error), WRONG);
    let error = service
        .change_password(
            &login.token,
            &json!({ "password": "third-password-33" }),
            at(3),
        )
        .await
        .expect_err("refused");
    assert_eq!(refusal(&error), AUTH_REQUIRED);
    db.drop_schema().await;
}

// ── the first owner ──────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn create_owner_makes_the_one_owner_who_signs_in_without_a_first_password_change() {
    let db = TestDb::start().await;
    let service = db.service();
    assert!(service.status("", "", at(0)).await.unwrap().is_none());

    let owner = service
        .create_owner("  Boss.01 ", OWNER_PASSWORD, at(0))
        .await
        .expect("created");
    assert_eq!(owner.username, "boss.01", "trimmed and lower-cased");
    assert_eq!(
        owner.name, "boss.01",
        "the name is the username until something sets another"
    );
    assert!(owner.active);
    assert_eq!(owner.version, 1);
    assert_eq!(owner.created_at, at(0));
    assert_eq!(db.count("owners").await, 1);

    let login = service
        .login(
            &json!({ "username": "BOSS.01", "password": OWNER_PASSWORD }),
            "10.0.0.1",
            at(1),
        )
        .await
        .expect("the owner signs in with the password given to create-owner");
    assert_eq!(owner_of(&login.who), &owner);
    let seen = service
        .status(&login.token, "", at(2))
        .await
        .unwrap()
        .expect("signed in");
    assert_eq!(owner_of(&seen), &owner);
    assert_eq!(
        db.count("employee_audit").await,
        0,
        "an owner's sign-in writes no audit line, as v2"
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn create_owner_is_refused_once_an_owner_exists() {
    let db = TestDb::start().await;
    let service = db.service();
    let first = service
        .create_owner("boss", OWNER_PASSWORD, at(0))
        .await
        .unwrap();
    for (username, password) in [
        ("boss", "another-password-9"),
        ("second", "another-password-9"),
        ("boss", OWNER_PASSWORD),
    ] {
        let error = service
            .create_owner(username, password, at(1))
            .await
            .expect_err("refused");
        assert!(
            matches!(error, AppError::Domain(_)),
            "a refusal, not a database error: {error:?}"
        );
    }
    assert_eq!(db.count("owners").await, 1);
    // An inactive owner still counts: the command never makes a second row.
    sqlx::query("UPDATE owners SET active = false")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        service
            .create_owner("third", "another-password-9", at(2))
            .await
            .is_err()
    );
    assert_eq!(db.count("owners").await, 1);
    sqlx::query("UPDATE owners SET active = true")
        .execute(&db.pool)
        .await
        .unwrap();

    // The first owner's password is untouched.
    let login = service
        .login(
            &json!({ "username": "boss", "password": OWNER_PASSWORD }),
            "10.0.0.1",
            at(3),
        )
        .await
        .unwrap();
    assert_eq!(owner_of(&login.who), &first);
    let error = service
        .login(
            &json!({ "username": "boss", "password": "another-password-9" }),
            "10.0.0.1",
            at(3),
        )
        .await
        .expect_err("refused");
    assert_eq!(refusal(&error), WRONG);
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn create_owner_refuses_usernames_and_passwords_v2_refuses() {
    let db = TestDb::start().await;
    let service = db.service();
    // v2's `create-owner.mjs:5521`: a-z 0-9 . _ - , 3 to 30 characters, after trim and lower-case.
    for username in [
        "",
        "  ",
        "ab",
        &"a".repeat(31),
        "bo ss",
        "บอส",
        "boss!",
        "boss@shop",
    ] {
        let error = service
            .create_owner(username, OWNER_PASSWORD, at(0))
            .await
            .err();
        assert!(
            matches!(error, Some(AppError::Domain(_))),
            "username {username:?} is refused: {error:?}"
        );
    }
    for password in ["", &"a".repeat(11), &"a".repeat(129)] {
        let error = service
            .create_owner("boss", password, at(0))
            .await
            .expect_err("refused");
        assert_eq!(refusal(&error), PASSWORD_LENGTH);
    }
    assert_eq!(db.count("owners").await, 0);

    // v2's `create-owner.mjs:5523`: not a name an employee already has.
    create_employee(
        &service,
        &new_employee("somchai", "FRONT", "MAIN", "first-password-1"),
        at(0),
    )
    .await;
    let error = service
        .create_owner(" SomChai ", OWNER_PASSWORD, at(0))
        .await
        .err();
    assert!(matches!(error, Some(AppError::Domain(_))), "{error:?}");
    assert_eq!(db.count("owners").await, 0);

    for username in ["abc", &"a".repeat(30), "b.o_s-9"] {
        let db = TestDb::start().await;
        let made = db
            .service()
            .create_owner(username, OWNER_PASSWORD, at(0))
            .await
            .expect("accepted");
        assert_eq!(made.username, username);
        db.drop_schema().await;
    }
    db.drop_schema().await;
}

// ── a read for the staff bridge and the snapshot (tickets T11a, T11b) ────

#[tokio::test(flavor = "multi_thread")]
async fn active_employees_are_the_active_people_of_one_branch() {
    let db = TestDb::start().await;
    let service = db.service();
    let a = create_employee(
        &service,
        &new_employee("anan", "FRONT", "MAIN", "first-password-1"),
        at(0),
    )
    .await;
    let b = create_employee(
        &service,
        &new_employee("busaba", "MANAGER", "MAIN", "first-password-1"),
        at(0),
    )
    .await;
    let c = create_employee(
        &service,
        &new_employee("chai", "FINANCE", "MAIN", "first-password-1"),
        at(0),
    )
    .await;
    let other = create_employee(
        &service,
        &new_employee("dao", "FRONT", "BR-2", "first-password-1"),
        at(0),
    )
    .await;
    service
        .employees("disable", &json!({ "id": c.id }), "owner", at(1))
        .await
        .unwrap();

    let mut main = service.active_employees("MAIN").await.unwrap();
    main.sort_by(|x, y| x.username.cmp(&y.username));
    assert_eq!(
        main,
        vec![a, b],
        "a first password still to change does not hide an employee; a disabled account does"
    );
    assert_eq!(service.active_employees("BR-2").await.unwrap(), vec![other]);
    assert!(
        service
            .active_employees("NOWHERE")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        service.active_employees("main").await.unwrap().is_empty(),
        "branch ids are compared exactly"
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_is_ordered_by_username() {
    let db = TestDb::start().await;
    let service = db.service();
    for username in ["mike", "alpha", "zulu", "bravo"] {
        create_employee(
            &service,
            &new_employee(username, "FRONT", "MAIN", "first-password-1"),
            at(0),
        )
        .await;
    }
    let names: Vec<String> = list(&service, at(1))
        .await
        .into_iter()
        .map(|e| e.username)
        .collect();
    assert_eq!(names, ["alpha", "bravo", "mike", "zulu"]);
    assert!(matches!(
        service
            .employees("list", &json!({ "id": "ignored" }), "owner", at(1))
            .await
            .unwrap(),
        EmployeesAnswer::List(_)
    ));
    db.drop_schema().await;
}

// ── at the same time ─────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_attempts_from_one_address_let_exactly_thirty_through() {
    let db = TestDb::start().await;
    let service = db.service();
    let mut calls = JoinSet::new();
    for i in 0..45 {
        let service = service.clone();
        calls.spawn(async move {
            service
                .login(
                    &json!({ "username": format!("ghost{i}"), "password": "first-password-1" }),
                    "10.9.0.1",
                    at(5),
                )
                .await
        });
    }
    let results = within("45 sign-ins from one address", calls.join_all()).await;
    let counts = tally(results);
    assert_eq!(
        counts.get(WRONG),
        Some(&30),
        "thirty attempts reach the password check: {counts:?}"
    );
    assert_eq!(
        counts.get(LIMIT),
        Some(&15),
        "the rest are refused by the limit: {counts:?}"
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_sign_ins_for_one_username_let_exactly_ten_through() {
    let db = TestDb::start().await;
    let service = db.service();
    working_employee(&service, "somchai", "second-password-2", at(0)).await;
    let audit_before = db.count("employee_audit").await;
    let sessions_before = db.count("employee_sessions").await;

    // A new window, so that the set-up sign-in is not counted.
    let now = at(15 * 60);
    let mut calls = JoinSet::new();
    for i in 0..24 {
        let service = service.clone();
        calls.spawn(async move {
            service
                .login(
                    &json!({ "username": "somchai", "password": "second-password-2" }),
                    &format!("10.9.1.{i}"),
                    now,
                )
                .await
        });
    }
    let results = within("24 sign-ins for one username", calls.join_all()).await;
    let tokens: std::collections::HashSet<String> = results
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .map(|l| l.token.clone())
        .collect();
    let counts = tally(results);
    assert_eq!(counts.get("ok"), Some(&10), "{counts:?}");
    assert_eq!(counts.get(LIMIT), Some(&14), "{counts:?}");
    assert_eq!(tokens.len(), 10, "ten sessions, ten tokens");
    assert_eq!(db.count("employee_sessions").await, sessions_before + 10);
    assert_eq!(
        db.count("employee_audit").await,
        audit_before + 10,
        "one LOGIN line per session"
    );
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_creates_of_one_username_make_one_employee() {
    let db = TestDb::start().await;
    let service = db.service();
    let mut calls = JoinSet::new();
    for i in 0..12 {
        let service = service.clone();
        calls.spawn(async move {
            let mut payload = new_employee(
                if i % 2 == 0 { "somchai" } else { " SomChai " },
                "FRONT",
                "MAIN",
                "first-password-1",
            );
            payload["name"] = json!(format!("คนที่ {i}"));
            service.employees("create", &payload, "owner", at(0)).await
        });
    }
    let counts = tally(within("12 creates of one username", calls.join_all()).await);
    assert_eq!(counts.get("ok"), Some(&1), "{counts:?}");
    assert_eq!(counts.get(EMPLOYEE_DATA), Some(&11), "{counts:?}");
    assert_eq!(db.count("employees").await, 1);
    let audit = db.audit().await;
    assert_eq!(audit.len(), 1, "one CREATE line: {audit:?}");
    assert_eq!(audit[0].3, "CREATE");
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_create_owner_makes_one_owner() {
    let db = TestDb::start().await;
    let service = db.service();
    // Eight rounds on an emptied table: a check-then-insert that is not atomic may survive one
    // round by luck, not eight.
    for round in 0..8 {
        sqlx::query("DELETE FROM owners")
            .execute(&db.pool)
            .await
            .unwrap();
        // The passwords are hashed first; the barrier then lets every caller reach the table together.
        let start = std::sync::Arc::new(tokio::sync::Barrier::new(24));
        let mut calls = JoinSet::new();
        for i in 0..24 {
            let service = service.clone();
            let start = start.clone();
            calls.spawn(async move {
                start.wait().await;
                service
                    .create_owner(&format!("boss{i}"), OWNER_PASSWORD, at(round))
                    .await
            });
        }
        let results = within("24 create-owner at once", calls.join_all()).await;
        let made: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
        assert_eq!(made.len(), 1, "round {round}: exactly one succeeds");
        for refused in results.iter().filter_map(|r| r.as_ref().err()) {
            assert!(
                matches!(refused, AppError::Domain(_)),
                "round {round}: the others are refused, not broken: {refused:?}"
            );
        }
        assert_eq!(db.count("owners").await, 1, "round {round}");
        let stored: String = sqlx::query_scalar("SELECT username FROM owners")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(
            stored, made[0].username,
            "round {round}: the row is the one that was answered"
        );
    }
    db.drop_schema().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_password_changes_with_one_session_change_it_once() {
    let db = TestDb::start().await;
    // The real cost: hashing the new password takes long enough that every caller has read
    // the session before the first of them stores anything.
    let service = db.service_with(PasswordHasher::new());
    let before = working_employee(&service, "somchai", "second-password-2", at(0)).await;
    let login = service
        .login(
            &json!({ "username": "somchai", "password": "second-password-2" }),
            "10.9.2.1",
            at(1),
        )
        .await
        .unwrap();
    let audit_before = db.count("employee_audit").await;

    let start = std::sync::Arc::new(tokio::sync::Barrier::new(8));
    let mut calls = JoinSet::new();
    for i in 0..8 {
        let service = service.clone();
        let token = login.token.clone();
        let start = start.clone();
        calls.spawn(async move {
            let password = format!("candidate-password-{i}");
            start.wait().await;
            // A voluntary change names the current password (row SP-1).
            service
                .change_password(
                    &token,
                    &json!({ "currentPassword": "second-password-2", "password": password }),
                    at(2),
                )
                .await
                .map(|()| password)
        });
    }
    let results = within("8 password changes with one session", calls.join_all()).await;
    let winners: Vec<String> = results
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .cloned()
        .collect();
    let counts = tally(results);
    assert_eq!(
        counts.get("ok"),
        Some(&1),
        "the session is over once the first change lands: {counts:?}"
    );
    assert_eq!(counts.get(AUTH_REQUIRED), Some(&7), "{counts:?}");

    let after = list(&service, at(3))
        .await
        .into_iter()
        .find(|e| e.id == before.id)
        .unwrap();
    assert_eq!(after.version, before.version + 1, "one change, one version");
    assert_eq!(
        db.count("employee_audit").await,
        audit_before + 1,
        "one CHANGE_PASSWORD line"
    );
    // The stored password is the winner's, and only the winner's.
    for i in 0..8 {
        let password = format!("candidate-password-{i}");
        let outcome = service
            .login(
                &json!({ "username": "somchai", "password": password }),
                &format!("10.9.3.{i}"),
                at(15 * 60 + i),
            )
            .await;
        assert_eq!(
            outcome.is_ok(),
            winners.contains(&password),
            "candidate {i}"
        );
    }
    db.drop_schema().await;
}
