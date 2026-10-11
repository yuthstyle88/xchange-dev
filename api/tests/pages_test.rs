//! Specification tests for T1c: the pages and the static files.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! - The server reads its pages and its static files from the directory the
//!   setting `web.dir` names, at request time. Nothing of them is compiled in.
//! - `presentation::pages::configure` registers the page routes;
//!   `presentation::pages::fallback` serves a static file before the plain 404.
//! - Who is signed in comes from a `pages::SessionReader` registered as
//!   `web::Data<dyn SessionReader>`. It hands out `xc_auth`'s `Actor` — one for
//!   the owner session, one for the staff session, either or both absent. When
//!   no reader is registered nobody is signed in.
//!
//! `fixtures/pages/v2_page_cases.json` was recorded by running v2's own
//! page-building code over the small pages under `fixtures/pages/web` and
//! `fixtures/pages/web-bare` (`api/tools/v2-fixtures/gen_pages.mjs`). Those
//! pages are this repository's own; none is v2's.
//!
//! No test here needs a database.

mod common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, HttpResponse, test, web};
use common::{FakeDatabase, settings, state_with};
use serde_json::Value;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{self, SessionReader, Sessions};
use xchange::presentation::screens::ScreenTable;
use xchange::presentation::ui::UiAssets;

// ------------------------------------------------------------------ helpers

/// The request header the fake session reader looks at:
/// `none` | `owner` | `staff:ROLE:BRANCH` | `owner+staff:ROLE:BRANCH`.
const WHO: &str = "x-test-sessions";
/// Every request names this host; redirects are built from it.
const HOST: &str = "xc.test";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pages")
}

fn cases() -> Value {
    let text = std::fs::read_to_string(fixtures().join("v2_page_cases.json"))
        .expect("read v2_page_cases.json");
    serde_json::from_str(&text).expect("v2_page_cases.json is JSON")
}

/// Answers from the `x-test-sessions` header and counts how often it is asked.
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

/// The whole application as `bootstrap::configure` builds it, with a fake
/// session reader in front.
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
        app_with!($web_dir, Arc::new(FakeSessions::default()))
    };
}

fn request(method: Method, uri: &str, who: &str) -> test::TestRequest {
    test::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((WHO, who))
}

fn get(uri: &str, who: &str) -> test::TestRequest {
    request(Method::GET, uri, who)
}

/// What a response was: status, headers (lower-case names), body bytes.
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

    /// v2's last answer: 404, plain text, `Not found`.
    fn is_the_plain_404(&self, what: &str) {
        assert_eq!(self.status, StatusCode::NOT_FOUND, "{what}");
        let content_type = self
            .header("content-type")
            .unwrap_or_else(|| panic!("{what}: a Content-Type header"))
            .to_ascii_lowercase();
        assert!(
            content_type.starts_with("text/plain"),
            "{what}: {content_type}"
        );
        assert_eq!(
            String::from_utf8_lossy(&self.body),
            "Not found",
            "{what}: the body"
        );
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

/// The four headers v2 sets on pages and files. A response must carry exactly
/// those of them the fixture lists, with those values.
const WATCHED: [&str; 4] = [
    "content-type",
    "cache-control",
    "x-content-type-options",
    "referrer-policy",
];

fn headers_are(answer: &Answer, expected: &Value, what: &str) {
    let expected = expected.as_object().expect("headers");
    for name in WATCHED {
        let want = expected.get(name).map(|v| v.as_str().expect("a string"));
        assert_eq!(answer.header(name), want, "{what}: header {name}");
    }
    for name in expected.keys() {
        assert!(
            WATCHED.contains(&name.as_str()),
            "the fixture lists a header this test does not watch: {name}"
        );
    }
}

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A directory of the test's own under the system's temporary directory,
/// removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "xc-t1c-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create a scratch directory");
        // the real path: on macOS the temporary directory sits behind a symlink
        Scratch(dir.canonicalize().expect("the scratch directory exists"))
    }

    fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("create directories");
        std::fs::write(&path, content).expect("write a scratch file");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The page addresses and whether a visitor must be signed in.
