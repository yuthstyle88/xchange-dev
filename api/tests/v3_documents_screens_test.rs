//! #1349 V3-documents (design `docs/design/xchange-v3-wave3-report-documents.md` §A, file **T** of §A.8): the two
//! pages are screens now. `/documents` (gate `signed-in`) and `/staff/documents` (gate `anyone`) answer through the
//! server's screen table, and the old server-written pages (`api/src/presentation/http/documents.rs`, registered
//! before the screen table) are gone. Written before the implementation by the test-writer claude/5d45b017, on the FE
//! lane controller's order (claude/5edf7bb5). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The server is `bootstrap::configure_with_screens` with a screen table read from a `dist` this file builds in a temp
//! directory: the two source entries `web/pages/documents.html` and `web/pages/staff-documents.html`, as they are in
//! the tree, plus an empty Vite manifest. So no `npm run build` is needed. Who is signed in is
//! `documents_common::FakeSessions` (the fixture's people). Needs `TEST_DATABASE_URL`; nothing skips.

mod common;
mod documents_common;
mod save_common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::{App, test, web};
use documents_common::*;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::SessionReader;
use xchange::presentation::screens::ScreenTable;

const ENTRIES: [(&str, &str); 2] = [
    ("documents.html", "/documents"),
    ("staff-documents.html", "/staff/documents"),
];

/// A `dist` holding the two source entries and an empty manifest.
fn documents_dist() -> PathBuf {
    let pages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/pages");
    let dir = std::env::temp_dir().join(save_common::unique("xchange-documents-dist"));
    std::fs::create_dir_all(dir.join("pages")).unwrap();
    std::fs::create_dir_all(dir.join(".vite")).unwrap();
    std::fs::write(dir.join(".vite/manifest.json"), "{}").unwrap();
    for (file, _) in ENTRIES {
        let source = pages.join(file);
        assert!(source.is_file(), "web/pages/{file} exists");
        std::fs::copy(&source, dir.join("pages").join(file)).unwrap();
    }
    dir
}

fn screens() -> ScreenTable {
    let dist = documents_dist();
    let table = ScreenTable::from_dist(&dist);
    assert!(
        table.is_ok(),
        "the documents entries make a screen table: {:?}",
        table.as_ref().err()
    );
    let table = table.unwrap();
    for (_, path) in ENTRIES {
        assert!(table.holds(path), "the screen table holds {path}");
    }
    table
}

async fn app_with(state: AppState, reader: Option<Arc<dyn SessionReader>>) -> TestApp {
    let screens = screens();
    match reader {
        Some(reader) => TestApp::new(
            test::init_service(
                App::new()
                    .configure(move |cfg| bootstrap::configure_with_screens(cfg, state, screens))
                    .app_data(web::Data::from(reader)),
            )
            .await,
            |t| t.to_request(),
        ),
        None => TestApp::new(
            test::init_service(
                App::new()
                    .configure(move |cfg| bootstrap::configure_with_screens(cfg, state, screens)),
            )
            .await,
            |t| t.to_request(),
        ),
    }
}

/// The `signed-in` gate's sign-in page (`pages::to_login`): v2's `/login`, now carrying the page asked for in
/// `next` (measured on main; a known difference from v2's bare `/login`, reported on #1349).
fn login_from(path_and_query: &str) -> String {
    let mut next = String::new();
    for b in path_and_query.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            next.push(b as char);
        } else {
            next.push_str(&format!("%{b:02X}"));
        }
    }
    format!("https://desk.example/login?next={next}")
}

fn fake(fixture: &serde_json::Value) -> Option<Arc<dyn SessionReader>> {
    Some(Arc::new(FakeSessions::from_fixture(fixture)))
}

/// The answer is the page's own entry: HTML that declares this path.
fn is_the_entry(answer: &Answer, path: &str, what: &str) {
    assert_eq!(
        answer.status, 200,
        "{what}: the page loads; body {:?}",
        answer.text
    );
    assert!(
        answer
            .headers
            .get("content-type")
            .is_some_and(|t| t.starts_with("text/html")),
        "{what}: HTML, got {:?}",
        answer.headers.get("content-type")
    );
    assert!(
        answer
            .text
            .contains(&format!("<meta name=\"xc-path\" content=\"{path}\">")),
        "{what}: the {path} entry, got {:?}",
        answer.text
    );
}

