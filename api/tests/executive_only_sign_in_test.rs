//! #5446: a session holding only an organisation-wide EXECUTIVE is signed in on every staff page and on
//! `/api/rpc`, by the same rule the session reader already applies (`is_staff_caller`, design
//! `docs/design/blueprint/user-management-rbac.md` §11.8).
//!
//! Today `GET /` answers 303 `/login?next=%2F` for that session while `/api/auth/session` answers it
//! signed in, and the sign-in page sends it back to `/`: the loop of #5446. The page gates
//! (`pages.rs` `who_signed_in` and `devices_receipt_handler`, `screens.rs` `Gate::SignedIn` and
//! `Gate::Branch`) and `caller_of_rpc` ask `Standing::of` instead, which has no answer for EXECUTIVE alone.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! Every test drives the real `AccountSessionReader` over a seeded Blueprint session (never a test
//! `SessionReader`). Needs `TEST_DATABASE_URL`, like every auth route test.

#[macro_use]
mod auth_http_common;
mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, test};
use auth_http_common::Desk;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use xc_testdb::people::{RoleRow, seed_person};
use xc_testdb::sessions::{SESSION_COOKIE, seed_session};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::SEEDED_XC_ORG_ID;
use xchange::presentation::screens::ScreenTable;

const HOST: &str = "xchange.test";
const ORIGIN: &str = "https://xchange.test";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Which page server answers: v2's pages (`pages.rs`), or the switched screen table (`screens.rs`).
#[derive(Clone, Copy, Debug)]
enum Pages {
    V2,
    Switched,
}

struct Reply {
    status: StatusCode,
    location: Option<String>,
    body: String,
}

/// One person with `roles` (valid from the epoch, ending at `valid_to`) and a live session.
async fn session_of(
    desk: &Desk,
    name: &str,
    roles: &[(&str, Option<&str>)],
    valid_to: Option<DateTime<Utc>>,
) -> String {
    let mut conn = desk.pool.acquire().await.expect("a connection");
    let rows: Vec<RoleRow<'_>> = roles
        .iter()
        .map(|(role_code, branch_id)| RoleRow {
            role_code,
            branch_id: *branch_id,
            valid_from: DateTime::<Utc>::UNIX_EPOCH,
            valid_to,
        })
        .collect();
    let id = seed_person(&mut conn, SEEDED_XC_ORG_ID, name, &rows)
        .await
        .unwrap_or_else(|e| panic!("seed {name}: {e}"));
    seed_session(&mut conn, &id, None, Utc::now() + Duration::hours(12))
        .await
        .unwrap_or_else(|e| panic!("seed the session of {name}: {e}"))
}

/// `method uri` with `token` as the Blueprint session, through the real reader.
async fn call(
    desk: &Desk,
    pages: Pages,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> Reply {
    let web_dir = fixtures().join("pages/web");
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_WEB__DIR",
            web_dir.to_str().expect("a UTF-8 path"),
        )])),
        desk.pool.clone(),
    );
    let app = match pages {
        Pages::V2 => {
            // the server before any screen, whether or not web/dist was built
            let screens = ScreenTable::from_dist(&fixtures().join("ui/empty"))
                .expect("the empty fixture dist loads");
            test::init_service(
                App::new()
                    .configure(move |cfg| bootstrap::configure_with_screens(cfg, state, screens)),
            )
            .await
        }
        Pages::Switched => {
            let screens = ScreenTable::from_dist(&fixtures().join("ui/switched"))
                .expect("the switched fixture dist loads");
            test::init_service(
                App::new()
                    .configure(move |cfg| bootstrap::configure_with_screens(cfg, state, screens)),
            )
            .await
        }
    };
    let mut req = test::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-forwarded-proto", "https"));
    if let Some(t) = token {
        req = req.insert_header((header::COOKIE, format!("{SESSION_COOKIE}={t}")));
    }
    if let Some(b) = body {
        req = req
            .insert_header((header::ORIGIN, ORIGIN))
            .insert_header(("x-xchange-request", "1"))
            .insert_header((header::CONTENT_TYPE, "application/json"))
            .set_payload(b.to_string());
    }
    let resp = test::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let location = resp
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().expect("a text Location").to_owned());
    let body = String::from_utf8_lossy(&test::read_body(resp).await).into_owned();
    Reply {
        status,
        location,
        body,
    }
}

