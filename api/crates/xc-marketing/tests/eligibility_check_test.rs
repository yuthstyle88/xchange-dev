use rust_decimal::Decimal;
use xc_kernel::money::Side;
use xc_marketing::domain::benefit::eligibility::{Eligibility, EligibilityContext, SkipReason};

#[test]
fn row_1_default_rule_and_context() {
    let rule = Eligibility::default();
    let ctx = EligibilityContext::default();
    assert_eq!(rule.check(&ctx), Ok(()));
}

#[test]
fn row_2_min_thb_boundary_equal() {
    let rule = Eligibility {
        min_thb: Some(Decimal::from(3000)),
        ..Default::default()
    };
    let ctx = EligibilityContext {
        thb_amount: Some(Decimal::from(3000)),
        ..Default::default()
    };
    assert_eq!(rule.check(&ctx), Ok(()));
}

#[test]
fn row_3_min_thb_missing_amount() {
    let rule = Eligibility {
        min_thb: Some(Decimal::from(3000)),
        ..Default::default()
    };
    let ctx = EligibilityContext {
        thb_amount: None,
        ..Default::default()
    };
    assert_eq!(rule.check(&ctx), Err(SkipReason::MinThbNotMet));
}

#[test]
fn row_4_channel_check() {
    let rule = Eligibility {
        channels: Some(vec!["KIOSK".to_string()]),
        ..Default::default()
    };
    let ctx_kiosk = EligibilityContext {
        channel: Some("KIOSK"),
        ..Default::default()
    };
    assert_eq!(rule.check(&ctx_kiosk), Ok(()));

    let ctx_desk = EligibilityContext {
        channel: Some("DESK"),
        ..Default::default()
    };
    assert_eq!(rule.check(&ctx_desk), Err(SkipReason::ChannelMismatch));

    // The ticket's second case: no channel at all is refused too.
    let ctx_none = EligibilityContext {
        channel: None,
        ..Default::default()
    };
    assert_eq!(rule.check(&ctx_none), Err(SkipReason::ChannelMismatch));
}

#[test]
fn row_5_shop_side_missing() {
    let rule = Eligibility {
        shop_side: Some(Side::Buy),
        ..Default::default()
    };
    let ctx = EligibilityContext {
        shop_side: None,
        ..Default::default()
    };
    assert_eq!(rule.check(&ctx), Err(SkipReason::SideMismatch));
}

#[test]
fn row_6_tier_check() {
    let rule = Eligibility {
        tiers: Some(vec!["GOLD".to_string()]),
        ..Default::default()
    };
    let ctx = EligibilityContext {
        customer_id: Some("C123"),
        tier: Some("GOLD"),
        ..Default::default()
    };
    assert_eq!(rule.check(&ctx), Ok(()));
}

#[test]
fn row_7_coupon_referred_segment_flags() {
    // Restated for #4139 by the BE-R6 sub-controller (claude/studio-claude-8): "requires a coupon" is the rule's
    // `requires_coupon` column, checked by the engine (`benefit_engine_test.rs`
    // `without_a_coupon_the_column_alone_decides`), not an `Eligibility` field; this row keeps the referred and
    // segment flags.
    let rule = Eligibility {
        referred: true,
        segment: true,
        ..Default::default()
    };
    let ctx_ok = EligibilityContext {
        is_referred: true,
        customer_id: Some("C123"),
        in_segment: true,
        marketing_consent: true,
        ..Default::default()
    };
    assert_eq!(rule.check(&ctx_ok), Ok(()));

    let ctx_not_referred = EligibilityContext {
        is_referred: false,
        ..ctx_ok.clone()
    };
    assert_eq!(rule.check(&ctx_not_referred), Err(SkipReason::NotReferred));

    let ctx_not_in_segment = EligibilityContext {
        in_segment: false,
        ..ctx_ok.clone()
    };
    assert_eq!(
        rule.check(&ctx_not_in_segment),
        Err(SkipReason::NotInSegment)
    );
}
