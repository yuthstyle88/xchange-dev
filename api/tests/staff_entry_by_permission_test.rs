//! ST-1 (#4528, design `docs/design/blueprint/user-management-rbac.md` §11.8): HTTP tests driving the real
//! `AccountSessionReader` and `SessionService` (never a test `SessionReader`).
//!
//! Covers rows S1, S2, S5, S6, S9, S10, S11, S12, S13, S14, and S15.

#[macro_use]
mod auth_http_common;
mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use actix_web::http::Method;
use actix_web::{App, test};
use auth_http_common::{Call, Desk, TestApp, send};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use xc_auth::domain::access::{
    Actor, Authorizer, Permission, Role, RoleGrants, ScopedRole, Target,
};
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_person};
use xc_testdb::sessions::{SESSION_COOKIE, seed_session};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::SEEDED_XC_ORG_ID;

const B1: &str = "B1";
const B2: &str = "B2";
const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F'];

struct Harness {
    desk: Desk,
    upload_dir: PathBuf,
    app: TestApp,
}

impl Harness {
    async fn start() -> Self {
        let desk = Desk::start().await;
        for (id, code, name) in [(B1, "A01", "สาขาหนึ่ง"), (B2, "Z02", "สาขาสอง")]
        {
            sqlx::query(
                "INSERT INTO branches (id, name, organization_id, code) \
                 VALUES ($1, $2, $3::uuid, $4)",
            )
            .bind(id)
            .bind(name)
            .bind(SEEDED_XC_ORG_ID)
            .bind(code)
            .execute(&desk.pool)
            .await
            .expect("insert branch");
        }
        let upload_dir = std::env::temp_dir().join(xc_testdb::unique_schema_name("st1-uploads"));
        let dir_str = upload_dir.to_str().expect("UTF-8 temp dir");
        let state = AppState::new(
            Arc::new(common::settings(&[("APP_FILES__DIR", dir_str)])),
            desk.pool.clone(),
        );
        let app = TestApp::new(
            test::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state)))
                .await,
            |t| t.to_request(),
        );
        Self {
            desk,
            upload_dir,
            app,
        }
    }

    async fn seed_user_session(
        &self,
        name: &str,
        roles: &[(&str, Option<&str>)],
        session_branch: Option<&str>,
    ) -> (String, String) {
        let mut conn = self.desk.pool.acquire().await.expect("acquire connection");
        let valid_from = DateTime::<Utc>::UNIX_EPOCH;
        let role_rows: Vec<RoleRow<'_>> = roles
            .iter()
            .map(|(role_code, branch_id)| RoleRow {
                role_code,
                branch_id: *branch_id,
                valid_from,
                valid_to: None,
            })
            .collect();
        let user_id = seed_person(&mut conn, SEEDED_XC_ORG_ID, name, &role_rows)
            .await
            .expect("seed person");
        let token = seed_session(
            &mut conn,
            &user_id,
            session_branch,
            Utc::now() + Duration::hours(12),
        )
        .await
        .expect("seed session");
        (user_id, token)
    }

    async fn finish(self) {
        drop(self.app);
        let _ = std::fs::remove_dir_all(&self.upload_dir);
        self.desk.drop_schema().await;
    }
}

fn get_with_session(path: &str, token: Option<&str>) -> Call {
    let mut call = Call {
        method: Some(Method::GET),
        path: path.to_owned(),
        proto: Some("https".to_owned()),
        marker: Some("1".to_owned()),
        ..Call::default()
    };
    if let Some(t) = token {
        call = call.session(t);
    }
    call
}

fn granted_permissions(rows: &[(&str, Option<&str>)], branch: &str) -> Vec<String> {
    let since = DateTime::<Utc>::UNIX_EPOCH;
    let actor = Actor::new(
        "u-oracle",
        SEEDED_XC_ORG_ID,
        rows.iter()
            .map(|(r, b)| {
                ScopedRole::new(
                    Role::parse(r).expect("role"),
                    b.map(BranchId::new),
                    since,
                    None,
                )
            })
            .collect(),
        None,
    );
    let target = Target::new(SEEDED_XC_ORG_ID, Some(BranchId::new(branch)), None);
    let mut out = BTreeSet::new();
    for (r, _) in rows {
        for p in RoleGrants::seed(Role::parse(r).expect("role")) {
            if Authorizer::check(Some(&actor), p, &target, Utc::now()).is_ok() {
                let name = serde_json::to_value::<Permission>(p)
                    .expect("serialize permission")
                    .as_str()
                    .expect("string")
                    .to_owned();
                out.insert(name);
            }
        }
    }
    out.into_iter().collect()
}

