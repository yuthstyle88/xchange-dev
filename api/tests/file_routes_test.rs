//! Ticket T6b-2 — the four file routes: `POST /api/upload`, `GET /files/{key}`,
//! `POST /api/staff/upload`, `GET /api/staff/file/{key}`.
//! Provided with the ticket — do not edit, delete or weaken; add tests in a file of your own.
//!
//! Every test drives the server's own route table (`bootstrap::configure`). They need a real
//! PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in `_test`); without it they
//! fail, none skips.

mod common;
#[macro_use]
mod file_routes_common;

use std::sync::Arc;

use actix_web::http::Method;
use actix_web::{App, test};
use file_routes_common::*;
use serde_json::json;
use xchange::bootstrap::{self, AppState};

const UPLOAD_PATHS: [&str; 2] = ["/api/upload", "/api/staff/upload"];

// ── v2's recorded behaviour ──────────────────────────────────────────────

#[actix_web::test]
async fn the_fixture_is_the_one_recorded_from_v2() {
    let fixture = fixture();
    assert_eq!(
        fixture["counts"],
        json!({ "requests": 118, "seeds": 5, "stored": 16,
                "byStatus": { "200": 42, "303": 2, "400": 47, "401": 7, "404": 20 } })
    );
    assert_eq!(
        fixture["steps"].as_array().unwrap().len(),
        123,
        "no step was removed from the fixture"
    );
}

#[actix_web::test]
async fn the_four_routes_answer_as_v2_answered() {
    let fixture = fixture();
    let desk = Desk::start().await;
    let app = desk_app!(desk, &fixture);
    replay(&app, &desk, &fixture).await;
    drop(app);
    desk.finish().await;
}

// ── around the recorded cases ────────────────────────────────────────────

#[actix_web::test]
async fn a_wrong_method_is_the_pages_404_not_a_405() {
    let fixture = fixture();
    let desk = Desk::start().await;
    let app = desk_app!(desk, &fixture);
    let key = send(
        &app,
        Call::upload("/api/upload", &["owner"], "image/jpeg", JPEG),
    )
    .await
    .upload_key();
    let before = desk.rows().await;
    let cases = [
        (
            "/api/upload".to_owned(),
            vec![Method::GET, Method::PUT, Method::DELETE, Method::PATCH],
        ),
        (
            "/api/staff/upload".to_owned(),
            vec![Method::GET, Method::PUT, Method::DELETE, Method::PATCH],
        ),
        (
            format!("/files/{key}"),
            vec![Method::POST, Method::PUT, Method::DELETE, Method::PATCH],
        ),
        (
            format!("/api/staff/file/{key}"),
            vec![Method::POST, Method::PUT, Method::DELETE, Method::PATCH],
        ),
    ];
    for (path, methods) in cases {
        for method in methods {
            let mut call = Call::upload(&path, &["owner", "front"], "image/jpeg", JPEG);
            call.method = Some(method.clone());
            let answer = send(&app, call).await;
            assert_eq!(
                (answer.status, answer.text().as_str()),
                (404, "Not found"),
                "{method} {path}"
            );
        }
    }
    assert_eq!(
        desk.rows().await,
        before,
        "none of them stored or removed anything"
    );
    drop(app);
    desk.finish().await;
}

#[actix_web::test]
async fn without_a_session_reader_nobody_is_signed_in() {
    // The route table alone, no reader registered beside it. (Once the real reader of ticket
    // T4c is in the table, a request without cookies is nobody just the same.)
    let desk = Desk::start().await;
    let state = desk.state();
    let app = TestApp::new(
        test::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state))).await,
        |t| t.to_request(),
    );
    let answer = send(&app, Call::upload("/api/upload", &[], "image/jpeg", JPEG)).await;
    assert_eq!(
        (answer.status, answer.error().as_str()),
        (401, "กรุณาเข้าสู่ระบบ")
    );
    let answer = send(
        &app,
        Call::upload("/api/staff/upload", &[], "image/jpeg", JPEG),
    )
    .await;
    assert_eq!(
        (answer.status, answer.error().as_str()),
        (401, "EMPLOYEE_AUTH_REQUIRED")
    );
    let key = "0".repeat(36);
    let answer = send(&app, Call::get(&format!("/files/{key}"), &[])).await;
    assert_eq!(answer.status, 303);
    assert_eq!(
        answer.headers.get("location").map(String::as_str),
        Some("https://desk.example/login")
    );
    let answer = send(&app, Call::get(&format!("/api/staff/file/{key}"), &[])).await;
    assert_eq!(
        (answer.status, answer.error().as_str()),
        (401, "EMPLOYEE_AUTH_REQUIRED")
    );
    assert!(desk.rows().await.is_empty());
    assert!(desk.names_on_disk().is_empty());
    drop(app);
    desk.finish().await;
}

