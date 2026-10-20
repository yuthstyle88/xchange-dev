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
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};
use xchange::presentation::screens::ScreenTable;

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

/// Same shape as the reader in pages_forms_test.rs: `none` | `owner` |
/// `staff:ROLE:BRANCH`, counting how often it is asked.
#[derive(Default)]
struct FakeSessions {
    asked: AtomicUsize,
}

impl SessionReader for FakeSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
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
                let role = Role::parse(parts.next().expect("a role")).expect("a role v2 knows");
                let branch = BranchId::new(parts.next().expect("a branch"));
                Actor::staff("employee-7", role, branch).expect("a staff role")
            });
            Sessions {
                owner: who.starts_with("owner").then(|| Actor::owner("owner-1")),
                staff,
            }
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
        let screens = ScreenTable::from_dist(&fixtures()).expect("empty screen table");
        test::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure_with_screens(cfg, state, screens))
                .app_data(web::Data::from(reader)),
        )
        .await
    }};
}

macro_rules! app {
    ($web_dir:expr) => {
        app_with!($web_dir, Arc::new(FakeSessions::default()))
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
    let reader = Arc::new(FakeSessions::default());
    let app = app_with!(&fixtures().join("web"), reader.clone());
    let answer = get!(&app, SCRIPT_PATH, "none");
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        reader.asked.load(Ordering::SeqCst),
        0,
        "the session reader was asked for the desk script"
    );
}

// ---------------------------------------------------------------------------------------------------------------------
// #546 (V3-desk DK-J) up-front tests by the preparer claude/dell-claude-2, on the FE lane controller claude/5edf7bb5's
// order. Do not edit, delete or weaken. Design `docs/design/xchange-v3-wave3-desk.md` §7.
//
// `web/pages/desk.html` (`xc-path /desk`, `xc-gate branch`) and `web/pages/staff-desk.html` (`xc-path /staff/desk`,
// `xc-gate front`) make the screen table hold both paths, so the server answers the v3 entry instead of v2's
// `desk.html`. The server here is `bootstrap::configure_with_screens` with a screen table read from a `dist` built in a
// temp directory from those two source entries plus an empty Vite manifest (the pattern of
// `v3_documents_screens_test.rs`), so the answer does not depend on whether `web/dist` was built.
//
// Mutations that must turn these red:
// - drop either entry, or give it another xc-path         -> dk_j_both_desk_entries_…, dk_j_the_v3_desk_…
// - serve the entries with `no-cache` (render_screen today) -> dk_j_both_desk_entries_answer_no_store
// - give /staff/desk the gate `branch` or `signed-in`     -> dk_j_a_session_that_is_not_front_…
// - give /desk the gate `anyone` or `signed-in`           -> dk_j_the_owner_on_desk_…
// ---------------------------------------------------------------------------------------------------------------------

/// The two desk entries, as committed under `web/pages/`, and the path each declares.
const DESK_ENTRIES: [(&str, &str); 2] =
    [("desk.html", "/desk"), ("staff-desk.html", "/staff/desk")];

/// A `dist` holding the two desk entries and an empty manifest, in a fresh temp directory.
fn desk_dist() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let pages = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/pages");
    let dir = std::env::temp_dir().join(format!(
        "xchange-desk-dist-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(dir.join("pages")).unwrap();
    std::fs::create_dir_all(dir.join(".vite")).unwrap();
    std::fs::write(dir.join(".vite/manifest.json"), "{}").unwrap();
    for (file, _) in DESK_ENTRIES {
        let source = pages.join(file);
        assert!(source.is_file(), "web/pages/{file} exists (#546 adds it)");
        std::fs::copy(&source, dir.join("pages").join(file)).unwrap();
    }
    dir
}

fn desk_screens() -> ScreenTable {
    let table = ScreenTable::from_dist(&desk_dist());
    assert!(
        table.is_ok(),
        "the desk entries make a screen table: {:?}",
        table.as_ref().err()
    );
    table.unwrap()
}

macro_rules! desk_app {
    () => {{
        let reader: Arc<dyn SessionReader> = Arc::new(FakeSessions::default());
        let state = state(&fixtures().join("web"));
        let screens = desk_screens();
        test::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure_with_screens(cfg, state, screens))
                .app_data(web::Data::from(reader)),
        )
        .await
    }};
}

