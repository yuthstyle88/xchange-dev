//! Ticket #4036 (A5b): ApprovalError::code variant coverage and Redactor snake-case / short values.

use serde_json::json;
use xc_approval::application::approval_service::ApprovalError;
use xc_approval::domain::redactor::Redactor;

fn check_exhaustive_approval_error(err: &ApprovalError) {
    match err {
        ApprovalError::OutOfScope(_) => {}
        ApprovalError::StaleVersion(_) => {}
        ApprovalError::IdempotencyConflict(_) => {}
        ApprovalError::PolicyRefused { .. } => {}
        ApprovalError::DependencyUnavailable { .. } => {}
        ApprovalError::AlreadyPending(_) => {}
        ApprovalError::NotFound(_) => {}
        ApprovalError::UnknownType(_) => {}
        ApprovalError::Database(_) => {}
        ApprovalError::Internal(_) => {}
    }
}

#[test]
fn row1_policy_refused_code_passthrough() {
    let err = ApprovalError::PolicyRefused {
        code: "approvalRequired",
        message: "approval required for cash transfer".to_string(),
    };
    check_exhaustive_approval_error(&err);
    assert_eq!(err.code(), "approvalRequired");
}

#[test]
fn row2_dependency_unavailable_code_passthrough() {
    let err = ApprovalError::DependencyUnavailable {
        code: "loyaltyUnavailable",
        message: "loyalty service is offline".to_string(),
    };
    check_exhaustive_approval_error(&err);
    assert_eq!(err.code(), "loyaltyUnavailable");
}

#[test]
fn row3_fixed_approval_error_codes() {
    let cases = [
        (ApprovalError::OutOfScope("doc".to_string()), "outOfScope"),
        (
            ApprovalError::StaleVersion("v1".to_string()),
            "staleVersion",
        ),
        (
            ApprovalError::IdempotencyConflict("idem-1".to_string()),
            "idempotencyConflict",
        ),
        (
            ApprovalError::AlreadyPending("obj-1".to_string()),
            "alreadyPending",
        ),
        (ApprovalError::NotFound("req-1".to_string()), "notFound"),
        (
            ApprovalError::UnknownType("UNKNOWN".to_string()),
            "unknownType",
        ),
        (
            ApprovalError::Database("db conn failed".to_string()),
            "databaseError",
        ),
        (
            ApprovalError::Internal("unexpected".to_string()),
            "internalError",
        ),
    ];

    for (err, expected_code) in &cases {
        check_exhaustive_approval_error(err);
        assert_eq!(err.code(), *expected_code);
    }
}

#[test]
fn row4_passport_no_snake_case_masked() {
    let input = json!({
        "passport_no": "AB123456"
    });
    let redacted = Redactor::redact(&input);
    assert_eq!(redacted["passport_no"], "***3456");
}

#[test]
fn row5_id_number_snake_case_masked() {
    let input = json!({
        "id_number": "110370012345"
    });
    let redacted = Redactor::redact(&input);
    assert_eq!(redacted["id_number"], "***2345");
}

#[test]
fn row6_short_masked_value_no_panic() {
    let input = json!({
        "phone": "123"
    });
    let redacted = Redactor::redact(&input);
    assert_eq!(redacted["phone"], "***123");

    // Also verify 0, 1, and 2 chars with other masked keys
    let input_short = json!({
        "passport_no": "12",
        "id_number": "",
    });
    let redacted_short = Redactor::redact(&input_short);
    assert_eq!(redacted_short["passport_no"], "***12");
    assert_eq!(redacted_short["id_number"], "***");
}
