//! Branch hours entity and window computation in branch timezone.

use std::str::FromStr;

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use pos_error::{AppError, AppResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HoursWindow {
    Closed,
    NotSet,
    Open {
        open: DateTime<Utc>,
        close: DateTime<Utc>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchHours {
    timezone: Tz,
    opening_hours: Option<serde_json::Value>,
}

fn parse_hh_mm(s: &str) -> Option<(u32, u32)> {
    if s.len() != 5 {
        return None;
    }
    let bytes = s.as_bytes();
    if bytes[2] != b':' {
        return None;
    }
    if !bytes[0].is_ascii_digit()
        || !bytes[1].is_ascii_digit()
        || !bytes[3].is_ascii_digit()
        || !bytes[4].is_ascii_digit()
    {
        return None;
    }
    let h = (bytes[0] - b'0') as u32 * 10 + (bytes[1] - b'0') as u32;
    let m = (bytes[3] - b'0') as u32 * 10 + (bytes[4] - b'0') as u32;
    if h > 23 || m > 59 {
        return None;
    }
    Some((h, m))
}

fn local_to_utc(tz: &Tz, naive: &chrono::NaiveDateTime) -> DateTime<Utc> {
    match tz.from_local_datetime(naive) {
        chrono::LocalResult::Single(dt) => dt.with_timezone(&Utc),
        chrono::LocalResult::Ambiguous(earliest, _) => earliest.with_timezone(&Utc),
        chrono::LocalResult::None => naive.and_utc(),
    }
}

impl BranchHours {
    pub fn new(timezone: &str, opening_hours: Option<&serde_json::Value>) -> AppResult<Self> {
        let tz = Tz::from_str(timezone)
            .map_err(|_| AppError::Validation(format!("invalid IANA timezone: {timezone:?}")))?;

        let opening_hours_owned = match opening_hours {
            None => None,
            Some(val) => {
                let obj = val.as_object().ok_or_else(|| {
                    AppError::Validation("opening_hours must be a JSON object".to_string())
                })?;

                for (_day_key, day_val) in obj {
                    let arr = day_val.as_array().ok_or_else(|| {
                        AppError::Validation("day hours must be an array".to_string())
                    })?;
                    for item in arr {
                        let window_obj = item.as_object().ok_or_else(|| {
                            AppError::Validation("window entry must be an object".to_string())
                        })?;
                        let open_val = window_obj
                            .get("open")
                            .ok_or_else(|| AppError::Validation("missing open".to_string()))?;
                        let close_val = window_obj
                            .get("close")
                            .ok_or_else(|| AppError::Validation("missing close".to_string()))?;
                        let open_str = open_val.as_str().ok_or_else(|| {
                            AppError::Validation("open must be a string".to_string())
                        })?;
                        let close_str = close_val.as_str().ok_or_else(|| {
                            AppError::Validation("close must be a string".to_string())
                        })?;
                        if parse_hh_mm(open_str).is_none() || parse_hh_mm(close_str).is_none() {
                            return Err(AppError::Validation(
                                "open and close must be HH:MM format".to_string(),
                            ));
                        }
                    }
                }

                Some(val.clone())
            }
        };

        Ok(Self {
            timezone: tz,
            opening_hours: opening_hours_owned,
        })
    }

    pub fn window(&self, date: NaiveDate) -> HoursWindow {
        let hours_val = match &self.opening_hours {
            None => return HoursWindow::NotSet,
            Some(h) => h,
        };

        let obj = match hours_val.as_object() {
            None => return HoursWindow::NotSet,
            Some(o) => o,
        };

        let key = match date.weekday() {
            chrono::Weekday::Mon => "mon",
            chrono::Weekday::Tue => "tue",
            chrono::Weekday::Wed => "wed",
            chrono::Weekday::Thu => "thu",
            chrono::Weekday::Fri => "fri",
            chrono::Weekday::Sat => "sat",
            chrono::Weekday::Sun => "sun",
        };

        let arr = match obj.get(key).and_then(|v| v.as_array()) {
            None => return HoursWindow::Closed,
            Some(a) => a,
        };

        if arr.is_empty() {
            return HoursWindow::Closed;
        }

        let first = &arr[0];
        let open_str = match first.get("open").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return HoursWindow::Closed,
        };
        let close_str = match first.get("close").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return HoursWindow::Closed,
        };

        let (open_h, open_m) = match parse_hh_mm(open_str) {
            Some(p) => p,
            None => return HoursWindow::Closed,
        };
        let (close_h, close_m) = match parse_hh_mm(close_str) {
            Some(p) => p,
            None => return HoursWindow::Closed,
        };

        let open_naive = match date.and_hms_opt(open_h, open_m, 0) {
            Some(dt) => dt,
            None => return HoursWindow::Closed,
        };

        let close_naive = if (close_h, close_m) <= (open_h, open_m) {
            match date
                .succ_opt()
                .and_then(|d| d.and_hms_opt(close_h, close_m, 0))
            {
                Some(dt) => dt,
                None => return HoursWindow::Closed,
            }
        } else {
            match date.and_hms_opt(close_h, close_m, 0) {
                Some(dt) => dt,
                None => return HoursWindow::Closed,
            }
        };

        let open_utc = local_to_utc(&self.timezone, &open_naive);
        let close_utc = local_to_utc(&self.timezone, &close_naive);

        HoursWindow::Open {
            open: open_utc,
            close: close_utc,
        }
    }
}
