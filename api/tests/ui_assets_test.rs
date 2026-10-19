//! Specification tests for V0-A (docs/design/xchange-v3-web.md §3.2, §3.3, §9):
//! the v3 screens compiled into the server, the table of the paths they take
//! over from v2, and the four page gates.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! - `presentation::screens::ScreenTable` is the table of v3 screens and the
//!   files they are built from. `ScreenTable::from_dist(dir)` reads a built
//!   `web/dist` at run time with the same pure functions `api/build.rs` uses
//!   (`api/build_support/screens.rs`), so these tests can hand the server a
//!   fixture `dist` instead of the one compiled in. The production server uses
//!   `ScreenTable::compiled()`.
//! - `bootstrap::configure_with_screens(cfg, state, table)` is
//!   `bootstrap::configure` with that table; `bootstrap::configure` passes
//!   `ScreenTable::compiled()`.
//! - The fixture `dist`s are under `fixtures/ui/`: `empty/` (what `build.rs`
//!   creates when nobody built `web/`) and `built/` (Vite's manifest, one
//!   hashed asset and five screen entries in `pages/`).
//! - v2's pages are the existing fixtures under `fixtures/pages/web`, as in
//!   `pages_test.rs`.
//!
//! No test here needs a database.

mod common;

#[path = "../build_support/screens.rs"]
mod screens_gen;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use common::{FakeDatabase, settings, state_with};
use serde_json::Value;
use xc_auth::domain::access::{Actor, Role};
use xc_kernel::ids::BranchId;
use xchange::bootstrap;
use xchange::presentation::pages::{self, Callers, SessionReader};
use xchange::presentation::screens::ScreenTable;
use xchange::presentation::test_callers;

use screens_gen::{Gate, ScreenTableError, check_dist, ensure_dist, screen_table};

const HOST: &str = "xc.test";
const AS: &str = "x-test-as";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn v2_web_dir() -> PathBuf {
    fixtures().join("pages/web")
}

fn empty_dist() -> PathBuf {
    fixtures().join("ui/empty")
}

fn built_dist() -> PathBuf {
    fixtures().join("ui/built")
}

fn table(dir: &Path) -> ScreenTable {
    ScreenTable::from_dist(dir)
        .unwrap_or_else(|e| panic!("the fixture dist {} must load: {e:?}", dir.display()))
}

fn built_file(rel: &str) -> Vec<u8> {
    std::fs::read(built_dist().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Who is signed in, from the request header `x-test-as`:
/// `owner`, `staff:<ROLE>:<branch>`, `owner+staff:<ROLE>:<branch>`, or absent.
struct HeaderCallers;

fn staff_roles(role: &str) -> &'static [Role] {
    match role {
        "FRONT" => &[Role::Teller],
        "BACKEND" | "MANAGER" => &[Role::BranchManager],
        "RATES" => &[Role::RateManager],
        "FINANCE" => &[Role::Finance],
        "SYSTEM" => &[Role::Admin],
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => &[Role::VerificationReviewer],
        other => panic!("unknown staff role {other:?}"),
    }
}

fn staff_of(spec: &str) -> Option<Actor> {
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let role = parts.next().expect("a role");
    let branch = parts.next().expect("a branch, possibly empty");
    Some(test_callers::staff(
        "employee-7",
        staff_roles(role),
        BranchId::new(branch),
    ))
}

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            if spec.is_empty() {
                return test_callers::nobody();
            }
            if spec == "owner" {
                return test_callers::callers(Some(test_callers::owner("owner-1")), None);
            }
            if let Some(staff) = spec.strip_prefix("owner+") {
                return test_callers::callers(
                    Some(test_callers::owner("owner-1")),
                    staff_of(staff),
                );
            }
            test_callers::callers(None, staff_of(&spec))
        })
    }
}

/// Everything about one answer that a v2 page and a v3 screen are compared on.
#[derive(Debug)]
struct Answer {
    status: StatusCode,
    location: Option<String>,
    content_type: String,
    cache_control: Option<String>,
    nosniff: bool,
    body: Vec<u8>,
}

