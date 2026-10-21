//! Up-front tests for TD-1 (#1293): pattern paths in the screen table
//! (`docs/design/blueprint/desk-transaction-detail.md` §4.1, decision D1).
//!
//! Written before the implementation by the rule-A test-writer
//! (claude/dell-claude-1, ordered by claude/ad05bf8d). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! A screen may declare an `xc-path` with an actix segment, `/desk/transactions/{id}`
//! (the Blueprint's `/desk/transactions/:id`). `ScreenTable::configure` already
//! registers it as `web::resource`; `ScreenTable::serve` must then find the
//! declaration by the pattern actix matched, not by the literal request path.
//! Literal screens, `holds` and every near-miss stay as they are.
//!
//! Each test builds its own table with `ScreenTable::from_dist` (as
//! `screen_table_release_test.rs:23` does) over a dist written to a temporary
//! folder: one pattern page and one literal page, both gated `branch`.
//! `pages_test.rs` `an_address_that_is_almost_a_page_is_not_one` stays the
//! guard for v2's literal pages.
//!
//! No test here needs a database.

mod common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use common::{FakeDatabase, settings, state_with};
use xc_auth::domain::access::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::screens::ScreenTable;
use xchange::presentation::test_callers;

const HOST: &str = "xc.test";
const AS: &str = "x-test-as";

/// The pattern page's declared path (D1).
const PATTERN: &str = "/desk/transactions/{id}";
/// A transaction id, as B04 is opened with.
const ID: &str = "7f0c2a4e-9b1d-4c3e-8f5a-1d2b3c4d5e6f";

const PATTERN_ENTRY: &str = "pages/desk-transaction.html";
const LITERAL_ENTRY: &str = "pages/desk.html";

/// `desk-transaction.html` sorts before `desk.html` ('-' < '.'), so the pattern
/// page is the table's first entry and the literal page its second.
const PATTERN_HTML: &str = "<!doctype html>\n<html lang=\"th\"><head><meta charset=\"utf-8\">\n\
<meta name=\"xc-path\" content=\"/desk/transactions/{id}\">\n\
<meta name=\"xc-gate\" content=\"branch\">\n\
<title>desk-transaction (TD-1 fixture)</title></head><body><div id=\"root\">desk-transaction</div></body></html>\n";

const LITERAL_HTML: &str = "<!doctype html>\n<html lang=\"th\"><head><meta charset=\"utf-8\">\n\
<meta name=\"xc-path\" content=\"/desk\">\n\
<meta name=\"xc-gate\" content=\"branch\">\n\
<title>desk (TD-1 fixture)</title></head><body><div id=\"root\">desk</div></body></html>\n";

fn v2_web_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pages/web")
}

/// A dist holding exactly the two pages, in a folder of its own, removed on drop.
struct Dist(PathBuf);

impl Dist {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "xc-td1-dist-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let pages = dir.join("pages");
        std::fs::create_dir_all(&pages).expect("create the dist's pages folder");
        std::fs::write(dir.join(PATTERN_ENTRY), PATTERN_HTML).expect("write the pattern page");
        std::fs::write(dir.join(LITERAL_ENTRY), LITERAL_HTML).expect("write the literal page");
        Dist(dir)
    }

    fn table(&self) -> ScreenTable {
        ScreenTable::from_dist(&self.0)
            .unwrap_or_else(|e| panic!("the TD-1 dist {} must load: {e:?}", self.0.display()))
    }
}

impl Drop for Dist {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Who is signed in, from the request header `x-test-as`:
/// `owner`, `staff:<ROLE>:<branch>`, or absent.
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
            let mut parts = spec.splitn(3, ':');
            assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
            let role = parts.next().expect("a role");
            let branch = parts.next().expect("a branch");
            test_callers::callers(
                None,
                Some(test_callers::staff(
                    "employee-7",
                    staff_roles(role),
                    BranchId::new(branch),
                )),
            )
        })
    }
}

