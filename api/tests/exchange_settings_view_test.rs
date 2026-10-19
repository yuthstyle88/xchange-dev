//! Ticket #3401 — `GET /api/v3/views/exchange_settings?branch=<id>` reads the organisation's quote
//! lifetime (`quoteTtlSeconds`) and the branch's approval limit (`approvalLimitThb`).

mod common;
mod quote_common;
mod tx4_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::*;
use serde_json::{Value, json};
use tx4_common::*;
use xc_auth::domain::access::{Permission, Role as AccessRole, RolePermissions};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{SessionReader, Sessions};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const VIEW: &str = "/api/v3/views/exchange_settings?branch=MAIN";
const SETTINGS: &str = "/api/exchange-settings";
const LIMIT: &str = "/api/branches/MAIN/exchange-limit";

struct HeaderSessions;

fn sessions_of(spec: &str) -> Sessions {
    match spec {
        "owner" => Sessions {
            owner: Some(Actor::owner("owner-1")),
            staff: None,
        },
        "teller" => Sessions {
            owner: None,
            staff: Actor::staff(TELLER, Role::Front, BranchId::new("MAIN")),
        },
        _ => Sessions {
            owner: None,
            staff: None,
        },
    }
}

impl SessionReader for HeaderSessions {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { sessions_of(&spec) })
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
}

async fn get(desk: &Desk, uri: &str, who: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), desk.db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(Method::GET)
        .uri(uri)
        .insert_header((header::HOST, HOST));
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn put(desk: &Desk, uri: &str, who: &str, body: &Value) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), desk.db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::default()
        .method(Method::PUT)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", unique("key")))
        .set_payload(body.to_string());
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

#[actix_web::test]
async fn r1_organisation_with_stored_ttl_120_answers_quote_ttl_seconds_120() {
    let desk = Desk::start().await;
    desk.db
        .exec(&format!(
            "UPDATE exchange_settings SET quote_ttl_seconds = 120 WHERE organization_id = '{ORG_1}'"
        ))
        .await;
    let a = get(&desk, VIEW, "owner").await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["data"]["quoteTtlSeconds"], json!(120));
}

#[actix_web::test]
async fn r2_organisation_with_no_exchange_settings_row_answers_quote_ttl_seconds_300() {
    let desk = Desk::start().await;
    desk.db
        .exec(&format!(
            "DELETE FROM exchange_settings WHERE organization_id = '{ORG_1}'"
        ))
        .await;
    let a = get(&desk, VIEW, "owner").await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["data"]["quoteTtlSeconds"], json!(300));
}

#[actix_web::test]
async fn r3_branch_with_limit_50000_answers_approval_limit_thb_50000_00() {
    let desk = Desk::start().await;
    desk.limit("MAIN", "50000").await;
    let a = get(&desk, VIEW, "owner").await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["data"]["approvalLimitThb"], json!("50000.00"));
}

#[actix_web::test]
async fn r4_branch_with_no_limit_row_or_null_answers_approval_limit_thb_null() {
    let desk = Desk::start().await;
    let a = get(&desk, VIEW, "owner").await;
    assert_eq!(a.status, StatusCode::OK, "body {}", a.text);
    assert_eq!(a.body["data"]["approvalLimitThb"], Value::Null);

    desk.limit("MAIN", "NULL").await;
    let b = get(&desk, VIEW, "owner").await;
    assert_eq!(b.status, StatusCode::OK, "body {}", b.text);
    assert_eq!(b.body["data"]["approvalLimitThb"], Value::Null);
}

#[actix_web::test]
async fn r5_caller_without_system_settings_is_refused() {
    assert!(!RolePermissions::of(AccessRole::Teller).contains(&Permission::SystemSettings));
    assert!(RolePermissions::of(AccessRole::Admin).contains(&Permission::SystemSettings));
    let desk = Desk::start().await;
    let a = get(&desk, VIEW, "teller").await;
    assert!(
        a.status.is_client_error() && matches!(a.code(), "roleNotAllowed" | "roleNotAllowedHere"),
        "expected policy refusal for teller, got {} {}",
        a.status,
        a.text
    );
}

#[actix_web::test]
async fn r6_a_save_then_a_read_shows_the_saved_values() {
    let desk = Desk::start().await;
    let s = put(&desk, SETTINGS, "owner", &json!({ "quoteTtlSeconds": 450 })).await;
    assert_eq!(s.status, StatusCode::OK, "save settings: {}", s.text);
    let l = put(
        &desk,
        LIMIT,
        "owner",
        &json!({ "approvalLimitThb": "12345.67" }),
    )
    .await;
    assert_eq!(l.status, StatusCode::OK, "save limit: {}", l.text);

    let a = get(&desk, VIEW, "owner").await;
    assert_eq!(a.status, StatusCode::OK, "read view: {}", a.text);
    assert_eq!(
        a.body["data"],
        json!({
            "quoteTtlSeconds": 450,
            "approvalLimitThb": "12345.67",
        })
    );
}
