//! DocumentRef value object and formatting.

use chrono::{DateTime, Utc};
use uuid::Uuid;
use xc_kernel::time::bangkok;

use xc_kernel::mode::Mode;

pub struct DocumentRef;

impl DocumentRef {
    pub fn format(mode: Mode, now: DateTime<Utc>, number: i64) -> String {
        let bkk_time = now.with_timezone(&bangkok());
        let date_str = bkk_time.format("%Y%m%d");
        let prefix = match mode {
            Mode::Demo => "TEST-XC-",
            Mode::Live => "XC-",
        };
        format!("{prefix}{date_str}-{number:06}")
    }

    pub fn counter_key(organization: Uuid, mode: Mode, now: DateTime<Utc>) -> String {
        let bkk_time = now.with_timezone(&bangkok());
        let date_str = bkk_time.format("%Y%m%d");
        let mode_str = match mode {
            Mode::Demo => "DEMO",
            Mode::Live => "LIVE",
        };
        format!("document-{organization}-{mode_str}-{date_str}")
    }
}
