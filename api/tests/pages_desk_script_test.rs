//! The kept desk pages load v3's own `/_xc/desk.js` (row DK-1, #332;
//! `docs/design/xchange-v3.md` §5.3 *Compiled-in page scripts after FV* and
//! *The kept desk's dead ends*). Written before the code. Do not edit, delete or weaken.
//!
//! - `/desk` and `/staff/desk` carry the one tag `TAG`, as the last of the page's
//!   scripts: after `/account-bar.js` and immediately before the form helper's tag.
//! - No other page, static file, redirect or refusal carries it.
//! - `GET /_xc/desk.js` answers the script compiled into the binary from
//!   `api/static/xc-desk.js`, whatever `web.dir` holds, to anyone, and only to GET.
//!
//! What the script does in the browser is pinned by `api/tests/js/xc_desk.test.mjs`.
//!
//! Mutations that must turn this red:
//! - drop the entry from `DESK_SCRIPTS`                 -> both_desk_pages_…
//! - put it before `/account-bar.js`                    -> both_desk_pages_…
//! - give it `defer`, or insert it with the helper      -> both_desk_pages_…
//! - add it to another page                             -> no_other_page_…
//! - serve the script from `web.dir`                    -> the_desk_script_does_not_come_…
//! - serve it to POST                                   -> only_get_reaches_the_desk_script
//! - look a session up for it                           -> the_desk_script_needs_no_session

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
use xchange::presentation::test_callers;

const WHO: &str = "x-test-sessions";
const HOST: &str = "xc.test";
/// The desk script's tag: an entry of the page's script list, so no `defer`.
const TAG: &str = "<script src=\"/_xc/desk.js\"></script>";
/// The form helper's tag (row FV), which must stay right after it.
const HELPER: &str = "<script src=\"/_xc/forms.js\" defer></script>";
/// The last of v2's own desk scripts on `main` today.
const ACCOUNT_BAR: &str = "<script src=\"/account-bar.js\"></script>";
const SCRIPT_PATH: &str = "/_xc/desk.js";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pages")
}

/// The script as committed; the server must answer exactly these bytes.
fn committed_script() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("static/xc-desk.js");
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

/// Same shape as the reader in pages_forms_test.rs: `none` | `owner` |
/// `staff:ROLE:BRANCH`, counting how often it is asked.
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

/// One desk tag, after v2's account bar and immediately before the form helper.
fn has_the_desk_script_last(body: &str, what: &str) {
    assert_eq!(body.matches(TAG).count(), 1, "{what}: exactly one desk tag");
    assert!(
        body.contains(&format!("{TAG}{HELPER}")),
        "{what}: the desk tag sits immediately before the form helper's tag"
    );
    let bar = body
        .find(ACCOUNT_BAR)
        .unwrap_or_else(|| panic!("{what}: v2's account bar is still loaded"));
    let tag = body.find(TAG).unwrap();
    assert!(
        bar < tag,
        "{what}: the desk tag comes after every v2 script"
    );
}

#[actix_web::test]
async fn both_desk_pages_load_the_desk_script_last() {
    let app = app!(&fixtures().join("web"));
    for (uri, who) in [
        ("/desk", "owner"),
        ("/desk?branch=XC-B01", "owner"),
        ("/desk", "staff:FRONT:XC-B01"),
        ("/staff/desk", "staff:FRONT:XC-B01"),
    ] {
        let what = format!("GET {uri} as {who}");
        let answer = get!(&app, uri, who);
        assert_eq!(answer.status, StatusCode::OK, "{what}");
        has_the_desk_script_last(&answer.text(), &what);
    }

    // On /staff/desk it also comes after the draft hook, so its wrapper of
    // `gasCall` is the outermost one.
    let body = get!(&app, "/staff/desk", "staff:FRONT:XC-B01").text();
    let hook = body
        .find("staffDraftId")
        .expect("the /staff/desk draft hook is still inserted");
    assert!(
        hook < body.find(TAG).unwrap(),
        "the hook runs before the desk script"
    );
}

