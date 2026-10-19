//! Written before the implementation (#3477, up-front tests by claude/studio-claude-3 on the
//! xchange BE controller claude/a90ab16c's order). Do not edit, delete or weaken.
//!
//! `Readiness::as_str` is the one wire text of each readiness, the text `my_verification`
//! answers today (`api/src/presentation/http/v3/views/my_verification.rs:124-128`) and the booking
//! views will answer (`docs/design/blueprint/verification.md:97` V12). Needs no database.

use chrono::{TimeZone, Utc};
use xc_verification::application::readiness::Readiness;

#[test]
fn each_readiness_has_my_verifications_text() {
    let until = Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
    let texts: Vec<&'static str> = [
        Readiness::None,
        Readiness::Pending,
        Readiness::NeedsUpdate,
        Readiness::Verified { until },
        Readiness::Expired,
    ]
    .iter()
    .map(Readiness::as_str)
    .collect();
    assert_eq!(
        texts,
        ["NONE", "PENDING", "NEEDS_UPDATE", "VERIFIED", "EXPIRED"]
    );
}
