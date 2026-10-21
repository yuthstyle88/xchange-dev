//! Pure unit tests for `xc_loyalty::application::rule_publisher` helpers (#4009, A4c).

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};
use xc_loyalty::application::rule_publisher::{
    LoyaltyRuleError, RuleKind, RuleVersion, prepare_draft_fields,
};

const VALID_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

fn valid_dto() -> Value {
    json!({
        "id": VALID_ID,
        "version": 3,
        "status": "DRAFT",
        "approvedRef": "APP-1",
        "approvedAt": "2026-10-20T00:00:00Z",
        "createdAt": "2026-10-19T00:00:00Z",
        "basis": "THB",
        "pointsRatio": "0.25"
    })
}

fn as_map(v: Value) -> Map<String, Value> {
    v.as_object().expect("JSON object").clone()
}

// Row 1: RuleKind::parse(k.as_str()) for each kind -> Some(k)
#[test]
fn row_1_rule_kind_parse_round_trips_each_kind() {
    for kind in [RuleKind::Point, RuleKind::Tier] {
        assert_eq!(RuleKind::parse(kind.as_str()), Some(kind));
    }
}

// Row 2: parse("point"), parse(""), parse("TIERS") -> None
#[test]
fn row_2_rule_kind_parse_rejects_lowercase_empty_and_unknown() {
    for bad in ["point", "tier", "", "TIERS", "POINTS", " point "] {
        assert_eq!(RuleKind::parse(bad), None, "expected None for {bad:?}");
    }
}

// Row 3: from_dto with id missing / not a UUID / version missing / version "3" /
// version 2147483648 / status missing -> Err(Invalid…) each
#[test]
fn row_3_rule_version_from_dto_rejects_missing_or_invalid_required_fields() {
    let mut no_id = valid_dto();
    no_id.as_object_mut().unwrap().remove("id");

    let mut bad_id = valid_dto();
    bad_id["id"] = json!("not-a-uuid");

    let mut no_version = valid_dto();
    no_version.as_object_mut().unwrap().remove("version");

    let mut str_version = valid_dto();
    str_version["version"] = json!("3");

    let mut overflow_version = valid_dto();
    overflow_version["version"] = json!(2_147_483_648_i64);

    let mut no_status = valid_dto();
    no_status.as_object_mut().unwrap().remove("status");

    for (label, dto) in [
        ("id missing", no_id),
        ("id not a UUID", bad_id),
        ("version missing", no_version),
        ("version as string \"3\"", str_version),
        ("version 2147483648 (i32::MAX + 1)", overflow_version),
        ("status missing", no_status),
    ] {
        let res = RuleVersion::from_dto(RuleKind::Point, dto);
        assert!(
            matches!(res, Err(LoyaltyRuleError::Invalid(_))),
            "{label}: expected Err(Invalid(_)), got {res:?}"
        );
    }
}

// Row 4: two DTOs equal except createdAt and approvedAt (and status/approvedRef) -> equal content
#[test]
fn row_4_non_content_keys_are_stripped_from_rule_version_content() {
    let mut dto_a = valid_dto();
    dto_a["createdAt"] = json!("2026-01-01T00:00:00Z");
    dto_a["approvedAt"] = json!("2026-01-02T00:00:00Z");
    dto_a["status"] = json!("DRAFT");
    dto_a["approvedRef"] = json!("REF-A");

    let mut dto_b = valid_dto();
    dto_b["createdAt"] = json!("2026-12-31T23:59:59Z");
    dto_b["approvedAt"] = json!("2026-12-31T23:59:59Z");
    dto_b["status"] = json!("ACTIVE");
    dto_b["approvedRef"] = json!("REF-B");

    let ver_a = RuleVersion::from_dto(RuleKind::Point, dto_a).expect("valid dto_a");
    let ver_b = RuleVersion::from_dto(RuleKind::Point, dto_b).expect("valid dto_b");

    assert_eq!(ver_a.content, ver_b.content);
    let content_obj = ver_a.content.as_object().expect("content is an object");
    for stripped in ["status", "approvedRef", "approvedAt", "createdAt"] {
        assert!(
            !content_obj.contains_key(stripped),
            "content must not contain non-content key {stripped:?}"
        );
    }
}

