//! Up-front tests for CS-9d (review of PR 6288 by the cloud reviewer, relayed by the lane controller claude/022a7453):
//! `branchShiftsSave` refuses a list that names one stored template twice, however the id is spelled. `replace_in`
//! parses ids with `Uuid::parse_str`, which accepts upper case, no hyphens, braces and `urn:uuid:`; comparing the raw
//! strings let `[A, A-in-upper-case, C]` through and the store updated A's row twice, losing A. Written by the ticket
//! author claude/35271802 — the implementer must not edit, delete or weaken these.
//!
//! The harness is a copy of `cs9_branch_shifts_test.rs`'s. MAIN starts on the migration's defaults A 06–14, B 14–22,
//! C 22–06. Needs `TEST_DATABASE_URL`.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use quote_common::*;
use serde_json::{Value, json};
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";

struct Who {
    id: String,
    spec: String,
}

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let roles = role
        .split('+')
        .map(|r| Role::parse(r).map(|r| ScopedRole::new(r, branch.clone(), from, None)))
        .collect::<Option<Vec<_>>>()?;
    let session_branch = Some(branch.clone().unwrap_or_else(|| BranchId::new("MAIN")));
    Some(User::new(id, ORG_1, roles, None).with_branch(session_branch))
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    fn reason(&self) -> &str {
        self.body["error"]["details"]["reason"]
            .as_str()
            .or_else(|| self.body["error"]["reason"].as_str())
            .unwrap_or("")
    }

    fn detail(&self) -> &str {
        self.body["error"]["details"]["detail"]
            .as_str()
            .or_else(|| self.body["error"]["detail"].as_str())
            .unwrap_or("")
    }
}

struct TestHarness {
    db: TestDb,
    admin: Who,
    teller: Who,
    scratch: std::path::PathBuf,
}

impl TestHarness {
    async fn start() -> Self {
        let db = TestDb::start().await;
        let scratch = std::env::temp_dir().join(unique("xc-cs9"));
        std::fs::create_dir_all(scratch.join("uploads")).expect("create scratch dir");

        let admin_id = xc_testdb::people::seed_person(
            &mut db.pool.acquire().await.expect("acquire"),
            ORG_1,
            "ADMIN",
            &[xc_testdb::people::RoleRow {
                role_code: "ADMIN",
                branch_id: None,
                valid_from: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                valid_to: None,
            }],
        )
        .await
        .expect("seed admin");

        let teller_id = xc_testdb::people::seed_person(
            &mut db.pool.acquire().await.expect("acquire"),
            ORG_1,
            "TELLER",
            &[xc_testdb::people::RoleRow {
                role_code: "TELLER",
                branch_id: Some("MAIN"),
                valid_from: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                valid_to: None,
            }],
        )
        .await
        .expect("seed teller");

        let admin = Who {
            spec: format!("{admin_id}|ADMIN|*"),
            id: admin_id,
        };
        let teller = Who {
            spec: format!("{teller_id}|TELLER|MAIN"),
            id: teller_id,
        };

        Self {
            db,
            admin,
            teller,
            scratch,
        }
    }

    async fn finish(self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
        self.db.finish().await;
    }

    async fn call(&self, who: &Who, method: Method, uri: &str, body: Option<Value>) -> Answer {
        let uploads = self.scratch.join("uploads");
        let state = AppState::new(
            Arc::new(common::settings(&[(
                "APP_FILES__DIR",
                uploads.to_str().expect("UTF-8 path"),
            )])),
            self.db.pool.clone(),
        );
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;

        let mut req = atest::TestRequest::default()
            .method(method)
            .uri(uri)
            .insert_header((header::HOST, HOST))
            .insert_header((AS, who.spec.clone()));

        if let Some(b) = &body {
            req = req
                .insert_header(("x-xchange-request", "1"))
                .insert_header((header::ORIGIN, SAME_ORIGIN))
                .insert_header(("idempotency-key", unique("idem")))
                .set_json(b);
        }

        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

        Answer { status, text, body }
    }
}

