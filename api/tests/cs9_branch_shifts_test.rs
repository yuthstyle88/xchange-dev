//! Up-front and behaviour tests for ticket CS-9 (#6188): `branchShiftsSave` command and `branch_shifts` view.
//!
//! Behaviour table:
//! E1: save by a holder of BranchesManage → 200, the stored plan
//! E2: save by a teller → 403
//! E3: a plan with a gap (A 06-14, B 16-22) → 422 shiftPlanInvalid...
//! E3b: startsAt "25:00" / bad time → 422, reason badTime ...
//! E4: view on a fresh branch → A 06–14, B 14–22, C 22–06 (default 3 shifts), cutover "06:00", defaultShiftHours 8

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

/// E1: save by a holder of BranchesManage → 200, the stored plan.
#[actix_web::test]
async fn cs9_e1_save_by_branches_manage_stores_and_returns_plan() {
    let h = TestHarness::start().await;

    let plan_input = json!({
        "branch": "MAIN",
        "templates": [
            {
                "name": "Day",
                "startsAt": "06:00",
                "endsAt": "18:00",
                "breakStartsAt": "12:00",
                "breakEndsAt": "13:00"
            },
            {
                "name": "Night",
                "startsAt": "18:00",
                "endsAt": "06:00"
            }
        ]
    });

    let res = h
        .call(
            &h.admin,
            Method::PUT,
            "/api/branches/shifts",
            Some(plan_input),
        )
        .await;

    assert_eq!(res.status, StatusCode::OK, "save failed: {}", res.text);
    assert_eq!(res.body["cutover"], "06:00");
    assert_eq!(res.body["defaultShiftHours"], 8);
    let templates = res.body["templates"].as_array().expect("templates array");
    assert_eq!(templates.len(), 2);
    assert_eq!(templates[0]["name"], "Day");
    assert_eq!(templates[0]["startsAt"], "06:00");
    assert_eq!(templates[0]["endsAt"], "18:00");
    assert_eq!(templates[0]["breakStartsAt"], "12:00");
    assert_eq!(templates[0]["breakEndsAt"], "13:00");
    assert!(templates[0]["id"].is_string());

    assert_eq!(templates[1]["name"], "Night");
    assert_eq!(templates[1]["startsAt"], "18:00");
    assert_eq!(templates[1]["endsAt"], "06:00");
    assert!(templates[1]["id"].is_string());

    // Reading back via view returns the same stored plan.
    let view_res = h
        .call(
            &h.admin,
            Method::GET,
            "/api/v3/views/branch_shifts?branch=MAIN",
            None,
        )
        .await;
    assert_eq!(view_res.status, StatusCode::OK, "{}", view_res.text);
    let view_data = &view_res.body["data"];
    assert_eq!(view_data["cutover"], "06:00");
    assert_eq!(view_data["defaultShiftHours"], 8);
    let view_templates = view_data["templates"].as_array().expect("view templates");
    assert_eq!(view_templates.len(), 2);
    assert_eq!(view_templates[0]["name"], "Day");
    assert_eq!(view_templates[1]["name"], "Night");

    h.finish().await;
}

/// E2: save by a teller → 403 / roleNotAllowedHere (400 in v3).
#[actix_web::test]
async fn cs9_e2_save_by_teller_is_forbidden() {
    let h = TestHarness::start().await;

    let plan_input = json!({
        "branch": "MAIN",
        "templates": [
            {
                "name": "Day",
                "startsAt": "06:00",
                "endsAt": "18:00"
            },
            {
                "name": "Night",
                "startsAt": "18:00",
                "endsAt": "06:00"
            }
        ]
    });

    let res = h
        .call(
            &h.teller,
            Method::PUT,
            "/api/branches/shifts",
            Some(plan_input),
        )
        .await;

    assert!(
        res.status == StatusCode::FORBIDDEN || res.status == StatusCode::BAD_REQUEST,
        "expected permission denial status, got: {}",
        res.status
    );
    assert_eq!(res.code(), "roleNotAllowedHere");

    h.finish().await;
}

