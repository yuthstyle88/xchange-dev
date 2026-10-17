//! K-2 up-front tests (#2054, design `docs/design/blueprint/bounded-inputs.md` §2 B-2, §4 row K-2 `:98`) — written
//! before the implementation by claude/studio-claude-2 on the order of the xchange BE lane controller
//! claude/56a74fcd. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Part 1 pins what `Money::parse`, `Rate::parse` and `ForeignAmount::parse` do on main today. These tests pass on
//! main and must still pass after K-2 moves the three onto `Bound` ("no change in behaviour").
//! Part 2 is new: `RatePercent::parse` gains the bound of its `numeric(9,4)` columns (|v| < 10^5) and `UnitSize`
//! gains the bound of its `numeric(20,4)` columns (v < 10^16). Both sides of every bound are pinned.

use std::str::FromStr;

use rust_decimal::Decimal;
use xc_kernel::money::{
    CurrencyCode, CurrencyUnits, ForeignAmount, MinorUnit, Money, NumberError, Rate, RatePercent,
    UnitSize,
};

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("test decimal")
}

fn units(places: u8) -> CurrencyUnits {
    CurrencyUnits::new(
        CurrencyCode::parse("USD").expect("USD"),
        UnitSize::ONE,
        MinorUnit::new(places).expect("minor unit"),
    )
}

/// The texts every one of the three refuses as `Invalid` today.
const INVALID_FOR_ALL: &[&str] = &[
    "", " 1", "1 ", "+1", "-1", "-0.00", ".5", "5.", "1e5", "1.2.3", "abc",
];

// ================================================================ part 1: today's behaviour, kept

#[test]
fn money_refuses_the_same_texts_as_today() {
    for t in INVALID_FOR_ALL {
        assert_eq!(Money::parse(t), Err(NumberError::Invalid), "{t:?}");
    }
    assert_eq!(Money::parse("1.234"), Err(NumberError::TooManyDecimals));
}

#[test]
fn money_accepts_zero_extra_zero_decimals_and_prints_two_places() {
    assert_eq!(Money::parse("0").expect("0").to_string(), "0.00");
    assert_eq!(Money::parse("1.5").expect("1.5").to_string(), "1.50");
    assert_eq!(Money::parse("1.230").expect("1.230").to_string(), "1.23");
    assert_eq!(Money::parse("100.5").expect("100.5").minor(), 10050);
}

#[test]
fn money_cap_is_inclusive_at_one_hundred_million() {
    assert_eq!(
        Money::parse("100000000").expect("cap").to_string(),
        "100000000.00"
    );
    assert_eq!(
        Money::parse("100000000.00").expect("cap.00").to_string(),
        "100000000.00"
    );
    assert_eq!(Money::parse("100000000.01"), Err(NumberError::Invalid));
    assert_eq!(Money::parse("100000001"), Err(NumberError::Invalid));
}

#[test]
fn rate_refuses_the_same_texts_as_today_and_zero() {
    for t in INVALID_FOR_ALL {
        assert_eq!(Rate::parse(t), Err(NumberError::Invalid), "{t:?}");
    }
    assert_eq!(Rate::parse("0"), Err(NumberError::Invalid));
    assert_eq!(Rate::parse("0.00000000"), Err(NumberError::Invalid));
    assert_eq!(
        Rate::parse("1.123456789"),
        Err(NumberError::TooManyDecimals)
    );
}

#[test]
fn rate_keeps_its_four_to_eight_place_rescale() {
    assert_eq!(Rate::parse("32.1").expect("32.1").to_string(), "32.1000");
    assert_eq!(
        Rate::parse("0.02152").expect("0.02152").to_string(),
        "0.02152"
    );
    assert_eq!(
        Rate::parse("25.12345678").expect("8 places").to_string(),
        "25.12345678"
    );
    assert_eq!(
        Rate::parse("1.123456780").expect("zero 9th").to_string(),
        "1.12345678"
    );
    assert_eq!(Rate::parse("32.1").expect("32.1").value(), d("32.1"));
}

#[test]
fn rate_cap_is_inclusive_at_one_hundred_million() {
    assert!(Rate::parse("100000000").is_ok());
    assert!(Rate::parse("100000000.00").is_ok());
    assert_eq!(Rate::parse("100000000.01"), Err(NumberError::Invalid));
    assert_eq!(Rate::parse("100000001"), Err(NumberError::Invalid));
}

#[test]
fn foreign_amount_refuses_the_same_texts_as_today_and_zero() {
    let u = units(2);
    for t in INVALID_FOR_ALL {
        assert_eq!(
            ForeignAmount::parse(t, &u),
            Err(NumberError::Invalid),
            "{t:?}"
        );
    }
    assert_eq!(ForeignAmount::parse("0", &u), Err(NumberError::Invalid));
    assert_eq!(
        ForeignAmount::parse("1.234", &u),
        Err(NumberError::TooManyDecimals)
    );
    assert_eq!(
        ForeignAmount::parse("1.5", &units(0)),
        Err(NumberError::TooManyDecimals)
    );
}

