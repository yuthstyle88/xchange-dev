//! The kiosk's cap of 10,000,000.00 holds for an amount past 64 bits.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! `KioskMachine::quote` refuses an amount above 10,000,000.00 with
//! `AmountOrCurrencyInvalid` (`kiosk_machine.rs`, step 3). Since #37,
//! `Money::try_from_minor_i128` is public and builds an amount of up to 96 bits.
//! `Money::minor()` cuts such an amount to 64 bits (`d.mantissa() as i64`), so a
//! cap that compares `minor()` sees what is left after the cut, not the amount:
//! 2^64 + 100,000 hundredths reads as 1,000.00 and 2^63 reads as a negative
//! number, and both pass. The cap must compare the whole amount.
//!
//! The ten-million boundary itself (`10000000` accepted, `10000000.01` refused)
//! is held by `kiosk_machine_test.rs`; this file repeats it only as the guard
//! that the cap still lets the largest legal amount through.

mod kiosk_common;

use kiosk_common::*;
use xc_kernel::money::{Money, Side};

/// Noon in Bangkok on 6 October 2026.
const T0: i64 = 1_791_262_800_000;
const ID: &str = "3fa9c1d2-7e57-4000-8000-00000000cafe";

fn open_at(rate_text: &str) -> CounterFacts {
    CounterFacts {
        open: true,
        rate: Some(rate(rate_text)),
    }
}

fn wants(side: Side, hundredths: i128) -> QuoteInput {
    QuoteInput {
        side: Some(side),
        currency: Some(code("USD")),
        amount: Some(Money::try_from_minor_i128(hundredths).expect("an amount Money can hold")),
        payment_method: None,
    }
}

fn quote(side: Side, hundredths: i128) -> Result<KioskRequest, KioskError> {
    KioskMachine::quote(
        &wants(side, hundredths),
        &open_at("32.1"),
        ID,
        KioskTime::from_epoch_millis(T0),
    )
}

/// 2^64 + 100,000 hundredths: `minor()` reads it as 100,000 (1,000.00).
const WRAPS_TO_A_THOUSAND: i128 = (1i128 << 64) + 100_000;
/// 2^63 hundredths: `minor()` reads it as `i64::MIN`.
const WRAPS_NEGATIVE: i128 = 1i128 << 63;
/// 2^64 + 1,000,000,000 hundredths: `minor()` reads it as exactly the cap.
const WRAPS_TO_THE_CAP: i128 = (1i128 << 64) + 1_000_000_000;

#[test]
fn the_three_amounts_are_what_this_file_says_they_are() {
    // The premise of every test below: these amounts exist, and their 64-bit cut
    // is under the cap. If `minor()` changes, this test says so first.
    for (hundredths, cut) in [
        (WRAPS_TO_A_THOUSAND, 100_000i64),
        (WRAPS_NEGATIVE, i64::MIN),
        (WRAPS_TO_THE_CAP, 1_000_000_000),
    ] {
        let m = Money::try_from_minor_i128(hundredths).expect("Money holds 96 bits");
        assert_eq!(m.minor_i128(), hundredths);
        assert_eq!(m.minor(), cut, "the 64-bit cut of {hundredths}");
    }
}

#[test]
fn an_amount_past_64_bits_that_cuts_to_a_thousand_is_refused() {
    for side in [Side::Buy, Side::Sell] {
        assert_eq!(
            quote(side, WRAPS_TO_A_THOUSAND).map(|r| r.quote.foreign),
            Err(KioskError::AmountOrCurrencyInvalid),
            "{side:?}"
        );
    }
}

#[test]
fn an_amount_whose_64_bit_cut_is_negative_is_refused() {
    for side in [Side::Buy, Side::Sell] {
        assert_eq!(
            quote(side, WRAPS_NEGATIVE).map(|r| r.quote.foreign),
            Err(KioskError::AmountOrCurrencyInvalid),
            "{side:?}"
        );
    }
}

#[test]
fn an_amount_past_64_bits_that_cuts_to_exactly_the_cap_is_refused() {
    for side in [Side::Buy, Side::Sell] {
        assert_eq!(
            quote(side, WRAPS_TO_THE_CAP).map(|r| r.quote.foreign),
            Err(KioskError::AmountOrCurrencyInvalid),
            "{side:?}"
        );
    }
}

#[test]
fn the_largest_legal_amount_is_still_quoted() {
    for side in [Side::Buy, Side::Sell] {
        assert!(
            quote(side, 1_000_000_000).is_ok(),
            "{side:?} at 10,000,000.00"
        );
        assert_eq!(
            quote(side, 1_000_000_001).map(|r| r.quote.foreign),
            Err(KioskError::AmountOrCurrencyInvalid),
            "{side:?} at 10,000,000.01"
        );
    }
}
