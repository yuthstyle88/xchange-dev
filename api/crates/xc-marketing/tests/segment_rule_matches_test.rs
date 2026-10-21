use rust_decimal::Decimal;
use xc_marketing::domain::segment::{metrics_source::CustomerMetrics, rule::SegmentRule};

fn empty_rule() -> SegmentRule {
    SegmentRule {
        tier: None,
        recency_days: None,
        frequency: None,
        value_thb: None,
        home_branch: None,
        lookback_months: None,
    }
}

fn base_metrics() -> CustomerMetrics {
    CustomerMetrics {
        customer_id: "cust-1".to_string(),
        customer_name: "Alice".to_string(),
        recency_days: Some(10),
        frequency: 5,
        value_thb: Decimal::new(500000, 2),
        tier_code: None,
        home_branch: None,
    }
}

#[test]
fn row_1_recency_rule_30_days_matches_30_and_rejects_none() {
    let rule = SegmentRule {
        recency_days: Some(30),
        ..empty_rule()
    };

    let at_boundary = CustomerMetrics {
        recency_days: Some(30),
        ..base_metrics()
    };
    let missing_recency = CustomerMetrics {
        recency_days: None,
        ..base_metrics()
    };

    assert!(rule.matches(&at_boundary));
    assert!(!rule.matches(&missing_recency));
}

#[test]
fn row_2_frequency_2_matches_2() {
    let rule = SegmentRule {
        frequency: Some(2),
        ..empty_rule()
    };
    let metrics = CustomerMetrics {
        frequency: 2,
        ..base_metrics()
    };

    assert!(rule.matches(&metrics));
}

#[test]
fn row_3_value_2000_00_matches_2000_00() {
    let rule = SegmentRule {
        value_thb: Some(Decimal::new(200_000, 2)),
        ..empty_rule()
    };
    let metrics = CustomerMetrics {
        value_thb: Decimal::new(200_000, 2),
        ..base_metrics()
    };

    assert!(rule.matches(&metrics));
}

#[test]
fn row_4_home_branch_main_matches_main_and_rejects_b2_and_none() {
    let rule = SegmentRule {
        home_branch: Some("MAIN".to_string()),
        ..empty_rule()
    };

    let main_customer = CustomerMetrics {
        home_branch: Some("MAIN".to_string()),
        ..base_metrics()
    };
    let b2_customer = CustomerMetrics {
        home_branch: Some("B2".to_string()),
        ..base_metrics()
    };
    let no_branch_customer = CustomerMetrics {
        home_branch: None,
        ..base_metrics()
    };

    assert!(rule.matches(&main_customer));
    assert!(!rule.matches(&b2_customer));
    assert!(!rule.matches(&no_branch_customer));
}