#[test]
fn foreign_amount_keeps_the_value_as_typed() {
    // ForeignAmount does not rescale today: "1.5" keeps scale 1, "1.50" keeps scale 2.
    let u = units(2);
    assert_eq!(
        ForeignAmount::parse("1.5", &u)
            .expect("1.5")
            .value()
            .to_string(),
        "1.5"
    );
    assert_eq!(
        ForeignAmount::parse("1.50", &u)
            .expect("1.50")
            .value()
            .to_string(),
        "1.50"
    );
    assert_eq!(
        ForeignAmount::parse("1.500", &u).expect("1.500").value(),
        d("1.5")
    );
    assert_eq!(
        ForeignAmount::parse("100", &units(0)).expect("100").value(),
        d("100")
    );
}

#[test]
fn foreign_amount_cap_is_inclusive_at_one_hundred_million() {
    let u = units(2);
    assert!(ForeignAmount::parse("100000000", &u).is_ok());
    assert!(ForeignAmount::parse("100000000.00", &u).is_ok());
    assert_eq!(
        ForeignAmount::parse("100000000.01", &u),
        Err(NumberError::Invalid)
    );
    assert_eq!(
        ForeignAmount::parse("100000001", &u),
        Err(NumberError::Invalid)
    );
}

#[test]
fn rate_percent_keeps_its_sign_handling_and_scale() {
    assert_eq!(
        RatePercent::parse("-2.5").expect("-2.5").value(),
        d("-2.5000")
    );
    assert_eq!(
        RatePercent::parse("+2.5").expect("+2.5").value(),
        d("2.5000")
    );
    assert_eq!(RatePercent::parse("0").expect("0").value(), d("0.0000"));
    assert_eq!(
        RatePercent::parse("1.23456"),
        Err(NumberError::TooManyDecimals)
    );
    for t in ["", "-", "+", ".5", "5.", "1e2", "--1", "+-1"] {
        assert_eq!(RatePercent::parse(t), Err(NumberError::Invalid), "{t:?}");
    }
}

#[test]
fn unit_size_keeps_its_refusals() {
    assert_eq!(UnitSize::parse("0"), Err(NumberError::Invalid));
    assert_eq!(UnitSize::parse("-1"), Err(NumberError::Invalid));
    assert_eq!(
        UnitSize::parse("1.00001"),
        Err(NumberError::TooManyDecimals)
    );
    assert_eq!(UnitSize::parse("100").expect("100").value(), d("100"));
    assert_eq!(UnitSize::from_decimal(d("0")), Err(NumberError::Invalid));
    assert_eq!(
        UnitSize::from_decimal(d("1.00001")),
        Err(NumberError::TooManyDecimals)
    );
}

// ================================================================ part 2: the new caps (red on main)

/// `rate_policy_*.{buy,sell}_{adjust,margin}_pct numeric(9,4)` (`20261010120000_rate_engine.sql:30-31`,
/// `20261205151000_rate_tiers_and_starting_prices.sql:8-9`, `:34`): |v| < 10^5.
#[test]
fn rate_percent_is_bounded_by_numeric_9_4_on_both_sides() {
    assert_eq!(
        RatePercent::parse("99999.9999").expect("max").value(),
        d("99999.9999")
    );
    assert_eq!(
        RatePercent::parse("-99999.9999").expect("-max").value(),
        d("-99999.9999")
    );
    assert_eq!(RatePercent::parse("100000"), Err(NumberError::Invalid));
    assert_eq!(RatePercent::parse("-100000"), Err(NumberError::Invalid));
    assert_eq!(RatePercent::parse("+100000"), Err(NumberError::Invalid));
    assert_eq!(
        RatePercent::parse(&"9".repeat(30)),
        Err(NumberError::Invalid)
    );
}

/// `currencies.unit_size numeric(20,4) CHECK (> 0)` (`20261010100000_rate_units.sql:6`): v < 10^16.
#[test]
fn unit_size_parse_is_bounded_by_numeric_20_4() {
    assert_eq!(
        UnitSize::parse("9999999999999999.9999")
            .expect("max")
            .value(),
        d("9999999999999999.9999")
    );
    assert_eq!(
        UnitSize::parse("10000000000000000"),
        Err(NumberError::Invalid)
    );
    assert_eq!(UnitSize::parse(&"9".repeat(30)), Err(NumberError::Invalid));
}

#[test]
fn unit_size_from_decimal_is_bounded_by_numeric_20_4() {
    assert!(UnitSize::from_decimal(d("9999999999999999.9999")).is_ok());
    assert_eq!(
        UnitSize::from_decimal(d("10000000000000000")),
        Err(NumberError::Invalid)
    );
}
