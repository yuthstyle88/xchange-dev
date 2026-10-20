//! A MARKETING-only user can enter the admin shell (#3929, BE ruling option (a):
//! MARKETING gets a staff standing at the lowest priority).
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! The reader here builds `Callers` the way `AccountSessionReader::callers` does for a
//! Blueprint session (`api/src/presentation/extractors/actor.rs`): `user` is the session's
//! actor, and `staff_user` is that actor only when `Standing::of` gives it a non-owner
//! standing. So the page gates see exactly what a signed-in MARKETING or CUSTOMER user gets.
//!
//! - `GET /` (`web/pages/admin.html`, gate `branch`) needs an owner or staff standing.
//! - `GET /staff/desk` (`web/pages/staff-desk.html`, gate `front`) stays TELLER only.
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
use xc_auth::domain::access::{Actor, Role, ScopedRole, Standing};
use xc_kernel::ids::BranchId;
use xchange::bootstrap;
use xchange::presentation::pages::{Callers, SEEDED_XC_ORG_ID, SessionReader, Sessions};
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
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        // no old v2 cookie
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move {
            let now = chrono::Utc::now();
            let since = now - chrono::Duration::days(1);
            let user = Actor::new(
                "marketing-1",
                SEEDED_XC_ORG_ID,
                self.roles
                    .iter()
                    .map(|(r, b)| ScopedRole::new(*r, b.map(BranchId::new), since, None))
                    .collect(),
                None,
            );
            // as AccountSessionReader::callers
            let staff_user = Standing::of(&user, now)
                .filter(|s| !s.is_owner())
                .map(|_| user.clone());
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

fn admin_shell_entry() -> Vec<u8> {
    let decl = ScreenTable::compiled()
        .entries()
        .iter()
        .find(|d| d.path == "/")
        .expect("`/` is a v3 screen (build web/dist first)")
        .clone();
    UiAssets::get(&decl.entry)
        .unwrap_or_else(|| panic!("entry {} missing in UiAssets", decl.entry))
        .data
        .to_vec()
}

const MARKETING_AT_MAIN: &[(Role, Option<&str>)] = &[(Role::Marketing, Some("MAIN"))];
const MARKETING_ORG_WIDE: &[(Role, Option<&str>)] = &[(Role::Marketing, None)];
const CUSTOMER_ONLY: &[(Role, Option<&str>)] = &[(Role::Customer, None)];

#[actix_web::test]
async fn a_marketing_only_user_gets_the_admin_shell_at_root() {
    let entry = admin_shell_entry();
    for (label, roles) in [
        ("MARKETING at MAIN", MARKETING_AT_MAIN),
        ("org-wide MARKETING", MARKETING_ORG_WIDE),
    ] {
        let (status, location, body) = get(roles, "/").await;
        assert_eq!(
            status,
            StatusCode::OK,
            "GET / as {label}: status (Location {location:?}); a 303 to /login is the #3929 loop"
        );
        assert_eq!(
            body, entry,
            "GET / as {label}: body is the admin shell entry"
        );
    }
}

#[actix_web::test]
async fn a_marketing_only_user_is_still_refused_the_front_desk() {
    // Gate::Front stays TELLER only: a MARKETING standing must not open it
    for (label, roles) in [
        ("MARKETING at MAIN", MARKETING_AT_MAIN),
        ("org-wide MARKETING", MARKETING_ORG_WIDE),
    ] {
        let (status, location, _) = get(roles, "/staff/desk").await;
        assert_eq!(
            status,
            StatusCode::FOUND,
            "GET /staff/desk as {label}: status (Location {location:?})"
        );
        assert_eq!(
            location, "http://xc.test/staff",
            "GET /staff/desk as {label}: sent to /staff"
        );
    }
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
