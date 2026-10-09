//! Ticket #598 (RE-1+RE-3) — `RateProposal::of`: a draft line → its proposed rates (RE-1).
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.2 (normalising: `base = ref × currency.unit_size ÷
//! ref.unit_size`, once), §4.4 (AUTO = `base × (1 + adjust_pct ÷ 100)`, rounded HALF_UP — away from
//! zero — to 8 places; MANUAL = its value with a reason; checks after rounding), D-1 (nothing else
//! divides by `unit_size`), D-6 (no policy ⇒ no AUTO side), §9 RE-1.
//!
//! Pure: no database.
//!
//! Signatures this test assumes (the design names the types and files; the exact Rust is chosen
//! here and kept minimal):
//! - `xc_rate::domain::draft::rate_draft::{DraftLine, LineSide}`:
//!   `enum LineSide { Auto, Manual(Rate) }` (`Debug + Clone + PartialEq`);
//!   `DraftLine::new(currency: CurrencyCode, buy: LineSide, sell: LineSide,
//!   reason: Option<String>) -> DraftLine` (`Debug + Clone + PartialEq`).
//! - `xc_rate::domain::reference::reference_batch::ReferenceLine::new(currency: CurrencyCode,
//!   unit_size: UnitSize, buy: Option<Rate>, sell: Option<Rate>) -> ReferenceLine` — one
//!   reference line in the provider's own unit (§4.2); `None` = the provider has no such side.
//! - `xc_rate::domain::policy::rate_policy::RatePolicy::new(id: impl Into<String>,
//!   buy_adjust_pct: RatePercent, sell_adjust_pct: RatePercent, guards: &serde_json::Value)
//!   -> Result<RatePolicy, RateEngineError>` — `guards` in §4.3's shape
//!   `{ "<CODE>": { "buy": {"min_pct": "<pct>", "max_pct": "<pct>"}, "sell": {…} } }`.
//! - `xc_rate::domain::draft::rate_proposal::RateProposal::of(line: &DraftLine,
//!   reference: Option<&ReferenceLine>, policy: Option<&RatePolicy>, units: &CurrencyUnits)
//!   -> Result<ProposedLine, RateEngineError>`; `ProposedLine` has public fields
//!   `buy: Option<Rate>` and `sell: Option<Rate>` (`None` = unavailable, §4.1).
//! - `xc_rate::domain::draft::rate_engine_error::RateEngineError` (`Debug`) with
//!   `code(&self) -> &'static str`, one of §4.7's codes ("validationFailed", "staleVersion",
//!   "approvalRequired", "rateUnavailable", "outOfScope", …).
//!
//! Every expected rate below is written out by hand. The two rounding cases end in exactly 5 at
//! the 9th place with an even 8th digit, so banker's rounding (`rust_decimal::round_dp`) and
//! HALF_UP give different answers and only HALF_UP passes.

use serde_json::json;
use xc_kernel::money::{CurrencyCode, CurrencyUnits, MinorUnit, Rate, RatePercent, UnitSize};
use xc_rate::domain::draft::rate_draft::{DraftLine, LineSide};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::draft::rate_proposal::{ProposedLine, RateProposal};
use xc_rate::domain::policy::rate_policy::RatePolicy;
use xc_rate::domain::reference::reference_batch::ReferenceLine;

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a known code")
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).expect("a valid rate")
}

fn unit(text: &str) -> UnitSize {
    UnitSize::parse(text).expect("a valid unit size")
}

fn pct(text: &str) -> RatePercent {
    RatePercent::parse(text).expect("a valid percent")
}

/// The register row of `currency` with `unit_size` (minor unit 2; it plays no part here).
fn units(currency: &str, unit_size: &str) -> CurrencyUnits {
    CurrencyUnits::new(code(currency), unit(unit_size), MinorUnit::TWO)
}

