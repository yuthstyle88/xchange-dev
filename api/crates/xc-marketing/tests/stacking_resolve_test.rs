//! Determinism tests for `Stacking::resolve` tie-breaks on `version_id` and `rule.id` (#4100).

use std::str::FromStr;

use chrono::{DateTime, TimeZone, Utc};
use serde_json::json;
use uuid::Uuid;

use xc_marketing::domain::benefit::eligibility::Eligibility;
use xc_marketing::domain::benefit::rule::{BenefitCaps, BenefitClass, BenefitRule};
use xc_marketing::domain::engine::active_rules::ActiveRule;
use xc_marketing::domain::engine::stacking::Stacking;

fn fixed_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 21, 0, 0, 0).unwrap()
}

fn make_rule(rule_id: &str, version_id: &str, stack_group: &str) -> ActiveRule {
    let r_id = Uuid::from_str(rule_id).unwrap();
    let v_id = Uuid::from_str(version_id).unwrap();
    ActiveRule {
        rule: BenefitRule {
            id: r_id,
            campaign_version_id: v_id,
            benefit_type: "RATE_IMPROVEMENT".to_string(),
            benefit_class: BenefitClass::Price,
            eligibility: Eligibility::default(),
            calculation: json!({ "delta": "0.10" }),
            caps: BenefitCaps::default(),
            requires_coupon: false,
            rule_no: 1,
        },
        stack_group: stack_group.to_string(),
        priority: 10,
        version_id: v_id,
        version_created_at: fixed_time(),
        segment_snapshot_id: None,
    }
}

fn winner_ids(rules: &[ActiveRule]) -> Vec<Uuid> {
    Stacking::resolve(rules)
        .into_iter()
        .map(|r| r.rule.id)
        .collect()
}

#[test]
fn row_1_empty_input_returns_empty() {
    let resolved = Stacking::resolve(&[]);
    assert!(resolved.is_empty());
}

#[test]
fn row_2_same_group_equal_priority_and_created_at_tie_breaks_deterministically() {
    // Different version_ids in the same stack_group with equal priority and version_created_at.
    let a = make_rule(
        "00000000-0000-0000-0000-000000000010",
        "00000000-0000-0000-0000-000000000001",
        "same-group",
    );
    let b = make_rule(
        "00000000-0000-0000-0000-000000000020",
        "00000000-0000-0000-0000-000000000002",
        "same-group",
    );

    let ab = winner_ids(&[a.clone(), b.clone()]);
    let ba = winner_ids(&[b, a]);
    assert_eq!(ab.len(), 1);
    assert_eq!(ab, ba);

    // Equal version_id as well, differing only in rule.id (line 29 tie-break).
    let c = make_rule(
        "00000000-0000-0000-0000-000000000001",
        "00000000-0000-0000-0000-000000000099",
        "same-group",
    );
    let d = make_rule(
        "00000000-0000-0000-0000-000000000002",
        "00000000-0000-0000-0000-000000000099",
        "same-group",
    );

    let cd = winner_ids(&[c.clone(), d.clone()]);
    let dc = winner_ids(&[d, c]);
    assert_eq!(cd.len(), 1);
    assert_eq!(cd, dc);
}

#[test]
fn row_3_two_price_rules_in_different_groups_resolve_independent_of_hashmap_order() {
    let a = make_rule(
        "00000000-0000-0000-0000-000000000010",
        "00000000-0000-0000-0000-000000000001",
        "group-alpha",
    );
    let b = make_rule(
        "00000000-0000-0000-0000-000000000020",
        "00000000-0000-0000-0000-000000000002",
        "group-beta",
    );

    let expected = winner_ids(&[a.clone(), b.clone()]);
    assert_eq!(expected.len(), 1);

    // Each call constructs a fresh HashMap with a new RandomState seed; all runs must match.
    for _ in 0..32 {
        assert_eq!(winner_ids(&[a.clone(), b.clone()]), expected);
        assert_eq!(winner_ids(&[b.clone(), a.clone()]), expected);
    }
}
