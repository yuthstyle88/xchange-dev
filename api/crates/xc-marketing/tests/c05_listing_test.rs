//! Up-front tests for C05 `my_benefits`, the pure core (design `marketing-campaigns.md` §6.3; O-3 decided on
//! #564 issuecomment-6262478258). Written before the implementation by claude/studio-claude-1 on the BE lane
//! controller claude/a90ab16c's order. Do not edit, delete or weaken.
//!
//! Behaviours locked (`xc_marketing::domain::benefit::listing`):
//! - channel: absent or containing APP ⇒ shown; DESK-only ⇒ hidden;
//! - tier: listed tier ⇒ shown; other tier or no tier ⇒ hidden;
//! - segment: in the version's snapshot AND MARKETING consent ⇒ shown; either missing ⇒ hidden;
//! - referred: only a referred customer sees a `referred` rule;
//! - a rule that needs a coupon is never shown;
//! - line conditions (branches, currencies, side, min THB) never hide a rule, and `line_conditions` returns them;
//! - `summary` reads the calculator's params as decimals and refuses what the desk refuses.

use std::str::FromStr;

use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_kernel::money::Side;
use xc_marketing::domain::benefit::eligibility::Eligibility;
use xc_marketing::domain::benefit::listing::{
    CustomerFacts, LineConditions, line_conditions, shown_on_app, summary,
};
use xc_marketing::domain::benefit::registry::CalculatorRegistry;
use xc_marketing::domain::benefit::rule::{BenefitCaps, BenefitClass, BenefitRule};

const CUSTOMER: &str = "cust-c05";

fn rule(eligibility: Value, calculation: Value, requires_coupon: bool) -> BenefitRule {
    BenefitRule {
        id: Uuid::new_v4(),
        campaign_version_id: Uuid::new_v4(),
        benefit_type: "RATE_IMPROVEMENT".to_string(),
        benefit_class: BenefitClass::Price,
        eligibility: serde_json::from_value::<Eligibility>(eligibility).expect("eligibility"),
        calculation,
        caps: BenefitCaps::default(),
        requires_coupon,
        rule_no: 1,
    }
}

fn open_rule(eligibility: Value) -> BenefitRule {
    rule(eligibility, json!({ "delta": "0.05" }), false)
}

fn facts<'a>(
    tier: Option<&'a str>,
    segments: &'a [Uuid],
    consent: bool,
    referred: bool,
) -> CustomerFacts<'a> {
    CustomerFacts {
        customer_id: CUSTOMER,
        tier,
        segment_memberships: segments,
        marketing_consent: consent,
        is_referred: referred,
    }
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

#[test]
fn a_rule_with_no_channel_or_with_app_is_shown_and_a_desk_only_rule_is_not() {
    let plain = facts(None, &[], false, false);
    assert!(
        shown_on_app(&open_rule(json!({})), None, &plain),
        "no channels = any channel"
    );
    assert!(shown_on_app(
        &open_rule(json!({ "channels": ["APP"] })),
        None,
        &plain
    ));
    assert!(shown_on_app(
        &open_rule(json!({ "channels": ["DESK", "APP"] })),
        None,
        &plain
    ));
    assert!(
        !shown_on_app(&open_rule(json!({ "channels": ["DESK"] })), None, &plain),
        "a DESK-only rule is not an app benefit"
    );
    assert!(!shown_on_app(
        &open_rule(json!({ "channels": ["KIOSK"] })),
        None,
        &plain
    ));
}

#[test]
fn a_tier_rule_is_shown_only_to_a_customer_of_a_listed_tier() {
    let r = open_rule(json!({ "tiers": ["GOLD", "PLATINUM"] }));
    assert!(shown_on_app(
        &r,
        None,
        &facts(Some("GOLD"), &[], false, false)
    ));
    assert!(!shown_on_app(
        &r,
        None,
        &facts(Some("SILVER"), &[], false, false)
    ));
    assert!(
        !shown_on_app(&r, None, &facts(None, &[], false, false)),
        "no mirrored tier never matches a tier rule"
    );
}