#[actix_web::test]
async fn the_sign_in_redirect_goes_to_the_requests_own_origin() {
    // v2: `Response.redirect(url.origin + "/login", 303)` — the scheme from X-Forwarded-Proto,
    // the host from Host. `send` always says https://desk.example; the recorded case covers it.
    // Here: who is signed in decides, not the key.
    let fixture = fixture();
    let desk = Desk::start().await;
    let app = desk_app!(desk, &fixture);
    for key in ["0".repeat(36), "NOT-A-KEY".to_owned(), "a".repeat(200)] {
        let answer = send(&app, Call::get(&format!("/files/{key}"), &[])).await;
        assert_eq!(answer.status, 303, "nobody, key {key}");
        assert_eq!(
            answer.headers.get("location").map(String::as_str),
            Some("https://desk.example/login")
        );
        let answer = send(&app, Call::get(&format!("/files/{key}"), &["owner"])).await;
        assert_eq!(
            (answer.status, answer.text().as_str()),
            (404, "Not found"),
            "the owner, key {key}"
        );
    }
    // without the HTTPS proxy's header the server's own origin is http
    let mut call = Call::get(&format!("/files/{}", "0".repeat(36)), &[]);
    call.plain_http = true;
    let answer = send(&app, call).await;
    assert_eq!(answer.status, 303);
    assert_eq!(
        answer.headers.get("location").map(String::as_str),
        Some("http://desk.example/login")
    );
    for (path, who, refused) in [
        ("/api/upload", "owner", "ORIGIN_REJECTED"),
        ("/api/upload", "front", "คำขอไม่ถูกต้อง"),
        ("/api/staff/upload", "front", "คำขอไม่ถูกต้อง"),
    ] {
        let mut call = Call::upload(path, &[who], "image/jpeg", JPEG);
        call.plain_http = true;
        let answer = send(&app, call.clone()).await;
        assert_eq!(
            (answer.status, answer.error().as_str()),
            (400, refused),
            "{path} as {who}: an https Origin on plain http"
        );
        call.origin = Some("http://desk.example".to_owned());
        let answer = send(&app, call).await;
        assert_eq!(answer.status, 200, "{path} as {who}: {:?}", answer.text());
    }
    drop(app);
    desk.finish().await;
}

#[actix_web::test]
async fn a_key_never_leads_out_of_the_directory() {
    let fixture = fixture();
    let desk = Desk::start().await;
    let app = desk_app!(desk, &fixture);
    // something worth finding one level up from the upload directory
    std::fs::create_dir_all(&desk.dir).unwrap();
    let outside = desk.dir.with_extension("secret");
    std::fs::write(&outside, b"not yours").unwrap();
    let name = outside.file_name().unwrap().to_str().unwrap().to_owned();
    // a row that names it, as nothing in the server would ever store
    sqlx::query("INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) VALUES ($1, 'MAIN', 'owner', 'image/png', 'x', now())")
        .bind(format!("../{name}"))
        .execute(&desk.pool)
        .await
        .unwrap();
    for key in [
        format!("..%2F{name}"),
        format!("%2E%2E%2F{name}"),
        "..".to_owned(),
        "%2e%2e".to_owned(),
        ".".to_owned(),
    ] {
        let answer = send(&app, Call::get(&format!("/files/{key}"), &["owner"])).await;
        assert_eq!(
            answer.status,
            404,
            "owner, /files/{key}: {:?}",
            answer.text()
        );
        let answer = send(&app, Call::get(&format!("/files/{key}"), &["front"])).await;
        assert_eq!(
            answer.status,
            404,
            "Front, /files/{key}: {:?}",
            answer.text()
        );
        let answer = send(
            &app,
            Call::get(&format!("/api/staff/file/{key}"), &["front"]),
        )
        .await;
        assert!(
            answer.status == 400 || answer.status == 404,
            "Front, /api/staff/file/{key}: {} {:?}",
            answer.status,
            answer.text()
        );
        assert!(!answer.text().contains("not yours"));
    }
    let _ = std::fs::remove_file(outside);
    drop(app);
    desk.finish().await;
}

#[actix_web::test]
async fn a_row_whose_file_is_gone_is_no_file() {
    let fixture = fixture();
    let desk = Desk::start().await;
    let app = desk_app!(desk, &fixture);
    let key = send(
        &app,
        Call::upload("/api/upload", &["owner"], "image/jpeg", JPEG),
    )
    .await
    .upload_key();
    assert_eq!(
        send(&app, Call::get(&format!("/files/{key}"), &["owner"]))
            .await
            .status,
        200
    );
    std::fs::remove_file(desk.dir.join(&key)).unwrap();

    let answer = send(&app, Call::get(&format!("/files/{key}"), &["owner"])).await;
    assert_eq!(
        (answer.status, answer.text().as_str()),
        (404, "Not found"),
        "the owner's way"
    );
    let answer = send(&app, Call::get(&format!("/files/{key}"), &["front"])).await;
    assert_eq!(
        (answer.status, answer.text().as_str()),
        (404, "Not found"),
        "an employee at /files"
    );
    let answer = send(
        &app,
        Call::get(&format!("/api/staff/file/{key}"), &["front"]),
    )
    .await;
    assert_eq!(
        (answer.status, answer.error().as_str()),
        (400, "ไม่พบไฟล์"),
        "the staff door"
    );
    drop(app);
    desk.finish().await;
}