/// MAIN's live templates as `(id, name, starts_at)`, in plan order.
async fn live(h: &TestHarness) -> Vec<(String, String, String)> {
    sqlx::query_as(
        "SELECT id::text, name, to_char(starts_at, 'HH24:MI') FROM shift_templates \
         WHERE branch_id = 'MAIN' AND retired_at IS NULL ORDER BY position",
    )
    .fetch_all(&h.db.pool)
    .await
    .expect("MAIN's live plan")
}

/// Save `[A as id_a, B (A's id written as `twin`), C new]` (a covering 24 h plan) as the admin.
async fn save_with_twin(h: &TestHarness, twin: &str) -> Answer {
    let plan = live(h).await;
    let a = &plan[0].0;
    h.call(
        &h.admin,
        Method::PUT,
        "/api/branches/shifts",
        Some(json!({
            "branch": "MAIN",
            "templates": [
                { "id": a, "name": "A", "startsAt": "06:00", "endsAt": "14:00" },
                { "id": twin, "name": "B", "startsAt": "14:00", "endsAt": "22:00" },
                { "name": "C", "startsAt": "22:00", "endsAt": "06:00" }
            ]
        })),
    )
    .await
}

async fn assert_refused_and_unchanged(h: &TestHarness, twin: &str, row: &str) {
    let before = live(h).await;
    let res = save_with_twin(h, twin).await;
    assert_eq!(
        res.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{row}: {}",
        res.text
    );
    assert_eq!(res.code(), "shiftPlanInvalid", "{row}: {}", res.text);
    assert_eq!(res.reason(), "duplicateId", "{row}: {}", res.text);
    assert_eq!(
        res.detail(),
        "1",
        "{row}: the second row names the repeated id: {}",
        res.text
    );
    assert_eq!(live(h).await, before, "{row}: nothing is stored");
}

/// D1: the same id twice, written the same way.
#[actix_web::test]
async fn d1_the_same_id_twice_is_refused() {
    let h = TestHarness::start().await;
    let a = live(&h).await[0].0.clone();
    assert_refused_and_unchanged(&h, &a, "D1").await;
    h.finish().await;
}

/// D2: A's id in upper case is still A.
#[actix_web::test]
async fn d2_the_id_in_upper_case_is_the_same_template() {
    let h = TestHarness::start().await;
    let a = live(&h).await[0].0.to_uppercase();
    assert_refused_and_unchanged(&h, &a, "D2").await;
    h.finish().await;
}

/// D3: A's id without hyphens is still A.
#[actix_web::test]
async fn d3_the_id_without_hyphens_is_the_same_template() {
    let h = TestHarness::start().await;
    let a = live(&h).await[0].0.replace('-', "");
    assert_refused_and_unchanged(&h, &a, "D3").await;
    h.finish().await;
}

/// D4: an id that is not a UUID names no stored template: 422 `shiftTemplateUnknown`, nothing stored.
#[actix_web::test]
async fn d4_an_id_that_is_not_a_uuid_is_unknown() {
    let h = TestHarness::start().await;
    let before = live(&h).await;
    let res = save_with_twin(&h, "not-a-uuid").await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.text);
    assert_eq!(res.code(), "shiftTemplateUnknown", "{}", res.text);
    assert_eq!(live(&h).await, before, "nothing is stored");
    h.finish().await;
}

/// D5: distinct ids, spelled canonically, still save: A and B keep their rows, C is new.
#[actix_web::test]
async fn d5_distinct_ids_still_save() {
    let h = TestHarness::start().await;
    let plan = live(&h).await;
    let res = h
        .call(
            &h.admin,
            Method::PUT,
            "/api/branches/shifts",
            Some(json!({
                "branch": "MAIN",
                "templates": [
                    { "id": plan[0].0, "name": "A", "startsAt": "06:00", "endsAt": "14:00" },
                    { "id": plan[1].0, "name": "B", "startsAt": "14:00", "endsAt": "22:00" },
                    { "name": "C2", "startsAt": "22:00", "endsAt": "06:00" }
                ]
            })),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    let after = live(&h).await;
    assert_eq!(after.len(), 3);
    assert_eq!(after[0].0, plan[0].0, "A keeps its row");
    assert_eq!(after[1].0, plan[1].0, "B keeps its row");
    assert_eq!(after[2].1, "C2");
    h.finish().await;
}