#[actix_web::test]
async fn the_staff_draft_hook_begins_the_draft_through_draft_begin() {
    // #3008: CA-10 retires `opsBeginTransaction`, so the /staff/desk hook begins its draft through v3 `draftBegin`
    // (`POST /api/tx-drafts`, `http/v3/commands/branch_day.rs:404-478`): the branch the page was given
    // (`window.XC_BRANCH`), the side, currency and amount, and an Idempotency-Key. The answer's `id` is the draft id.
    let app = app!(&fixtures().join("web"));
    let body = get!(&app, "/staff/desk", "staff:FRONT:XC-B01").text();
    let at = body
        .find("staffDraftId")
        .expect("the /staff/desk draft hook is inserted");
    let start = body[..at]
        .rfind("<script>")
        .expect("the hook is a <script> element");
    let end = at
        + body[at..]
            .find("</script>")
            .expect("the hook's script ends");
    let hook = &body[start..end];
    assert!(
        !hook.contains("opsBeginTransaction"),
        "the hook no longer calls opsBeginTransaction: {hook}"
    );
    for needle in [
        "/api/tx-drafts",
        "idempotency-key",
        "XC_BRANCH",
        "side:",
        "currency:",
        "amount:",
        "staffDraftId",
    ] {
        assert!(
            hook.contains(needle),
            "the hook's draftBegin call carries {needle:?}: {hook}"
        );
    }
}

#[actix_web::test]
async fn no_other_page_loads_the_desk_script() {
    let app = app!(&fixtures().join("web"));
    for (path, who) in [
        ("/", "owner"),
        ("/receipt-sample", "owner"),
        ("/staff", "owner"),
        ("/staff/report", "none"),
        ("/devices", "owner"),
        ("/login", "none"),
        ("/kiosk", "none"),
        ("/kiosk-queue.html", "owner"),
        ("/kiosk-currencies.html", "owner"),
        ("/staff-guide", "owner"),
        ("/staff-guide.html", "owner"),
    ] {
        let answer = get!(&app, path, who);
        assert_eq!(answer.status, StatusCode::OK, "GET {path} as {who}");
        assert!(
            !answer.text().contains(SCRIPT_PATH),
            "GET {path} as {who} loads the desk script"
        );
    }

    for (method, uri, who, status) in [
        (Method::GET, "/staff/desk", "none", StatusCode::FOUND),
        (
            Method::GET,
            "/staff/desk",
            "staff:BACKEND:XC-B01",
            StatusCode::FOUND,
        ),
        (
            Method::GET,
            "/desk?branch=%22",
            "owner",
            StatusCode::BAD_REQUEST,
        ),
        (Method::GET, "/app.js", "owner", StatusCode::OK),
        (Method::POST, "/desk", "owner", StatusCode::NOT_FOUND),
    ] {
        let answer = ask(&app, req(method.clone(), uri, who).to_request()).await;
        assert_eq!(answer.status, status, "{method} {uri} as {who}");
        assert!(
            !answer.text().contains(SCRIPT_PATH),
            "{method} {uri} as {who}: {}",
            answer.text()
        );
    }
}

#[actix_web::test]
async fn the_desk_script_is_the_one_compiled_in() {
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
        "the answer is api/static/xc-desk.js, byte for byte"
    );
    let again = get!(&app, &format!("{SCRIPT_PATH}?v=3"), "owner");
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.body, answer.body);
}

#[actix_web::test]
async fn the_desk_script_does_not_come_from_the_web_directory() {
    let dir = std::env::temp_dir().join(format!("xc-desk-shadow-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("_xc")).unwrap();
    std::fs::write(dir.join("_xc/desk.js"), "shadow();").unwrap();
    let app = app!(&dir);
    let answer = get!(&app, SCRIPT_PATH, "none");
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer.body,
        committed_script(),
        "the web directory's copy won"
    );

    let missing = std::env::temp_dir().join(format!("xc-desk-none-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    let app = app!(&missing);
    let answer = get!(&app, SCRIPT_PATH, "none");
    assert_eq!(answer.status, StatusCode::OK, "no web directory");
    assert_eq!(answer.body, committed_script());
}

#[actix_web::test]
async fn only_get_reaches_the_desk_script() {
    let app = app!(&fixtures().join("web"));
    // GET first, so this test cannot pass while the route is missing.
    assert_eq!(get!(&app, SCRIPT_PATH, "owner").status, StatusCode::OK);
    for method in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
        let answer = ask(&app, req(method.clone(), SCRIPT_PATH, "owner").to_request()).await;
        assert_eq!(answer.status, StatusCode::NOT_FOUND, "{method}");
        assert_eq!(answer.text(), "Not found", "{method}: the plain 404");
    }
}

#[actix_web::test]
async fn the_desk_script_needs_no_session() {
    let reader = Arc::new(FakeCallers::default());
    let app = app_with!(&fixtures().join("web"), reader.clone());
    let answer = get!(&app, SCRIPT_PATH, "none");
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        reader.asked.load(Ordering::SeqCst),
        0,
        "the session reader was asked for the desk script"
    );
}
