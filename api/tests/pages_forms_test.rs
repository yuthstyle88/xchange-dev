//! Every page loads v3's own form helper (row FV, `docs/design/xchange-v3.md` §5.3
//! *The shared form helper*). Written before the code. Do not edit, delete or weaken.
//!
//! - Every HTML page the server answers carries the one tag `HELPER`, immediately
//!   before the version line of #293, on both page paths (plain and `/staff/desk`).
//! - Static files, redirects, refusals and 404s carry none.
//! - `GET /_xc/forms.js` answers the script compiled into the binary from
//!   `api/static/xc-forms.js`, whatever `web.dir` holds, to anyone, and only to GET.
//!
//! What the script does in the browser is pinned by `api/tests/js/xc_forms.test.mjs`.
//!
//! Mutations that must turn this red:
//! - drop the tag on the desk-variant path            -> every_page_…
//! - drop the tag on the plain page path              -> every_page_…
//! - add the tag twice / after the version line       -> every_page_… / …_body_end_…
//! - add the tag to a static file or a redirect       -> static_files_redirects_…
//! - serve the script from `web.dir`                  -> the_script_does_not_come_from_…
//! - serve it to POST                                 -> only_get_reaches_the_script
//! - look a session up for it                         -> the_script_needs_no_session

mod common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test, web};
use common::{FakeDatabase, settings, state_with};
use xc_auth::domain::access::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::screens::ScreenTable;
use xchange::presentation::test_callers;

const WHO: &str = "x-test-sessions";
const HOST: &str = "xc.test";
/// The one tag every page carries (§5.3 *The shared form helper*, [required]).
const HELPER: &str = "<script src=\"/_xc/forms.js\" defer></script>";
/// The start of the version line (#293).
const VERSION: &str = "<div id=\"xc-version\"";
const SCRIPT_PATH: &str = "/_xc/forms.js";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pages")
}

/// The script as committed; the server must answer exactly these bytes.
fn committed_script() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("static/xc-forms.js");
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn roles_of(role: &str) -> &'static [Role] {
    match role {
        "FRONT" => &[Role::Teller],
        "BACKEND" | "MANAGER" => &[Role::BranchManager],
        "RATES" => &[Role::RateManager],
        "FINANCE" => &[Role::Finance],
        "SYSTEM" => &[Role::Admin],
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => &[Role::VerificationReviewer],
        other => panic!("unknown v2 role: {other}"),
    }
}

/// Same shape as the reader in pages_test.rs: `none` | `owner` | `staff:ROLE:BRANCH`,
/// counting how often it is asked.
#[derive(Default)]
struct FakeCallers {
    asked: AtomicUsize,
}

impl SessionReader for FakeCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let who = req
            .headers()
            .get(WHO)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("none")
            .to_owned();
        Box::pin(async move {
            let staff = who.find("staff:").map(|at| {
                let mut parts = who[at..].split(':').skip(1);
                let roles = roles_of(parts.next().expect("a role"));
                let branch = BranchId::new(parts.next().expect("a branch"));
                test_callers::staff("employee-7", roles, branch)
            });
            test_callers::callers(
                who.starts_with("owner")
                    .then(|| test_callers::owner("owner-1")),
                staff,
            )
        })
    }
}

fn state(web_dir: &Path) -> AppState {
    state_with(
        settings(&[("APP_WEB__DIR", web_dir.to_str().expect("a UTF-8 path"))]),
        FakeDatabase::Up,
    )
}

macro_rules! app_with {
    ($web_dir:expr, $reader:expr) => {{
        let reader: Arc<dyn SessionReader> = $reader;
        let state = state($web_dir);
        test::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await
    }};
}

macro_rules! app {
    ($web_dir:expr) => {
        app_with!($web_dir, Arc::new(FakeCallers::default()))
    };
}

fn req(method: Method, uri: &str, who: &str) -> test::TestRequest {
    test::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((WHO, who))
}