#[test]
fn a_segment_rule_needs_the_versions_snapshot_and_marketing_consent() {
    let snapshot = Uuid::new_v4();
    let other_snapshot = Uuid::new_v4();
    let r = open_rule(json!({ "segment": true }));
    let member = [snapshot];
    let elsewhere = [other_snapshot];

    assert!(shown_on_app(
        &r,
        Some(snapshot),
        &facts(None, &member, true, false)
    ));
    assert!(
        !shown_on_app(&r, Some(snapshot), &facts(None, &member, false, false)),
        "a withdrawn or missing MARKETING consent stops segment eligibility (D-13)"
    );
    assert!(
        !shown_on_app(&r, Some(snapshot), &facts(None, &elsewhere, true, false)),
        "a member of another snapshot is not in this version's segment"
    );
    assert!(
        !shown_on_app(&r, None, &facts(None, &member, true, false)),
        "a segment rule on a version with no snapshot matches nobody"
    );
}

#[test]
fn a_referred_rule_is_shown_only_to_a_referred_customer() {
    let r = open_rule(json!({ "referred": true }));
    assert!(shown_on_app(&r, None, &facts(None, &[], false, true)));
    assert!(!shown_on_app(&r, None, &facts(None, &[], false, false)));
}

#[test]
fn a_rule_that_needs_a_coupon_is_never_shown_as_an_open_benefit() {
    let by_flag = rule(json!({}), json!({ "delta": "0.05" }), true);
    let by_eligibility = open_rule(json!({ "requiresCoupon": true }));
    let everyone = facts(Some("GOLD"), &[], true, true);
    assert!(
        !shown_on_app(&by_eligibility, None, &everyone),
        "eligibility.requiresCoupon"
    );
    assert!(
        !shown_on_app(&by_flag, None, &everyone),
        "benefit_rules.requires_coupon alone also hides it"
    );
    let both = rule(
        json!({ "requiresCoupon": true }),
        json!({ "delta": "0.05" }),
        true,
    );
    assert!(!shown_on_app(&both, None, &everyone));
}

#[test]
fn line_conditions_never_hide_a_rule_and_are_returned_for_the_screen() {
    let r = open_rule(json!({
        "branches": ["B-MAIN"],
        "currencies": ["USD", "EUR"],
        "shopSide": "BUY",
        "minThb": "1000.00",
        "channels": ["APP"],
    }));
    assert!(
        shown_on_app(&r, None, &facts(None, &[], false, false)),
        "branch, currency, side and min THB need a transaction line and are not evaluated in the app"
    );
    assert_eq!(
        line_conditions(&r.eligibility),
        LineConditions {
            min_thb: Some(dec("1000.00")),
            currencies: Some(vec!["USD".to_string(), "EUR".to_string()]),
            shop_side: Some(Side::Buy),
        }
    );
    assert_eq!(
        line_conditions(&open_rule(json!({})).eligibility),
        LineConditions::default()
    );
}

#[test]
fn summary_reads_the_calculators_params_as_decimals() {
    let registry = CalculatorRegistry::new();

    let rate = summary(
        &rule(json!({}), json!({ "delta": "0.05" }), false),
        &registry,
    )
    .expect("rate");
    assert_eq!(rate.benefit_type, "RATE_IMPROVEMENT");
    assert_eq!(rate.benefit_class, BenefitClass::Price);
    assert_eq!(rate.delta, Some(dec("0.05")));
    assert_eq!((rate.percent, rate.amount, rate.fixed), (None, None, None));

    let mut fee = rule(json!({}), json!({ "percent": "50" }), false);
    fee.benefit_type = "FEE_WAIVER".to_string();
    let fee = summary(&fee, &registry).expect("fee");
    assert_eq!(fee.benefit_type, "FEE_WAIVER");
    assert_eq!(fee.percent, Some(dec("50")));
    assert_eq!(fee.delta, None);
}

#[test]
fn summary_refuses_an_unknown_type_and_params_the_desk_refuses() {
    let registry = CalculatorRegistry::new();

    let mut unknown = rule(json!({}), json!({ "delta": "0.05" }), false);
    unknown.benefit_type = "NO_SUCH_KIND".to_string();
    let err = summary(&unknown, &registry).expect_err("unknown type");
    assert_eq!(err.field, "benefitType");

    let number = rule(json!({}), json!({ "delta": 0.05 }), false);
    assert!(
        summary(&number, &registry).is_err(),
        "a JSON number is not a decimal string (D-6a), so it is refused, never read as a value"
    );

    let none = rule(json!({}), json!({}), false);
    assert!(
        summary(&none, &registry).is_err(),
        "params with none of the kind's keys are refused"
    );
}