/// S1, S2, S5, S6: `staffMe` (`GET /api/v3/views/staff_me?branch=B1`) over the real `AccountSessionReader`.
#[actix_web::test]
async fn s1_s2_s5_s6_staff_me_entry_and_permissions() {
    let h = Harness::start().await;

    // S1: owner (org-wide EXECUTIVE + org-wide ADMIN) gets 200 with permissions from both groups
    let owner_rows = [("EXECUTIVE", None), ("ADMIN", None)];
    let (owner_id, owner_tok) = h.seed_user_session("Somchai", &owner_rows, Some(B1)).await;
    let s1 = send(
        &h.app,
        get_with_session("/api/v3/views/staff_me?branch=B1", Some(&owner_tok)),
    )
    .await;
    assert_eq!(s1.status, 200, "S1 owner staff_me: {}", s1.text);
    let s1_data = &s1.json()["data"];
    assert_eq!(s1_data["userId"], json!(owner_id));
    assert_eq!(s1_data["name"], json!("Somchai"));
    assert_eq!(s1_data["branchId"], json!(B1));
    let s1_perms: Vec<String> = s1_data["permissions"]
        .as_array()
        .expect("permissions array")
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(s1_perms, granted_permissions(&owner_rows, B1));

    // S2: EXECUTIVE-only gets 200
    let exec_rows = [("EXECUTIVE", None)];
    let (exec_id, exec_tok) = h.seed_user_session("ExecOnly", &exec_rows, Some(B1)).await;
    let s2 = send(
        &h.app,
        get_with_session("/api/v3/views/staff_me?branch=B1", Some(&exec_tok)),
    )
    .await;
    assert_eq!(s2.status, 200, "S2 EXECUTIVE-only staff_me: {}", s2.text);
    let s2_data = &s2.json()["data"];
    assert_eq!(s2_data["userId"], json!(exec_id));
    let s2_perms: Vec<String> = s2_data["permissions"]
        .as_array()
        .expect("permissions array")
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(s2_perms, granted_permissions(&exec_rows, B1));

    // S5: CUSTOMER-only gets 401 employeeAuthRequired
    let (_, cust_tok) = h
        .seed_user_session("CustomerOnly", &[("CUSTOMER", Some(B1))], Some(B1))
        .await;
    let s5 = send(
        &h.app,
        get_with_session("/api/v3/views/staff_me?branch=B1", Some(&cust_tok)),
    )
    .await;
    assert_eq!(s5.status, 401, "S5 customer staff_me: {}", s5.text);
    assert_eq!(s5.json()["error"]["code"], json!("employeeAuthRequired"));

    // S6: no cookie gets 401 employeeAuthRequired
    let s6 = send(
        &h.app,
        get_with_session("/api/v3/views/staff_me?branch=B1", None),
    )
    .await;
    assert_eq!(s6.status, 401, "S6 no cookie staff_me: {}", s6.text);
    assert_eq!(s6.json()["error"]["code"], json!("employeeAuthRequired"));

    h.finish().await;
}