async fn get(desk: &Desk, pages: Pages, uri: &str, token: Option<&str>) -> Reply {
    call(desk, pages, Method::GET, uri, token, None).await
}

fn assert_sent_to_login(r: &Reply, uri: &str, case: &str) {
    assert_eq!(
        r.status,
        StatusCode::SEE_OTHER,
        "{case}: {uri} → {} {}",
        r.status,
        r.body
    );
    let loc = r.location.as_deref().unwrap_or("");
    assert!(
        loc.contains("/login?next="),
        "{case}: {uri} is sent to the sign-in page, got Location {loc:?}"
    );
}

fn assert_admin_shell(r: &Reply, uri: &str, case: &str) {
    assert_eq!(
        r.status,
        StatusCode::OK,
        "{case}: {uri} → {} Location {:?} (the #5446 loop when it is 303 /login?next=%2F)",
        r.status,
        r.location
    );
    assert!(r.location.is_none(), "{case}: {uri} is not redirected");
}

/// E1 / E2 (v2 pages): EXECUTIVE org-wide only gets the admin shell at `/`, at `MAIN` when no branch is
/// named and at the named branch otherwise (the owner's path through `page_branch`).
#[actix_web::test]
async fn e1_e2_executive_only_gets_the_admin_shell_at_root() {
    let desk = Desk::start().await;
    let exec = session_of(&desk, "ExecOnly", &[("EXECUTIVE", None)], None).await;

    let root = get(&desk, Pages::V2, "/", Some(&exec)).await;
    assert_admin_shell(&root, "/", "E1");
    assert!(
        root.body.contains("window.XC_BRANCH=\"MAIN\""),
        "E1: no branch named → MAIN: {}",
        root.body
    );

    let named = get(&desk, Pages::V2, "/?branch=B2", Some(&exec)).await;
    assert_admin_shell(&named, "/?branch=B2", "E2");
    assert!(
        named.body.contains("window.XC_BRANCH=\"B2\""),
        "E2: the named branch is the page's branch: {}",
        named.body
    );
    desk.drop_schema().await;
}

/// E3 (switched screens): the same session gets the switched admin entry at `/`, not the sign-in page.
#[actix_web::test]
async fn e3_executive_only_gets_the_switched_admin_screen_at_root() {
    let desk = Desk::start().await;
    let exec = session_of(&desk, "ExecOnly", &[("EXECUTIVE", None)], None).await;
    let root = get(&desk, Pages::Switched, "/", Some(&exec)).await;
    assert_admin_shell(&root, "/", "E3");
    desk.drop_schema().await;
}

/// E4: `/devices/receipt` passes the sign-in check for EXECUTIVE only (it then answers on the reference,
/// never with the sign-in page).
#[actix_web::test]
async fn e4_executive_only_passes_the_devices_receipt_sign_in_check() {
    let desk = Desk::start().await;
    let exec = session_of(&desk, "ExecOnly", &[("EXECUTIVE", None)], None).await;
    let r = get(
        &desk,
        Pages::V2,
        "/devices/receipt?branch=MAIN",
        Some(&exec),
    )
    .await;
    assert_ne!(
        r.status,
        StatusCode::SEE_OTHER,
        "E4: /devices/receipt sends EXECUTIVE only to {:?}",
        r.location
    );
    desk.drop_schema().await;
}

