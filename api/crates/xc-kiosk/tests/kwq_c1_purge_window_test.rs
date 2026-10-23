//! KWQ-C1 — `KioskMachine::is_purgeable_for` (design `docs/design/blueprint/kiosk-shared-phone.md`
//! §4.2 D4, §4.4 B5): a web request waits 60 minutes for its staff scan; a kiosk quote still
//! lives 10. Only a `QUOTED` request is ever purgeable; the boundary is "older than".
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.

use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};
use xc_kiosk::domain::kiosk::entities::kiosk_machine::KioskMachine;
use xc_kiosk::domain::kiosk::entities::kiosk_request::KioskRequest;
use xc_kiosk::domain::kiosk::value_objects::kiosk_quote::KioskQuote;
use xc_kiosk::domain::kiosk::value_objects::kiosk_status::KioskStatus;
use xc_kiosk::domain::kiosk::value_objects::kiosk_time::KioskTime;
use xc_kiosk::domain::kiosk::value_objects::payment_method::PaymentMethod;

const T0: i64 = 1_791_262_800_000;

fn request(status: KioskStatus) -> KioskRequest {
    KioskRequest {
        id: "r1".to_owned(),
        status,
        created: KioskTime::from_epoch_millis(T0),
        staff: None,
        quote: KioskQuote {
            side: Side::Buy,
            currency: CurrencyCode::parse("USD").unwrap(),
            rate: Rate::parse("35.00").unwrap(),
            unit_size: UnitSize::parse("1").unwrap(),
            line_id: None,
            foreign: Money::from_minor(10_000).unwrap(),
            thb: Money::from_minor(350_000).unwrap(),
            input_amount: Money::from_minor(10_000).unwrap(),
            payment_method: PaymentMethod::Cash,
        },
        transaction: None,
    }
}

fn at(ms: i64) -> KioskTime {
    KioskTime::from_epoch_millis(T0 + ms)
}

#[test]
fn the_windows_are_ten_minutes_for_the_kiosk_and_sixty_for_the_web() {
    assert_eq!(KioskMachine::KIOSK_QUOTE_LIFE_MS, 600_000);
    assert_eq!(KioskMachine::WEB_REQUEST_LIFE_MS, 3_600_000);
}

#[test]
fn a_kiosk_quote_is_purged_after_ten_minutes_as_today() {
    let r = request(KioskStatus::Quoted);
    assert!(!KioskMachine::is_purgeable_for(&r, false, at(600_000)));
    assert!(KioskMachine::is_purgeable_for(&r, false, at(600_001)));
    // and the kiosk's own call keeps its answer
    assert_eq!(
        KioskMachine::is_purgeable(&r, at(600_001)),
        KioskMachine::is_purgeable_for(&r, false, at(600_001))
    );
}

#[test]
fn a_web_request_waits_sixty_minutes() {
    let r = request(KioskStatus::Quoted);
    assert!(
        !KioskMachine::is_purgeable_for(&r, true, at(600_001)),
        "not at the kiosk's 10 min"
    );
    assert!(!KioskMachine::is_purgeable_for(&r, true, at(59 * 60_000)));
    assert!(!KioskMachine::is_purgeable_for(&r, true, at(3_600_000)));
    assert!(KioskMachine::is_purgeable_for(&r, true, at(3_600_001)));
}

#[test]
fn only_a_quoted_request_is_ever_purgeable() {
    for status in [
        KioskStatus::Waiting,
        KioskStatus::Accepted,
        KioskStatus::IdChecked,
        KioskStatus::Completed,
        KioskStatus::Cancelled,
    ] {
        let r = request(status);
        assert!(
            !KioskMachine::is_purgeable_for(&r, true, at(10 * 3_600_000)),
            "{status:?}"
        );
        assert!(
            !KioskMachine::is_purgeable_for(&r, false, at(10 * 3_600_000)),
            "{status:?}"
        );
    }
}
