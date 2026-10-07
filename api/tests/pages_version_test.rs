//! Every page shows which build is running (#293): one line before `</body>`,
//! `v{version} · {build, 7} · {channel}`, with the values `/health/ready` reports.
//! Written before the code.
//!
//! Mutations that must turn this red:
//! - drop the call on the desk-variant path         -> desk_variant_page_…
//! - drop the call on the plain page path           -> plain_page_…
//! - take the build from a constant, not the state  -> …_matches_health_ready
//! - add the line to static files or redirects      -> static_file_… / a_redirect_…
//! - skip the escaping                              -> the_values_are_escaped

mod common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test, web};
use common::{FakeDatabase, settings, state_with};
use serde_json::Value;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState, BuildInfo};
use xchange::presentation::pages::{SessionReader, Sessions};

const WHO: &str = "x-test-sessions";
const HOST: &str = "xc.test";
const MARK: &str = "id=\"xc-version\"";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pages")
}

/// Same shape as the reader in pages_test.rs: `none` | `owner` | `staff:ROLE:BRANCH`.
#[derive(Default)]
struct FakeSessions;

impl SessionReader for FakeSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
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

fn state(web_dir: &Path, build: Option<BuildInfo>) -> AppState {
    let state = state_with(
        settings(&[("APP_WEB__DIR", web_dir.to_str().expect("a UTF-8 path"))]),
        FakeDatabase::Up,
    );
    match build {
        Some(b) => state.with_build(b),
        None => state,
    }
}

macro_rules! app {
    ($web_dir:expr, $build:expr) => {{
        let reader: Arc<dyn SessionReader> = Arc::new(FakeSessions);
        let state = state($web_dir, $build);
        test::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await
    }};
}

fn req(uri: &str, who: &str) -> test::TestRequest {
    test::TestRequest::get()
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((WHO, who))
}

async fn send<S, R, B>(app: &S, req: R) -> (StatusCode, String)
where
    S: Service<R, Response = ServiceResponse<B>, Error = actix_web::Error>,
    B: MessageBody,
{
    let resp = test::call_service(app, req).await;
    let status = resp.status();
    let body = test::read_body(resp).await;
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// GET `uri` as `who`.
macro_rules! get {
    ($app:expr, $uri:expr, $who:expr) => {
        send($app, req($uri, $who).to_request()).await
    };
}

/// The text of the one version element, or a failure naming how many there were.
fn version_text(body: &str, what: &str) -> String {
    assert_eq!(
        body.matches(MARK).count(),
        1,
        "{what}: exactly one version line"
    );
    let at = body.find(MARK).unwrap();
    let open_end = at + body[at..].find('>').unwrap() + 1;
    let close = open_end + body[open_end..].find("</div>").unwrap();
    // immediately before </body>
    assert!(
        body[close..].starts_with("</div></body>") || !body.contains("</body>"),
        "{what}: the line sits right before </body>"
    );
    body[open_end..close].to_owned()
}

/// What `/health/ready` on the same app says, as the version line should read.
macro_rules! expected {
    ($app:expr) => {{
        let (status, body) = get!($app, "/health/ready", "none");
        assert_eq!(status, StatusCode::OK);
        let v: Value = serde_json::from_str(&body).expect("health json");
        let build = v["build"].as_str().unwrap();
        format!(
            "v{} · {} · {}",
            v["version"].as_str().unwrap(),
            &build[..7],
            v["channel"].as_str().unwrap()
        )
    }};
}

#[actix_web::test]
async fn plain_page_shows_the_build_and_it_matches_health_ready() {
    let app = app!(&fixtures().join("web"), None);
    let want = expected!(&app);
    let (status, body) = get!(&app, "/login", "none");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(version_text(&body, "/login"), want);
}

#[actix_web::test]
async fn desk_variant_page_shows_the_build_and_it_matches_health_ready() {
    let app = app!(&fixtures().join("web"), None);
    let want = expected!(&app);
    let (status, body) = get!(&app, "/staff/desk", "staff:FRONT:XC-B01");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(version_text(&body, "/staff/desk"), want);
}

#[actix_web::test]
async fn a_redirect_carries_no_version_line() {
    let app = app!(&fixtures().join("web"), None);
    let (status, body) = get!(&app, "/staff/desk", "none");
    assert_eq!(status, StatusCode::FOUND);
    assert!(!body.contains(MARK), "a redirect: {body}");
}

#[actix_web::test]
async fn static_file_is_served_as_it_is_on_disk() {
    let dir = fixtures().join("web");
    let app = app!(&dir, None);
    let (status, body) = get!(&app, "/style.css", "none");
    assert_eq!(status, StatusCode::OK);
    let on_disk = std::fs::read_to_string(dir.join("style.css")).unwrap();
    assert_eq!(body, on_disk);
}

#[actix_web::test]
async fn the_values_are_escaped() {
    let build = BuildInfo {
        version: "1.0.0".to_string(),
        build: "<b>x</b>yz".to_string(),
        built_at: "2001-02-03T04:05:06Z".to_string(),
    };
    let app = app!(&fixtures().join("web"), Some(build));
    let (_, body) = get!(&app, "/login", "none");
    let text = version_text(&body, "/login with an odd build");
    assert!(!text.contains('<'), "raw HTML in the version line: {text}");
    assert!(text.contains("&lt;b&gt;x&lt;"), "escaped build: {text}");
}

#[actix_web::test]
async fn a_page_without_a_body_end_gets_the_line_at_the_end_once() {
    let dir = std::env::temp_dir().join(format!("xc-version-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("login.html"), "<html><head></head><p>no end tags").unwrap();
    let app = app!(&dir, None);
    let (status, body) = get!(&app, "/login", "none");
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.matches(MARK).count(), 1, "{body}");
    assert!(body.trim_end().ends_with("</div>"), "{body}");
}
