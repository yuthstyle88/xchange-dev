//! KWQ-E1 — `KioskSettings` (design `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D14,
//! §4.4 B18): the defaults of a branch with no row, and the range `result_seconds` must stay in
//! (`3..=120`, the same as the migration's CHECK), refused with the field's name.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.

use xc_kernel::ids::BranchId;
use xc_kiosk::domain::kiosk::entities::kiosk_settings::{
    DEFAULT_RESULT_SECONDS, InvalidSetting, KioskSettings, MAX_RESULT_SECONDS, MIN_RESULT_SECONDS,
};

#[test]
fn a_branch_with_no_row_reads_the_defaults() {
    let d = KioskSettings::defaults(BranchId::new("MAIN"));
    assert_eq!(
        d,
        KioskSettings {
            branch_id: BranchId::new("MAIN"),
            result_seconds: DEFAULT_RESULT_SECONDS,
            updated_by: None,
            updated_at: None,
            version: 0,
        }
    );
    assert_eq!(DEFAULT_RESULT_SECONDS, 10);
}

#[test]
fn result_seconds_inside_the_range_is_accepted() {
    for v in [MIN_RESULT_SECONDS, 10, MAX_RESULT_SECONDS] {
        assert_eq!(KioskSettings::validate_result_seconds(i64::from(v)), Ok(v));
    }
    assert_eq!((MIN_RESULT_SECONDS, MAX_RESULT_SECONDS), (3, 120));
}

#[test]
fn result_seconds_outside_the_range_is_refused_naming_the_field() {
    let refused = Err(InvalidSetting {
        field: "resultSeconds",
    });
    for v in [
        i64::from(MIN_RESULT_SECONDS) - 1,
        i64::from(MAX_RESULT_SECONDS) + 1,
        0,
        -5,
        i64::from(i32::MAX) + 1,
        i64::MIN,
    ] {
        assert_eq!(KioskSettings::validate_result_seconds(v), refused, "{v}");
    }
}