struct Answer {
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

async fn ask<S, R, B>(app: &S, req: R) -> Answer
where
    S: Service<R, Response = ServiceResponse<B>, Error = actix_web::Error>,
    B: MessageBody,
{
    let resp = test::call_service(app, req).await;
    let status = resp.status();
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_ascii_lowercase(),
                v.to_str().expect("a text header value").to_owned(),
            )
        })
        .collect();
    let body = test::read_body(resp).await.to_vec();
    Answer {
        status,
        headers,
        body,
    }
}

macro_rules! get {
    ($app:expr, $uri:expr, $who:expr) => {
        ask($app, req(Method::GET, $uri, $who).to_request()).await
    };
}

/// One helper tag, and it sits right before the one version line.
fn has_the_helper_once(body: &str, what: &str) {
    assert_eq!(
        body.matches(HELPER).count(),
        1,
        "{what}: exactly one form helper tag"
    );
    assert_eq!(
        body.matches(VERSION).count(),
        1,
        "{what}: exactly one version line"
    );
    assert!(
        body.contains(&format!("{HELPER}{VERSION}")),
        "{what}: the helper tag sits immediately before the version line"
    );
}

/// Every page address, and who must ask for it to get the page.
const PAGES: [(&str, &str); 13] = [
    ("/", "owner"),
    ("/desk", "owner"),
    ("/receipt-sample", "owner"),
    ("/staff", "owner"),
    ("/staff/report", "none"),
    ("/staff/desk", "staff:FRONT:XC-B01"),
    ("/devices", "owner"),
    ("/login", "none"),
    ("/kiosk", "none"),
    ("/kiosk-queue.html", "owner"),
    ("/kiosk-currencies.html", "owner"),
    ("/staff-guide", "owner"),
    ("/staff-guide.html", "owner"),
];

#[actix_web::test]
async fn every_page_carries_the_helper_once_right_before_the_version_line() {
    let app = app!(&fixtures().join("web"));
    for (path, who) in PAGES {
        let what = format!("GET {path} as {who}");
        let answer = get!(&app, path, who);
        assert_eq!(answer.status, StatusCode::OK, "{what}");
        assert!(
            answer
                .header("content-type")
                .is_some_and(|c| c.starts_with("text/html")),
            "{what}: an HTML page"
        );
        if ScreenTable::compiled().holds(path) {
            assert!(
                !answer.text().contains(HELPER),
                "{what}: a v3 screen carries no form helper tag"
            );
        } else {
            has_the_helper_once(&answer.text(), &what);
        }
    }
    // a staff member on a branch page, and the owner naming a branch
    for (uri, who) in [
        ("/", "staff:MANAGER:XC-B01"),
        ("/desk?branch=XC-B01", "owner"),
    ] {
        let path = uri.split('?').next().unwrap();
        let answer = get!(&app, uri, who);
        assert_eq!(answer.status, StatusCode::OK, "GET {uri} as {who}");
        if ScreenTable::compiled().holds(path) {
            assert!(
                !answer.text().contains(HELPER),
                "GET {uri} as {who}: a v3 screen carries no form helper tag"
            );
        } else {
            has_the_helper_once(&answer.text(), &format!("GET {uri} as {who}"));
        }
    }
}