/// E5: `/api/rpc` answers EXECUTIVE only as signed in: `adminSnapshot` at MAIN is 200 like the owner's,
/// never 401 "กรุณาเข้าสู่ระบบ".
#[actix_web::test]
async fn e5_executive_only_is_signed_in_on_api_rpc() {
    let desk = Desk::start().await;
    let exec = session_of(&desk, "ExecOnly", &[("EXECUTIVE", None)], None).await;
    let owner = desk.owner_session("Owner").await;
    let ask = json!({ "action": "adminSnapshot", "payload": { "branchId": "MAIN" } });

    let as_owner = call(
        &desk,
        Pages::V2,
        Method::POST,
        "/api/rpc",
        Some(&owner),
        Some(ask.clone()),
    )
    .await;
    assert_eq!(
        as_owner.status,
        StatusCode::OK,
        "E5 baseline, the owner: {}",
        as_owner.body
    );

    let as_exec = call(
        &desk,
        Pages::V2,
        Method::POST,
        "/api/rpc",
        Some(&exec),
        Some(ask),
    )
    .await;
    assert_eq!(
        as_exec.status,
        StatusCode::OK,
        "E5: EXECUTIVE only on /api/rpc adminSnapshot: {}",
        as_exec.body
    );
    desk.drop_schema().await;
}

/// E6–E9: what stays as today.
/// - E6: CUSTOMER only → `/` sends to the sign-in page (the sign-in page then sends a customer to `/app`).
/// - E7: no session → the sign-in page.
/// - E8: EXECUTIVE whose `valid_to` has passed, nothing else → the sign-in page (`/` and `screens.rs`).
/// - E9: ADMIN org-wide only and the owner → the admin shell, as today.
#[actix_web::test]
async fn e6_to_e9_other_callers_keep_todays_answer() {
    let desk = Desk::start().await;
    let customer = session_of(&desk, "Cust", &[("CUSTOMER", None)], None).await;
    let lapsed = session_of(
        &desk,
        "ExecGone",
        &[("EXECUTIVE", None)],
        Some(Utc::now() - Duration::days(1)),
    )
    .await;
    let admin = session_of(&desk, "AdminOnly", &[("ADMIN", None)], None).await;
    let owner = desk.owner_session("Owner").await;

    for pages in [Pages::V2, Pages::Switched] {
        assert_sent_to_login(
            &get(&desk, pages, "/", Some(&customer)).await,
            "/",
            &format!("E6 {pages:?}"),
        );
        assert_sent_to_login(
            &get(&desk, pages, "/", None).await,
            "/",
            &format!("E7 {pages:?}"),
        );
        assert_sent_to_login(
            &get(&desk, pages, "/", Some(&lapsed)).await,
            "/",
            &format!("E8 {pages:?}"),
        );
        assert_admin_shell(
            &get(&desk, pages, "/", Some(&admin)).await,
            "/",
            &format!("E9 admin {pages:?}"),
        );
        assert_admin_shell(
            &get(&desk, pages, "/", Some(&owner)).await,
            "/",
            &format!("E9 owner {pages:?}"),
        );
    }

    let ask = json!({ "action": "adminSnapshot", "payload": { "branchId": "MAIN" } });
    let as_customer = call(
        &desk,
        Pages::V2,
        Method::POST,
        "/api/rpc",
        Some(&customer),
        Some(ask),
    )
    .await;
    assert_eq!(
        as_customer.status,
        StatusCode::UNAUTHORIZED,
        "E6: CUSTOMER only stays not signed in on /api/rpc: {}",
        as_customer.body
    );
    desk.drop_schema().await;
}

/// E10: the TELLER-only desk is unchanged for EXECUTIVE only: `/staff/desk` (gate `front`) still sends it to
/// `/staff`, because it holds no TELLER role.
#[actix_web::test]
async fn e10_executive_only_still_cannot_open_the_teller_desk() {
    let desk = Desk::start().await;
    let exec = session_of(&desk, "ExecOnly", &[("EXECUTIVE", None)], None).await;
    let r = get(&desk, Pages::Switched, "/staff/desk", Some(&exec)).await;
    assert_ne!(
        r.status,
        StatusCode::OK,
        "E10: /staff/desk stays TELLER only: {}",
        r.body
    );
    desk.drop_schema().await;
}