/// The v3 entry for `path`: its own `xc-path` meta, and none of v2's desk page.
fn is_the_v3_entry(answer: &Answer, path: &str, what: &str) {
    assert_eq!(answer.status, StatusCode::OK, "{what}");
    let body = answer.text();
    assert!(
        body.contains(&format!("<meta name=\"xc-path\" content=\"{path}\">")),
        "{what}: the v3 entry for {path}, not v2's page: {body}"
    );
    assert!(
        !body.contains(SCRIPT_PATH),
        "{what}: v2's desk script is not loaded"
    );
    assert!(
        !body.contains("staffDraftId"),
        "{what}: v2's /staff/desk hook is not inserted"
    );
}

#[actix_web::test]
async fn dk_j_both_desk_entries_make_the_screen_table_hold_the_desk_paths() {
    let table = desk_screens();
    for (file, path) in DESK_ENTRIES {
        assert!(
            table.holds(path),
            "web/pages/{file} puts {path} in the screen table"
        );
    }
}

#[actix_web::test]
async fn dk_j_both_desk_entries_answer_no_store() {
    // [required] (#546 §7, design §7): the desk shows passport data, so both entries keep v2's `no-store`, where every
    // other v3 screen answers `no-cache`.
    let app = desk_app!();
    for (uri, path, who) in [
        ("/desk?branch=XC-B01", "/desk", "owner"),
        ("/desk", "/desk", "staff:FRONT:XC-B01"),
        ("/staff/desk", "/staff/desk", "staff:FRONT:XC-B01"),
    ] {
        let what = format!("GET {uri} as {who}");
        let answer = get!(&app, uri, who);
        is_the_v3_entry(&answer, path, &what);
        let cache = answer.header("cache-control").unwrap_or("");
        assert!(
            cache.split(',').any(|d| d.trim() == "no-store"),
            "{what}: Cache-Control is no-store, got {cache:?}"
        );
    }
}

#[actix_web::test]
async fn dk_j_a_session_that_is_not_front_on_staff_desk_goes_to_staff() {
    // #546 §9 row 1: anyone but a FRONT employee is redirected to /staff (`screens.rs:149-151`).
    let app = desk_app!();
    for who in ["none", "owner", "staff:BACKEND:XC-B01"] {
        let what = format!("GET /staff/desk as {who}");
        let answer = get!(&app, "/staff/desk", who);
        assert_eq!(answer.status, StatusCode::FOUND, "{what}");
        assert_eq!(
            answer.header("location"),
            Some(format!("http://{HOST}/staff").as_str()),
            "{what}"
        );
        assert!(
            !answer.text().contains("xc-path"),
            "{what}: no entry is served"
        );
    }
    // ...and a FRONT employee gets the entry (the gate is `front`, not `branch`).
    let answer = get!(&app, "/staff/desk", "staff:FRONT:XC-B01");
    is_the_v3_entry(
        &answer,
        "/staff/desk",
        "GET /staff/desk as staff:FRONT:XC-B01",
    );
}

#[actix_web::test]
async fn dk_j_the_owner_on_desk_gets_the_v3_desk_for_the_branch_asked_or_main_and_a_bad_branch_is_refused()
 {
    // #546 §9 row 2: the owner on /desk?branch=B gets the desk for B. The entry reads B from its own URL; the server
    // checks it first through the `branch` gate (`pages.rs:681-716`), as v2's /desk did.
    let app = desk_app!();
    for branch in ["XC-B01", "XC-B02"] {
        let uri = format!("/desk?branch={branch}");
        let answer = get!(&app, &uri, "owner");
        is_the_v3_entry(&answer, "/desk", &format!("GET {uri} as owner"));
    }
    // No branch asked: the branch gate defaults to MAIN, as v2's /desk did (`v2_page_cases.json` "the owner
    // asks": `/desk`, no query, owner → 200 with `XC_BRANCH="MAIN"`). Restated by the controller claude/5edf7bb5
    // on #546 (2026-10-20): the first version expected 400 here, which contradicts that recording (W9).
    let answer = get!(&app, "/desk", "owner");
    is_the_v3_entry(&answer, "/desk", "GET /desk as owner (MAIN)");
    // A malformed branch is refused before any entry is served.
    let answer = get!(&app, "/desk?branch=%22", "owner");
    assert_eq!(
        answer.status,
        StatusCode::BAD_REQUEST,
        "GET /desk?branch=%22 as owner"
    );
    assert!(
        !answer.text().contains("xc-path"),
        "GET /desk?branch=%22 as owner: no entry is served"
    );
    // Nobody signed in goes to the sign-in page, not to the entry.
    let answer = get!(&app, "/desk?branch=XC-B01", "none");
    assert_eq!(
        answer.status,
        StatusCode::SEE_OTHER,
        "GET /desk?branch=XC-B01 as none"
    );
}