impl Answer {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn header_of(resp: &actix_web::dev::ServiceResponse, name: header::HeaderName) -> Option<String> {
    resp.headers()
        .get(name)
        .map(|v| v.to_str().expect("a text header value").to_owned())
}

/// `GET uri` as `who` (see `HeaderCallers`), on a server whose screen table
/// was read from `dist`, with v2's pages in `fixtures/pages/web`.
async fn get(dist: &Path, who: &str, uri: &str) -> Answer {
    let screens = table(dist);
    let state = state_with(
        settings(&[("APP_WEB__DIR", v2_web_dir().to_str().expect("a UTF-8 path"))]),
        FakeDatabase::Up,
    );
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure_with_screens(cfg, state, screens))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::get()
        .uri(uri)
        .insert_header((header::HOST, HOST));
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    answer_of(resp).await
}

async fn answer_of(resp: actix_web::dev::ServiceResponse) -> Answer {
    let status = resp.status();
    let location = header_of(&resp, header::LOCATION);
    let content_type = header_of(&resp, header::CONTENT_TYPE).unwrap_or_default();
    let cache_control = header_of(&resp, header::CACHE_CONTROL);
    let nosniff = header_of(&resp, header::X_CONTENT_TYPE_OPTIONS).as_deref() == Some("nosniff");
    let body = atest::read_body(resp).await.to_vec();
    Answer {
        status,
        location,
        content_type,
        cache_control,
        nosniff,
        body,
    }
}

fn is_ui_not_built(a: &Answer, what: &str) {
    assert_eq!(
        a.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{what}: status; body {:?}",
        a.text()
    );
    assert!(
        a.content_type.starts_with("text/plain"),
        "{what}: Content-Type {:?}",
        a.content_type
    );
    assert_eq!(a.text(), "ui not built", "{what}: body");
}

fn is_plain_404(a: &Answer, what: &str) {
    assert_eq!(
        a.status,
        StatusCode::NOT_FOUND,
        "{what}: status; body {:?}",
        a.text()
    );
    assert_eq!(a.content_type, "text/plain", "{what}: Content-Type");
    assert_eq!(a.text(), "Not found", "{what}: body");
}

/// The screen's built HTML entry, byte for byte: no version line, no
/// `/_xc/` script, no `window.XC_BRANCH` (§3.2, §3.3).
fn is_screen(a: &Answer, entry: &str, what: &str) {
    assert_eq!(
        a.status,
        StatusCode::OK,
        "{what}: status; body {:?}",
        a.text()
    );
    assert!(
        a.content_type.starts_with("text/html"),
        "{what}: Content-Type {:?}",
        a.content_type
    );
    assert_eq!(
        a.cache_control.as_deref(),
        Some("no-cache"),
        "{what}: Cache-Control"
    );
    assert!(a.nosniff, "{what}: X-Content-Type-Options: nosniff");
    assert_eq!(
        a.body,
        built_file(entry),
        "{what}: the body is the built entry {entry} unchanged; got {:?}",
        a.text()
    );
}

// ------------------------------------------------------------ not built

#[actix_web::test]
async fn an_empty_dist_answers_every_ui_path_with_503_ui_not_built() {
    for uri in [
        "/ui/",
        "/ui/admin/",
        "/ui/assets/index-AbC123.js",
        "/ui/anything/else",
    ] {
        for who in ["", "owner", "staff:FRONT:XC-B01"] {
            let what = format!("GET {uri} as {who:?} with an empty dist");
            is_ui_not_built(&get(&empty_dist(), who, uri).await, &what);
        }
    }
}

#[actix_web::test]
async fn an_empty_dist_leaves_v2s_pages_where_they_were() {
    let a = get(&empty_dist(), "", "/login").await;
    assert_eq!(a.status, StatusCode::OK, "v2's /login; body {:?}", a.text());
    assert!(
        a.text().contains("/_xc/forms.js"),
        "v2's page still carries the form helper: {:?}",
        a.text()
    );
}

#[actix_web::test]
async fn health_ready_reports_ui_missing_for_an_empty_dist_and_built_for_a_built_one() {
    for (dist, want) in [(empty_dist(), "missing"), (built_dist(), "built")] {
        let a = get(&dist, "", "/health/ready").await;
        assert_eq!(a.status, StatusCode::OK, "{}: status", dist.display());
        let body: Value = serde_json::from_slice(&a.body).expect("a JSON body");
        assert_eq!(body["ui"], want, "{}: ui in {body}", dist.display());
        // the fields it had keep their place
        assert_eq!(body["database"], true, "{}: database", dist.display());
        assert!(body["build"].is_string(), "{}: build", dist.display());
    }
}

// ------------------------------------------------------------ built

