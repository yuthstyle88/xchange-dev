//! `KioskSettings` (design `kiosk-shared-phone.md` §4.2 D14, §4.4 B18): per-branch queue-machine
//! settings. Only `result_seconds` remains (how long the issued number stays on the queue
//! machine). A branch with no stored row reads `KioskSettings::defaults`.

use chrono::{DateTime, Utc};
use xc_kernel::ids::BranchId;

/// `[proposed]` default and range, the same as the migration's DEFAULT and CHECK.
pub const DEFAULT_RESULT_SECONDS: i32 = 10;
pub const MIN_RESULT_SECONDS: i32 = 3;
pub const MAX_RESULT_SECONDS: i32 = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KioskSettings {
    pub branch_id: BranchId,
    pub result_seconds: i32,
    /// Who saved the row last; `None` for defaults.
    pub updated_by: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
    /// Bumped on every save; `0` for defaults (the poll seam's version).
    pub version: i64,
}

/// A refused value: the field it names, for `400 validationFailed` (B18).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidSetting {
    pub field: &'static str,
}

impl KioskSettings {
    /// The settings of a branch with no stored row: `DEFAULT_RESULT_SECONDS`, no author, version 0.
    pub fn defaults(branch_id: BranchId) -> Self {
        Self {
            branch_id,
            result_seconds: DEFAULT_RESULT_SECONDS,
            updated_by: None,
            updated_at: None,
            version: 0,
        }
    }

    /// `Ok(result_seconds)` inside `MIN_RESULT_SECONDS..=MAX_RESULT_SECONDS`, otherwise
    /// `Err(InvalidSetting { field: "resultSeconds" })`.
    pub fn validate_result_seconds(result_seconds: i64) -> Result<i32, InvalidSetting> {
        if (i64::from(MIN_RESULT_SECONDS)..=i64::from(MAX_RESULT_SECONDS)).contains(&result_seconds)
        {
            Ok(result_seconds as i32)
        } else {
            Err(InvalidSetting {
                field: "resultSeconds",
            })
        }
    }
}
