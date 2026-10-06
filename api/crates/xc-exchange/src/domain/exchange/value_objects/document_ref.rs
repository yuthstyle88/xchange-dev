//! DocumentRef value object and formatting.

use chrono::{DateTime, Utc};

use crate::domain::customer::value_objects::mode::Mode;

pub struct DocumentRef;

impl DocumentRef {
    pub fn format(mode: Mode, now: DateTime<Utc>, number: i64) -> String {
        let bangkok_offset = chrono::FixedOffset::east_opt(7 * 3600).expect("UTC+7 offset");
        let bkk_time = now.with_timezone(&bangkok_offset);
        let date_str = bkk_time.format("%Y%m%d");
        let prefix = match mode {
            Mode::Demo => "TEST-XC-",
            Mode::Live => "XC-",
        };
        format!("{prefix}{date_str}-{number:06}")
    }

    pub fn counter_key(mode: Mode, now: DateTime<Utc>) -> String {
        let bangkok_offset = chrono::FixedOffset::east_opt(7 * 3600).expect("UTC+7 offset");
        let bkk_time = now.with_timezone(&bangkok_offset);
        let date_str = bkk_time.format("%Y%m%d");
        let mode_str = match mode {
            Mode::Demo => "DEMO",
            Mode::Live => "LIVE",
        };
        format!("document-{mode_str}-{date_str}")
    }
}