#[actix_web::test]
async fn a_hashed_asset_is_served_immutable() {
    let a = get(&built_dist(), "", "/ui/assets/index-AbC123.js").await;
    assert_eq!(a.status, StatusCode::OK, "status; body {:?}", a.text());
    assert!(
        a.content_type.starts_with("text/javascript"),
        "Content-Type {:?}",
        a.content_type
    );
    assert_eq!(
        a.cache_control.as_deref(),
        Some("public, max-age=31536000, immutable"),
        "Cache-Control"
    );
    assert!(a.nosniff, "X-Content-Type-Options: nosniff");
    assert_eq!(a.body, built_file("assets/index-AbC123.js"));
}

#[actix_web::test]
async fn a_screen_entry_is_served_no_cache_and_unchanged() {
    is_screen(
        &get(&built_dist(), "", "/login").await,
        "pages/login.html",
        "GET /login (gate anyone) as nobody",
    );
    is_screen(
        &get(&built_dist(), "owner", "/ui/admin/").await,
        "pages/admin.html",
        "GET /ui/admin/ (gate signed-in) as the owner",
    );
}

#[actix_web::test]
async fn a_path_in_neither_table_is_the_plain_404() {
    for uri in [
        "/no-such-page",
        "/ui/assets/no-such-XyZ987.js",
        "/ui/no-such",
        // the entries are served only at their own path, behind their gate
        "/ui/pages/staff-guide.html",
        "/ui/pages/login.html",
        "/ui/.vite/manifest.json",
        "/ui/assets/%2e%2e/pages/staff-guide.html",
    ] {
        for who in ["", "owner"] {
            let what = format!("GET {uri} as {who:?} with a built dist");
            is_plain_404(&get(&built_dist(), who, uri).await, &what);
        }
    }
}

// ------------------------------------------------------------ cutover

#[actix_web::test]
async fn a_screen_declaring_staff_guide_is_served_instead_of_v2s_page() {
    let v2 = get(&empty_dist(), "owner", "/staff-guide").await;
    assert_eq!(
        v2.status,
        StatusCode::OK,
        "v2's /staff-guide is served first"
    );
    assert!(
        v2.text().contains("guide (fixture)"),
        "v2's fixture page: {:?}",
        v2.text()
    );

    for who in ["owner", "staff:BACKEND:XC-B01", "staff:FRONT:XC-B01"] {
        is_screen(
            &get(&built_dist(), who, "/staff-guide").await,
            "pages/staff-guide.html",
            &format!("GET /staff-guide as {who} once a v3 screen holds it"),
        );
    }
}

#[actix_web::test]
async fn the_signed_in_gate_redirects_nobody_as_v2s_page_did() {
    let v2 = get(&empty_dist(), "", "/staff-guide").await;
    let v3 = get(&built_dist(), "", "/staff-guide").await;
    assert_eq!(v2.status, StatusCode::SEE_OTHER, "v2: status");
    assert_eq!(
        v2.location.as_deref(),
        Some("http://xc.test/login?next=%2Fstaff-guide"),
        "v2: Location"
    );
    assert_eq!(v3.status, v2.status, "v3: status as v2's");
    assert_eq!(v3.location, v2.location, "v3: Location as v2's");
    assert_eq!(v3.body, v2.body, "v3: body as v2's");
}

#[actix_web::test]
async fn page_handler_hands_a_held_path_to_the_screen_even_when_registered_first() {
    // `pages::configure` registered before the screen table: v2's resource for
    // /staff-guide matches first, so only `page_handler` skipping a path the
    // table holds keeps v2's page from being served.
    let screens = table(&built_dist());
    let state = state_with(
        settings(&[("APP_WEB__DIR", v2_web_dir().to_str().expect("a UTF-8 path"))]),
        FakeDatabase::Up,
    );
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| {
                cfg.app_data(web::Data::new(state.clone()));
                pages::configure(cfg, &state);
                screens.configure(cfg, &state);
            })
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri("/staff-guide")
        .insert_header((header::HOST, HOST))
        .insert_header((AS, "owner"))
        .to_request();
    let a = answer_of(atest::call_service(&app, req).await).await;
    is_screen(
        &a,
        "pages/staff-guide.html",
        "GET /staff-guide with v2's routes registered first",
    );
}

// ------------------------------------------------------------ gates