#[actix_web::test]
async fn an_employee_reads_by_the_sessions_branch_whatever_the_request_says() {
    let fixture = fixture();
    let desk = Desk::start().await;
    let app = desk_app!(desk, &fixture);
    let in_br2 = send(
        &app,
        Call::upload("/api/upload?branch=BR-2", &["owner"], "image/jpeg", JPEG),
    )
    .await
    .upload_key();
    for path in [
        format!("/files/{in_br2}?branch=BR-2"),
        format!("/files/{in_br2}?branchId=BR-2"),
        format!("/api/staff/file/{in_br2}?branch=BR-2"),
    ] {
        let answer = send(&app, Call::get(&path, &["front"])).await;
        assert!(
            answer.status == 404 || answer.status == 400,
            "Front of MAIN, {path}: {}",
            answer.status
        );
        let answer = send(&app, Call::get(&path, &["front2"])).await;
        assert_eq!(answer.status, 200, "Front of BR-2, {path}");
        assert_eq!(answer.bytes, JPEG);
    }
    drop(app);
    desk.finish().await;
}

#[actix_web::test]
async fn uploads_at_once_are_all_kept() {
    let fixture = fixture();
    let desk = Desk::start().await;
    let app = desk_app!(desk, &fixture);
    let body = |n: u8| {
        let mut bytes = JPEG.to_vec();
        bytes.push(n);
        bytes
    };
    let upload = |n: u8| {
        let (path, who) = if n.is_multiple_of(2) {
            ("/api/upload", "owner")
        } else {
            ("/api/staff/upload", "front")
        };
        send(&app, Call::upload(path, &[who], "image/jpeg", &body(n)))
    };
    let answers = tokio::join!(
        upload(0),
        upload(1),
        upload(2),
        upload(3),
        upload(4),
        upload(5),
        upload(6),
        upload(7)
    );
    let answers = [
        answers.0, answers.1, answers.2, answers.3, answers.4, answers.5, answers.6, answers.7,
    ];
    let keys: std::collections::BTreeSet<String> = answers.iter().map(Answer::upload_key).collect();
    assert_eq!(keys.len(), 8, "eight uploads, eight keys");
    assert_eq!(desk.rows().await.len(), 8);
    assert_eq!(
        desk.names_on_disk(),
        keys,
        "one file per key and nothing else in the directory"
    );
    for (n, answer) in answers.iter().enumerate() {
        let key = answer.upload_key();
        assert_eq!(
            std::fs::read(desk.dir.join(&key)).unwrap(),
            body(n as u8),
            "upload {n}"
        );
    }
    drop(app);
    desk.finish().await;
}

#[actix_web::test]
async fn when_the_database_is_down_the_routes_answer_500() {
    let fixture = fixture();
    let dir = std::env::temp_dir().join(unique("xchange-t6b2-down"));
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            dir.to_str().unwrap(),
        )])),
        common::unreachable_pool(),
    );
    let reader: Arc<dyn xchange::presentation::pages::SessionReader> =
        Arc::new(FakeSessions::from_fixture(&fixture));
    let app = TestApp::new(
        test::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(actix_web::web::Data::from(reader)),
        )
        .await,
        |t| t.to_request(),
    );
    let key = "0".repeat(36);
    let calls = [
        Call::upload("/api/upload", &["owner"], "image/jpeg", JPEG),
        Call::upload("/api/upload", &["front"], "image/jpeg", JPEG),
        Call::upload("/api/staff/upload", &["front"], "image/jpeg", JPEG),
        Call::get(&format!("/files/{key}"), &["owner"]),
        Call::get(&format!("/files/{key}"), &["front"]),
        Call::get(&format!("/api/staff/file/{key}"), &["front"]),
    ];
    for call in calls {
        let what = format!("{:?} {} as {:?}", call.method, call.path, call.signed_in);
        let answer = send(&app, call).await;
        // v2: `server.mjs:8297`
        assert_eq!(answer.status, 500, "{what}: {:?}", answer.text());
        assert_eq!(
            answer.json(),
            json!({ "error": SERVER_ERROR }),
            "{what}: nothing of the cause is shown"
        );
    }
    assert!(!dir.exists(), "a failed upload leaves no directory behind");
    for path in UPLOAD_PATHS {
        // what needs no database is still judged
        let answer = send(&app, Call::upload(path, &["manager"], "image/jpeg", JPEG)).await;
        assert_eq!(
            (answer.status, answer.error().as_str()),
            (400, "บทบาทนี้อัปโหลดหลักฐานไม่ได้"),
            "{path}"
        );
    }
}