/// S9: an owner session on a desk action its groups permit is granted, and an ADMIN-only session on a desk action
/// ADMIN's set lacks gets the permission refusal (`roleNotAllowed` / `PolicyError::RoleNotAllowed`), never
/// `employeeAuthRequired`.
#[actix_web::test]
async fn s9_desk_action_is_decided_by_group_permissions_not_caller_rule() {
    let h = Harness::start().await;
    let (_, owner_tok) = h
        .seed_user_session("Somchai", &[("EXECUTIVE", None), ("ADMIN", None)], Some(B1))
        .await;
    let (_, admin_tok) = h
        .seed_user_session("AdminOnly", &[("ADMIN", None)], Some(B1))
        .await;

    // v3 desk view `staff_holds` (needs CustomerHoldPlace | VerificationReview: owner holds it, ADMIN lacks it)
    let owner_holds = send(
        &h.app,
        get_with_session("/api/v3/views/staff_holds?branch=B1", Some(&owner_tok)),
    )
    .await;
    assert_eq!(
        owner_holds.status, 200,
        "S9 owner on staff_holds: {}",
        owner_holds.text
    );

    let admin_holds = send(
        &h.app,
        get_with_session("/api/v3/views/staff_holds?branch=B1", Some(&admin_tok)),
    )
    .await;
    assert_eq!(
        admin_holds.status,
        PolicyError::RoleNotAllowed.status(),
        "S9 ADMIN on staff_holds gets permission refusal, not 401: {}",
        admin_holds.text
    );
    assert_eq!(admin_holds.json()["error"]["code"], json!("roleNotAllowed"));

    // Compat desk action `adminDeskConfig` on `/api/staff/rpc` (needs ExchangeTransact: owner holds it, ADMIN lacks it)
    let owner_desk_cfg = send(
        &h.app,
        Call::post(
            "/api/staff/rpc",
            json!({ "action": "adminDeskConfig", "payload": {} }),
        )
        .session(&owner_tok),
    )
    .await;
    assert_eq!(
        owner_desk_cfg.status, 200,
        "S9 owner on /api/staff/rpc adminDeskConfig: {}",
        owner_desk_cfg.text
    );

    let admin_desk_cfg = send(
        &h.app,
        Call::post(
            "/api/staff/rpc",
            json!({ "action": "adminDeskConfig", "payload": {} }),
        )
        .session(&admin_tok),
    )
    .await;
    assert_eq!(
        admin_desk_cfg.status,
        PolicyError::FrontOnly.status(),
        "S9 ADMIN on /api/staff/rpc adminDeskConfig: {}",
        admin_desk_cfg.text
    );
    assert_eq!(
        admin_desk_cfg.error(),
        PolicyError::FrontOnly.to_string(),
        "S9 ADMIN gets permission refusal, not EMPLOYEE_AUTH_REQUIRED"
    );

    h.finish().await;
}

/// S10: owner (`EXECUTIVE` org-wide + `ADMIN` org-wide) `POST /api/staff/rpc` `adminSnapshot` with session branch B1
/// gets 200 with the staff view: `employee` is the caller's name, role text `OWNER`, branch B1.
#[actix_web::test]
async fn s10_owner_staff_rpc_admin_snapshot_answers_staff_view_as_owner_at_b1() {
    let h = Harness::start().await;
    let (owner_id, owner_tok) = h
        .seed_user_session("Somchai", &[("EXECUTIVE", None), ("ADMIN", None)], Some(B1))
        .await;
    let s10 = send(
        &h.app,
        Call::post(
            "/api/staff/rpc",
            json!({ "action": "adminSnapshot", "payload": {} }),
        )
        .session(&owner_tok),
    )
    .await;
    assert_eq!(s10.status, 200, "S10 owner adminSnapshot: {}", s10.text);
    let s10_body = s10.json();
    assert_eq!(s10_body["employee"]["id"], json!(owner_id));
    assert_eq!(s10_body["employee"]["name"], json!("Somchai"));
    assert_eq!(s10_body["employee"]["role"], json!("OWNER"));
    assert_eq!(s10_body["employee"]["branchId"], json!(B1));
    h.finish().await;
}