/// v2's `PageKind::Branch` answers (`pages.rs:235-270`) that the v3 `branch`
/// gate must give too: every case but the plain 200 is compared whole.
#[actix_web::test]
async fn the_branch_gate_answers_as_v2s_branch_page() {
    let seventy_one = format!("staff:FRONT:{}", "a".repeat(71));
    let cases: Vec<(&str, String)> = vec![
        ("nobody signed in", "".into()),
        ("staff with a branch", "staff:FRONT:XC-B01".into()),
        (
            "staff with a branch, naming another",
            "staff:FRONT:XC-B01".into(),
        ),
        ("staff whose branch is empty", "staff:FRONT:".into()),
        ("staff whose branch is not a name", seventy_one),
        ("staff whose branch has a quote", "staff:BACKEND:\"x".into()),
        ("the owner with ?branch=X", "owner".into()),
        ("the owner with ?branch=", "owner".into()),
        ("the owner with no query", "owner".into()),
        ("the owner with an unparsable query", "owner".into()),
        ("the owner with an invalid name", "owner".into()),
        (
            "the owner and a staff session",
            "owner+staff:FRONT:XC-B01".into(),
        ),
    ];
    let queries = [
        "",
        "",
        "?branch=XC-B02",
        "",
        "",
        "",
        "?branch=XC-B02",
        "?branch=",
        "",
        "?branch=%ff",
        "?branch=XC_B02",
        "?branch=bad%20name",
    ];
    let mut ok = 0;
    let mut refused = 0;
    for ((label, who), query) in cases.iter().zip(queries) {
        let uri = format!("/receipt-sample{query}");
        let v2 = get(&empty_dist(), who, &uri).await;
        let v3 = get(&built_dist(), who, &uri).await;
        let what = format!("GET {uri} as {label}");
        assert_eq!(
            v3.status,
            v2.status,
            "{what}: status as v2's; v3 body {:?}",
            v3.text()
        );
        if v2.status == StatusCode::OK {
            ok += 1;
            assert!(
                v2.text().contains("window.XC_BRANCH="),
                "{what}: v2 tells the branch"
            );
            is_screen(&v3, "pages/receipt-sample.html", &what);
        } else {
            refused += 1;
            assert_eq!(v3.location, v2.location, "{what}: Location as v2's");
            assert_eq!(
                v3.content_type, v2.content_type,
                "{what}: Content-Type as v2's"
            );
            assert_eq!(v3.body, v2.body, "{what}: body as v2's ({:?})", v2.text());
        }
    }
    // the list covers both sides: four answers of 200, the rest refused
    assert!(ok >= 4, "cases answered 200: {ok}");
    assert!(refused >= 5, "cases refused: {refused}");
}

/// v2's desk variant (`pages.rs:175-180`): anyone but a FRONT employee is sent
/// to `/staff`.
#[actix_web::test]
async fn the_front_gate_answers_as_v2s_desk_variant() {
    for who in [
        "",
        "owner",
        "staff:BACKEND:XC-B01",
        "staff:RATES:XC-B01",
        "staff:MANAGER:XC-B01",
    ] {
        let v2 = get(&empty_dist(), who, "/staff/desk").await;
        let v3 = get(&built_dist(), who, "/staff/desk").await;
        let what = format!("GET /staff/desk as {who:?}");
        assert_eq!(v2.status, StatusCode::FOUND, "{what}: v2 status");
        assert_eq!(
            v2.location.as_deref(),
            Some("http://xc.test/staff"),
            "{what}: v2 Location"
        );
        assert_eq!(v3.status, v2.status, "{what}: status as v2's");
        assert_eq!(v3.location, v2.location, "{what}: Location as v2's");
        assert_eq!(v3.body, v2.body, "{what}: body as v2's");
    }
    is_screen(
        &get(&built_dist(), "staff:FRONT:XC-B01", "/staff/desk").await,
        "pages/staff-desk.html",
        "GET /staff/desk as FRONT",
    );
}

#[actix_web::test]
async fn the_anyone_gate_sends_the_screen_to_nobody_signed_in() {
    is_screen(
        &get(&built_dist(), "", "/login").await,
        "pages/login.html",
        "GET /login as nobody",
    );
}

// ------------------------------------------------------------ the generator

fn entry(path: Option<&str>, gate: Option<&str>) -> String {
    let mut head = String::from("<!doctype html><html><head><meta charset=\"utf-8\">");
    if let Some(p) = path {
        head.push_str(&format!("<meta name=\"xc-path\" content=\"{p}\">"));
    }
    if let Some(g) = gate {
        head.push_str(&format!("<meta name=\"xc-gate\" content=\"{g}\">"));
    }
    head.push_str("</head><body></body></html>");
    head
}