const PAGES: [(&str, bool); 13] = [
    ("/", true),
    ("/desk", true),
    ("/receipt-sample", true),
    ("/staff", true),
    ("/staff/report", false),
    ("/staff/desk", false),
    ("/devices", true),
    ("/login", false),
    ("/kiosk", false),
    ("/kiosk-queue.html", true),
    ("/kiosk-currencies.html", true),
    ("/staff-guide", true),
    ("/staff-guide.html", true),
];

// ----------------------------------------------------------------- settings

#[actix_web::test]
async fn the_web_directory_is_a_setting() {
    // the committed default, relative to where the server is started
    assert_eq!(settings(&[]).web.dir, "web");
    // and the environment replaces it, as it does every other setting
    assert_eq!(
        settings(&[("APP_WEB__DIR", "/srv/xchange/web")]).web.dir,
        "/srv/xchange/web"
    );
}

// -------------------------------------------------------------------- pages

/// The body without what the server adds to a page: the form helper's tag (row FV)
/// and, immediately after it, the one `<div id="xc-version" …>…</div>` (#293);
/// `is_page` says whether exactly one of each must be there, or none.
///
/// Also stripped: the run of v3's compiled-in page scripts that ends immediately
/// before the helper's tag — tags of the exact form `<script src="/_xc/…"></script>`
/// (rows DK-1, SP-1; `docs/design/xchange-v3.md` §5.3 *Compiled-in page scripts
/// after FV*). Nothing else. Which page carries which of them is pinned by each
/// script's own page test (`pages_desk_script_test.rs`, `pages_staff_password_test.rs`).
fn without_version_line(body: &str, is_page: bool, what: &str) -> String {
    const MARK: &str = "<div id=\"xc-version\"";
    const HELPER: &str = "<script src=\"/_xc/forms.js\" defer></script>";
    let count = body.matches(MARK).count();
    let helpers = body.matches(HELPER).count();
    if !is_page {
        assert_eq!(count, 0, "{what}: a static file carries no version line");
        assert_eq!(helpers, 0, "{what}: a static file carries no form helper");
        return body.to_owned();
    }
    assert_eq!(count, 1, "{what}: exactly one version line");
    assert_eq!(helpers, 1, "{what}: exactly one form helper tag");
    let line = body.find(MARK).unwrap();
    assert!(
        body[..line].ends_with(HELPER),
        "{what}: the form helper tag sits immediately before the version line"
    );
    let mut start = line - HELPER.len();
    while let Some(tag) = compiled_script_tag_ending_at(body, start) {
        start -= tag;
    }
    let end = line + body[line..].find("</div>").expect("the line is closed") + "</div>".len();
    format!("{}{}", &body[..start], &body[end..])
}

/// The length of the tag `<script src="/_xc/NAME"></script>` that ends exactly at
/// `end`, where NAME holds no quote and no angle bracket; `None` when another text
/// ends there.
fn compiled_script_tag_ending_at(body: &str, end: usize) -> Option<usize> {
    const OPEN: &str = "<script src=\"/_xc/";
    const CLOSE: &str = "\"></script>";
    let before = body[..end].strip_suffix(CLOSE)?;
    let open = before.rfind(OPEN)?;
    let name = &before[open + OPEN.len()..];
    if name.is_empty() || name.contains(['"', '<', '>']) {
        return None;
    }
    Some(end - open)
}

