//! `/kiosk` is this repository's own page (`api/static/kiosk/`, compiled in by `presentation::pages`):
//! the shop's live kiosk (xc-th.work, captured 2026-11-05), with its 36 s attract loop and 16 languages.
//! Before, install extracted it once from the v2 bundle into `web.dir`, so it froze at an old version.
//!
//! Mutations that must turn this red:
//! - read `/kiosk` from `web.dir` again                    -> the_kiosk_is_the_compiled_page_…
//! - drop a kiosk file route (it falls back to `web.dir`)  -> every_kiosk_file_is_compiled_in_…
//! - drop the sign-in gate of #6096                        -> the_kiosk_still_needs_a_sign_in
//! - lose a language                                       -> the_kiosk_has_16_languages
//! - fetch `/api/rate-board` again (we have none)          -> the_kiosk_reads_no_rate_board

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
use xc_auth::domain::access::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const WHO: &str = "x-test-sessions";
const HOST: &str = "xc.test";

const PAGE: &str = include_str!("../static/kiosk/kiosk.html");
const FILES: [(&str, &str, &str); 9] = [
    (
        "/kiosk.js",
        "text/javascript",
        include_str!("../static/kiosk/kiosk.js"),
    ),
    (
        "/kiosk-i18n.js",
        "text/javascript",
        include_str!("../static/kiosk/kiosk-i18n.js"),
    ),
    (
        "/kiosk-language-extras.js",
        "text/javascript",
        include_str!("../static/kiosk/kiosk-language-extras.js"),
    ),
    (
        "/kiosk-attract.js",
        "text/javascript",
        include_str!("../static/kiosk/kiosk-attract.js"),
    ),
    (
        "/kiosk.css",
        "text/css",
        include_str!("../static/kiosk/kiosk.css"),
    ),
    (
        "/kiosk-attract.css",
        "text/css",
        include_str!("../static/kiosk/kiosk-attract.css"),
    ),
    (
        "/kiosk-fonts.css",
        "text/css",
        include_str!("../static/kiosk/kiosk-fonts.css"),
    ),
    (
        "/kiosk-xchange.css",
        "text/css",
        include_str!("../static/kiosk/kiosk-xchange.css"),
    ),
    (
        "/xc-tokens.css",
        "text/css",
        include_str!("../static/kiosk/xc-tokens.css"),
    ),
];

/// `none` | `owner` | `staff:ROLE:BRANCH`, as in pages_test.rs.
struct FakeCallers;

