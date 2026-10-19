//! Ticket T4c — `/api/auth`, `/api/staff/{login,logout,password,me}`, `/api/employees`, and the
//! session reader the pages use.
//! Provided with the ticket — do not edit, delete or weaken; add tests in a file of your own.
//!
//! Every test drives the server's own route table (`bootstrap::configure`). They need a real
//! PostgreSQL (`TEST_DATABASE_URL`, a database whose name ends in `_test`); without it they
//! fail, none skips.
//!
//! Re-expected for CA-10 #626 by its ticket author (claude/c4d72c; cash-shift-transfer.md §6 Q7): the
//! `entity` and `enable` actions of `/api/employees` are retired with their custody port methods, so
//! they are answered exactly as an action the route does not know (`nope`) and custody is asked
//! nothing; the payload-handoff test of `entity` goes; `disable` still asks custody, unchanged.

mod auth_http_common;
mod common;

use std::sync::Arc;
use std::time::Duration;

use actix_web::http::Method;
use actix_web::{App, HttpRequest, HttpResponse, test, web};
use auth_http_common::*;
use serde_json::{Value, json};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::actor_kind::ActorKind;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::http;
use xchange::presentation::pages::{self, SessionReader};

const PATHS: [&str; 5] = [
    "/api/auth",
    "/api/staff/login",
    "/api/staff/logout",
    "/api/staff/password",
    "/api/staff/me",
];

fn login(username: &str, password: &str) -> Value {
    json!({ "action": "login", "payload": { "username": username, "password": password } })
}

// ── v2's recorded behaviour ──────────────────────────────────────────────

#[actix_web::test]
async fn the_fixture_is_the_one_recorded_from_v2() {
    let fixture = fixture();
    assert_eq!(
        fixture["counts"],
        json!({ "stories": 4, "requests": 200,
                "byStatus": { "200": 52, "400": 133, "401": 10, "403": 2, "404": 3 } })
    );
    let requests: usize = fixture["stories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            s["steps"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|step| step.get("request").is_some())
                .count()
        })
        .sum();
    assert_eq!(requests, 200, "no request was removed from the fixture");
}

#[actix_web::test]
async fn v2_story_auth() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    replay(&app, &desk, &story(&fixture(), "auth")).await;
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn v2_story_staff_door() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    replay(&app, &desk, &story(&fixture(), "staff door")).await;
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn v2_story_addresses() {
    stay_inside_one_attempt_window(Duration::from_secs(180)).await;
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    replay(
        &app,
        &desk,
        &behind_the_proxy(story(&fixture(), "addresses")),
    )
    .await;
    drop(app);
    desk.drop_schema().await;
}

/// Ticket #130: the replay plays the proxy for the staff door.
///
/// v2's staff door read `CF-Connecting-IP` first; v3 reads `X-Real-IP` at every door and never
/// `CF-Connecting-IP` (a known difference, `presentation/http/client_address.rs`). The proxy in
/// front of v3 is what turns the one into the other: it writes the address Cloudflare reports
/// into `X-Real-IP`. So for each recorded request to `/api/staff/login` that carries a
/// `CF-Connecting-IP`, the request is sent with `X-Real-IP` set to that value — the address v2
/// counted the caller under. The fixture file is not changed, every other header of the step is
/// sent as recorded, and every recorded answer is still compared in full.
///
/// Changed that way: steps 32–62 of the story `addresses` (31 requests), and no other step of
/// any story. Their recorded `X-Real-IP` (10.1.0.1 … 10.1.0.31, "differs every time and is
/// ignored") is what is no longer replayed. The `/api/auth` steps 0–30 carry both headers too
/// and are sent untouched: v2 read `X-Real-IP` there, as v3 does.
fn behind_the_proxy(mut story: Value) -> Value {
    let mut rewritten = 0;
    for step in story["steps"].as_array_mut().expect("steps") {
        let Some(request) = step.get_mut("request") else {
            continue;
        };
        if request["path"] != "/api/staff/login" {
            continue;
        }
        if let Some(reported) = request["headers"].get("cf-connecting-ip").cloned() {
            request["headers"]["x-real-ip"] = reported;
            rewritten += 1;
        }
    }
    assert_eq!(
        rewritten, 31,
        "the proxy's part is played for the 31 staff-door steps recorded with CF-Connecting-IP"
    );
    story
}