#[actix_web::test]
async fn every_page_answers_as_v2_does() {
    // S-3 (docs/design/xchange-v3-wave3-staff.md §8.1 row S-3: the staff page is signed-in).
    const V3_DIFFERS_FROM_V2: [(&str, &str, &str); 1] = [("web", "/staff", "none")];

    let cases = cases();
    let pages = cases["pages"].as_array().expect("pages");
    // the recording is whole: 94 answers from `web`, 3 from `web-bare`
    assert_eq!(pages.len(), 97, "v2_page_cases.json was cut short");
    let mut seen = [0usize; 4]; // 200, 400, 303, 302

    for dir in ["web", "web-bare"] {
        let app = app!(&fixtures().join(dir));
        for case in pages.iter().filter(|c| c["dir"] == dir) {
            let path = case["path"].as_str().unwrap();
            let query = case["query"].as_str().unwrap();
            let who = case["visitor"].as_str().unwrap();
            let uri = if query.is_empty() {
                path.to_owned()
            } else {
                format!("{path}?{query}")
            };
            let what = format!(
                "{dir}: GET {uri} as {who} ({})",
                case["label"].as_str().unwrap()
            );
            let answer = ask(&app, get(&uri, who).to_request()).await;
            if V3_DIFFERS_FROM_V2.contains(&(dir, path, who)) {
                assert_eq!(answer.status.as_u16(), 303, "{what}: status");
                assert_eq!(
                    answer.header("location"),
                    Some(format!("http://{HOST}/login").as_str()),
                    "{what}: Location"
                );
                seen[2] += 1;
                continue;
            }
            let status = case["status"].as_u64().unwrap() as u16;
            assert_eq!(answer.status.as_u16(), status, "{what}: status");
            match status {
                200 => {
                    seen[0] += 1;
                    if ScreenTable::compiled().holds(path) {
                        assert_eq!(
                            answer.header("cache-control"),
                            Some("no-cache"),
                            "{what}: cache-control"
                        );
                        assert!(
                            answer
                                .header("content-type")
                                .is_some_and(|c| c.starts_with("text/html")),
                            "{what}: content-type"
                        );
                        assert_eq!(
                            answer.header("x-content-type-options"),
                            Some("nosniff"),
                            "{what}: nosniff"
                        );
                        let decl = ScreenTable::compiled()
                            .entries()
                            .iter()
                            .find(|d| d.path == path)
                            .unwrap()
                            .clone();
                        let entry = UiAssets::get(&decl.entry).unwrap_or_else(|| {
                            panic!("{what}: entry {} missing in UiAssets", decl.entry)
                        });
                        assert_eq!(answer.body, entry.data.as_ref(), "{what}: body");
                    } else {
                        headers_are(&answer, &case["headers"], &what);
                        // #293: every page (every 200 recorded here is an HTML page,
                        // `/kiosk-queue.html` included) carries one version line v2
                        // did not have.
                        let is_page = answer
                            .header("content-type")
                            .is_some_and(|c| c.starts_with("text/html"));
                        let body = String::from_utf8_lossy(&answer.body);
                        let body = without_version_line(&body, is_page, &what);
                        assert_eq!(body, case["body"].as_str().unwrap(), "{what}: body");
                    }
                }
                400 => {
                    seen[1] += 1;
                    assert_eq!(
                        String::from_utf8_lossy(&answer.body),
                        "Invalid branch",
                        "{what}"
                    );
                    let content_type = answer
                        .header("content-type")
                        .expect("a Content-Type")
                        .to_ascii_lowercase();
                    assert!(content_type.starts_with("text/plain"), "{what}");
                }
                303 | 302 => {
                    seen[if status == 303 { 2 } else { 3 }] += 1;
                    let to = case["location"].as_str().unwrap();
                    assert_eq!(
                        answer.header("location"),
                        Some(format!("http://{HOST}{to}").as_str()),
                        "{what}: Location"
                    );
                }
                other => {
                    panic!("{what}: the fixture has a status this test does not know: {other}")
                }
            }
        }
    }
    // one /staff case moved 200 → 303 by S-3
    assert_eq!(seen, [63, 15, 12, 7], "200 / 400 / 303 / 302");
}

/// (X-Forwarded-Proto, Host, further headers, the origin a redirect must name)
type Forwarded = (
    Option<&'static str>,
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static str,
);