impl SessionReader for FakeCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let who = req
            .headers()
            .get(WHO)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("none")
            .to_owned();
        Box::pin(async move {
            let staff = who.find("staff:").map(|at| {
                let mut parts = who[at..].split(':').skip(1);
                let roles: &[Role] = match parts.next().expect("a role") {
                    "FRONT" => &[Role::Teller],
                    other => panic!("unknown role: {other}"),
                };
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

/// A `web.dir` that still holds an old kiosk under every name the compiled kiosk uses.
struct StaleWebDir(PathBuf);

impl StaleWebDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("xc-kiosk-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("kiosk.html"),
            "<!doctype html><html><head></head><body>old v2 kiosk</body></html>",
        )
        .unwrap();
        for (path, _, _) in FILES {
            std::fs::write(dir.join(&path[1..]), "/* stale */").unwrap();
        }
        Self(dir)
    }
}

impl Drop for StaleWebDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

macro_rules! app {
    ($web_dir:expr) => {{
        let reader: Arc<dyn SessionReader> = Arc::new(FakeCallers);
        let state = state_with(
            settings(&[("APP_WEB__DIR", $web_dir.to_str().expect("a UTF-8 path"))]),
            FakeDatabase::Up,
        );
        test::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await
    }};
}

async fn send<S, R, B>(app: &S, req: R) -> (StatusCode, Option<String>, Option<String>, String)
where
    S: Service<R, Response = ServiceResponse<B>, Error = actix_web::Error>,
    B: MessageBody,
{
    let resp = test::call_service(app, req).await;
    let status = resp.status();
    let header_of = |name| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let content_type = header_of(header::CONTENT_TYPE);
    let location = header_of(header::LOCATION);
    let body = test::read_body(resp).await;
    (
        status,
        content_type,
        location,
        String::from_utf8_lossy(&body).into_owned(),
    )
}

macro_rules! get {
    ($app:expr, $uri:expr, $who:expr) => {
        send(
            $app,
            test::TestRequest::get()
                .uri($uri)
                .insert_header((header::HOST, HOST))
                .insert_header((WHO, $who))
                .to_request(),
        )
        .await
    };
}

/// The "not set" screen values the server puts into the kiosk page (`application::kiosk_screen`).
fn page_screen(body: &str) -> serde_json::Value {
    const OPEN: &str = "<script>window.XC_KIOSK_SCREEN=";
    let at = body.find(OPEN).expect("the page carries the screen values") + OPEN.len();
    let end = at + body[at..].find(";</script>").unwrap();
    serde_json::from_str(&body[at..end]).expect("JSON")
}

/// The page without what the server adds: the screen values, the form helper and the version line.
fn without_server_additions(body: &str) -> String {
    const OPEN: &str = "<script>window.XC_KIOSK_SCREEN=";
    assert_eq!(body.matches(OPEN).count(), 1, "one screen-values script");
    let at = body.find(OPEN).unwrap();
    assert!(
        body[..at].ends_with("<head>"),
        "the screen values open the head"
    );
    let end = at + body[at..].find(";</script>").unwrap() + ";</script>".len();
    let body = &format!("{}{}", &body[..at], &body[end..]);
    const HELPER: &str = "<script src=\"/_xc/forms.js\" defer></script>";
    const MARK: &str = "<div id=\"xc-version\"";
    assert_eq!(body.matches(HELPER).count(), 1, "one form helper");
    assert_eq!(body.matches(MARK).count(), 1, "one version line");
    let at = body.find(HELPER).unwrap();
    let line = body.find(MARK).unwrap();
    assert_eq!(
        at + HELPER.len(),
        line,
        "the version line follows the helper"
    );
    let end = line + body[line..].find("</div>").unwrap() + "</div>".len();
    format!("{}{}", &body[..at], &body[end..])
}

#[actix_web::test]
async fn the_kiosk_is_the_compiled_page_not_the_one_in_web_dir() {
    let web = StaleWebDir::new("page");
    let app = app!(&web.0);
    for who in ["owner", "staff:FRONT:XC-B01"] {
        let (status, content_type, _, body) = get!(&app, "/kiosk", who);
        assert_eq!(status, StatusCode::OK, "{who}");
        assert!(content_type.unwrap().starts_with("text/html"), "{who}");
        assert!(!body.contains("old v2 kiosk"), "{who}: read from web.dir");
        assert_eq!(
            without_server_additions(&body),
            PAGE,
            "{who}: the compiled page"
        );
        assert!(body.contains("<script src=\"/kiosk-attract.js\"></script>"));
        assert!(body.contains("XCHANGE · Customer Touch Screen"));
    }
}

#[actix_web::test]
async fn the_kiosk_is_served_with_no_web_dir_at_all() {
    let app = app!(Path::new("/nonexistent/xc-kiosk-web"));
    let (status, _, _, body) = get!(&app, "/kiosk", "owner");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(without_server_additions(&body), PAGE);
    for (path, _, want) in FILES {
        let (status, _, _, body) = get!(&app, path, "none");
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(body, want, "{path}");
    }
}

#[actix_web::test]
async fn every_kiosk_file_is_compiled_in_and_wins_over_web_dir() {
    let web = StaleWebDir::new("files");
    let app = app!(&web.0);
    for (path, kind, want) in FILES {
        // public, as the static files were: the page itself is what is gated
        let (status, content_type, _, body) = get!(&app, path, "none");
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(
            content_type.as_deref().unwrap_or("").starts_with(kind),
            "{path}: {content_type:?}"
        );
        assert_eq!(body, want, "{path}: the compiled file, not web.dir's");
    }
}

#[actix_web::test]
async fn the_kiosk_still_needs_a_sign_in() {
    // F7 / #6096 (docs/design/blueprint/kiosk-shared-phone.md:235, D19): unchanged by the swap
    let web = StaleWebDir::new("gate");
    let app = app!(&web.0);
    let (status, _, location, body) = get!(&app, "/kiosk", "none");
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        location.as_deref(),
        Some(format!("http://{HOST}/login?next=%2Fkiosk").as_str())
    );
    assert!(
        !body.contains("kiosk-attract"),
        "no page before the sign-in"
    );
}

/// The language codes of the `window.KIOSK_LANGS=[…]` list in `kiosk-language-extras.js`.
fn kiosk_langs(extras: &str) -> Vec<String> {
    let start = extras.find("window.KIOSK_LANGS=[").expect("the list");
    let end = start + extras[start..].find("];").expect("its end");
    extras[start..end]
        .split("{code:'")
        .skip(1)
        .map(|rest| rest[..rest.find('\'').unwrap()].to_owned())
        .collect()
}

#[actix_web::test]
async fn the_kiosk_has_16_languages() {
    let app = app!(Path::new("/nonexistent/xc-kiosk-web"));
    let (_, _, _, extras) = get!(&app, "/kiosk-language-extras.js", "none");
    let mut codes = kiosk_langs(&extras);
    codes.sort();
    let mut want = [
        "en", "th", "zh", "zh-HK", "ja", "ko", "ru", "hi", "ar", "lo", "my", "vi", "ms", "de",
        "fr", "id",
    ];
    want.sort();
    assert_eq!(codes, want);

    // the four languages kiosk-i18n.js does not hold come from kiosk-language-extras.js
    let (_, _, _, i18n) = get!(&app, "/kiosk-i18n.js", "none");
    for code in ["ms", "de", "fr", "id"] {
        assert!(i18n.contains(&format!("\n{code}:[")), "{code} translations");
    }
}

#[actix_web::test]
async fn the_kiosk_reads_no_rate_board() {
    // we have no GET /api/rate-board and will not add one (docs/design/blueprint/rate-display.md:314)
    let app = app!(Path::new("/nonexistent/xc-kiosk-web"));
    let (_, _, _, script) = get!(&app, "/kiosk.js", "none");
    assert!(
        !script.contains("fetch('/api/rate-board"),
        "fetches /api/rate-board"
    );
    assert!(
        script.contains("await api('rates')"),
        "reads the kiosk rates answer"
    );
    assert!(
        script.contains("'X-Xchange-Request':'1'"),
        "/api/kiosk refuses a call without the marker"
    );
}

#[actix_web::test]
async fn the_page_carries_the_not_set_screen_values() {
    // owner 2026-11-05: "ว่า อันไหน คอนฟิก ได้ อย่า ฝังใหน code" — kiosk.js reads these, it holds none
    let app = app!(Path::new("/nonexistent/xc-kiosk-web"));
    let (_, _, _, body) = get!(&app, "/kiosk", "owner");
    assert_eq!(
        page_screen(&body),
        serde_json::json!({
            "quickAmounts": [100, 500, 1000, 5000],
            "tiles": 6,
            "idleSeconds": 120,
            "reviewSeconds": 3,
            "refreshSeconds": 10,
            "popularDays": 30,
            "defaultPaymentMethod": "Cash",
            "set": false,
            "defaults": [
                "quickAmounts", "tiles", "idleSeconds", "reviewSeconds", "refreshSeconds",
                "popularDays", "defaultPaymentMethod",
            ],
            "source": "default",
        })
    );

    // kiosk.js's fallback for a server too old to send them is the same values
    let script = include_str!("../static/kiosk/kiosk.js");
    let at = script
        .find("const NOT_SET_FALLBACK=")
        .expect("the fallback")
        + "const NOT_SET_FALLBACK=".len();
    let fallback = &script[at..at + script[at..].find(";").unwrap()];
    let screen = page_screen(&body);
    for key in [
        "quickAmounts",
        "tiles",
        "idleSeconds",
        "reviewSeconds",
        "refreshSeconds",
        "defaultPaymentMethod",
    ] {
        let want = format!("{key}:{}", screen[key].to_string().replace(' ', ""));
        assert!(
            fallback.contains(&want),
            "kiosk.js fallback {fallback} lacks {want}"
        );
    }
}

#[actix_web::test]
async fn the_amount_area_keeps_the_approved_look() {
    // owner 2026-11-05 ("ไม่เปลี่ยน", "ผมว่ามัน ok"): the field at full width with the code on the left and
    // the number on the right, the four quick amounts in one row; web/src/kiosk locks the order
    let css = include_str!("../static/kiosk/kiosk-xchange.css");
    assert!(
        css.contains(".amount-field #amount{width:100%;"),
        "full width"
    );
    let field = &css[css.find(".amount-field #amount{width:100%;").unwrap()..];
    let field = &field[..field.find('}').unwrap()];
    assert!(
        field.contains("text-align:right"),
        "the number on the right: {field}"
    );
    assert!(
        css.contains(".amount-code{position:absolute;left:"),
        "the code on the left"
    );
    assert!(
        css.contains(".quick-amounts{display:grid;grid-template-columns:repeat(4,"),
        "4 in a row"
    );
    assert!(!css.contains("amount-row"), "no side-by-side layout");
}
