//! Unit tests for LY-6 (`#1411`) `LoyaltyConfig.service_token` (R3), `rule_publisher` and `HttpLoyaltyRuleWriter` (R4).

use std::time::Duration;

use serde_json::json;
use uuid::Uuid;
use xc_loyalty::application::rule_publisher::{
    LoyaltyRuleError, RuleKind, RuleVersion, prepare_draft_fields,
};
use xc_loyalty::infrastructure::config::LoyaltyConfig;
use xc_loyalty::infrastructure::http_rule_writer::HttpLoyaltyRuleWriter;

const TENANT: &str = "3f7c2a10-6b4e-4d8a-9c15-2e7a8b9d0c41";

#[test]
fn loyalty_config_reads_optional_service_token_and_redacts_it_in_debug() {
    let without_token = LoyaltyConfig::from_raw(
        Some("http://127.0.0.1:9999"),
        Some(TENANT),
        Some("whsec-test-secret"),
        Some("2500"),
        None,
    )
    .expect("valid config without service_token");
    assert_eq!(without_token.service_token, None);
    assert_eq!(without_token.timeout, Duration::from_millis(2500));
    assert!(HttpLoyaltyRuleWriter::from_config(&without_token).is_none());

    let blank_token = LoyaltyConfig::from_raw(
        Some("http://127.0.0.1:9999"),
        Some(TENANT),
        Some("whsec-test-secret"),
        Some("2500"),
        Some("   "),
    )
    .expect("valid config with blank service_token");
    assert_eq!(blank_token.service_token, None);
    assert!(HttpLoyaltyRuleWriter::from_config(&blank_token).is_none());

    let secret_token = "super-secret-bearer-token-ly6";
    let with_token = LoyaltyConfig::from_raw(
        Some("http://127.0.0.1:9999/"),
        Some(TENANT),
        Some("whsec-test-secret"),
        Some("1500"),
        Some(secret_token),
    )
    .expect("valid config with service_token");
    assert_eq!(
        with_token.service_token.as_deref(),
        Some("super-secret-bearer-token-ly6")
    );
    let dbg = format!("{with_token:?}");
    assert!(
        !dbg.contains(secret_token),
        "LoyaltyConfig Debug must never print service_token: {dbg}"
    );
    assert!(
        !dbg.contains("whsec-test-secret"),
        "LoyaltyConfig Debug must never print webhook_secret: {dbg}"
    );

    let writer = HttpLoyaltyRuleWriter::from_config(&with_token)
        .expect("writer built when service_token is present");
    assert_eq!(writer.base_url(), "http://127.0.0.1:9999");
    assert_eq!(writer.timeout(), Duration::from_millis(1500));
    let writer_dbg = format!("{writer:?}");
    assert!(
        !writer_dbg.contains(secret_token),
        "HttpLoyaltyRuleWriter Debug must never print service_token: {writer_dbg}"
    );
}

#[test]
fn prepare_draft_fields_strips_forbidden_keys_and_normalises_decimals_to_strings() {
    let raw = json!({
        "version": 3,
        "basis": "SETTLE_THB",
        "pointsRatio": 0.25,
        "rounding": "FLOOR",
        "minimumBasisAmount": 200,
        "expiryMonths": 18,
        "validFrom": "2027-01-01T00:00:00Z",
        "validTo": null,
        "status": "APPROVED",
        "current": true,
        "isCurrent": true
    });
    let cleaned = prepare_draft_fields(RuleKind::Point, raw.as_object().unwrap());
    assert!(!cleaned.contains_key("status"));
    assert!(!cleaned.contains_key("current"));
    assert!(!cleaned.contains_key("isCurrent"));
    assert_eq!(cleaned.get("pointsRatio"), Some(&json!("0.25")));
    assert_eq!(cleaned.get("minimumBasisAmount"), Some(&json!("200")));
}

#[test]
fn rule_version_from_dto_strips_status_and_approval_metadata_from_content() {
    let id = Uuid::new_v4();
    let dto_pending = json!({
        "id": id.to_string(),
        "tenantId": TENANT,
        "version": 3,
        "basis": "SETTLE_THB",
        "pointsRatio": "0.25",
        "rounding": "FLOOR",
        "minimumBasisAmount": "200.00",
        "expiryMonths": 18,
        "validFrom": "2027-01-01T00:00:00Z",
        "validTo": null,
        "status": "PENDING_APPROVAL",
        "approvedRef": null,
        "approvedAt": null,
        "createdAt": "2026-10-15T00:00:00Z"
    });
    let dto_approved = json!({
        "id": id.to_string(),
        "tenantId": TENANT,
        "version": 3,
        "basis": "SETTLE_THB",
        "pointsRatio": "0.25",
        "rounding": "FLOOR",
        "minimumBasisAmount": "200.00",
        "expiryMonths": 18,
        "validFrom": "2027-01-01T00:00:00Z",
        "validTo": null,
        "status": "APPROVED",
        "approvedRef": "req-123",
        "approvedAt": "2026-10-15T01:00:00Z",
        "createdAt": "2026-10-15T00:00:00Z"
    });
    let v1 = RuleVersion::from_dto(RuleKind::Point, dto_pending).expect("parse pending");
    let v2 = RuleVersion::from_dto(RuleKind::Point, dto_approved).expect("parse approved");
    assert_eq!(v1.id, id);
    assert_eq!(v2.approved_ref.as_deref(), Some("req-123"));
    assert_eq!(
        v1.content, v2.content,
        "approval transition does not change rule content"
    );

    let bad = RuleVersion::from_dto(RuleKind::Tier, json!("not-an-object"));
    assert!(matches!(bad, Err(LoyaltyRuleError::Invalid(_))));
}