#[actix_web::test]
async fn a_redirect_names_the_host_the_request_named() {
    let app = app!(&fixtures().join("web"));
    let table: [Forwarded; 7] = [
        (None, "desk.example", &[], "http://desk.example"),
        (Some("https"), "desk.example", &[], "https://desk.example"),
        (Some("http"), "desk.example", &[], "http://desk.example"),
        // only the exact word `https` means https
        (Some("HTTPS"), "desk.example", &[], "http://desk.example"),
        (
            Some("https, http"),
            "desk.example",
            &[],
            "http://desk.example",
        ),
        // a port is part of the host
        (
            Some("https"),
            "desk.example:8443",
            &[],
            "https://desk.example:8443",
        ),
        // no other header moves the visitor to another host
        (
            Some("https"),
            "desk.example",
            &[
                ("x-forwarded-host", "evil.example"),
                ("forwarded", "host=evil.example;proto=http"),
            ],
            "https://desk.example",
        ),
    ];
    for (proto, host, more, origin) in table {
        for (uri, who, status, to) in [
            ("/desk", "none", 303, "/login"),
            ("/?branch=XC-B02", "none", 303, "/login"),
            ("/staff/desk", "none", 302, "/staff"),
            ("/staff/desk?x=1", "staff:FINANCE:XC-B01", 302, "/staff"),
        ] {
            let mut req = test::TestRequest::get()
                .uri(uri)
                .insert_header((header::HOST, host))
                .insert_header((WHO, who));
            if let Some(proto) = proto {
                req = req.insert_header(("x-forwarded-proto", proto));
            }
            for (name, value) in more {
                req = req.insert_header((*name, *value));
            }
            let answer = ask(&app, req.to_request()).await;
            let what = format!("GET {uri} as {who}, proto {proto:?}, host {host}, {more:?}");
            assert_eq!(answer.status.as_u16(), status, "{what}");
            assert_eq!(
                answer.header("location"),
                Some(format!("{origin}{to}").as_str()),
                "{what}"
            );
        }
    }
}

#[actix_web::test]
async fn only_get_reaches_a_page() {
    let app = app!(&fixtures().join("web"));
    for (path, _) in PAGES {
        for method in [
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::HEAD,
            Method::OPTIONS,
        ] {
            for who in ["owner+staff:FRONT:XC-B01", "none"] {
                let what = format!("{method} {path} as {who}");
                let answer = ask(&app, request(method.clone(), path, who).to_request()).await;
                if method == Method::HEAD {
                    // a HEAD response carries no body to compare
                    assert_eq!(answer.status, StatusCode::NOT_FOUND, "{what}");
                } else {
                    answer.is_the_plain_404(&what);
                }
            }
        }
    }
}

#[actix_web::test]
async fn an_address_that_is_almost_a_page_is_not_one() {
    let app = app!(&fixtures().join("web"));
    for path in [
        "/desk/",
        "/Desk",
        "/DESK",
        "/staff/",
        "/staff/report/",
        "/staff/desk/",
        "/staff/Desk",
        "/login/",
        "/kiosk/",
        "/devices/",
        "/receipt-sample/",
        "/kiosk-queue",
        "/kiosk-currencies",
        "/kiosk-queue.htm",
        "/staff-guide.htm",
        "/staff-guide/",
        "/staff-guide.html/",
        "/index.html",
        "/admin",
        "/staff/login",
        "/staff/report/x",
        "/desk/x",
    ] {
        for who in ["owner+staff:FRONT:XC-B01", "none"] {
            ask(&app, get(path, who).to_request())
                .await
                .is_the_plain_404(&format!("GET {path} as {who}"));
        }
    }
}