fn reference(
    currency: &str,
    ref_unit: &str,
    buy: Option<&str>,
    sell: Option<&str>,
) -> ReferenceLine {
    ReferenceLine::new(
        code(currency),
        unit(ref_unit),
        buy.map(rate),
        sell.map(rate),
    )
}

/// A policy with these adjustments and a wide guard on every currency the tests use; guards play
/// no part in a proposal.
fn policy(buy: &str, sell: &str) -> RatePolicy {
    let wide = json!({ "min_pct": "-50.0000", "max_pct": "50.0000" });
    let guards = json!({
        "USD": { "buy": wide, "sell": wide },
        "JPY": { "buy": wide, "sell": wide },
        "KRW": { "buy": wide, "sell": wide },
    });
    RatePolicy::new("policy-under-test", pct(buy), pct(sell), &guards).expect("a valid policy")
}

fn auto() -> DraftLine {
    DraftLine::new(code("USD"), LineSide::Auto, LineSide::Auto, None)
}

fn line(currency: &str, buy: LineSide, sell: LineSide, reason: Option<&str>) -> DraftLine {
    DraftLine::new(code(currency), buy, sell, reason.map(str::to_owned))
}

fn proposed(result: Result<ProposedLine, RateEngineError>, what: &str) -> ProposedLine {
    result.unwrap_or_else(|e| panic!("{what}: expected a proposal, got {e:?} ({})", e.code()))
}

fn refused(result: Result<ProposedLine, RateEngineError>, what: &str) -> &'static str {
    match result {
        Err(e) => e.code(),
        Ok(p) => panic!(
            "{what}: expected a refusal, got buy {:?} sell {:?}",
            p.buy, p.sell
        ),
    }
}

// ── AUTO: base × (1 + pct ÷ 100), HALF_UP to 8 places ───────────────────────────

#[test]
fn auto_rounds_half_up_to_eight_places_on_both_sides() {
    // buy:  32.125 × (1 + 0.0001/100) = 32.125032125   → HALF_UP 32.12503213 (banker's: …212)
    // sell: 32.175 × (1 − 0.0001/100) = 32.174967825   → HALF_UP 32.17496783 (banker's: …782)
    let r = reference("USD", "1", Some("32.125"), Some("32.175"));
    let p = proposed(
        RateProposal::of(
            &auto(),
            Some(&r),
            Some(&policy("0.0001", "-0.0001")),
            &units("USD", "1"),
        ),
        "USD AUTO",
    );
    assert_eq!(
        p.buy,
        Some(rate("32.12503213")),
        "buy: HALF_UP at the 9th place"
    );
    assert_eq!(
        p.sell,
        Some(rate("32.17496783")),
        "sell: HALF_UP at the 9th place"
    );
}

#[test]
fn auto_with_no_adjustment_is_the_reference_itself() {
    let r = reference("USD", "1", Some("32.1"), Some("32.45"));
    let p = proposed(
        RateProposal::of(
            &auto(),
            Some(&r),
            Some(&policy("0", "0")),
            &units("USD", "1"),
        ),
        "USD AUTO 0%",
    );
    assert_eq!(p.buy, Some(rate("32.1")));
    assert_eq!(p.sell, Some(rate("32.45")));
}

#[test]
fn jpy_referenced_per_100_with_unit_size_100_is_not_divided_twice() {
    // BoT quotes JPY per 100 (§4.2 stores it raw, unit 100); the currency sells per 100 (A1).
    // base = 23.5 × 100 ÷ 100 = 23.5; × 1.005 = 23.6175. Dividing by 100 anywhere gives 0.236175.
    let r = reference("JPY", "100", Some("23.2"), Some("23.5"));
    let jpy = line("JPY", LineSide::Auto, LineSide::Auto, None);
    let p = proposed(
        RateProposal::of(
            &jpy,
            Some(&r),
            Some(&policy("0", "0.5")),
            &units("JPY", "100"),
        ),
        "JPY AUTO per 100",
    );
    assert_eq!(p.buy, Some(rate("23.2")), "buy per 100, unchanged at 0%");
    assert_eq!(p.sell, Some(rate("23.6175")), "sell per 100, +0.5%");
}

