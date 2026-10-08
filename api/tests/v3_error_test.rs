//! Specification tests for V0-B (docs/design/xchange-v3-web.md §4.3, §9): a v3
//! refusal names its field.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! `presentation::http::v3::error::V3Error` is an `actix_web::ResponseError`:
//! `{"error":{"code","field","message"}}` with the status beside it, `field`
//! only when one field is wrong. Every `PolicyError` becomes one, keeping the
//! policy's status and sentence.
//!
//! No test here needs a database.

use std::collections::HashSet;

use actix_web::ResponseError;
use actix_web::body::MessageBody;
use actix_web::http::{StatusCode, header};
use serde_json::{Value, json};
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xchange::presentation::http::v3::error::V3Error;

fn body_of(e: &V3Error) -> (StatusCode, String, Option<String>, Value) {
    let resp = e.error_response();
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().expect("text").to_owned())
        .unwrap_or_default();
    let cache = resp
        .headers()
        .get(header::CACHE_CONTROL)
        .map(|v| v.to_str().expect("text").to_owned());
    let bytes = resp
        .into_body()
        .try_into_bytes()
        .unwrap_or_else(|_| panic!("a body in one piece"));
    let json = serde_json::from_slice(&bytes).expect("a JSON body");
    (status, content_type, cache, json)
}

#[test]
fn a_refusal_with_a_field_serialises_code_field_and_message_with_its_status() {
    let message = "รหัสผ่านต้องมี 12–128 ตัวอักษร";
    let e = V3Error::new(
        "employeePasswordLength",
        Some("password"),
        message,
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(e.status_code(), StatusCode::BAD_REQUEST);
    let (status, content_type, cache, body) = body_of(&e);
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        content_type.starts_with("application/json"),
        "Content-Type {content_type:?}"
    );
    assert_eq!(cache.as_deref(), Some("no-store"));
    assert_eq!(
        body,
        json!({ "error": { "code": "employeePasswordLength", "field": "password", "message": message } })
    );
}

#[test]
fn a_refusal_without_a_field_has_no_field_key() {
    let e = V3Error::new("signIn", None, "กรุณาเข้าสู่ระบบ", StatusCode::UNAUTHORIZED);
    let (status, _, _, body) = body_of(&e);
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        body,
        json!({ "error": { "code": "signIn", "message": "กรุณาเข้าสู่ระบบ" } })
    );
}

fn all_policy_errors() -> Vec<PolicyError> {
    vec![
        PolicyError::SignIn,
        PolicyError::OwnerOnly,
        PolicyError::EmployeeAuthRequired,
        PolicyError::InvalidRequest,
        PolicyError::RoleNotAllowed,
        PolicyError::RoleNotAllowedHere,
        PolicyError::OwnerEnables,
        PolicyError::FrontOnly,
        PolicyError::DocumentsNotReadable,
        PolicyError::FilesNotReadable,
        PolicyError::UploadNotAllowed,
        PolicyError::NotFound,
        PolicyError::DeviceStepNotAllowed,
        PolicyError::KioskOwnerOrRates,
        PolicyError::KioskStaff,
        PolicyError::KioskUnknown,
        PolicyError::SignNotAllowed,
        PolicyError::BackendUsesCommit,
        PolicyError::EmployeeAccount,
        PolicyError::ClockInFirst,
        PolicyError::PaymentOwnerOnly,
        PolicyError::DrawerRoleNotAllowed,
    ]
}

#[test]
fn every_policy_refusal_keeps_its_status_and_sentence_under_a_code_of_its_own() {
    let mut codes = HashSet::new();
    for p in all_policy_errors() {
        let e = V3Error::from(p.clone());
        let (status, _, _, body) = body_of(&e);
        assert_eq!(status.as_u16(), p.status(), "{p:?}: status");
        assert_eq!(body["error"]["message"], p.to_string(), "{p:?}: sentence");
        assert!(body["error"].get("field").is_none(), "{p:?}: no field");
        let code = body["error"]["code"].as_str().expect("a code").to_owned();
        assert!(
            code.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && code.chars().all(|c| c.is_ascii_alphanumeric()),
            "{p:?}: camelCase code, got {code:?}"
        );
        assert!(
            codes.insert(code.clone()),
            "{p:?}: code {code:?} used twice"
        );
    }
}

#[test]
fn a_policy_code_is_its_variant_name_in_camel_case() {
    for (p, code) in [
        (PolicyError::SignIn, "signIn"),
        (PolicyError::EmployeeAuthRequired, "employeeAuthRequired"),
        (PolicyError::RoleNotAllowedHere, "roleNotAllowedHere"),
        (PolicyError::FrontOnly, "frontOnly"),
    ] {
        let (_, _, _, body) = body_of(&V3Error::from(p.clone()));
        assert_eq!(body["error"]["code"], code, "{p:?}");
    }
}
