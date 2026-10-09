//! Ticket #595 (RU-1) — rates per `unit_size` at scale 8, foreign amounts by minor unit, one arithmetic.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §3 (PR 570) and its correction (the conversion of stored
//! rates moves to RE-4; `Rate` prints 4..8 places). Blueprint §05: THB per `unit_size`, quote =
//! quantity × rate ÷ unit_size, rate scale 8, % scale 4, THB rounded HALF_UP once at the final
//! amount, foreign amounts by the currency's minor unit; §17: never divide by the unit twice.

use std::str::FromStr;

use rust_decimal::Decimal;
use xc_kernel::money::{
    CurrencyCode, CurrencyUnits, ExchangeArithmetic, ForeignAmount, MinorUnit, Money, NumberError,
    Quote, Rate, RatePercent, Side, UnitSize,
};

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

fn units(code: &str, unit: &str, minor: u8) -> CurrencyUnits {
    CurrencyUnits::new(
        CurrencyCode::parse(code).expect("a known code"),
        UnitSize::parse(unit).expect("a unit size"),
        MinorUnit::new(minor).expect("a minor unit"),
    )
}

fn foreign(text: &str, u: &CurrencyUnits) -> ForeignAmount {
    ForeignAmount::parse(text, u).unwrap_or_else(|e| panic!("{text:?}: {e:?}"))
}

fn rate(text: &str) -> Rate {
    Rate::parse(text).unwrap_or_else(|e| panic!("{text:?}: {e:?}"))
}

fn money(text: &str) -> Money {
    Money::parse(text).unwrap_or_else(|e| panic!("{text:?}: {e:?}"))
}

// ── Rate: up to 8 places, printed with at least 4 and at most 8 ────────────────

#[test]
fn a_rate_takes_up_to_eight_places() {
    for (typed, shown) in [
        ("32.1", "32.1000"),
        ("0.2195", "0.2195"),
        ("0.02152", "0.02152"),
        ("23.5", "23.5000"),
        ("23.50000000", "23.5000"),
        ("25.12345678", "25.12345678"),
        ("0.00000001", "0.00000001"),
        ("100000000", "100000000.0000"),
    ] {
        let r = rate(typed);
        assert_eq!(r.to_string(), shown, "{typed:?}: Display");
        assert_eq!(r.value(), d(shown), "{typed:?}: value is the same number");
        assert_eq!(
            r.value().to_string(),
            shown,
            "{typed:?}: value prints as Display"
        );
    }
}

#[test]
fn a_ninth_place_is_too_many() {
    for typed in ["0.000000001", "1.123456789", "32.100000001"] {
        assert_eq!(
            Rate::parse(typed),
            Err(NumberError::TooManyDecimals),
            "{typed:?}"
        );
    }
    // trailing zeros past the eighth place are not a ninth place
    assert_eq!(rate("1.0000000000").to_string(), "1.0000");
}

// ── UnitSize, MinorUnit, RatePercent ─────────────────────────────────────────