#[test]
fn a_reference_in_another_unit_is_normalised_once_to_the_currency_unit() {
    // A provider quoting JPY per 1 (unit 1): base = 0.235 × 100 ÷ 1 = 23.5 per 100.
    let r = reference("JPY", "1", Some("0.232"), Some("0.235"));
    let jpy = line("JPY", LineSide::Auto, LineSide::Auto, None);
    let p = proposed(
        RateProposal::of(
            &jpy,
            Some(&r),
            Some(&policy("0", "0.5")),
            &units("JPY", "100"),
        ),
        "JPY AUTO from a per-1 reference",
    );
    assert_eq!(p.buy, Some(rate("23.2")));
    assert_eq!(p.sell, Some(rate("23.6175")));
    // and KRW per 1000 from a per-1 reference: 0.02512345 × 1000 = 25.12345
    let r = reference("KRW", "1", Some("0.02512345"), Some("0.02612345"));
    let krw = line("KRW", LineSide::Auto, LineSide::Auto, None);
    let p = proposed(
        RateProposal::of(
            &krw,
            Some(&r),
            Some(&policy("0", "0")),
            &units("KRW", "1000"),
        ),
        "KRW AUTO from a per-1 reference",
    );
    assert_eq!(p.buy, Some(rate("25.12345")));
    assert_eq!(p.sell, Some(rate("26.12345")));
}

// ── unavailable: never guessed ──────────────────────────────────────────────────

#[test]
fn a_missing_reference_makes_the_auto_side_unavailable() {
    // no reference line at all: both AUTO sides unavailable, and that is not an error
    let p = proposed(
        RateProposal::of(&auto(), None, Some(&policy("0", "0")), &units("USD", "1")),
        "USD AUTO with no reference",
    );
    assert_eq!((p.buy, p.sell), (None, None));

    // a reference with no sell side: only the AUTO sell is unavailable
    let r = reference("USD", "1", Some("32.1"), None);
    let p = proposed(
        RateProposal::of(
            &auto(),
            Some(&r),
            Some(&policy("0", "0")),
            &units("USD", "1"),
        ),
        "USD AUTO with no reference sell",
    );
    assert_eq!(p.buy, Some(rate("32.1")));
    assert_eq!(p.sell, None, "the missing side is unavailable, not guessed");

    // a MANUAL side next to an unavailable AUTO side keeps its own value
    let mixed = line(
        "USD",
        LineSide::Auto,
        LineSide::Manual(rate("32.5")),
        Some("no feed"),
    );
    let p = proposed(
        RateProposal::of(&mixed, None, Some(&policy("0", "0")), &units("USD", "1")),
        "USD AUTO buy + MANUAL sell, no reference",
    );
    assert_eq!(p.buy, None);
    assert_eq!(p.sell, Some(rate("32.5")));
}

#[test]
fn with_no_policy_an_auto_side_is_unavailable() {
    // D-6: a draft built under no policy has no AUTO side.
    let r = reference("USD", "1", Some("32.1"), Some("32.45"));
    let p = proposed(
        RateProposal::of(&auto(), Some(&r), None, &units("USD", "1")),
        "USD AUTO with no policy",
    );
    assert_eq!((p.buy, p.sell), (None, None));
}

// ── MANUAL ──────────────────────────────────────────────────────────────────────

#[test]
fn a_manual_side_is_its_own_value_whatever_the_reference_and_the_adjustment() {
    let r = reference("USD", "1", Some("32.1"), Some("32.45"));
    let l = line(
        "USD",
        LineSide::Manual(rate("31.98765432")),
        LineSide::Manual(rate("33")),
        Some("counter rate"),
    );
    let p = proposed(
        RateProposal::of(
            &l,
            Some(&r),
            Some(&policy("1.5", "-1.5")),
            &units("USD", "1"),
        ),
        "USD MANUAL",
    );
    assert_eq!(p.buy, Some(rate("31.98765432")));
    assert_eq!(p.sell, Some(rate("33")));
}

