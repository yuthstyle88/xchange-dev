//! STUB (ticket #88).

use chrono::NaiveDate;

use super::InputError;

/// The earliest day accepted.
pub const DATE_FLOOR: NaiveDate = match NaiveDate::from_ymd_opt(1920, 4, 2) {
    Some(d) => d,
    None => unreachable!(),
};

/// A calendar day written `dddd-dd-dd`.
pub fn calendar_date(text: &str) -> Result<NaiveDate, InputError> {
    if text.len() != 10 {
        return Err(InputError::DateFormat);
    }
    let b = text.as_bytes();
    if b[4] != b'-' || b[7] != b'-' {
        return Err(InputError::DateFormat);
    }
    if !b[0..4].iter().all(u8::is_ascii_digit)
        || !b[5..7].iter().all(u8::is_ascii_digit)
        || !b[8..10].iter().all(u8::is_ascii_digit)
    {
        return Err(InputError::DateFormat);
    }

    let y = (b[0] - b'0') as i32 * 1000
        + (b[1] - b'0') as i32 * 100
        + (b[2] - b'0') as i32 * 10
        + (b[3] - b'0') as i32;
    let m = (b[5] - b'0') as u32 * 10 + (b[6] - b'0') as u32;
    let d = (b[8] - b'0') as u32 * 10 + (b[9] - b'0') as u32;

    let date = NaiveDate::from_ymd_opt(y, m, d).ok_or(InputError::Date)?;
    if date < DATE_FLOOR {
        return Err(InputError::Date);
    }
    Ok(date)
}
