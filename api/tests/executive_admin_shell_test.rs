//! An EXECUTIVE-only user can enter the signed-in v3 screens (#5446).
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! Before the fix, `Gate::SignedIn` and `Gate::Branch` in `api/src/presentation/screens.rs` ask
//! `owner_standing()` / `staff_standing()`. `Standing::of` gives no standing to EXECUTIVE alone
//! (`api/crates/xc-auth/src/domain/access/standing.rs:13`), so `GET /` answers `303 /login?next=%2F`.
//! The login page then sends every non-customer back to `/` (`web/src/screens/login/home.ts`), and
//! the browser bounces between the two forever.
//!
//! The reader here builds `Callers` the way `AccountSessionReader::callers` does for a Blueprint
//! session (`api/src/presentation/extractors/actor.rs:117-130`): `user` is the session's actor, and
//! `staff_user` is that actor when `is_staff_caller` holds (some valid role that is not CUSTOMER).
//!
//! No test here needs a database. Needs `web/dist` (`npm --prefix web run build`).

mod common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test, web};
use common::{FakeDatabase, settings, state_with};
use xc_auth::domain::access::staff_caller::is_staff_caller;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap;
use xchange::presentation::pages::{Callers, SEEDED_XC_ORG_ID, SessionReader};
use xchange::presentation::screens::ScreenTable;
use xchange::presentation::ui::UiAssets;

fn web_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pages/web")
}

/// One signed-in Blueprint user holding exactly these roles.
struct BlueprintSession {
    roles: Vec<(Role, Option<&'static str>)>,
}

impl SessionReader for BlueprintSession {
    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move {
            let now = chrono::Utc::now();
            let since = now - chrono::Duration::days(1);
            let user = Actor::new(
                "executive-1",
                SEEDED_XC_ORG_ID,
                self.roles
                    .iter()
                    .map(|(r, b)| ScopedRole::new(*r, b.map(BranchId::new), since, None))
                    .collect(),
                None,
            );
            // as AccountSessionReader::callers
            let staff_user = is_staff_caller(&user, now).then(|| user.clone());
            Callers {
                user: Some(user),
                staff_user,
            }
        })
    }
}

/// Status, `Location` and body of `GET uri` for a user holding `roles`.
async fn get(roles: &[(Role, Option<&'static str>)], uri: &str) -> (StatusCode, String, Vec<u8>) {
    let reader: Arc<dyn SessionReader> = Arc::new(BlueprintSession {
        roles: roles.to_vec(),
    });
    let dir = web_dir();
    let state = state_with(
        settings(&[("APP_WEB__DIR", dir.to_str().expect("a UTF-8 path"))]),
        FakeDatabase::Up,
    );
    let app = test::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = test::TestRequest::get()
        .uri(uri)
        .insert_header((header::HOST, "xc.test"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let status = resp.status();
    let location = resp
        .headers()
        .get(header::LOCATION)
        .map(|v| v.to_str().expect("a text header value").to_owned())
        .unwrap_or_default();
    let body = test::read_body(resp).await.to_vec();
    (status, location, body)
}

fn screen_entry(path: &str) -> Vec<u8> {
    let decl = ScreenTable::compiled()
        .entries()
        .iter()
        .find(|d| d.path == path)
        .unwrap_or_else(|| panic!("`{path}` is a v3 screen (build web/dist first)"))
        .clone();
    UiAssets::get(&decl.entry)
        .unwrap_or_else(|| panic!("entry {} missing in UiAssets", decl.entry))
        .data
        .to_vec()
}

const EXECUTIVE_ORG_WIDE: &[(Role, Option<&str>)] = &[(Role::Executive, None)];
const EXECUTIVE_AT_MAIN: &[(Role, Option<&str>)] = &[(Role::Executive, Some("MAIN"))];
const OWNER: &[(Role, Option<&str>)] = &[(Role::Executive, None), (Role::Admin, None)];
const CUSTOMER_ONLY: &[(Role, Option<&str>)] = &[(Role::Customer, None)];

#[actix_web::test]
async fn an_executive_only_user_gets_the_admin_shell_at_root() {
    let entry = screen_entry("/");
    for (label, roles) in [
        ("org-wide EXECUTIVE", EXECUTIVE_ORG_WIDE),
        ("EXECUTIVE at MAIN", EXECUTIVE_AT_MAIN),
    ] {
        let (status, location, body) = get(roles, "/").await;
        assert_eq!(
            status,
            StatusCode::OK,
            "GET / as {label}: status (Location {location:?}); a 303 to /login is the #5446 loop"
        );
        assert_eq!(
            body, entry,
            "GET / as {label}: body is the admin shell entry"
        );
    }
}

#[actix_web::test]
async fn an_executive_only_user_passes_the_signed_in_gate() {
    // `/staff-guide` is `Gate::SignedIn` (api/tests/ui_assets_test.rs:572)
    let entry = screen_entry("/staff-guide");
    let (status, location, body) = get(EXECUTIVE_ORG_WIDE, "/staff-guide").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "GET /staff-guide as org-wide EXECUTIVE: status (Location {location:?})"
    );
    assert_eq!(
        body, entry,
        "GET /staff-guide as org-wide EXECUTIVE: body is its entry"
    );
}

#[actix_web::test]
async fn the_owner_still_gets_the_admin_shell_at_root() {
    let (status, location, body) = get(OWNER, "/").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "GET / as owner: status (Location {location:?})"
    );
    assert_eq!(
        body,
        screen_entry("/"),
        "GET / as owner: body is the admin shell entry"
    );
}

#[actix_web::test]
async fn a_customer_only_user_still_goes_to_login_at_root() {
    let (status, location, _) = get(CUSTOMER_ONLY, "/").await;
    assert_eq!(status, StatusCode::SEE_OTHER, "GET / as CUSTOMER: status");
    assert!(
        location.ends_with("/login?next=%2F"),
        "GET / as CUSTOMER: Location {location:?}"
    );
}

#[actix_web::test]
async fn an_executive_only_user_is_still_refused_the_front_desk() {
    // Gate::Front stays TELLER only
    let (status, location, _) = get(EXECUTIVE_ORG_WIDE, "/staff/desk").await;
    assert_eq!(
        status,
        StatusCode::FOUND,
        "GET /staff/desk as org-wide EXECUTIVE: status (Location {location:?})"
    );
    assert_eq!(
        location, "http://xc.test/staff",
        "GET /staff/desk as org-wide EXECUTIVE: sent to /staff"
    );
}