#[test]
fn a_unit_size_is_a_positive_decimal() {
    assert_eq!(UnitSize::parse("100").expect("100").value(), d("100"));
    assert_eq!(UnitSize::parse("1000").expect("1000").value(), d("1000"));
    assert_eq!(UnitSize::parse("1").expect("1").value(), d("1"));
    for bad in ["0", "", "-1", "abc"] {
        assert!(UnitSize::parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_minor_unit_is_zero_to_three() {
    for m in 0..=3u8 {
        assert!(MinorUnit::new(m).is_ok(), "{m}");
    }
    assert!(MinorUnit::new(4).is_err());
}

#[test]
fn a_rate_percent_is_signed_at_four_places() {
    assert_eq!(
        RatePercent::parse("-2.5").expect("-2.5").value(),
        d("-2.5000")
    );
    assert_eq!(RatePercent::parse("0.5").expect("0.5").value(), d("0.5000"));
    assert_eq!(
        RatePercent::parse("1.2345").expect("4 places").value(),
        d("1.2345")
    );
    assert_eq!(
        RatePercent::parse("1.23456"),
        Err(NumberError::TooManyDecimals)
    );
}

// ── ForeignAmount: the currency's minor unit, refused never rounded ───────────

#[test]
fn a_foreign_amount_follows_its_minor_unit() {
    let jpy = units("JPY", "100", 0);
    let usd = units("USD", "1", 2);
    let bhd = units("BHD", "1", 3);
    assert_eq!(foreign("1000", &jpy).value(), d("1000"));
    assert_eq!(foreign("12.34", &usd).value(), d("12.34"));
    assert_eq!(foreign("1.234", &bhd).value(), d("1.234"));
    assert_eq!(
        ForeignAmount::parse("100.5", &jpy),
        Err(NumberError::TooManyDecimals)
    );
    assert_eq!(
        ForeignAmount::parse("1.234", &usd),
        Err(NumberError::TooManyDecimals)
    );
    assert_eq!(
        ForeignAmount::parse("1.2345", &bhd),
        Err(NumberError::TooManyDecimals)
    );
    assert_eq!(ForeignAmount::parse("0", &usd), Err(NumberError::Invalid));
}

// ── ExchangeArithmetic: amount × rate ÷ unit_size, HALF_UP once ───────────────

#[test]
fn thb_divides_by_the_unit_once() {
    let jpy = units("JPY", "100", 0);
    let krw = units("KRW", "1000", 0);
    let usd = units("USD", "1", 2);
    let cases = [
        // 1,000 JPY at 23.5 per 100 = 235.00 (not 23,500.00 and not 2.35)
        (
            foreign("1000", &jpy),
            rate("23.5"),
            jpy.unit_size(),
            "235.00",
        ),
        (foreign("100", &jpy), rate("23.5"), jpy.unit_size(), "23.50"),
        // 1,000 KRW at 25.12345678 per 1000 = 25.12345678 → 25.12
        (
            foreign("1000", &krw),
            rate("25.12345678"),
            krw.unit_size(),
            "25.12",
        ),
        // 100 USD at 32.1 per 1 = 3210.00
        (
            foreign("100", &usd),
            rate("32.1"),
            usd.unit_size(),
            "3210.00",
        ),
    ];
    for (amount, r, unit, thb) in cases {
        assert_eq!(
            ExchangeArithmetic::thb(amount, r, unit),
            money(thb),
            "{amount:?} × {r} ÷ {unit:?}"
        );
    }
}

#[test]
fn thb_rounds_half_up_once_at_the_end() {
    let usd = units("USD", "1", 2);
    // 3 × 0.335 = 1.005 → 1.01 (HALF_UP), not 1.00 (banker's)
    assert_eq!(
        ExchangeArithmetic::thb(foreign("3", &usd), rate("0.335"), usd.unit_size()),
        money("1.01")
    );
    // 1 × 0.004 = 0.004 → 0.00; 1 × 0.005 = 0.005 → 0.01
    assert_eq!(
        ExchangeArithmetic::thb(foreign("1", &usd), rate("0.004"), usd.unit_size()),
        money("0")
    );
    assert_eq!(
        ExchangeArithmetic::thb(foreign("1", &usd), rate("0.005"), usd.unit_size()),
        money("0.01")
    );
    // rounding once: 7 JPY at 0.715 per 100 = 0.05005 → 0.05, never a rounded intermediate
    let jpy = units("JPY", "100", 0);
    assert_eq!(
        ExchangeArithmetic::thb(foreign("7", &jpy), rate("0.715"), jpy.unit_size()),
        money("0.05")
    );
}

#[test]
fn foreign_for_thb_truncates_to_the_minor_unit() {
    let jpy = units("JPY", "100", 0);
    let usd = units("USD", "1", 2);
    let bhd = units("BHD", "1", 3);
    // 100.00 THB at 23.5 per 100 JPY = 425.53… JPY → 425 (minor unit 0)
    assert_eq!(
        ExchangeArithmetic::foreign_for_thb(money("100"), rate("23.5"), &jpy),
        foreign("425", &jpy)
    );
    // 100.00 THB at 32.1 per USD = 3.1152… → 3.11
    assert_eq!(
        ExchangeArithmetic::foreign_for_thb(money("100"), rate("32.1"), &usd),
        foreign("3.11", &usd)
    );
    // 100.00 THB at 85.12345 per BHD = 1.17476… → 1.174
    assert_eq!(
        ExchangeArithmetic::foreign_for_thb(money("100"), rate("85.12345"), &bhd),
        foreign("1.174", &bhd)
    );
}

// ── Quote::per_unit beside Quote::compute ─────────────────────────────────────

#[test]
fn per_unit_buys_and_sells_through_the_one_arithmetic() {
    let jpy = units("JPY", "100", 0);
    let buy = Quote::per_unit(
        Side::Buy,
        foreign("1000", &jpy),
        rate("23.5"),
        jpy.unit_size(),
        money("5"),
    )
    .expect("a buy");
    assert_eq!(
        (buy.principal, buy.paid, buy.received),
        (money("235"), money("230"), Money::ZERO)
    );
    let sell = Quote::per_unit(
        Side::Sell,
        foreign("1000", &jpy),
        rate("23.5"),
        jpy.unit_size(),
        money("5"),
    )
    .expect("a sell");
    assert_eq!(
        (sell.principal, sell.received, sell.paid),
        (money("235"), money("240"), Money::ZERO)
    );
}

#[test]
fn per_unit_at_unit_one_agrees_with_compute() {
    let usd = units("USD", "1", 2);
    for (amount, r, fee) in [
        ("100", "32.1", "0"),
        ("3", "0.335", "0.01"),
        ("99999999.99", "32.1", "0"),
    ] {
        let old = Quote::compute(Side::Sell, money(amount), rate(r), money(fee)).expect("compute");
        let new = Quote::per_unit(
            Side::Sell,
            foreign(amount, &usd),
            rate(r),
            usd.unit_size(),
            money(fee),
        )
        .expect("per_unit");
        assert_eq!(old, new, "{amount} × {r}");
    }
}
