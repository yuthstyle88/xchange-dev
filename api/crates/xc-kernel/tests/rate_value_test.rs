//! Ticket #315 — `Rate::value()` is public and is the number `Display` prints.
//! Written before the implementation. Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! Design: `docs/design/xchange-v3.md` class `Rate { +parse(str) Result~Rate~;
//! +value() Decimal }` (docs PR 312). Four places outside `xc-kernel` rebuild
//! the decimal from the `Display` text instead (`device_save_step.rs:34-35`,
//! `kiosk_repository.rs:108-110`, `validation.rs:62-64`,
//! `kiosk_machine.rs:21-31`); `value()` replaces them, so it must give the same
//! decimal they build — four places, the same digits — or a caller that moves
//! onto it changes what it hands on.

use std::str::FromStr;

use rust_decimal::Decimal;
use xc_kernel::money::Rate;

/// Typed text → the four-place text `Display` prints.
const CASES: [(&str, &str); 4] = [
    ("32.1", "32.1000"),
    ("0.0001", "0.0001"),
    ("100000000", "100000000.0000"),
    ("0.2195", "0.2195"),
];

#[test]
fn value_is_the_rate_as_display_prints_it() {
    for (typed, shown) in CASES {
        let rate = Rate::parse(typed).unwrap_or_else(|e| panic!("{typed:?}: {e:?}"));
        let value = rate.value();
        assert_eq!(
            value,
            Decimal::from_str(shown).expect("a decimal"),
            "{typed:?}: the same number"
        );
        assert_eq!(value.scale(), 4, "{typed:?}: four places");
        assert_eq!(
            value.to_string(),
            rate.to_string(),
            "{typed:?}: the same text as Display"
        );
    }
}
