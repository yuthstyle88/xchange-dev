//! Up-front tests for BQ-1 (design `docs/design/blueprint/binding-quote.md` D-3, D-10, D-14, D-15, D-17; §4.2 Q1,
//! Q16, Q17, Q18, Q19, Q20, Q25, Q26 at the domain level). Written by the sub-controller claude/644b3958 on
//! `feat/binding-quote` before the implementation. **Do not edit, delete or weaken.**
//!
//! Pure: `Quote::is_valid` by type and `BindingQuotePolicy`. No database.

use chrono::{DateTime, Duration, TimeZone, Utc};
use pos_error::AppError;
use serde_json::json;
use uuid::Uuid;
use xc_exchange::domain::quote::binding_policy::{
    BindingQuotePolicy, DESK_ONLY, NOT_ENABLED, NOT_VERIFIED,
};
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel, QuoteDisplayStatus, QuoteStatus};
use xc_exchange::domain::quote::quote_type::QuoteType;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Money;

fn at(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 27, 12, 0, second).unwrap()
}

fn issued_under() -> Uuid {
    Uuid::from_u128(1)
}

fn published_since() -> Uuid {
    Uuid::from_u128(2)
}

fn quote(quote_type: QuoteType, expires_at: DateTime<Utc>) -> Quote {
    Quote {
        id: Uuid::from_u128(10),
        organization_id: Uuid::from_u128(20),
        branch_id: BranchId::new("MAIN"),
        customer_id: Some("CU-1".to_string()),
        quote_type,
        channel: QuoteChannel::Desk,
        status: QuoteStatus::Active,
        publication_id: issued_under(),
        expires_at,
        benefit_snapshot: json!({}),
        created_by: "teller-1".to_string(),
        accepted_by_transaction_id: None,
        created_at: at(0),
        updated_at: at(0),
        lines: vec![],
        stale: false,
        cash_settlement: false,
    }
}

fn on() -> BindingQuotePolicy {
    BindingQuotePolicy {
        ttl: Some(Duration::seconds(600)),
    }
}

fn money(text: &str) -> Money {
    Money::parse(text).expect("an amount")
}

fn refused_with<T: std::fmt::Debug>(got: pos_error::AppResult<T>, text: &str) {
    match got {
        Err(AppError::Domain(m)) => assert_eq!(m, text),
        other => panic!("expected Domain({text:?}), got {other:?}"),
    }
}

// ---- D-3: validity by type ----

#[test]
fn q17_a_binding_quote_stays_valid_after_a_new_publication_until_expires_at() {
    let q = quote(QuoteType::Binding, at(30));
    assert!(q.is_valid(at(29), published_since()));
    assert_eq!(
        q.status_at(at(29), published_since()),
        QuoteDisplayStatus::Active
    );
}

#[test]
fn q17_a_binding_quote_ends_at_expires_at_whatever_is_published() {
    let q = quote(QuoteType::Binding, at(30));
    assert!(!q.is_valid(at(30), issued_under()));
    assert!(!q.is_valid(at(30), published_since()));
    assert_eq!(
        q.status_at(at(31), issued_under()),
        QuoteDisplayStatus::Expired
    );
}

#[test]
fn q18_an_indicative_quote_ends_on_a_new_publication_as_today() {
    let q = quote(QuoteType::Indicative, at(30));
    assert!(q.is_valid(at(29), issued_under()));
    assert!(!q.is_valid(at(29), published_since()));
    assert!(!q.is_valid(at(30), issued_under()));
}

#[test]
fn quote_type_round_trips_its_stored_text() {
    for (t, text) in [
        (QuoteType::Indicative, "INDICATIVE"),
        (QuoteType::Binding, "BINDING"),
    ] {
        assert_eq!(t.as_str(), text);
        assert_eq!(QuoteType::parse_db(text), Some(t));
    }
    assert_eq!(QuoteType::parse_db("indicative"), None);
    assert_eq!(QuoteType::parse_db(""), None);
}

// ---- D-10: off until a lifetime is entered ----

#[test]
fn q1_no_lifetime_means_binding_quotes_are_off() {
    refused_with(BindingQuotePolicy { ttl: None }.enabled_ttl(), NOT_ENABLED);
}

#[test]
fn q2_a_lifetime_is_the_locks_length() {
    assert_eq!(on().enabled_ttl().unwrap(), Duration::seconds(600));
    let short = BindingQuotePolicy {
        ttl: Some(Duration::seconds(1)),
    };
    assert_eq!(short.enabled_ttl().unwrap(), Duration::seconds(1));
}

// ---- D-14: DESK only ----

#[test]
fn q16_desk_is_the_only_channel() {
    on().check_channel(QuoteChannel::Desk).expect("DESK issues");
    for channel in [
        QuoteChannel::App,
        QuoteChannel::Kiosk,
        QuoteChannel::Booking,
    ] {
        refused_with(on().check_channel(channel), DESK_ONLY);
    }
}

#[test]
fn q16_desk_only_names_the_channel_field() {
    assert!(DESK_ONLY.starts_with("validationFailed:channel:"));
    assert!(NOT_VERIFIED.starts_with("validationFailed: customerId"));
    assert!(NOT_ENABLED.starts_with("validationFailed"));
}

// ---- D-17: a verified customer ----

#[test]
fn q26_verified_until_after_now_issues() {
    on().check_customer(Some(at(31)), at(30))
        .expect("verified until after now");
}

#[test]
fn q25_no_customer_or_not_verified_is_refused() {
    refused_with(on().check_customer(None, at(30)), NOT_VERIFIED);
}

#[test]
fn q25_verified_until_now_or_before_is_refused() {
    refused_with(on().check_customer(Some(at(30)), at(30)), NOT_VERIFIED);
    refused_with(on().check_customer(Some(at(29)), at(30)), NOT_VERIFIED);
}

// ---- D-15: the branch limit ----

#[test]
fn q19_at_or_under_the_limit_or_no_limit_needs_no_approval() {
    assert!(!on().needs_approval(money("50000.00"), Some(money("50000.00"))));
    assert!(!on().needs_approval(money("49999.99"), Some(money("50000.00"))));
    assert!(!on().needs_approval(money("99999999.99"), None));
}

#[test]
fn q20_over_the_limit_needs_approval() {
    assert!(on().needs_approval(money("50000.01"), Some(money("50000.00"))));
}