// Row 5: content of a parsed version still holds id and version
#[test]
fn row_5_parsed_rule_version_content_retains_id_and_version() {
    let ver = RuleVersion::from_dto(RuleKind::Point, valid_dto()).expect("valid dto");
    assert_eq!(ver.content["id"], json!(VALID_ID));
    assert_eq!(ver.content["version"], json!(3));
}

// Row 6: prepare_draft_fields(Tier, {rollingMonths, tiers, grace, basis, pointsRatio: 0.25, status})
// -> only the Tier keys remain
#[test]
fn row_6_prepare_draft_fields_for_tier_keeps_only_tier_allowlist_keys() {
    let input = as_map(json!({
        "rollingMonths": 12,
        "tiers": [{"code": "GOLD", "minAmount": "10000"}],
        "grace": 1,
        "basis": "THB",
        "pointsRatio": 0.25,
        "status": "DRAFT"
    }));
    let out = prepare_draft_fields(RuleKind::Tier, &input);
    assert_eq!(
        out,
        as_map(json!({
            "rollingMonths": 12,
            "tiers": [{"code": "GOLD", "minAmount": "10000"}],
            "grace": 1
        }))
    );
}

// Row 7: prepare_draft_fields(Point, {expiryMonths: 18, …}) -> expiryMonths is still a JSON number 18
#[test]
fn row_7_prepare_draft_fields_for_point_keeps_expiry_months_as_number() {
    let input = as_map(json!({
        "version": 2,
        "expiryMonths": 18,
        "basis": "THB",
        "pointsRatio": 0.25,
        "minimumBasisAmount": 100
    }));
    let out = prepare_draft_fields(RuleKind::Point, &input);
    assert_eq!(out.get("expiryMonths"), Some(&json!(18)));
    assert_eq!(out.get("version"), Some(&json!(2)));
    assert_eq!(out.get("pointsRatio"), Some(&json!("0.25")));
    assert_eq!(out.get("minimumBasisAmount"), Some(&json!("100")));
}

// Row 8: prepare_draft_fields(Point, {pointsRatio: "0.25"}) -> passes through as the string "0.25"
#[test]
fn row_8_prepare_draft_fields_for_point_passes_string_points_ratio_through() {
    let input = as_map(json!({
        "pointsRatio": "0.25",
        "minimumBasisAmount": "50.00"
    }));
    let out = prepare_draft_fields(RuleKind::Point, &input);
    assert_eq!(out.get("pointsRatio"), Some(&json!("0.25")));
    assert_eq!(out.get("minimumBasisAmount"), Some(&json!("50.00")));
}

// Row 9: client_message() for every LoyaltyRuleError variant -> its exact string, each different
#[test]
fn row_9_client_message_returns_distinct_exact_string_per_variant() {
    let cases = [
        (
            LoyaltyRuleError::Unavailable("upstream timeout".to_owned()),
            "the loyalty service is not available, try again later",
        ),
        (
            LoyaltyRuleError::Invalid("bad field".to_owned()),
            "the loyalty service refused the rule",
        ),
        (LoyaltyRuleError::NotFound, "loyalty rule version not found"),
        (
            LoyaltyRuleError::InvalidTransition("ACTIVE -> DRAFT".to_owned()),
            "the loyalty rule version cannot move from its current status",
        ),
    ];
    let mut seen = BTreeSet::new();
    for (err, expected) in &cases {
        let msg = err.client_message();
        assert_eq!(msg, *expected, "wrong client_message for {err:?}");
        assert!(seen.insert(msg), "duplicate client_message {msg:?}");
    }
    assert_eq!(seen.len(), 4);
}
