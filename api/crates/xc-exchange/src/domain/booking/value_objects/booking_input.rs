//! Booking input parsing and validation functions.

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::booking_error::BookingError;

/// Validates a required text field: trimmed string, 1..=max UTF-16 units.
pub fn required_text(v: &Value, max: usize) -> Result<String, BookingError> {
    let s = v.as_str().ok_or(BookingError::FillIn)?;
    let trimmed = s.trim();
    let count = trimmed.encode_utf16().count();
    if count == 0 || count > max {
        return Err(BookingError::FillIn);
    }
    Ok(trimmed.to_owned())
}

/// Validates a contact phone: trimmed string, text <= 30, only `+0-9 ()-`, at least 7 digits.
pub fn contact_phone(v: &Value) -> Result<String, BookingError> {
    let s = v.as_str().ok_or(BookingError::BadPhone)?;
    let trimmed = s.trim();
    let count = trimmed.encode_utf16().count();
    if count == 0 || count > 30 {
        return Err(BookingError::BadPhone);
    }

    let mut digit_count = 0;
    for b in trimmed.bytes() {
        if b.is_ascii_digit() {
            digit_count += 1;
        } else if !(b == b'+' || b == b' ' || b == b'(' || b == b')' || b == b'-') {
            return Err(BookingError::BadPhone);
        }
    }

    if digit_count < 7 {
        return Err(BookingError::BadPhone);
    }

    Ok(trimmed.to_owned())
}

/// Validates a member secret: text <= 80 first, then `^[a-f0-9-]{36}$`.
pub fn member_secret(v: &Value) -> Result<String, BookingError> {
    let text = required_text(v, 80)?;
    if text.len() != 36
        || !text
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f' | b'-'))
    {
        return Err(BookingError::BadMemberCode);
    }
    Ok(text)
}

/// SHA-256 hex digest of the member secret alone.
pub fn token_hash(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    format!("{:x}", hasher.finalize())
}
