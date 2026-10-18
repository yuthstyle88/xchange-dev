//! Written before the implementation (ticket T2). Do not edit, delete or weaken.

use xc_kernel::money::{Money, NumberError, Rate};

#[test]
fn money_reads_plain_decimals_and_prints_two_places() {
    for (text, shown, minor) in [
        ("0", "0.00", 0),
        ("0.01", "0.01", 1),
        ("100", "100.00", 10_000),
        ("100.5", "100.50", 10_050),
        ("1234.56", "1234.56", 123_456),
        ("0100", "100.00", 10_000),
        ("100.000", "100.00", 10_000),
        ("100000000", "100000000.00", 10_000_000_000),
    ] {
        let m = Money::parse(text).unwrap_or_else(|e| panic!("{text}: {e:?}"));
        assert_eq!(m.to_string(), shown, "display of {text}");
        assert_eq!(m.try_minor().unwrap(), minor, "minor of {text}");
    }
}

#[test]
fn money_refuses_what_is_not_a_plain_non_negative_decimal_within_the_cap() {
    for text in [
        "",
        " ",
        "abc",
        "-1",
        "-0.01",
        "+5",
        "1e3",
        "1,000",
        "100.",
        ".5",
        " 100",
        "100 ",
        "١٠٠",
        "NaN",
        "Infinity",
        "100000000.01",
        "100000001",
    ] {
        assert_eq!(Money::parse(text), Err(NumberError::Invalid), "{text:?}");
    }
}

#[test]
fn money_refuses_a_non_zero_third_decimal_however_small() {
    for text in ["1.001", "0.005", "100.129", "1.0000001"] {
        assert_eq!(
            Money::parse(text),
            Err(NumberError::TooManyDecimals),
            "{text:?}"
        );
    }
}

#[test]
fn equal_amounts_are_equal_however_they_were_written() {
    assert_eq!(
        Money::parse("100").unwrap(),
        Money::parse("100.00").unwrap()
    );
    assert_eq!(Money::parse("0").unwrap(), Money::ZERO);
    assert!(Money::ZERO.is_zero());
    assert!(!Money::parse("0.01").unwrap().is_zero());
    assert!(Money::parse("99.99").unwrap() < Money::parse("100").unwrap());
}

#[test]
fn minor_units_round_trip_and_computed_figures_may_pass_the_input_cap() {
    for minor in [
        0,
        1,
        99,
        100,
        321_000,
        10_000_000_000,
        320_999_999_968,
        1_000_000_000_000_000_000,
    ] {
        assert_eq!(
            Money::from_minor(minor).unwrap().try_minor().unwrap(),
            minor
        );
    }
    assert_eq!(Money::from_minor(321_000).unwrap().to_string(), "3210.00");
    assert_eq!(Money::from_minor(-1), Err(NumberError::Invalid));
    assert_eq!(
        Money::from_minor(320_999_999_968).unwrap().to_string(),
        "3209999999.68"
    );
}

#[test]
fn rate_reads_up_to_four_places_and_prints_four() {
    for (text, shown) in [
        ("32.1", "32.1000"),
        ("0.2195", "0.2195"),
        ("0.0001", "0.0001"),
        ("1", "1.0000"),
        ("32.10000", "32.1000"),
        ("100000000", "100000000.0000"),
    ] {
        assert_eq!(
            Rate::parse(text)
                .unwrap_or_else(|e| panic!("{text}: {e:?}"))
                .to_string(),
            shown
        );
    }
}

#[test]
fn rate_must_be_positive_plain_and_within_the_cap() {
    for text in [
        "",
        "0",
        "0.0000",
        "-3",
        "abc",
        "1e2",
        "100000000.0001",
        " 1",
    ] {
        assert_eq!(Rate::parse(text), Err(NumberError::Invalid), "{text:?}");
    }
    // RU-1 (ticket author's edit, design docs/design/blueprint/rates.md §9: Blueprint §05 rate
    // scale 8): a ninth place is too many; up to eight now parse (rate_units_test.rs pins them).
    for text in ["32.123456789", "0.000000001", "1.000000001"] {
        assert_eq!(
            Rate::parse(text),
            Err(NumberError::TooManyDecimals),
            "{text:?}"
        );
    }
}