#[actix_web::test]
async fn without_a_session_reader_nobody_is_signed_in() {
    // T4c registers the real reader. Until then, and in any application that
    // registers none, the pages behave as for a visitor with no session — the
    // header below must change nothing.
    let web_dir = fixtures().join("web");
    let state = state(&web_dir);
    let app =
        test::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state))).await;

    for (path, signed_in) in PAGES {
        let answer = ask(&app, get(path, "owner+staff:FRONT:XC-B01").to_request()).await;
        let what = format!("GET {path}");
        if path == "/staff/desk" {
            assert_eq!(answer.status.as_u16(), 302, "{what}");
            assert_eq!(
                answer.header("location"),
                Some(format!("http://{HOST}/staff").as_str()),
                "{what}"
            );
        } else if signed_in {
            assert_eq!(answer.status.as_u16(), 303, "{what}");
            assert_eq!(
                answer.header("location"),
                Some(format!("http://{HOST}/login").as_str()),
                "{what}"
            );
        } else {
            assert_eq!(answer.status, StatusCode::OK, "{what}");
        }
    }
    let answer = ask(&app, get("/app.js", "none").to_request()).await;
    assert_eq!(answer.status, StatusCode::OK);
}

#[actix_web::test]
async fn a_page_whose_file_is_missing_is_the_plain_404_after_the_sign_in_check() {
    // `web-bare` holds admin.html and desk.html only
    let app = app!(&fixtures().join("web-bare"));
    for (path, signed_in) in PAGES {
        if ["/", "/desk", "/staff/desk"].contains(&path) {
            continue;
        }
        let held = ScreenTable::compiled().holds(path);
        let answer = ask(&app, get(path, "owner").to_request()).await;
        if held {
            assert_eq!(answer.status, StatusCode::OK, "GET {path} as the owner");
            assert_eq!(answer.header("cache-control"), Some("no-cache"));
        } else {
            answer.is_the_plain_404(&format!("GET {path} as the owner"));
        }
        let answer = ask(&app, get(path, "none").to_request()).await;
        if signed_in {
            assert_eq!(answer.status.as_u16(), 303, "GET {path} as nobody");
        } else if held {
            assert_eq!(answer.status, StatusCode::OK, "GET {path} as nobody");
            assert_eq!(answer.header("cache-control"), Some("no-cache"));
        } else {
            answer.is_the_plain_404(&format!("GET {path} as nobody"));
        }
    }

    // a directory that is not there at all: the same, and the server still runs
    let nowhere = fixtures().join("no-such-directory");
    let app = app!(&nowhere);
    for (path, signed_in) in PAGES {
        if path == "/staff/desk" {
            continue;
        }
        let held = ScreenTable::compiled().holds(path);
        let answer = ask(&app, get(path, "owner").to_request()).await;
        if held {
            assert_eq!(
                answer.status,
                StatusCode::OK,
                "no directory: GET {path} as the owner"
            );
            assert_eq!(answer.header("cache-control"), Some("no-cache"));
        } else {
            answer.is_the_plain_404(&format!("no directory: GET {path} as the owner"));
        }
        if signed_in {
            let answer = ask(&app, get(path, "none").to_request()).await;
            assert_eq!(answer.status.as_u16(), 303, "no directory: GET {path}");
        } else if held {
            let answer = ask(&app, get(path, "none").to_request()).await;
            assert_eq!(
                answer.status,
                StatusCode::OK,
                "no directory: GET {path} as nobody"
            );
            assert_eq!(answer.header("cache-control"), Some("no-cache"));
        }
    }
    ask(&app, get("/staff/desk", "staff:FRONT:XC-B01").to_request())
        .await
        .is_the_plain_404("no directory: the desk variant");
    let answer = ask(&app, get("/staff/desk", "owner").to_request()).await;
    assert_eq!(answer.status.as_u16(), 302, "no directory: /staff/desk");
    ask(&app, get("/app.js", "none").to_request())
        .await
        .is_the_plain_404("no directory: a static file");
}