/// E3: a plan with a gap (A 06-14, B 16-22) → 422 shiftPlanInvalid.
#[actix_web::test]
async fn cs9_e3_save_plan_with_gap_is_refused_422() {
    let h = TestHarness::start().await;

    let plan_with_gap = json!({
        "branch": "MAIN",
        "templates": [
            {
                "name": "A",
                "startsAt": "06:00",
                "endsAt": "14:00"
            },
            {
                "name": "B",
                "startsAt": "16:00",
                "endsAt": "22:00"
            }
        ]
    });

    let res = h
        .call(
            &h.admin,
            Method::PUT,
            "/api/branches/shifts",
            Some(plan_with_gap),
        )
        .await;

    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.text);
    assert_eq!(res.code(), "shiftPlanInvalid");
    assert_eq!(res.reason(), "gap");
    assert_eq!(res.detail(), "14:00-16:00");

    h.finish().await;
}

/// E3b: startsAt "25:00" / bad time → 422, reason badTime ...
#[actix_web::test]
async fn cs9_e3b_save_plan_with_bad_time_is_refused_422() {
    let h = TestHarness::start().await;

    let bad_start = json!({
        "branch": "MAIN",
        "templates": [
            {
                "name": "A",
                "startsAt": "25:00",
                "endsAt": "14:00"
            }
        ]
    });

    let res = h
        .call(
            &h.admin,
            Method::PUT,
            "/api/branches/shifts",
            Some(bad_start),
        )
        .await;

    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.text);
    assert_eq!(res.code(), "shiftPlanInvalid");
    assert_eq!(res.reason(), "badTime");
    assert_eq!(res.detail(), "0");

    // Incomplete break test: breakStartsAt without breakEndsAt
    let incomplete_break = json!({
        "branch": "MAIN",
        "templates": [
            {
                "name": "A",
                "startsAt": "06:00",
                "endsAt": "14:00",
                "breakStartsAt": "10:00"
            },
            {
                "name": "B",
                "startsAt": "14:00",
                "endsAt": "22:00"
            },
            {
                "name": "C",
                "startsAt": "22:00",
                "endsAt": "06:00"
            }
        ]
    });

    let res_break = h
        .call(
            &h.admin,
            Method::PUT,
            "/api/branches/shifts",
            Some(incomplete_break),
        )
        .await;

    assert_eq!(
        res_break.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        res_break.text
    );
    assert_eq!(res_break.code(), "shiftPlanInvalid");
    assert_eq!(res_break.reason(), "breakIncomplete");
    assert_eq!(res_break.detail(), "0");

    h.finish().await;
}

/// E4: view on a fresh branch → A 06–14, B 14–22, C 22–06 (default 3 shifts), cutover "06:00", defaultShiftHours 8.
#[actix_web::test]
async fn cs9_e4_view_on_fresh_branch_returns_defaults() {
    let h = TestHarness::start().await;

    // Create a fresh branch; the trigger branches_default_shift_templates seeds default templates.
    sqlx::query(
        "INSERT INTO branches (id, organization_id, name, timezone) VALUES ('BR-FRESH', $1::uuid, 'Fresh Branch', 'Asia/Bangkok')"
    )
    .bind(ORG_1)
    .execute(&h.db.pool)
    .await
    .expect("insert fresh branch");

    let res = h
        .call(
            &h.admin,
            Method::GET,
            "/api/v3/views/branch_shifts?branch=BR-FRESH",
            None,
        )
        .await;

    assert_eq!(res.status, StatusCode::OK, "view failed: {}", res.text);
    let data = &res.body["data"];
    assert_eq!(data["cutover"], "06:00");
    assert_eq!(data["defaultShiftHours"], 8);

    let templates = data["templates"].as_array().expect("templates array");
    assert_eq!(templates.len(), 3);

    assert_eq!(templates[0]["name"], "A");
    assert_eq!(templates[0]["startsAt"], "06:00");
    assert_eq!(templates[0]["endsAt"], "14:00");
    assert!(templates[0]["breakStartsAt"].is_null());
    assert!(templates[0]["breakEndsAt"].is_null());

    assert_eq!(templates[1]["name"], "B");
    assert_eq!(templates[1]["startsAt"], "14:00");
    assert_eq!(templates[1]["endsAt"], "22:00");
    assert!(templates[1]["breakStartsAt"].is_null());
    assert!(templates[1]["breakEndsAt"].is_null());

    assert_eq!(templates[2]["name"], "C");
    assert_eq!(templates[2]["startsAt"], "22:00");
    assert_eq!(templates[2]["endsAt"], "06:00");
    assert!(templates[2]["breakStartsAt"].is_null());
    assert!(templates[2]["breakEndsAt"].is_null());

    h.finish().await;
}