#[actix_web::test]
async fn a_wrong_method_is_the_screen_tables_404() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = app_with(world.state(), fake(&fixture)).await;
    let reference = &world.exchange("cash").document_ref;
    for path in [
        format!("/documents?ref={reference}"),
        format!("/staff/documents?ref={reference}"),
    ] {
        for method in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
            let answer = send(&app, method.clone(), &path, &["owner", "front"]).await;
            assert_eq!(
                (answer.status, answer.text.as_str()),
                (404, "Not found"),
                "{method} {path}"
            );
        }
    }
    drop(app);
    world.shop.finish().await;
}

#[actix_web::test]
async fn nobody_and_a_first_password_employee_are_sent_to_login_from_documents() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = app_with(world.state(), fake(&fixture)).await;
    let reference = &world.exchange("cash").document_ref;
    for who in [&[][..], &["newcomer"][..]] {
        let answer = get(&app, &format!("/documents?ref={reference}"), who).await;
        assert_eq!(answer.status, 303, "as {who:?}: {}", answer.text);
        assert_eq!(
            answer.headers.get("location").cloned(),
            Some(login_from(&format!("/documents?ref={reference}"))),
            "as {who:?}"
        );
    }
    drop(app);
    world.shop.finish().await;
}

#[actix_web::test]
async fn without_a_session_reader_documents_is_sent_to_login() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = app_with(world.state(), None).await;
    let reference = &world.exchange("cash").document_ref;
    let answer = get(&app, &format!("/documents?ref={reference}"), &[]).await;
    assert_eq!(answer.status, 303, "{}", answer.text);
    assert_eq!(
        answer.headers.get("location").cloned(),
        Some(login_from(&format!("/documents?ref={reference}")))
    );
    // the staff page is a page anyone loads; its view sends the visitor back (D-A4)
    let answer = get(&app, &format!("/staff/documents?ref={reference}"), &[]).await;
    is_the_entry(
        &answer,
        "/staff/documents",
        "/staff/documents with no reader",
    );
    drop(app);
    world.shop.finish().await;
}

#[actix_web::test]
async fn the_pages_load_and_leave_found_or_not_found_to_the_view() {
    let fixture = fixture();
    let world = World::start(&fixture).await;
    let app = app_with(world.state(), fake(&fixture)).await;
    let (cash, other) = (world.exchange("cash"), world.exchange("other"));
    for (path, who, what) in [
        (
            format!("/documents?ref={}", cash.document_ref),
            "owner",
            "an own exchange",
        ),
        (
            format!("/documents?ref={}", other.document_ref),
            "owner",
            "another branch's exchange (v2: 404)",
        ),
        (
            "/documents?ref=XC-NOPE".to_owned(),
            "owner",
            "an unknown ref (v2: 404)",
        ),
        (
            "/documents?branch=NOWHERE&ref=XC-1".to_owned(),
            "owner",
            "no such branch (v2: 404)",
        ),
        (
            "/documents?branch=MAIN_1&ref=XC-1".to_owned(),
            "owner",
            "a malformed branch (v2: 404)",
        ),
        (
            format!("/documents?ref={}", cash.document_ref),
            "front",
            "an employee",
        ),
    ] {
        is_the_entry(
            &get(&app, &path, &[who]).await,
            "/documents",
            &format!("{what}: {path} as {who}"),
        );
    }
    for (who, what) in [
        (&[][..], "nobody (v2: 302)"),
        (&["owner"][..], "the owner (v2: 302)"),
        (&["newcomer"][..], "a first-password employee (v2: 302)"),
        (&["front"][..], "Front"),
        (&["rates"][..], "Rates (v2: 302)"),
    ] {
        let path = format!("/staff/documents?ref={}", cash.document_ref);
        is_the_entry(
            &get(&app, &path, who).await,
            "/staff/documents",
            &format!("{what}: {path}"),
        );
    }
    drop(app);
    world.shop.finish().await;
}

#[actix_web::test]
async fn when_the_database_is_down_the_gate_still_answers() {
    let fixture = fixture();
    let dir = std::env::temp_dir().join(save_common::unique("xchange-documents-screens-down"));
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            dir.to_str().unwrap(),
        )])),
        common::unreachable_pool(),
    );
    let app = app_with(state, fake(&fixture)).await;
    // nobody signed in is sent to sign in: that needs no database
    assert_eq!(get(&app, "/documents?ref=XC-1", &[]).await.status, 303);
    // the staff page loads; its view answers 500 and the screen goes back (D-A4)
    is_the_entry(
        &get(&app, "/staff/documents?ref=XC-1", &[]).await,
        "/staff/documents",
        "database down, nobody",
    );
}
