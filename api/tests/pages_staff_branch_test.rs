//! A staff member's own branch is judged like any other branch name on a
//! branch page (#262; #228 item 6(c); ticket #80 §8.3 item 3, last
//! sentence: "This also applies to a staff member's own branch").
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! `pages_test.rs` builds its staff actors from a request header split on `:`,
//! and every branch it uses is well formed, so nothing there fails when the
//! check at `api/src/presentation/pages.rs` (`if !is_valid_branch(&name)`) is
//! skipped for the name taken from the staff actor. The reader here hands out
//! one fixed pair of sessions, so any `BranchId` can be the staff member's.
//!
//! No test here needs a database.

mod common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test, web};
use common::{FakeDatabase, settings, state_with};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap;
use xchange::presentation::pages::{SessionReader, Sessions};
use xchange::presentation::screens::ScreenTable;
use xchange::presentation::ui::UiAssets;

/// The three branch pages of §8.1.
const BRANCH_PAGES: [&str; 3] = ["/", "/desk", "/receipt-sample"];

fn web_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pages/web")
}

/// Hands out the same sessions for every request.
struct FixedSessions {
    owner: bool,
    staff_branch: String,
}

impl SessionReader for FixedSessions {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let owner = self.owner.then(|| Actor::owner("owner-1"));
        let staff = Actor::staff(
            "employee-7",
            Role::Front,
            BranchId::new(self.staff_branch.clone()),
        );
        assert!(staff.is_some(), "FRONT is a staff role");
        Box::pin(async move { Sessions { owner, staff } })
    }
}

/// Status, Content-Type and body of `GET uri` with these sessions.
async fn get(owner: bool, staff_branch: &str, uri: &str) -> (StatusCode, String, String) {
    let reader: Arc<dyn SessionReader> = Arc::new(FixedSessions {
        owner,
        staff_branch: staff_branch.to_owned(),
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
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().expect("a text header value").to_owned())
        .unwrap_or_default();
    let body = String::from_utf8_lossy(&test::read_body(resp).await).into_owned();
    (status, content_type, body)
}

fn is_invalid_branch(answer: &(StatusCode, String, String), what: &str) {
    let (status, content_type, body) = answer;
    assert_eq!(
        *status,
        StatusCode::BAD_REQUEST,
        "{what}: status; body {body:?}"
    );
    assert!(
        content_type.to_ascii_lowercase().starts_with("text/plain"),
        "{what}: Content-Type {content_type:?}"
    );
    assert_eq!(body, "Invalid branch", "{what}: body");
}

fn is_told(answer: &(StatusCode, String, String), branch: &str, what: &str) {
    let (status, _, body) = answer;
    assert_eq!(*status, StatusCode::OK, "{what}: status; body {body:?}");
    let json = serde_json::to_string(branch).expect("a JSON string");
    let told = format!("<head><script>window.XC_BRANCH={json};</script>");
    assert!(body.contains(&told), "{what}: {told} in {body:?}");
}

#[actix_web::test]
async fn a_staff_members_own_branch_that_is_not_a_branch_name_is_refused() {
    let seventy_one = "a".repeat(71);
    let bad = [
        ("71 characters", seventy_one.as_str()),
        ("empty", ""),
        ("an underscore", "XC_B01"),
        ("a space", "XC B01"),
        ("a quote and a tag", "\"><script>"),
        ("a Thai word", "สาขา"),
        ("a colon", "XC:B01"),
    ];
    for page in BRANCH_PAGES {
        for (label, branch) in bad {
            let what = format!("GET {page} as FRONT staff of a branch with {label}");
            is_invalid_branch(&get(false, branch, page).await, &what);
            // the address cannot stand in for the staff member's own branch
            let uri = format!("{page}?branch=XC-B02");
            let what = format!("GET {uri} as FRONT staff of a branch with {label}");
            is_invalid_branch(&get(false, branch, &uri).await, &what);
        }
    }
}

#[actix_web::test]
async fn a_staff_members_own_branch_of_one_to_seventy_name_characters_is_told() {
    let seventy = "a".repeat(70);
    for page in BRANCH_PAGES {
        let held = ScreenTable::compiled().holds(page);
        for branch in ["X", "XC-B01", "xc-b01-2", seventy.as_str()] {
            let uri = format!("{page}?branch=XC-B02");
            let what = format!("GET {uri} as FRONT staff of {branch:?}");
            let answer = get(false, branch, &uri).await;
            if held {
                assert_eq!(answer.0, StatusCode::OK, "{what}: status");
                assert!(
                    !answer.2.contains("window.XC_BRANCH"),
                    "{what}: a v3 screen carries no window.XC_BRANCH"
                );
                let decl = ScreenTable::compiled()
                    .entries()
                    .iter()
                    .find(|d| d.path == page)
                    .unwrap()
                    .clone();
                let entry = UiAssets::get(&decl.entry)
                    .unwrap_or_else(|| panic!("{what}: entry {} missing in UiAssets", decl.entry));
                assert_eq!(
                    answer.2.as_bytes(),
                    entry.data.as_ref(),
                    "{what}: body is v3 screen entry"
                );
            } else {
                is_told(&answer, branch, &what);
            }
        }
    }
}

#[actix_web::test]
async fn with_an_owner_session_the_staff_members_branch_is_not_judged() {
    // §8.3 item 1: the staff branch is used only when the visitor is not the
    // owner, so a malformed one beside an owner session changes nothing.
    for page in BRANCH_PAGES {
        let held = ScreenTable::compiled().holds(page);
        let what = format!("GET {page} as owner and FRONT staff of \"XC_B01\"");
        let answer = get(true, "XC_B01", page).await;
        if held {
            assert_eq!(answer.0, StatusCode::OK, "{what}: status");
            assert!(
                !answer.2.contains("window.XC_BRANCH"),
                "{what}: a v3 screen carries no window.XC_BRANCH"
            );
            let decl = ScreenTable::compiled()
                .entries()
                .iter()
                .find(|d| d.path == page)
                .unwrap()
                .clone();
            let entry = UiAssets::get(&decl.entry)
                .unwrap_or_else(|| panic!("{what}: entry {} missing in UiAssets", decl.entry));
            assert_eq!(
                answer.2.as_bytes(),
                entry.data.as_ref(),
                "{what}: body is v3 screen entry"
            );
        } else {
            is_told(&answer, "MAIN", &what);
        }
        let uri = format!("{page}?branch=XC-B02");
        let what = format!("GET {uri} as owner and FRONT staff of \"XC_B01\"");
        let answer = get(true, "XC_B01", &uri).await;
        if held {
            assert_eq!(answer.0, StatusCode::OK, "{what}: status");
            assert!(
                !answer.2.contains("window.XC_BRANCH"),
                "{what}: a v3 screen carries no window.XC_BRANCH"
            );
            let decl = ScreenTable::compiled()
                .entries()
                .iter()
                .find(|d| d.path == page)
                .unwrap()
                .clone();
            let entry = UiAssets::get(&decl.entry)
                .unwrap_or_else(|| panic!("{what}: entry {} missing in UiAssets", decl.entry));
            assert_eq!(
                answer.2.as_bytes(),
                entry.data.as_ref(),
                "{what}: body is v3 screen entry"
            );
        } else {
            is_told(&answer, "XC-B02", &what);
        }
    }
}