#[actix_web::test]
async fn pages_and_files_are_read_when_they_are_asked_for() {
    // the directory is written at deploy time, possibly while the server runs
    let scratch = Scratch::new("live");
    let app = app!(&scratch.0);
    let login_held = ScreenTable::compiled().holds("/login");
    // a v2 branch page read from the directory: `/` until V2-switch (#472) hands it to the
    // v3 admin screen, then `/desk` (`pages.rs` `PAGE_SPECS`)
    let (live_path, live_file) = if ScreenTable::compiled().holds("/") {
        ("/desk", "desk.html")
    } else {
        ("/", "admin.html")
    };

    if login_held {
        let answer = ask(&app, get("/login", "none").to_request()).await;
        assert_eq!(answer.status, StatusCode::OK);
        assert_eq!(answer.header("cache-control"), Some("no-cache"));
    } else {
        ask(&app, get("/login", "none").to_request())
            .await
            .is_the_plain_404("before the page exists");
    }
    ask(&app, get("/late.js", "none").to_request())
        .await
        .is_the_plain_404("before the file exists");

    scratch.write("login.html", "<html><body>first</body></html>");
    scratch.write(live_file, "<html><head></head><body>first</body></html>");
    scratch.write("late.js", "first();");
    if login_held {
        let answer = ask(&app, get("/login", "none").to_request()).await;
        assert_eq!(answer.status, StatusCode::OK);
        assert_eq!(answer.header("cache-control"), Some("no-cache"));
    } else {
        let answer = ask(&app, get("/login", "none").to_request()).await;
        assert_eq!(
            without_version_line(&String::from_utf8_lossy(&answer.body), true, "/login"),
            "<html><body>first</body></html>"
        );
    }
    let answer = ask(&app, get("/late.js", "none").to_request()).await;
    assert_eq!(String::from_utf8_lossy(&answer.body), "first();");
    let answer = ask(&app, get(live_path, "owner").to_request()).await;
    assert!(String::from_utf8_lossy(&answer.body).contains("<body>first"));

    scratch.write("login.html", "<html><body>second</body></html>");
    scratch.write(live_file, "<html><head></head><body>second</body></html>");
    scratch.write("late.js", "second();");
    if login_held {
        let answer = ask(&app, get("/login", "none").to_request()).await;
        assert_eq!(answer.status, StatusCode::OK);
        assert_eq!(answer.header("cache-control"), Some("no-cache"));
    } else {
        let answer = ask(&app, get("/login", "none").to_request()).await;
        assert_eq!(
            without_version_line(&String::from_utf8_lossy(&answer.body), true, "/login"),
            "<html><body>second</body></html>"
        );
    }
    let answer = ask(&app, get("/late.js", "none").to_request()).await;
    assert_eq!(String::from_utf8_lossy(&answer.body), "second();");
    let answer = ask(&app, get(live_path, "owner").to_request()).await;
    assert!(String::from_utf8_lossy(&answer.body).contains("<body>second"));
}

#[actix_web::test]
async fn a_relative_web_directory_is_taken_from_where_the_server_runs() {
    // `cargo test` runs a package's tests in the package directory, `api/`
    let app = app!(Path::new("tests/fixtures/pages/web"));
    let answer = ask(&app, get("/login", "none").to_request()).await;
    assert_eq!(answer.status, StatusCode::OK);
    if ScreenTable::compiled().holds("/login") {
        assert_eq!(answer.header("cache-control"), Some("no-cache"));
        assert!(
            answer
                .header("content-type")
                .is_some_and(|c| c.starts_with("text/html"))
        );
    } else {
        assert!(String::from_utf8_lossy(&answer.body).contains("login (fixture)"));
    }
    let answer = ask(&app, get("/fonts/OFL.txt", "none").to_request()).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(String::from_utf8_lossy(&answer.body), "OFL fixture\n");
}

// ------------------------------------------------------------- static files