/// S11: EXECUTIVE-only `POST /api/staff/rpc` `adminSnapshot` with session branch B1 gets 200 with the staff view:
/// `employee` is the caller's name, role text `OWNER`, branch B1.
#[actix_web::test]
async fn s11_executive_only_staff_rpc_admin_snapshot_answers_staff_view_as_owner_at_b1() {
    let h = Harness::start().await;
    let (exec_id, exec_tok) = h
        .seed_user_session("ExecOnly", &[("EXECUTIVE", None)], Some(B1))
        .await;
    let s11 = send(
        &h.app,
        Call::post(
            "/api/staff/rpc",
            json!({ "action": "adminSnapshot", "payload": {} }),
        )
        .session(&exec_tok),
    )
    .await;
    assert_eq!(
        s11.status, 200,
        "S11 EXECUTIVE-only adminSnapshot: {}",
        s11.text
    );
    let s11_body = s11.json();
    assert_eq!(s11_body["employee"]["id"], json!(exec_id));
    assert_eq!(s11_body["employee"]["name"], json!("ExecOnly"));
    assert_eq!(s11_body["employee"]["role"], json!("OWNER"));
    assert_eq!(s11_body["employee"]["branchId"], json!(B1));
    h.finish().await;
}

/// S12: EXECUTIVE-only `/api/upload?branch=B2` takes the owner branch and honours `?branch=B2` (even when the session
/// branch is `B1`).
#[actix_web::test]
async fn s12_executive_only_upload_takes_owner_branch_and_honours_branch_query() {
    let h = Harness::start().await;
    let (exec_id, exec_tok) = h
        .seed_user_session("ExecOnly", &[("EXECUTIVE", None)], Some(B1))
        .await;

    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            h.upload_dir.to_str().unwrap(),
        )])),
        h.desk.pool.clone(),
    );
    let raw_app =
        test::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state))).await;
    let req = test::TestRequest::post()
        .uri("/api/upload?branch=B2")
        .insert_header(("host", auth_http_common::HOST))
        .insert_header(("origin", auth_http_common::ORIGIN))
        .insert_header(("x-forwarded-proto", "https"))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("content-type", "image/jpeg"))
        .insert_header(("cookie", format!("{SESSION_COOKIE}={exec_tok}")))
        .set_payload(JPEG)
        .to_request();
    let resp = test::call_service(&raw_app, req).await;
    assert_eq!(resp.status().as_u16(), 200, "S12 upload status");
    let body_bytes = test::read_body(resp).await;
    let upload_json: Value = serde_json::from_slice(&body_bytes).expect("upload json");
    let key = upload_json["uploadKey"]
        .as_str()
        .expect("uploadKey")
        .to_owned();

    let (stored_branch, stored_owner): (String, String) =
        sqlx::query_as("SELECT branch_id, owner_id FROM files WHERE key = $1")
            .bind(&key)
            .fetch_one(&h.desk.pool)
            .await
            .expect("file row");
    assert_eq!(
        stored_branch, B2,
        "S12: EXECUTIVE-only /api/upload?branch=B2 must store in B2, not session branch B1"
    );
    assert_eq!(stored_owner, exec_id);

    h.finish().await;
}

/// S13: EXECUTIVE-only `/files/{key}` reads with `open_owned` (succeeds across branches for its own upload).
#[actix_web::test]
async fn s13_executive_only_files_reads_with_open_owned() {
    let h = Harness::start().await;
    let (_, exec_tok) = h
        .seed_user_session("ExecOnly", &[("EXECUTIVE", None)], Some(B1))
        .await;

    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            h.upload_dir.to_str().unwrap(),
        )])),
        h.desk.pool.clone(),
    );
    let raw_app =
        test::init_service(App::new().configure(move |cfg| bootstrap::configure(cfg, state))).await;

    let req = test::TestRequest::post()
        .uri("/api/upload?branch=B2")
        .insert_header(("host", auth_http_common::HOST))
        .insert_header(("origin", auth_http_common::ORIGIN))
        .insert_header(("x-forwarded-proto", "https"))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("content-type", "image/jpeg"))
        .insert_header(("cookie", format!("{SESSION_COOKIE}={exec_tok}")))
        .set_payload(JPEG)
        .to_request();
    let resp = test::call_service(&raw_app, req).await;
    assert_eq!(resp.status().as_u16(), 200);
    let body_bytes = test::read_body(resp).await;
    let upload_json: Value = serde_json::from_slice(&body_bytes).expect("upload json");
    let key = upload_json["uploadKey"]
        .as_str()
        .expect("uploadKey")
        .to_owned();

    // GET /files/{key} as EXECUTIVE-only (session branch B1) reads via open_owned (finds its own file in B2)
    let get_own = test::TestRequest::get()
        .uri(&format!("/files/{key}"))
        .insert_header(("host", auth_http_common::HOST))
        .insert_header(("x-forwarded-proto", "https"))
        .insert_header(("cookie", format!("{SESSION_COOKIE}={exec_tok}")))
        .to_request();
    let own_resp = test::call_service(&raw_app, get_own).await;
    assert_eq!(
        own_resp.status().as_u16(),
        200,
        "S13: EXECUTIVE-only /files/{{key}} reads own file across branches via open_owned"
    );
    assert_eq!(
        own_resp
            .headers()
            .get("content-disposition")
            .and_then(|v| v.to_str().ok()),
        Some("inline"),
        "S13: open_owned sets Content-Disposition: inline"
    );
    let own_bytes = test::read_body(own_resp).await;
    assert_eq!(own_bytes.as_ref(), JPEG);

    h.finish().await;
}

