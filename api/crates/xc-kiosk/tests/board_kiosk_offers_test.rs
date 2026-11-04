//! Board currencies BC-3 (`docs/design/board-currencies.md` D-6, §4): the kiosk offers what the branch's rows say,
//! with no default list. Written before the implementation by the sub-controller (claude/94389a6b). Do not edit,
//! delete or weaken; add tests in a file of your own.

use xc_kernel::money::CurrencyCode;
use xc_kiosk::domain::kiosk::value_objects::currency_switch::CurrencySwitch;
use xc_kiosk::domain::kiosk::value_objects::kiosk_currencies::KioskCurrencies;

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).expect("a known code")
}

#[test]
fn bc3_no_rows_offers_nothing() {
    let offers = KioskCurrencies::new(&[]);
    assert!(!offers.offers(code("USD")), "no default list");
}

#[test]
fn bc3_a_row_on_is_offered_and_a_row_off_is_not() {
    let offers = KioskCurrencies::new(&[
        CurrencySwitch {
            code: code("SGD"),
            enabled: true,
        },
        CurrencySwitch {
            code: code("USD"),
            enabled: false,
        },
    ]);
    assert!(offers.offers(code("SGD")));
    assert!(!offers.offers(code("USD")));
}
