//! XC-V9-PRICING PV-iv (pricing-tiers.md §15.7, B5: "manual prices are never hidden"): `Freshness::of_side` no longer
//! marks a MANUAL side stale. A line whose `source_snapshot.pricing.source` is `MANUAL` or `MAIN_PUSH` is an approved
//! price, not a BoT draft, so an old reference age never makes it stale. AUTO and FOLLOW keep today's rule (FOLLOW is
//! hidden by `FollowVisibility`, not by this flag). Written before the implementation by the sub-controller
//! claude/451f0116 (XC-V9-PRICING skeleton). Do not edit, delete or weaken; add tests in a file of your own.

use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use xc_kernel::money::Side;
use xc_rate::domain::publication::freshness::Freshness;

fn now() -> DateTime<Utc> {
    "2026-10-27T03:00:00Z".parse().unwrap()
}

/// A snapshot fetched 5 hours ago under a 3-hour limit: stale by today's rule.
fn old_snapshot(pricing: Option<Value>) -> Value {
    let mut s = json!({
        "source_date": "2026-10-26",
        "fetched_at": (now() - Duration::hours(5)).to_rfc3339(),
        "max_reference_age_hours": 3,
    });
    if let Some(p) = pricing {
        s["pricing"] = p;
    }
    s
}

/// PV-iv / PV-15: a MANUAL line with an old reference is never stale, either side.
#[test]
fn a_manual_side_is_never_stale() {
    let snap = old_snapshot(Some(
        json!({ "tier_set_id": "8d6a2c1e-0000-4000-8000-000000000001", "source": "MANUAL" }),
    ));
    assert!(!Freshness::of_side(&snap, Side::Buy, now()).stale);
    assert!(!Freshness::of_side(&snap, Side::Sell, now()).stale);
}

/// PV-iv / PV-15: `MAIN_PUSH` reads as MANUAL (§15.7).
#[test]
fn a_main_push_side_is_never_stale() {
    let snap = old_snapshot(Some(
        json!({ "tier_set_id": "8d6a2c1e-0000-4000-8000-000000000002", "source": "MAIN_PUSH" }),
    ));
    assert!(!Freshness::of_side(&snap, Side::Buy, now()).stale);
}

/// PV-iv: AUTO (no `pricing`) and FOLLOW keep today's age rule.
#[test]
fn auto_and_follow_sides_keep_the_age_rule() {
    assert!(Freshness::of_side(&old_snapshot(None), Side::Buy, now()).stale);
    let follow = old_snapshot(Some(
        json!({ "tier_set_id": "8d6a2c1e-0000-4000-8000-000000000003", "source": "FOLLOW" }),
    ));
    assert!(Freshness::of_side(&follow, Side::Sell, now()).stale);
}

/// PV-iv: the source date is still read for a MANUAL side (the board's date line).
#[test]
fn a_manual_side_keeps_its_source_date() {
    let snap = old_snapshot(Some(json!({ "source": "MANUAL" })));
    assert_eq!(
        Freshness::of_side(&snap, Side::Buy, now())
            .source_date
            .map(|d| d.to_string()),
        Some("2026-10-26".to_string())
    );
}