#[derive(Debug)]
struct Answer {
    path: String,
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

/// `method uri` as `who`, on the server `bootstrap::configure_with_screens`
/// builds with the TD-1 table and v2's fixture pages.
async fn ask(method: Method, who: &str, uri: &str) -> Answer {
    let dist = Dist::new();
    let screens = dist.table();
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
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST));
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let path = resp.request().path().to_owned();
    let status = resp.status();
    let location = header_of(&resp, header::LOCATION);
    let content_type = header_of(&resp, header::CONTENT_TYPE).unwrap_or_default();
    let cache_control = header_of(&resp, header::CACHE_CONTROL);
    let nosniff = header_of(&resp, header::X_CONTENT_TYPE_OPTIONS).as_deref() == Some("nosniff");
    let body = atest::read_body(resp).await.to_vec();
    Answer {
        path,
        status,
        location,
        content_type,
        cache_control,
        nosniff,
        body,
    }
}

async fn get(who: &str, uri: &str) -> Answer {
    ask(Method::GET, who, uri).await
}

fn expected_cache_control(path: &str) -> &'static str {
    if matches!(path, "/desk" | "/staff/desk") {
        "no-store"
    } else {
        "no-cache"
    }
}

fn is_screen(a: &Answer, html: &str, what: &str) {
    assert_eq!(
        a.status,
        StatusCode::OK,
        "{what}: status; body {:?}",
        a.text()
    );
    assert_eq!(
        a.content_type, "text/html; charset=utf-8",
        "{what}: Content-Type"
    );
    assert_eq!(
        a.cache_control.as_deref(),
        Some(expected_cache_control(&a.path)),
        "{what}: Cache-Control"
    );
    assert!(a.nosniff, "{what}: X-Content-Type-Options: nosniff");
    assert_eq!(
        a.text(),
        html,
        "{what}: the body is that page's entry, unchanged"
    );
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

fn the_same(got: &Answer, want: &Answer, what: &str) {
    assert_eq!(
        got.status,
        want.status,
        "{what}: status; body {:?}",
        got.text()
    );
    assert_eq!(got.location, want.location, "{what}: Location");
    assert_eq!(got.content_type, want.content_type, "{what}: Content-Type");
    assert_eq!(got.body, want.body, "{what}: body");
}

// ------------------------------------------------------------ the table

#[test]
fn the_test_table_holds_one_pattern_page_and_one_literal_page() {
    let dist = Dist::new();
    let table = dist.table();
    let paths: Vec<(&str, &str)> = table
        .entries()
        .iter()
        .map(|d| (d.path.as_str(), d.entry.as_str()))
        .collect();
    assert_eq!(
        paths,
        vec![(PATTERN, PATTERN_ENTRY), ("/desk", LITERAL_ENTRY)],
        "the pattern page loads as declared, `{{id}}` and all"
    );
}

// ------------------------------------------------------------ §4.1 row 1

/// A staff session opens the pattern page at any one segment: 200, the page,
/// `Cache-Control: no-cache`, as every screen.
#[actix_web::test]
async fn the_pattern_page_is_served_at_its_address_to_a_staff_session() {
    for who in ["staff:FRONT:XC-B01", "staff:BACKEND:XC-B01"] {
        for id in [ID, "1", "XC-20261006-000001"] {
            let uri = format!("/desk/transactions/{id}");
            is_screen(
                &get(who, &uri).await,
                PATTERN_HTML,
                &format!("GET {uri} as {who}"),
            );
        }
    }
}

/// The owner naming a branch is let through the `branch` gate as on `/desk`.
#[actix_web::test]
async fn the_pattern_page_is_served_to_the_owner_naming_a_branch() {
    let uri = format!("/desk/transactions/{ID}?branch=XC-B01");
    is_screen(
        &get("owner", &uri).await,
        PATTERN_HTML,
        &format!("GET {uri} as the owner"),
    );
}

// ------------------------------------------------------------ §4.1 row 2

/// Signed out: the same 303 to `/login` as every `branch`-gated screen.
#[actix_web::test]
async fn signed_out_the_pattern_page_redirects_to_login_as_every_branch_screen() {
    let uri = format!("/desk/transactions/{ID}");
    let got = get("", &uri).await;
    assert_eq!(
        got.status,
        StatusCode::SEE_OTHER,
        "GET {uri} signed out: status; body {:?}",
        got.text()
    );
    assert_eq!(
        got.location.as_deref(),
        Some(
            "http://xc.test/login?next=%2Fdesk%2Ftransactions%2F7f0c2a4e-9b1d-4c3e-8f5a-1d2b3c4d5e6f"
        ),
        "GET {uri} signed out: Location"
    );
    let want = get("", "/desk").await;
    let what = format!("GET {uri} signed out, against the literal branch screen /desk");
    assert_eq!(
        got.status,
        want.status,
        "{what}: status; body {:?}",
        got.text()
    );
    assert_eq!(got.content_type, want.content_type, "{what}: Content-Type");
    assert_eq!(got.body, want.body, "{what}: body");
}

// ------------------------------------------------------------ §4.1 row 3

/// The owner's branch on a `branch`-gated screen is read from the query as on
/// every such screen (`pages::page_branch`): with no `?branch=` the pattern page
/// answers exactly as the literal `/desk` does, and an unreadable or invalid
/// branch is `400 Invalid branch` on both.
///
/// §4.1 writes this row as "owner session, no `?branch=` ⇒ `400 Invalid branch`".
/// At `e6e6d22c` no `branch`-gated screen answers that: `extract_branch_query`
/// (`pages.rs:580-599`) reads a missing `?branch=` as `MAIN`, so `/desk` serves
/// the page. The row's binding part, "as every `branch`-gated screen", is what
/// this test pins; the mismatch is reported on #1293.
#[actix_web::test]
async fn the_owner_s_branch_is_judged_as_on_every_branch_screen() {
    let uri = format!("/desk/transactions/{ID}");
    let literal = get("owner", "/desk").await;
    let got = get("owner", &uri).await;
    assert_eq!(
        got.status,
        literal.status,
        "GET {uri} as the owner, no ?branch=: status as /desk's; body {:?}",
        got.text()
    );
    is_screen(
        &got,
        PATTERN_HTML,
        &format!("GET {uri} as the owner, no ?branch= (read as MAIN, as /desk)"),
    );
    for query in ["?branch=%ff", "?branch=XC_B02", "?branch=bad%20name"] {
        let got = get("owner", &format!("{uri}{query}")).await;
        assert_eq!(
            got.status,
            StatusCode::BAD_REQUEST,
            "GET {uri}{query} as the owner: status; body {:?}",
            got.text()
        );
        assert_eq!(
            got.text(),
            "Invalid branch",
            "GET {uri}{query} as the owner: body"
        );
        the_same(
            &got,
            &get("owner", &format!("/desk{query}")).await,
            &format!("GET {uri}{query} as the owner, against /desk{query}"),
        );
    }
}

// ------------------------------------------------------------ §4.1 row 4

/// An address that is almost the pattern page is not one.
#[actix_web::test]
async fn an_address_that_is_almost_the_pattern_page_is_the_plain_404() {
    for uri in [
        "/desk/transactions",
        "/desk/transactions/",
        "/desk/transactions/a/b",
        "/desk/transactions//",
        "/desk/transaction/7f0c",
        "/Desk/transactions/7f0c",
        "/desk/Transactions/7f0c",
    ] {
        for who in ["staff:FRONT:XC-B01", "owner", ""] {
            is_plain_404(&get(who, uri).await, &format!("GET {uri} as {who:?}"));
        }
    }
}

// ------------------------------------------------------------ §4.1 row 5

#[actix_web::test]
async fn a_post_to_the_pattern_page_is_the_plain_404() {
    let uri = format!("/desk/transactions/{ID}");
    for who in ["staff:FRONT:XC-B01", ""] {
        is_plain_404(
            &ask(Method::POST, who, &uri).await,
            &format!("POST {uri} as {who:?}"),
        );
    }
}

// ------------------------------------------------------------ §4.1 row 6

/// The literal screen in the same table answers as before: its own page, its
/// gate, and its near-misses still the plain 404.
#[actix_web::test]
async fn the_literal_page_beside_a_pattern_page_is_unchanged() {
    is_screen(
        &get("staff:FRONT:XC-B01", "/desk").await,
        LITERAL_HTML,
        "GET /desk as staff",
    );
    is_screen(
        &get("owner", "/desk?branch=XC-B01").await,
        LITERAL_HTML,
        "GET /desk?branch=XC-B01 as the owner",
    );
    let out = get("", "/desk").await;
    assert_eq!(out.status, StatusCode::SEE_OTHER, "GET /desk signed out");
    assert_eq!(
        out.location.as_deref(),
        Some("http://xc.test/login?next=%2Fdesk"),
        "GET /desk signed out: Location"
    );
    is_screen(
        &get("owner", "/desk").await,
        LITERAL_HTML,
        "GET /desk as the owner, no ?branch= (read as MAIN)",
    );
    let owner = get("owner", "/desk?branch=%ff").await;
    assert_eq!(
        owner.status,
        StatusCode::BAD_REQUEST,
        "GET /desk?branch=%ff as owner"
    );
    assert_eq!(
        owner.text(),
        "Invalid branch",
        "GET /desk?branch=%ff as owner: body"
    );
    for uri in ["/desk/", "/Desk", "/desk/x", "/desk/transactions"] {
        is_plain_404(
            &get("staff:FRONT:XC-B01", uri).await,
            &format!("GET {uri} as staff, beside the pattern page"),
        );
    }
}

/// A `PAGE_SPECS` path the table does not hold is still v2's page.
#[actix_web::test]
async fn a_v2_page_the_table_does_not_hold_is_unchanged() {
    let a = get("owner", "/staff-guide").await;
    assert_eq!(a.status, StatusCode::OK, "GET /staff-guide as the owner");
    assert!(
        a.text().contains("guide (fixture)"),
        "v2's fixture page: {:?}",
        a.text()
    );
}

// ------------------------------------------------------------ the two pages apart

/// Each request gets its own page, never the other's.
#[actix_web::test]
async fn a_literal_page_and_a_pattern_page_in_one_table_each_answer_their_own() {
    let who = "staff:FRONT:XC-B01";
    let literal = get(who, "/desk").await;
    let pattern = get(who, &format!("/desk/transactions/{ID}")).await;
    is_screen(&literal, LITERAL_HTML, "GET /desk");
    is_screen(&pattern, PATTERN_HTML, "GET /desk/transactions/<id>");
    assert_ne!(
        literal.body, pattern.body,
        "the two addresses answer two different pages"
    );
}

// ------------------------------------------------------------ §4.1 row 7

/// `holds` stays literal: it knows the declared text, never a request that
/// the pattern would match.
#[test]
fn holds_stays_a_literal_lookup() {
    let dist = Dist::new();
    let table = dist.table();
    assert!(table.holds("/desk"), "holds(\"/desk\")");
    assert!(table.holds(PATTERN), "holds the declared text {PATTERN:?}");
    for path in [
        format!("/desk/transactions/{ID}"),
        "/desk/transactions/1".to_string(),
        "/desk/transactions".to_string(),
        "/desk/transactions/".to_string(),
        "/desk/".to_string(),
        "/Desk".to_string(),
    ] {
        assert!(!table.holds(&path), "holds({path:?}) stays false");
    }
}
