//! Specification tests for V2-switch (#472; docs/design/xchange-v3-web.md §3.3,
//! §8 row "V2-switch", §12.13): the v3 admin entry moves from `/ui/admin/` to
//! `/`, keeps the `branch` gate, and the review path answers `308` to `/`.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! - The source entry `web/pages/admin.html` is read as it is in the tree, so
//!   the first test needs no `web/dist` and no build.
//! - The routing tests hand the server the fixture `dist`
//!   `fixtures/ui/switched/`: Vite's manifest, one hashed asset and one entry,
//!   `pages/admin.html` declaring `xc-path` `/` and `xc-gate` `branch` — what
//!   `web/dist` holds after this ticket. v2's pages are the fixtures under
//!   `fixtures/pages/web`, as in `ui_assets_test.rs`; an empty screen table
//!   (`fixtures/ui/empty`) is the server before any screen, so its answers are
//!   v2's.
//!
//! No test here needs a database.

mod common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use common::{FakeDatabase, settings, state_with};
use xc_auth::domain::access::{Actor, Role};
use xc_kernel::ids::BranchId;
use xchange::bootstrap;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::screens::ScreenTable;
use xchange::presentation::test_callers;

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

fn switched_dist() -> PathBuf {
    fixtures().join("ui/switched")
}