#[actix_web::test]
async fn every_static_file_answers_as_v2_does() {
    let cases = cases();
    let statics = cases["statics"].as_array().expect("statics");
    assert_eq!(statics.len(), 36, "v2_page_cases.json was cut short");
    let reader = Arc::new(FakeSessions::default());
    let web_dir = fixtures().join("web");
    let app = app_with!(&web_dir, reader.clone());
    let mut served = 0;

    for case in statics {
        let path = case["path"].as_str().unwrap();
        // a file is the same for everyone, and a query changes nothing
        for (uri, who) in [
            (path.to_owned(), "none"),
            (format!("{path}?v=3&branch=%22"), "none"),
            (path.to_owned(), "staff:RATES:XC-B01"),
        ] {
            let what = format!("GET {uri} as {who}");
            let answer = ask(&app, get(&uri, who).to_request()).await;
            if case["status"] == 200 {
                assert_eq!(answer.status, StatusCode::OK, "{what}");
                headers_are(&answer, &case["headers"], &what);
                let file = web_dir.join(case["file"].as_str().unwrap());
                let bytes = std::fs::read(&file).expect("the fixture file");
                assert_eq!(answer.body, bytes, "{what}: the file's bytes");
            } else {
                answer.is_the_plain_404(&what);
            }
        }
        if case["status"] == 200 {
            served += 1;
        }
    }
    assert_eq!(served, 15, "files v2 serves from the fixture directory");
    // a static file is public: finding it does not look any session up
    assert_eq!(
        reader.asked.load(Ordering::SeqCst),
        0,
        "the session reader was asked for a static file"
    );
}

#[actix_web::test]
async fn only_get_reaches_a_static_file() {
    let app = app!(&fixtures().join("web"));
    for method in [
        Method::POST,
        Method::PUT,
        Method::DELETE,
        Method::HEAD,
        Method::OPTIONS,
    ] {
        for path in ["/app.js", "/fonts/OFL.txt", "/favicon.svg"] {
            let what = format!("{method} {path}");
            let answer = ask(&app, request(method.clone(), path, "owner").to_request()).await;
            if method == Method::HEAD {
                assert_eq!(answer.status, StatusCode::NOT_FOUND, "{what}");
            } else {
                answer.is_the_plain_404(&what);
            }
        }
    }
}

#[actix_web::test]
async fn nothing_outside_the_web_directory_is_served() {
    // <scratch>/web        the web directory
    // <scratch>/web2       a neighbour whose name starts the same
    // <scratch>/secret.js  a file beside it
    let scratch = Scratch::new("outside");
    scratch.write("web/app.js", "inside();");
    scratch.write("web/sub/deep.js", "deep();");
    scratch.write("web2/other.js", "neighbour();");
    scratch.write("secret.js", "secret();");
    scratch.write("secret.txt", "secret");
    // files whose names are what a careless decoder would look for
    scratch.write("web/%zz.js", "percent();");
    scratch.write("web/\u{fffd}.js", "replacement();");
    let app = app!(&scratch.0.join("web"));

    let answer = ask(&app, get("/app.js", "none").to_request()).await;
    assert_eq!(String::from_utf8_lossy(&answer.body), "inside();");
    let answer = ask(&app, get("/sub/deep.js", "none").to_request()).await;
    assert_eq!(String::from_utf8_lossy(&answer.body), "deep();");

    // a literal percent sign has to be written %25
    let answer = ask(&app, get("/%25zz.js", "none").to_request()).await;
    assert_eq!(String::from_utf8_lossy(&answer.body), "percent();");
    let answer = ask(&app, get("/%EF%BF%BD.js", "none").to_request()).await;
    assert_eq!(String::from_utf8_lossy(&answer.body), "replacement();");

    for path in [
        // out of the directory, written every way a client can write it
        "/../secret.js",
        "/%2e%2e/secret.js",
        "/%2E%2E/secret.js",
        "/..%2fsecret.js",
        "/..%2Fsecret.js",
        "/%2e%2e%2fsecret.js",
        "/sub/../../secret.js",
        "/sub/%2e%2e/%2e%2e/secret.js",
        "/sub%2f..%2f..%2fsecret.js",
        "/../web2/other.js",
        "/%2e%2e/web2/other.js",
        "/..%2fweb2%2fother.js",
        "/../secret.txt",
        // [required] in v3 a `..` segment is refused even when it stays inside
        "/sub/../app.js",
        "/sub/%2e%2e/app.js",
        "/sub%2f..%2fapp.js",
        "/..",
        "/%2e%2e",
        // an absolute path after decoding
        "/%2f%2fetc%2fhosts.txt",
        // not a percent-encoding, not UTF-8, a NUL
        "/%zz.js",
        "/app.js%",
        "/app.js%2",
        "/%ff.js",
        "/%c3%28.js",
        "/app.js%00",
        "/%00app.js",
        "/app%00.js",
    ] {
        for who in ["none", "owner"] {
            ask(&app, get(path, who).to_request())
                .await
                .is_the_plain_404(&format!("GET {path} as {who}"));
        }
    }
}

