//! Branch calendar day helper (ticket #5220).

use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::Tz;

/// The start (inclusive) of `now`'s calendar day in `tz`, as a UTC instant.
pub fn local_day_start(tz: Tz, now: DateTime<Utc>) -> DateTime<Utc> {
    let midnight = now
        .with_timezone(&tz)
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("00:00:00 is valid");
    tz.from_local_datetime(&midnight)
        .earliest()
        .or_else(|| {
            tz.from_local_datetime(&(midnight + Duration::hours(1)))
                .earliest()
        })
        .expect("local day start exists")
        .with_timezone(&Utc)
}
