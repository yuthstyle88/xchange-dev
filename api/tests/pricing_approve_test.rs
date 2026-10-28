//! PT-6 (pricing-tiers.md §2, §5.2, §6, §11; owner P-3 "Use xchange's flow"): `pricingApprove`
//! (`POST /api/pricing/approve`). For 1..n currencies: one new policy version with their margins (through
//! `PolicyVersionWriter`), then one rate draft with one AUTO line per currency, submitted; in bound ⇒ published by
//! policy, out of bound ⇒ waits in A06 for EXECUTIVE. Written before the implementation by the ticket author
//! (claude/647e1967, PT-0). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Choices pinned here (the implementer follows them):
//! - Each approved tier price becomes the margin that produces it over the draft path's base (§2, §5.2):
//!   `margin = (price ÷ base − 1) × 100 − adjust_pct`, rounded to scale 4, per side, where base is the currency's
//!   `ReferenceBase::newest_in` line at its unit size. A currency with no base ⇒ 400 `validationFailed`.
//! - `[decided by BE controller 2026-10-20]` (claude/a90ab16c, reading A, recorded on #3827): approve is **not** a frame change. Any caller with
//!   `RatePolicySave` + `RatesDraft` may approve (V.9's `pricing.tiers:approve` is FULL, its `pricing.policy:write` is
//!   owner-only), the margins are written without the save-time guard refusal, and the guard decides through the
//!   draft: in bound ⇒ `PUBLISHED`, out of bound ⇒ `PENDING_APPROVAL` (§11's two approve rows).
//! - A `policyVersionId` that is not the branch's current version ⇒ 409 `staleVersion`, nothing written.
//! - The `Idempotency-Key` header is the command's `requestId` (`rates.md` §5.6): the same key twice ⇒ one version,
//!   one draft, the same answer.
//!
//! Prices are §12.3's worked example (USD BoT 32.4500 / 32.9000, adjust −0.5 / +0.4, see `pricing_common`).
//! Runs once PT-1, PT-2, PT-3 and PT-4 are on `feat/pricing-tiers`.
//!
//! RESTATED 2026-10-27 by the XC-V9-PRICING sub-controller (claude/451f0116), pricing-tiers.md §15.0 V-1/V-2: the
//! owner ruled *"แก้ให้ ตรงกับ เอกสารที่เค้าส่งมาล่า สุด"* and *"Follow the new pricing document for B2-B6"* — the
//! approved tiers are the counter price (no margins, no policy version), with no EXECUTIVE step (B3) and 5 decimals
//! (B2). The six margin-model tests that pinned the replaced behaviour are removed; their successors are in
//! `v9_pricing_approve_test.rs` / `v9_pricing_push_test.rs`:
//! - `approve_in_bound_writes_a_version_with_the_margins_and_publishes_the_prices`, `a_rate_manager_may_approve_in_bound`
//!   → `v9_approve_manual_publishes_the_approved_tiers_as_the_counter_price_with_no_executive_wait`;
//! - `bulk_approve_writes_one_version_and_one_draft_for_every_currency` → each currency is its own tier set
//!   (`request_id = <key>:<currency>`), covered by the same test and the push tests;
//! - `approve_out_of_bound_waits_for_executive` → reversed by B3 (same v9 test);
//! - `a_stale_policy_version_is_refused_and_nothing_is_written` →
//!   `v9_approve_a_tier_set_id_older_than_the_latest_is_a_conflict_and_writes_nothing` (the policy-version guard
//!   itself still answers `staleVersion`);
//! - `the_same_request_twice_writes_once` → `v9_push_the_same_request_twice_writes_main_and_each_branch_once`.

mod pricing_common;

use pricing_common::*;
use serde_json::{Value, json};
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xchange::presentation::http::v3::error::V3Error;

const ROUTE: &str = "/api/pricing/approve";

/// USD prices whose margins are T1 +0.4/−0.4, T2 +0.45/−0.45, T3 +0.5/−0.5, T4 +0.6/−0.6.
const USD_PRICES: [(&str, &str); 4] = [
    ("32.41755", "32.90"),
    ("32.433775", "32.88355"),
    ("32.45", "32.8671"),
    ("32.48245", "32.8342"),
];
const USD_MARGINS: [(&str, &str); 4] = [
    ("0.4", "-0.4"),
    ("0.45", "-0.45"),
    ("0.5", "-0.5"),
    ("0.6", "-0.6"),
];

/// EUR prices at the branch's own margins (T1 0/0 … T4 +0.3/−0.3).
const EUR_PRICES: [(&str, &str); 4] = [
    ("37.81", "38.7544"),
    ("37.848", "38.7158"),
    ("37.886", "38.6772"),
    ("37.924", "38.6386"),
];

fn row(currency: &str, prices: [(&str, &str); 4]) -> Value {
    json!({
        "currency": currency,
        "tiers": prices.iter().enumerate()
            .map(|(i, (b, s))| json!({ "tierNo": i + 1, "buy": b, "sell": s }))
            .collect::<Vec<_>>(),
    })
}

fn body(v1: uuid::Uuid, rows: Vec<Value>) -> Value {
    json!({ "policyVersionId": v1.to_string(), "rows": rows, "reason": "PT-6 test" })
}

async fn nothing_written(room: &Room, what: &str) {
    assert_eq!(room.versions(B01).await.len(), 1, "{what}: no new version");
    assert_eq!(
        room.count("rate_policy_currency_tiers").await,
        0,
        "{what}: no override row"
    );
    assert_eq!(room.count("rate_drafts").await, 0, "{what}: no draft");
}

#[tokio::test]
async fn a_caller_without_rate_policy_save_is_forbidden() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let a = command(
        &room,
        &teller(),
        ROUTE,
        B01,
        body(v1, vec![row("USD", USD_PRICES)]),
        None,
    )
    .await;
    // The permission layer refuses a role outside the command's row (`ratePolicySave`'s, inherited) as it does
    // for `ratePolicySave` itself: `roleNotAllowedHere` with its status.
    let err = PolicyError::RoleNotAllowedHere;
    assert_eq!(
        a.status.as_u16(),
        err.status(),
        "§11: user without RatePolicySave; body {}",
        a.text
    );
    assert_eq!(a.code(), json!(V3Error::from(err).code), "{}", a.text);
    nothing_written(&room, "TELLER").await;
    room.finish().await;
}

#[tokio::test]
async fn bad_rows_are_refused_and_nothing_is_written() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;

    let a = command(&room, &owner(), ROUTE, B01, body(v1, vec![]), None).await;
    a.refused(400, "validationFailed", "no rows");

    let mut three = row("USD", USD_PRICES);
    three["tiers"].as_array_mut().unwrap().pop();
    let a = command(&room, &owner(), ROUTE, B01, body(v1, vec![three]), None).await;
    a.refused(400, "validationFailed", "3 tiers");

    // GBP has no BoT line and no START line: no base, so no margin (§5.2 "no base ⇒ no margin").
    let a = command(
        &room,
        &owner(),
        ROUTE,
        B01,
        body(v1, vec![row("USD", USD_PRICES), row("GBP", USD_PRICES)]),
        None,
    )
    .await;
    a.refused(400, "validationFailed", "a currency with no base");

    nothing_written(&room, "bad rows").await;
    room.finish().await;
}
