//! Tests for the Bangkok day boundary in document references (ticket #191, IN-3 of #88).

use chrono::{TimeZone, Utc};
use xc_exchange::domain::exchange::value_objects::document_ref::DocumentRef;
use xc_kernel::mode::Mode;

/// Ticket #670 (TX-2, D-4): the counter key names the organisation. Updated by the ticket author
/// (claude/f03345fe) on the lane controller's ruling of 2026-10-10; OR-1's first organisation.
const ORG_1: uuid::Uuid = uuid::Uuid::from_u128(1);

#[test]
fn document_ref_and_counter_key_change_day_at_17_00_utc_live_and_demo() {
    let before = Utc.with_ymd_and_hms(2026, 10, 6, 16, 59, 59).unwrap();
    let after = Utc.with_ymd_and_hms(2026, 10, 6, 17, 0, 0).unwrap();

    // LIVE mode at 16:59:59Z (Bangkok 23:59:59)
    assert_eq!(
        DocumentRef::format(Mode::Live, before, 1),
        "XC-20261006-000001"
    );
    assert_eq!(
        DocumentRef::counter_key(ORG_1, Mode::Live, before),
        "document-00000000-0000-0000-0000-000000000001-LIVE-20261006"
    );

    // LIVE mode at 17:00:00Z (Bangkok 00:00:00 next day)
    assert_eq!(
        DocumentRef::format(Mode::Live, after, 1),
        "XC-20261007-000001"
    );
    assert_eq!(
        DocumentRef::counter_key(ORG_1, Mode::Live, after),
        "document-00000000-0000-0000-0000-000000000001-LIVE-20261007"
    );

    // DEMO mode at 16:59:59Z (Bangkok 23:59:59)
    assert_eq!(
        DocumentRef::format(Mode::Demo, before, 1),
        "TEST-XC-20261006-000001"
    );
    assert_eq!(
        DocumentRef::counter_key(ORG_1, Mode::Demo, before),
        "document-00000000-0000-0000-0000-000000000001-DEMO-20261006"
    );

    // DEMO mode at 17:00:00Z (Bangkok 00:00:00 next day)
    assert_eq!(
        DocumentRef::format(Mode::Demo, after, 1),
        "TEST-XC-20261007-000001"
    );
    assert_eq!(
        DocumentRef::counter_key(ORG_1, Mode::Demo, after),
        "document-00000000-0000-0000-0000-000000000001-DEMO-20261007"
    );
}