#[actix_web::test]
async fn a_page_without_a_body_end_gets_the_helper_once() {
    let held = ScreenTable::compiled().holds("/login");
    let dir = std::env::temp_dir().join(format!("xc-forms-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("login.html"), "<html><head></head><p>no end tags").unwrap();
    let app = app!(&dir);
    let answer = get!(&app, "/login", "none");
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(answer.status, StatusCode::OK);
    let body = answer.text();
    if held {
        assert!(
            !body.contains(HELPER),
            "a v3 screen carries no form helper tag"
        );
    } else {
        has_the_helper_once(&body, "a page with no </body>");
        assert!(body.trim_end().ends_with("</div>"), "{body}");
    }
}

#[actix_web::test]
async fn static_files_redirects_and_refusals_carry_no_helper() {
    let dir = fixtures().join("web");
    let app = app!(&dir);

    for file in ["style.css", "app.js", "data.json"] {
        let answer = get!(&app, &format!("/{file}"), "owner");
        assert_eq!(answer.status, StatusCode::OK, "/{file}");
        let on_disk = std::fs::read(dir.join(file)).unwrap();
        assert_eq!(answer.body, on_disk, "/{file} is served as it is on disk");
    }

    for (method, uri, who, status) in [
        (Method::GET, "/staff/desk", "none", StatusCode::FOUND),
        (Method::GET, "/devices", "none", StatusCode::SEE_OTHER),
        (
            Method::GET,
            "/?branch=%22",
            "owner",
            StatusCode::BAD_REQUEST,
        ),
        (Method::GET, "/missing.js", "none", StatusCode::NOT_FOUND),
        (Method::GET, "/login.html", "none", StatusCode::NOT_FOUND),
        (Method::POST, "/login", "none", StatusCode::NOT_FOUND),
    ] {
        let what = format!("{method} {uri} as {who}");
        let answer = ask(&app, req(method, uri, who).to_request()).await;
        assert_eq!(answer.status, status, "{what}");
        assert!(!answer.text().contains(HELPER), "{what}: {}", answer.text());
        assert!(
            !answer.text().contains(SCRIPT_PATH),
            "{what}: {}",
            answer.text()
        );
    }
}

#[actix_web::test]
async fn the_script_is_the_one_compiled_in() {
    let app = app!(&fixtures().join("web"));
    let answer = get!(&app, SCRIPT_PATH, "none");
    assert_eq!(answer.status, StatusCode::OK, "GET {SCRIPT_PATH}");
    assert_eq!(
        answer.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(answer.header("cache-control"), Some("no-cache"));
    assert_eq!(answer.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(
        answer.body,
        committed_script(),
        "the answer is api/static/xc-forms.js, byte for byte"
    );
    // a query changes nothing
    let again = get!(&app, &format!("{SCRIPT_PATH}?v=3"), "owner");
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.body, answer.body);
}

#[actix_web::test]
async fn the_script_does_not_come_from_the_web_directory() {
    // a web directory that holds a file of the same name
    let dir = std::env::temp_dir().join(format!("xc-forms-shadow-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("_xc")).unwrap();
    std::fs::write(dir.join("_xc/forms.js"), "shadow();").unwrap();
    let app = app!(&dir);
    let answer = get!(&app, SCRIPT_PATH, "none");
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer.body,
        committed_script(),
        "the web directory's copy won"
    );

    // and a web directory that does not exist at all
    let missing = std::env::temp_dir().join(format!("xc-forms-none-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    let app = app!(&missing);
    let answer = get!(&app, SCRIPT_PATH, "none");
    assert_eq!(answer.status, StatusCode::OK, "no web directory");
    assert_eq!(answer.body, committed_script());
}

#[actix_web::test]
async fn only_get_reaches_the_script() {
    let app = app!(&fixtures().join("web"));
    for method in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
        let answer = ask(&app, req(method.clone(), SCRIPT_PATH, "owner").to_request()).await;
        assert_eq!(answer.status, StatusCode::NOT_FOUND, "{method}");
        assert_eq!(answer.text(), "Not found", "{method}: the plain 404");
    }
}

#[actix_web::test]
async fn the_script_needs_no_session() {
    // `/login` loads it before anyone is signed in
    let reader = Arc::new(FakeCallers::default());
    let app = app_with!(&fixtures().join("web"), reader.clone());
    let answer = get!(&app, SCRIPT_PATH, "none");
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        reader.asked.load(Ordering::SeqCst),
        0,
        "the session reader was asked for the form helper"
    );
}