#[test]
fn a_manual_side_without_a_reason_is_refused() {
    let r = reference("USD", "1", Some("32.1"), Some("32.45"));
    for (what, reason) in [("no reason", None), ("an empty reason", Some(""))] {
        let l = line(
            "USD",
            LineSide::Manual(rate("32.2")),
            LineSide::Auto,
            reason,
        );
        assert_eq!(
            refused(
                RateProposal::of(&l, Some(&r), Some(&policy("0", "0")), &units("USD", "1")),
                what
            ),
            "validationFailed",
            "a MANUAL buy with {what}"
        );
        let l = line(
            "USD",
            LineSide::Auto,
            LineSide::Manual(rate("32.5")),
            reason,
        );
        assert_eq!(
            refused(
                RateProposal::of(&l, Some(&r), Some(&policy("0", "0")), &units("USD", "1")),
                what
            ),
            "validationFailed",
            "a MANUAL sell with {what}"
        );
    }
    // two AUTO sides need no reason
    assert!(
        RateProposal::of(
            &auto(),
            Some(&r),
            Some(&policy("0", "0")),
            &units("USD", "1")
        )
        .is_ok()
    );
}

// ── checks after rounding ──────────────────────────────────────────────────────

#[test]
fn buy_above_sell_after_rounding_is_validation_failed_and_equal_is_allowed() {
    let r = reference("USD", "1", Some("32.1"), Some("32.45"));
    // MANUAL buy 32.45000001 against AUTO sell 32.45: above by one unit of the 8th place
    let above = line(
        "USD",
        LineSide::Manual(rate("32.45000001")),
        LineSide::Auto,
        Some("x"),
    );
    assert_eq!(
        refused(
            RateProposal::of(
                &above,
                Some(&r),
                Some(&policy("0", "0")),
                &units("USD", "1")
            ),
            "buy above sell"
        ),
        "validationFailed"
    );
    // equal is not above
    let equal = line(
        "USD",
        LineSide::Manual(rate("32.45")),
        LineSide::Auto,
        Some("x"),
    );
    let p = proposed(
        RateProposal::of(
            &equal,
            Some(&r),
            Some(&policy("0", "0")),
            &units("USD", "1"),
        ),
        "buy equal to sell",
    );
    assert_eq!(p.buy, p.sell);

    // AUTO on both sides: the buy reference above the sell reference after the adjustment
    // 32.4 × 1.002 = 32.4648 > 32.45 × 1.0 = 32.45
    let both_auto = reference("USD", "1", Some("32.4"), Some("32.45"));
    assert_eq!(
        refused(
            RateProposal::of(
                &auto(),
                Some(&both_auto),
                Some(&policy("0.2", "0")),
                &units("USD", "1")
            ),
            "AUTO buy above AUTO sell"
        ),
        "validationFailed"
    );
}

#[test]
fn an_auto_rate_that_rounds_to_zero_or_below_is_validation_failed() {
    // every rate > 0 (§4.4): −100% leaves nothing
    let r = reference("USD", "1", Some("32.1"), Some("32.45"));
    assert_eq!(
        refused(
            RateProposal::of(
                &auto(),
                Some(&r),
                Some(&policy("-100", "0")),
                &units("USD", "1")
            ),
            "buy at -100%"
        ),
        "validationFailed"
    );
    // 0.00000001 × (1 − 60/100) = 0.000000004, which rounds HALF_UP to 0.00000000 at 8 places
    let tiny = reference("USD", "1", Some("0.00000001"), Some("32.45"));
    assert_eq!(
        refused(
            RateProposal::of(
                &auto(),
                Some(&tiny),
                Some(&policy("-60", "0")),
                &units("USD", "1")
            ),
            "buy rounding to zero"
        ),
        "validationFailed"
    );
}