fn switched_file(rel: &str) -> Vec<u8> {
    std::fs::read(switched_dist().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Who is signed in, from the request header `x-test-as`:
/// `owner`, `staff:<ROLE>:<branch>`, `owner+staff:<ROLE>:<branch>`, or absent.
struct HeaderCallers;

fn roles_of(role: &str) -> &'static [Role] {
    match role {
        "FRONT" => &[Role::Teller],
        "BACKEND" | "MANAGER" => &[Role::BranchManager],
        "RATES" => &[Role::RateManager],
        "FINANCE" => &[Role::Finance],
        "SYSTEM" => &[Role::Admin],
        "MONITOR" | "DOCUMENTS" | "DOCUMENT_REVIEW" => &[Role::VerificationReviewer],
        other => panic!("unknown role {other}"),
    }
}

fn staff_of(spec: &str) -> Option<Actor> {
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "staff spec {spec:?}");
    let roles = roles_of(parts.next().expect("a role"));
    let branch = parts.next().expect("a branch, possibly empty");
    Some(test_callers::staff(
        "employee-7",
        roles,
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

#[derive(Debug)]
struct Answer {
    status: StatusCode,
    location: Option<String>,
    content_type: String,
    cache_control: Option<String>,
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

/// `method uri` as `who`, on a server whose screen table was read from `dist`,
/// with v2's pages in `fixtures/pages/web`.
async fn call(dist: &Path, method: actix_web::http::Method, who: &str, uri: &str) -> Answer {
    let screens = ScreenTable::from_dist(dist)
        .unwrap_or_else(|e| panic!("the fixture dist {} must load: {e:?}", dist.display()));
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
    let status = resp.status();
    let location = header_of(&resp, header::LOCATION);
    let content_type = header_of(&resp, header::CONTENT_TYPE).unwrap_or_default();
    let cache_control = header_of(&resp, header::CACHE_CONTROL);
    let body = atest::read_body(resp).await.to_vec();
    Answer {
        status,
        location,
        content_type,
        cache_control,
        body,
    }
}

async fn get(dist: &Path, who: &str, uri: &str) -> Answer {
    call(dist, actix_web::http::Method::GET, who, uri).await
}

/// The `<meta name=… content=…>` values of one name in an HTML entry.
fn metas(html: &str, name: &str) -> Vec<String> {
    let needle = format!("<meta name=\"{name}\" content=\"");
    html.match_indices(&needle)
        .map(|(at, _)| {
            let rest = &html[at + needle.len()..];
            rest[..rest.find('"').expect("a closing quote")].to_string()
        })
        .collect()
}

// ------------------------------------------------------------ the entry

/// X1: the admin entry in the source tree takes `/`, with the `branch` gate.
/// One line of `web/pages/admin.html`; nothing else in the entry changes.
#[test]
fn the_admin_entry_declares_root_with_the_branch_gate() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/pages/admin.html");
    let html =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert_eq!(
        metas(&html, "xc-path"),
        vec!["/".to_string()],
        "web/pages/admin.html declares exactly one xc-path, and it is /"
    );
    assert_eq!(
        metas(&html, "xc-gate"),
        vec!["branch".to_string()],
        "web/pages/admin.html keeps exactly one xc-gate, branch (v2's PageKind::Branch for /)"
    );
    assert!(
        !html.contains("/ui/admin/"),
        "web/pages/admin.html no longer names the review path /ui/admin/"
    );
    assert!(
        html.contains(r#"<script type="module" src="/src/screens/admin/main.tsx"></script>"#),
        "web/pages/admin.html still mounts the admin shell (V2-shell, #460)"
    );
}

// ------------------------------------------------------------ GET /

/// X1, X2: once the table holds `/`, a signed-in caller gets the v3 entry,
/// byte for byte, `no-cache` — not v2's `admin.html` and none of the scripts
/// v2's page carried.
#[actix_web::test]
async fn root_serves_the_v3_admin_entry_once_the_table_holds_it() {
    for (who, uri) in [
        ("owner", "/?branch=XC-B01"),
        ("staff:FRONT:XC-B01", "/"),
        ("staff:BACKEND:XC-B01", "/"),
        ("owner+staff:FRONT:XC-B01", "/"),
    ] {
        let what = format!("GET {uri} as {who}");
        let a = get(&switched_dist(), who, uri).await;
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
        assert_eq!(
            a.body,
            switched_file("pages/admin.html"),
            "{what}: the body is the built admin entry unchanged; got {:?}",
            a.text()
        );
        let body = a.text();
        for v2_only in [
            "/admin-devices.js",
            "/account-bar.js",
            "/_xc/",
            "window.XC_BRANCH=",
        ] {
            assert!(
                !body.contains(v2_only),
                "{what}: no {v2_only:?} from v2's page"
            );
        }
    }
}

/// X1: the gate of `/` answers what v2's `PageKind::Branch` answered for every
/// refused caller — status, Location, Content-Type and body.
#[actix_web::test]
async fn root_refuses_as_v2s_branch_page_did() {
    for (label, who, uri) in [
        ("nobody signed in", "", "/"),
        ("nobody signed in, naming a branch", "", "/?branch=XC-B02"),
        ("the owner with an invalid name", "owner", "/?branch=%ZZ"),
        ("the owner with a quote", "owner", "/?branch=%22"),
        ("staff whose branch is empty", "staff:FRONT:", "/"),
    ] {
        let what = format!("GET {uri} as {label}");
        let v2 = get(&empty_dist(), who, uri).await;
        assert_ne!(
            v2.status,
            StatusCode::OK,
            "{what}: the case is one v2 refuses (fixture check); body {:?}",
            v2.text()
        );
        let v3 = get(&switched_dist(), who, uri).await;
        assert_eq!(
            v3.status,
            v2.status,
            "{what}: status as v2's; v3 body {:?}",
            v3.text()
        );
        assert_eq!(v3.location, v2.location, "{what}: Location as v2's");
        assert_eq!(
            v3.content_type, v2.content_type,
            "{what}: Content-Type as v2's"
        );
        assert_eq!(v3.body, v2.body, "{what}: body as v2's ({:?})", v2.text());
    }
}

// ------------------------------------------------------------ the review path

/// X3: `/ui/admin/` and `/ui/admin` answer `308` to `/`, keeping the query
/// string, whoever asks (the gate of `/` decides after the redirect).
#[actix_web::test]
async fn the_review_path_redirects_to_root_keeping_the_query() {
    for (uri, to) in [
        ("/ui/admin/", "/"),
        ("/ui/admin", "/"),
        ("/ui/admin/?branch=XC-B01", "/?branch=XC-B01"),
        ("/ui/admin?branch=XC-B01", "/?branch=XC-B01"),
        ("/ui/admin/?branch=a%20b&x=1", "/?branch=a%20b&x=1"),
    ] {
        for who in ["owner", "staff:FRONT:XC-B01", ""] {
            let what = format!("GET {uri} as {who:?}");
            let a = get(&switched_dist(), who, uri).await;
            assert_eq!(
                a.status,
                StatusCode::PERMANENT_REDIRECT,
                "{what}: status; body {:?}",
                a.text()
            );
            assert_eq!(a.location.as_deref(), Some(to), "{what}: Location");
        }
    }
}

/// The redirect takes only the review path: the hashed assets under `/ui/`
/// are still served, and another path under `/ui/` is still the plain 404.
#[actix_web::test]
async fn the_redirect_leaves_the_assets_and_other_ui_paths_alone() {
    let asset = get(&switched_dist(), "", "/ui/assets/index-AbC123.js").await;
    assert_eq!(asset.status, StatusCode::OK, "the hashed asset");
    assert_eq!(
        asset.body,
        switched_file("assets/index-AbC123.js"),
        "the asset bytes"
    );

    for uri in ["/ui/admin/x", "/ui/adminx", "/ui/nothing"] {
        let a = get(&switched_dist(), "owner", uri).await;
        assert_eq!(
            a.status,
            StatusCode::NOT_FOUND,
            "GET {uri}: status; body {:?}",
            a.text()
        );
        assert_eq!(a.location, None, "GET {uri}: no redirect");
    }
}

/// Only `GET` is redirected; v2 and the screens answered other methods through
/// the fallback, and so does the review path.
#[actix_web::test]
async fn the_review_path_redirects_get_only() {
    let a = call(
        &switched_dist(),
        actix_web::http::Method::POST,
        "owner",
        "/ui/admin/",
    )
    .await;
    assert_ne!(
        a.status,
        StatusCode::PERMANENT_REDIRECT,
        "POST /ui/admin/ is not redirected"
    );
    assert_eq!(a.location, None, "POST /ui/admin/: no Location");
}
