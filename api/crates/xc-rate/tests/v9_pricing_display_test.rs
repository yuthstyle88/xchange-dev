//! XC-V9-PRICING PV-16 / PV-18 (pricing-tiers.md §15.8; `H:` §8; V9:display `DEFAULT_DISPLAY`, `validateDisplay`,
//! `applyDisplay`, `marketOnly`): the storefront's default display settings, `rateDisplaySave`'s checks, and the board's
//! currency order with the `more` list. `H:` is the owner's handoff `PRICING-DESIGN.md` and `V9:` its source excerpt
//! (both kept out of the repo). Written before the implementation by the sub-controller claude/451f0116 (XC-V9-PRICING
//! skeleton). Do not edit, delete or weaken; add tests in a file of your own. Pure: no database, no clock.

use xc_rate::domain::pricing::display_settings::{DisplayOrder, DisplaySettings};

fn codes(list: &[&str]) -> Vec<String> {
    list.iter().map(|c| c.to_string()).collect()
}

/// `H:` §8's default featured list, in its order.
const V9_FEATURED: [&str; 10] = [
    "MYR", "CNY", "INR", "RUB", "KRW", "JPY", "GBP", "USD", "TWD", "EUR",
];

// ── default ────────────────────────────────────────────────────────────────────────────────────────────────────

// Restated for GD-1 (owner-set-values.md §3, row 23, D-6): `DisplaySettings::v9_default()` is replaced by
// `DisplaySettings::not_set()`. What "not set" reads is pinned through the store
// (`v9_pricing_stores_test.rs`) and the board (`api/tests/v9_pricing_display_test.rs` PV-17).
#[test]
fn the_default_is_not_set() {
    assert_eq!(
        DisplaySettings::not_set(),
        DisplaySettings {
            tiers: vec![1, 2, 3, 4],
            featured: Vec::new(),
            highlight: false,
            all_market: true,
        }
    );
    assert_eq!(DisplaySettings::MAX_FEATURED, 10);
}

// ── validate ───────────────────────────────────────────────────────────────────────────────────────────────────

fn validate(tiers: &[u8], featured: &[&str]) -> Result<DisplaySettings, String> {
    DisplaySettings::validate(tiers, &codes(featured), true, true)
}

/// PV-16 (§15.8; `H:` §8 "featured สูงสุด 10 สกุล"): 11 featured codes are refused.
#[test]
fn eleven_featured_codes_are_refused() {
    let mut eleven = V9_FEATURED.to_vec();
    eleven.push("AUD");
    assert!(validate(&[1, 2, 3, 4], &eleven).is_err());
}

/// PV-16 (§15.8; `H:` §8 "`display.tiers` เลือกอย่างน้อย 1 จาก 1–4"): no tiers is refused.
#[test]
fn no_tiers_is_refused() {
    assert!(validate(&[], &["USD"]).is_err());
}

/// PV-16 (§15.8; V9 `validateDisplay`: tiers are 1–4, no repeats): tier 5, tier 0 and a repeated tier are refused.
#[test]
fn a_tier_outside_one_to_four_or_repeated_is_refused() {
    assert!(validate(&[5], &["USD"]).is_err(), "tier 5");
    assert!(validate(&[0, 1], &["USD"]).is_err(), "tier 0");
    assert!(validate(&[1, 2, 2], &["USD"]).is_err(), "tier 2 twice");
}

/// PV-16 (§15.8; V9 `validateDisplay`: featured codes are upper-case codes, no repeats): a repeated code, a
/// lower-case code and a code that is not three letters are refused.
#[test]
fn a_repeated_or_malformed_featured_code_is_refused() {
    assert!(validate(&[1], &["USD", "EUR", "USD"]).is_err(), "USD twice");
    assert!(validate(&[1], &["usd"]).is_err(), "lower case");
    assert!(validate(&[1], &["US"]).is_err(), "two letters");
}

/// PV-16 (§15.8): 10 featured codes and one tier are accepted, and the answer is the settings as given.
#[test]
fn ten_featured_codes_and_one_tier_are_accepted() {
    assert_eq!(
        DisplaySettings::validate(&[1], &codes(&V9_FEATURED), false, false),
        Ok(DisplaySettings {
            tiers: vec![1],
            featured: codes(&V9_FEATURED),
            highlight: false,
            all_market: false,
        })
    );
    assert!(
        validate(&[], &[]).is_err(),
        "no featured codes is fine, but no tiers is not"
    );
    assert!(validate(&[4, 2], &[]).is_ok(), "no featured codes");
}

// ── order ──────────────────────────────────────────────────────────────────────────────────────────────────────

fn settings(featured: &[&str], all_market: bool) -> DisplaySettings {
    DisplaySettings {
        tiers: vec![1, 2, 3, 4],
        featured: codes(featured),
        highlight: true,
        all_market,
    }
}

/// The branch prices these (given out of order on purpose).
fn priced() -> Vec<String> {
    codes(&["ZAR", "KRW", "AUD", "USD", "GBP", "EUR", "CNY"])
}

/// Super Rich's market order: lists KHR and LAK, which the branch does not price.
fn sr_order() -> Vec<String> {
    codes(&["GBP", "CNY", "USD", "KHR", "KRW", "LAK"])
}

/// PV-18 (§15.8; `H:` §8 "เรียงตามที่ตั้ง → ลำดับตลาด SR → รหัสสกุล"; V9 `applyDisplay`): featured first in the set's
/// order (USD, EUR; JPY is featured but not priced, so it is not listed), then Super Rich's order (GBP, CNY, KRW), then
/// by code (AUD, ZAR). With `allMarket`, `more` holds Super Rich's currencies that have no price, in Super Rich's
/// order: KHR, LAK — "contact the counter", never a price.
#[test]
fn featured_then_super_rich_then_code_and_more_lists_khr() {
    // reading: `ordered` holds only priced currencies (DisplayOrder's doc); a featured code with no price is not in
    // it (V9 `applyDisplay` sorts the priced rates only).
    assert_eq!(
        settings(&["USD", "EUR", "JPY"], true).order(&priced(), &sr_order()),
        DisplayOrder {
            ordered: codes(&["USD", "EUR", "GBP", "CNY", "KRW", "AUD", "ZAR"]),
            more: codes(&["KHR", "LAK"]),
        }
    );
}

/// PV-18 (§15.8 "a `more` list when `allMarket`"; V9 `marketOnly`): without `allMarket`, `more` is empty and the order
/// is unchanged.
#[test]
fn without_all_market_there_is_no_more_list() {
    assert_eq!(
        settings(&["USD", "EUR", "JPY"], false).order(&priced(), &sr_order()),
        DisplayOrder {
            ordered: codes(&["USD", "EUR", "GBP", "CNY", "KRW", "AUD", "ZAR"]),
            more: vec![],
        }
    );
}

/// PV-18 (§15.8): with no featured codes and no Super Rich list, the priced currencies are by code and `more` is
/// empty.
#[test]
fn with_nothing_featured_and_no_market_list_the_order_is_by_code() {
    assert_eq!(
        settings(&[], true).order(&priced(), &[]),
        DisplayOrder {
            ordered: codes(&["AUD", "CNY", "EUR", "GBP", "KRW", "USD", "ZAR"]),
            more: vec![],
        }
    );
}