#[test]
fn the_generator_reads_path_gate_and_entry_of_every_screen() {
    let login = entry(Some("/login"), Some("anyone"));
    let guide = entry(Some("/staff-guide"), Some("signed-in"));
    let sample = entry(Some("/receipt-sample"), Some("branch"));
    let desk = entry(Some("/staff/desk"), Some("front"));
    let table = screen_table(&[
        ("pages/login.html", login.as_str()),
        ("pages/staff-guide.html", guide.as_str()),
        ("pages/receipt-sample.html", sample.as_str()),
        ("pages/staff-desk.html", desk.as_str()),
    ])
    .expect("four well-formed entries");
    assert_eq!(table.len(), 4, "{table:?}");
    for (path, gate, file) in [
        ("/login", Gate::Anyone, "pages/login.html"),
        ("/staff-guide", Gate::SignedIn, "pages/staff-guide.html"),
        ("/receipt-sample", Gate::Branch, "pages/receipt-sample.html"),
        ("/staff/desk", Gate::Front, "pages/staff-desk.html"),
    ] {
        let found = table
            .iter()
            .find(|s| s.path == path)
            .unwrap_or_else(|| panic!("{path} in {table:?}"));
        assert_eq!(found.gate, gate, "{path}: gate");
        assert_eq!(found.entry, file, "{path}: entry");
    }
}

#[test]
fn the_generator_refuses_two_entries_with_the_same_path() {
    let a = entry(Some("/login"), Some("anyone"));
    let b = entry(Some("/login"), Some("signed-in"));
    let err = screen_table(&[("pages/a.html", a.as_str()), ("pages/b.html", b.as_str())])
        .expect_err("two entries take /login");
    assert_eq!(
        err,
        ScreenTableError::DuplicatePath {
            path: "/login".into(),
            first: "pages/a.html".into(),
            second: "pages/b.html".into(),
        }
    );
}

#[test]
fn the_generator_refuses_an_entry_without_a_gate() {
    let a = entry(Some("/login"), None);
    let err = screen_table(&[("pages/a.html", a.as_str())]).expect_err("no xc-gate");
    assert_eq!(
        err,
        ScreenTableError::MissingGate {
            entry: "pages/a.html".into()
        }
    );
}

#[test]
fn the_generator_refuses_a_gate_outside_the_four() {
    for gate in ["", "owner", "Anyone", "signed_in", "staff"] {
        let a = entry(Some("/login"), Some(gate));
        let err = screen_table(&[("pages/a.html", a.as_str())])
            .expect_err("a gate other than anyone, signed-in, branch, front");
        assert_eq!(
            err,
            ScreenTableError::UnknownGate {
                entry: "pages/a.html".into(),
                gate: gate.into(),
            },
            "gate {gate:?}"
        );
    }
}

#[test]
fn the_generator_refuses_an_entry_without_a_path() {
    let a = entry(None, Some("anyone"));
    let err = screen_table(&[("pages/a.html", a.as_str())]).expect_err("no xc-path");
    assert_eq!(
        err,
        ScreenTableError::MissingPath {
            entry: "pages/a.html".into()
        }
    );
}

#[test]
fn a_release_build_refuses_a_dist_without_vites_manifest() {
    assert!(
        check_dist("release", &empty_dist()).is_err(),
        "release with an empty dist"
    );
    assert!(
        check_dist("release", &built_dist()).is_ok(),
        "release with a built dist"
    );
    assert!(
        check_dist("debug", &empty_dist()).is_ok(),
        "a debug build works with an empty dist"
    );
}

#[test]
fn a_missing_dist_is_created_empty_so_a_rust_only_build_works() {
    let root = std::env::temp_dir().join(format!(
        "xc-ui-dist-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos()
    ));
    let dist = root.join("web/dist");
    assert!(!dist.exists());
    ensure_dist(&dist).expect("create the missing dist");
    assert!(dist.is_dir(), "{} is a directory", dist.display());
    assert_eq!(
        std::fs::read_dir(&dist).expect("read it").count(),
        0,
        "and it is empty"
    );
    // a dist that exists is left alone
    std::fs::write(dist.join("keep.txt"), b"x").expect("write");
    ensure_dist(&dist).expect("an existing dist is fine");
    assert!(dist.join("keep.txt").exists(), "nothing removed");
    std::fs::remove_dir_all(&root).expect("clean up");
}