/// S14 & S15: TELLER@B1 `adminSnapshot` on `/api/staff/rpc` is unchanged (S14), and EXECUTIVE + TELLER@B1 without
/// org-wide ADMIN keeps that staff standing on `/api/staff/rpc` (S15).
#[actix_web::test]
async fn s14_s15_teller_and_executive_plus_teller_keep_staff_standing_on_staff_rpc() {
    let h = Harness::start().await;

    // S14: TELLER@B1 unchanged
    let (teller_id, teller_tok) = h
        .seed_user_session("Nok", &[("TELLER", Some(B1))], Some(B1))
        .await;
    let s14 = send(
        &h.app,
        Call::post(
            "/api/staff/rpc",
            json!({ "action": "adminSnapshot", "payload": {} }),
        )
        .session(&teller_tok),
    )
    .await;
    assert_eq!(s14.status, 200, "S14 TELLER@B1 adminSnapshot: {}", s14.text);
    let s14_body = s14.json();
    assert_eq!(s14_body["employee"]["id"], json!(teller_id));
    assert_eq!(s14_body["employee"]["name"], json!("Nok"));
    assert_eq!(s14_body["employee"]["role"], json!("FRONT"));
    assert_eq!(s14_body["employee"]["branchId"], json!(B1));

    // S15: EXECUTIVE + TELLER@B1 (without org ADMIN) keeps the TELLER staff standing on /api/staff/rpc
    let (combo_id, combo_tok) = h
        .seed_user_session(
            "ExecTeller",
            &[("EXECUTIVE", None), ("TELLER", Some(B1))],
            Some(B1),
        )
        .await;
    let s15 = send(
        &h.app,
        Call::post(
            "/api/staff/rpc",
            json!({ "action": "adminSnapshot", "payload": {} }),
        )
        .session(&combo_tok),
    )
    .await;
    assert_eq!(
        s15.status, 200,
        "S15 EXECUTIVE + TELLER@B1 adminSnapshot: {}",
        s15.text
    );
    let s15_body = s15.json();
    assert_eq!(s15_body["employee"]["id"], json!(combo_id));
    assert_eq!(s15_body["employee"]["name"], json!("ExecTeller"));
    assert_eq!(s15_body["employee"]["role"], json!("FRONT"));
    assert_eq!(s15_body["employee"]["branchId"], json!(B1));

    h.finish().await;
}

/// S5: CUSTOMER-only `POST /api/staff/rpc` `adminSnapshot` gets 401 `employeeAuthRequired`.
#[actix_web::test]
async fn s5_customer_only_staff_rpc_admin_snapshot_gets_401_employee_auth_required() {
    let h = Harness::start().await;
    let (_, cust_tok) = h
        .seed_user_session("CustomerOnly", &[("CUSTOMER", Some(B1))], Some(B1))
        .await;
    let resp = send(
        &h.app,
        Call::post(
            "/api/staff/rpc",
            json!({ "action": "adminSnapshot", "payload": {} }),
        )
        .session(&cust_tok),
    )
    .await;
    assert_eq!(resp.status, 401, "S5 customer adminSnapshot: {}", resp.text);
    assert_eq!(resp.error(), PolicyError::EmployeeAuthRequired.to_string());
    h.finish().await;
}