#[cfg(unix)]
#[actix_web::test]
async fn a_link_out_of_the_web_directory_is_not_followed() {
    use std::os::unix::fs::symlink;

    let scratch = Scratch::new("links");
    scratch.write("web/app.js", "inside();");
    scratch.write("secret.js", "secret();");
    scratch.write("outside/x.js", "outside();");
    symlink(scratch.0.join("secret.js"), scratch.0.join("web/link.js")).expect("link a file");
    // a neighbour whose path begins with the web directory's own path
    scratch.write("web2/other.js", "neighbour();");
    symlink(
        scratch.0.join("web2/other.js"),
        scratch.0.join("web/neighbour.js"),
    )
    .expect("link into the neighbour");
    symlink(scratch.0.join("outside"), scratch.0.join("web/out")).expect("link a directory");
    symlink(&scratch.0, scratch.0.join("web/up")).expect("link the parent");
    // the web directory itself may be reached through a link
    symlink(scratch.0.join("web"), scratch.0.join("current")).expect("link the web directory");

    for dir in ["web", "current"] {
        let app = app!(&scratch.0.join(dir));
        let answer = ask(&app, get("/app.js", "none").to_request()).await;
        assert_eq!(answer.status, StatusCode::OK, "{dir}: /app.js");
        assert_eq!(String::from_utf8_lossy(&answer.body), "inside();");
        for path in ["/link.js", "/neighbour.js", "/out/x.js", "/up/secret.js"] {
            ask(&app, get(path, "none").to_request())
                .await
                .is_the_plain_404(&format!("{dir}: GET {path}"));
        }
    }
}

// ----------------------------------------------------------------- fallback

#[actix_web::test]
async fn the_fallback_alone_serves_static_files_and_no_page() {
    // `pages::fallback` is what `bootstrap::configure` registers as the default
    // service. It finds the web directory in the `AppState` of the application
    // it runs in; the page routes are not its business.
    let state = state(&fixtures().join("web"));
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state.clone()))
            .default_service(web::to(pages::fallback)),
    )
    .await;

    let answer = ask(&app, get("/app.js", "none").to_request()).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        String::from_utf8_lossy(&answer.body),
        "console.log(\"app fixture\");\n"
    );
    assert_eq!(
        answer.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    for path in [
        "/login",
        "/",
        "/desk",
        "/staff",
        "/login.html",
        "/missing.js",
    ] {
        ask(&app, get(path, "owner").to_request())
            .await
            .is_the_plain_404(&format!("the fallback alone: GET {path}"));
    }
    ask(&app, request(Method::POST, "/app.js", "owner").to_request())
        .await
        .is_the_plain_404("the fallback alone: POST /app.js");

    // called as a function, as T1b's tests call it
    let req = test::TestRequest::get()
        .uri("/style.css")
        .app_data(web::Data::new(state))
        .to_http_request();
    let resp: HttpResponse = pages::fallback(req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body = actix_web::body::to_bytes(resp.into_body())
        .await
        .expect("read the body");
    assert_eq!(&body[..], b"body{color:#123}\n");
}