// ── reading the request ──────────────────────────────────────────────────

#[actix_web::test]
async fn a_body_that_is_not_json_is_a_bad_request() {
    // v2 answered the JavaScript parser's own message here. v3 answers its sentence for a
    // request that cannot be understood (ticket T4c, "Decided").
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = owner_signs_in(&app).await;
    for body in ["not json", "{", "[1,2", "{\"action\":\"status\"", "nul"] {
        for path in PATHS {
            let answer = send(&app, Call::raw(path, body).owner(&owner)).await;
            assert_eq!(answer.status, 400, "{path} {body:?}: {}", answer.text);
            assert_eq!(answer.error(), BAD_REQUEST, "{path} {body:?}");
            assert_eq!(
                answer.cache_control.as_deref(),
                Some("no-store"),
                "{path} {body:?}"
            );
            assert!(
                answer.cookies.is_empty(),
                "{path} {body:?}: no cookie is touched"
            );
        }
    }
    // The content type is never looked at: JSON sent as text/plain is read all the same.
    let answer = send(
        &app,
        Call::raw("/api/auth", r#"{"action":"status"}"#).owner(&owner),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    assert_eq!(answer.json()["user"]["role"], "OWNER");
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn body_limits_count_characters_not_bytes() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let padded =
        |json: &str, length: usize| format!("{json}{}", " ".repeat(length - json.chars().count()));

    // /api/auth: 4000 characters are read, 4001 are not
    let status = r#"{"action":"status","payload":{}}"#;
    let at_limit = send(&app, Call::raw("/api/auth", &padded(status, 4000))).await;
    assert_eq!(at_limit.status, 200, "4000 characters: {}", at_limit.text);
    let over = send(&app, Call::raw("/api/auth", &padded(status, 4001))).await;
    assert_eq!(
        (over.status, over.error().as_str()),
        (400, "ข้อมูลคำขอใหญ่เกินกำหนด")
    );
    // 3000 Thai letters are 9000 bytes and 3000 characters
    let thai = format!(
        r#"{{"action":"status","payload":{{"note":"{}"}}}}"#,
        "ก".repeat(3000)
    );
    let answer = send(&app, Call::raw("/api/auth", &thai)).await;
    assert_eq!(answer.status, 200, "3000 Thai letters: {}", answer.text);

    // the staff door: 120000 — 300 kB of Thai text is still inside it
    for path in [
        "/api/staff/login",
        "/api/staff/logout",
        "/api/staff/password",
        "/api/staff/me",
    ] {
        let over = send(&app, Call::raw(path, &" ".repeat(120_001))).await;
        assert_eq!(
            (over.status, over.error().as_str()),
            (400, "ข้อมูลมากเกินไป"),
            "{path}"
        );
    }
    let me = r#"{"payload":{}}"#;
    let at_limit = send(&app, Call::raw("/api/staff/me", &padded(me, 120_000))).await;
    assert_eq!(
        (at_limit.status, at_limit.error().as_str()),
        (401, AUTH_REQUIRED),
        "120000 characters are read"
    );
    let thai = format!(r#"{{"payload":{{"note":"{}"}}}}"#, "ก".repeat(100_000));
    let answer = send(&app, Call::raw("/api/staff/me", &thai)).await;
    assert_eq!(
        (answer.status, answer.error().as_str()),
        (401, AUTH_REQUIRED),
        "100000 Thai letters are read"
    );
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn the_origin_is_the_forwarded_scheme_and_the_host() {
    // v2 builds the request's own origin from X-Forwarded-Proto ("https" only when the header
    // says exactly that, else "http") and the Host header (`server.mjs:8235`).
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let status = || Call::post("/api/auth", json!({ "action": "status" }));
    let cases: [(Option<&str>, &str, u16); 8] = [
        (Some("https"), "https://desk.example", 200),
        (Some("https"), "http://desk.example", 400),
        (None, "http://desk.example", 200),
        (None, "https://desk.example", 400),
        (Some("http"), "http://desk.example", 200),
        (Some("HTTPS"), "http://desk.example", 200),
        (Some("https"), "https://desk.example:443", 400),
        (Some("https"), "https://other.example", 400),
    ];
    for (proto, origin, expected) in cases {
        let mut call = status();
        call.proto = proto.map(str::to_owned);
        call.origin = Some(origin.to_owned());
        let answer = send(&app, call).await;
        assert_eq!(
            answer.status, expected,
            "X-Forwarded-Proto {proto:?}, Origin {origin}: {}",
            answer.text
        );
        if expected == 400 {
            assert_eq!(answer.error(), BAD_REQUEST);
        }
    }
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn the_session_cookies_are_found_among_other_cookies() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = owner_signs_in(&app).await;
    let (id, staff) = working_employee(&app, &desk, &owner, "somchai", "FRONT", "MAIN").await;

    let status = |cookie: String| {
        let mut call = Call::post("/api/auth", json!({ "action": "status" }));
        call.cookie = Some(cookie);
        call
    };
    let answer = send(
        &app,
        status(format!("theme=dark;  __Host-xc_owner={owner} ;lang=th")),
    )
    .await;
    assert_eq!(
        answer.json()["user"]["role"],
        "OWNER",
        "spaces around the pair are trimmed: {}",
        answer.text
    );
    let answer = send(&app, status(format!("a=b; __Host-xc_staff={staff}; c=d"))).await;
    assert_eq!(answer.json()["user"]["id"], id.as_str(), "{}", answer.text);
    // the first cookie of that name is the one that counts (`server.mjs:7819`)
    let answer = send(
        &app,
        status(format!("__Host-xc_owner=junk; __Host-xc_owner={owner}")),
    )
    .await;
    assert_eq!(answer.json()["user"], Value::Null, "{}", answer.text);
    // a cookie whose name merely ends like ours is not ours
    let answer = send(&app, status(format!("x__Host-xc_owner={owner}"))).await;
    assert_eq!(answer.json()["user"], Value::Null, "{}", answer.text);
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn a_wrong_method_is_the_pages_404_not_a_405() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = owner_signs_in(&app).await;
    for path in PATHS {
        for method in [Method::GET, Method::PUT, Method::DELETE, Method::PATCH] {
            let answer = send(
                &app,
                Call::post(path, json!({}))
                    .owner(&owner)
                    .method(method.clone()),
            )
            .await;
            assert_eq!(answer.status, 404, "{method} {path}");
            assert_eq!(answer.text, "Not found", "{method} {path}");
        }
    }
    drop(app);
    desk.drop_schema().await;
}

#[actix_web::test]
async fn the_staff_routes_leave_their_neighbours_alone() {
    // `/api/staff/rpc` and `/api/staff/upload` belong to later tickets, registered after this
    // module in the route table. A catch-all `/api/staff/{op}` here would answer for them.
    let desk = Desk::start().await;
    let state = desk.state();
    let app = TestApp::new(
        test::init_service(
            App::new()
                .app_data(web::Data::new(state.clone()))
                .configure(|cfg| http::auth::configure(cfg, &state))
                .route(
                    "/api/staff/rpc",
                    web::post().to(|| async { HttpResponse::Ok().body("rpc") }),
                )
                .route(
                    "/api/staff/upload",
                    web::post().to(|| async { HttpResponse::Ok().body("upload") }),
                )
                .default_service(web::to(pages::fallback)),
        )
        .await,
        |t| t.to_request(),
    );
    for (path, body) in [("/api/staff/rpc", "rpc"), ("/api/staff/upload", "upload")] {
        let answer = send(&app, Call::post(path, json!({}))).await;
        assert_eq!((answer.status, answer.text.as_str()), (200, body), "{path}");
    }
    let answer = send(&app, Call::post("/api/staff/me", json!({}))).await;
    assert_eq!(
        (answer.status, answer.error().as_str()),
        (401, AUTH_REQUIRED),
        "the module's own routes are there"
    );
    drop(app);
    desk.drop_schema().await;
}

// ── the session reader of the pages (ticket T1c's port) ──────────────────

fn actor_json(actor: Option<&Actor>) -> Value {
    match actor {
        None => Value::Null,
        Some(a) => json!({
            "id": a.id(),
            "kind": match a.kind() { ActorKind::Owner => "owner", ActorKind::Staff => "staff" },
            "role": a.role().as_str(),
            "branch": a.branch_id().map(|b| b.as_str().to_owned()),
        }),
    }
}

/// A page-like handler: finds the reader the way `pages.rs` does and reports what it hands out.
async fn who(req: HttpRequest) -> HttpResponse {
    let Some(reader) = req.app_data::<web::Data<dyn SessionReader>>() else {
        return HttpResponse::NotImplemented().body("no SessionReader is registered");
    };
    let sessions = reader.sessions(&req).await;
    HttpResponse::Ok().json(json!({
        "owner": actor_json(sessions.owner.as_ref()),
        "staff": actor_json(sessions.staff.as_ref()),
    }))
}

async fn app_with_who(state: AppState) -> TestApp {
    TestApp::new(
        test::init_service(
            App::new()
                .route("/who", web::get().to(who))
                .configure(move |cfg| bootstrap::configure(cfg, state)),
        )
        .await,
        |t| t.to_request(),
    )
}

async fn sessions_of(app: &TestApp, owner: Option<&str>, staff: Option<&str>) -> Value {
    let mut call = Call::post("/who", json!({})).method(Method::GET);
    // a page request carries neither Origin nor the marker header
    call.origin = None;
    call.marker = None;
    if let Some(token) = owner {
        call = call.owner(token);
    }
    if let Some(token) = staff {
        call = call.staff(token);
    }
    let answer = send(app, call).await;
    assert_eq!(answer.status, 200, "{}", answer.text);
    answer.json()
}

#[actix_web::test]
async fn when_the_database_is_down_the_routes_answer_500_and_the_reader_nobody() {
    // No database at all: a pool that cannot connect (acquire timeout 1 s).
    let state = AppState::new(Arc::new(common::settings(&[])), common::unreachable_pool());
    let app = app_with_who(state).await;
    let token = "a".repeat(64);
    let calls = [
        Call::post("/api/auth", login("boss", OWNER_PASSWORD)),
        Call::post("/api/auth", json!({ "action": "status" })).owner(&token),
        Call::post("/api/auth", json!({ "action": "status" })).staff(&token),
        Call::post(
            "/api/staff/login",
            json!({ "payload": { "username": "somchai", "password": "first-password-1" } }),
        ),
        Call::post("/api/staff/me", json!({})).staff(&token),
        Call::post(
            "/api/staff/password",
            json!({ "payload": { "password": "second-password-2" } }),
        )
        .staff(&token),
    ];
    for call in calls {
        let what = format!("{} {}", call.path, call.body);
        let answer = send(&app, call).await;
        // v2: `server.mjs:8297`
        assert_eq!(answer.status, 500, "{what}: {}", answer.text);
        assert_eq!(answer.json(), json!({ "error": SERVER_ERROR }), "{what}");
        assert!(
            answer.content_type.starts_with("application/json"),
            "{what}: {}",
            answer.content_type
        );
        assert!(answer.cookies.is_empty(), "{what}: no cookie is touched");
        assert!(
            !answer.text.contains("pool") && !answer.text.contains("timed out"),
            "{what}: nothing of the cause is shown"
        );
    }
    // The port has no way to say "failed" (ticket T1c): the reader answers nobody.
    assert_eq!(
        sessions_of(&app, Some(&token), Some(&token)).await,
        json!({ "owner": null, "staff": null })
    );
}
