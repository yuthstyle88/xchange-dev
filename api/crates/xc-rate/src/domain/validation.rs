//! Domain validation and error constants for xc-rate.

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use pos_error::AppError;

pub const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
pub const BAD_TEXT: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";
pub const BAD_REQUEST_ID: &str = "รหัสคำขอไม่ถูกต้อง";
pub const BAD_NUMBER: &str = "จำนวนเงินหรือเรทไม่ถูกต้อง";
pub const TOO_MANY_DECIMALS: &str = "ทศนิยมเกินกำหนด";
pub const SELL_BELOW_BUY: &str = "เรทร้านขายต้องไม่น้อยกว่าเรทร้านรับซื้อ";
pub const ABANDONED: &str = "คำขอนี้ถูกยกเลิกแล้ว กรุณาโหลดข้อมูลล่าสุด";
pub const PARTNER_PAIR: &str = "กรอกชื่อคู่ค้าและเรทต้นทุนให้ครบคู่";
pub const REQUEST_CHANGED: &str = "คำขอเดิมมีข้อมูลเปลี่ยน";
pub const EDITED_ELSEWHERE: &str = "เรทถูกแก้ไขจากหน้าจออื่น กรุณารีเฟรชก่อน";
pub const NAME_TAKEN: &str = "ชื่อสาขานี้มีอยู่แล้ว";
pub const TOO_MANY_BRANCHES: &str = "รองรับสูงสุด 20 สาขา";

pub fn cut_to_millis(t: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(t.timestamp_millis())
        .single()
        .unwrap_or(t)
}

pub fn format_iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn validate_text(s: &str, max: usize, required: bool) -> Result<&str, AppError> {
    let trimmed = s.trim();
    let len = trimmed.encode_utf16().count();
    if (required && trimmed.is_empty()) || len > max {
        return Err(AppError::Domain(BAD_TEXT.to_string()));
    }
    Ok(trimmed)
}

pub fn validate_request_id(s: &str) -> Result<&str, AppError> {
    let trimmed = validate_text(s, 60, true)?;
    if trimmed.len() < 16
        || trimmed.len() > 60
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(AppError::Domain(BAD_REQUEST_ID.to_string()));
    }
    Ok(trimmed)
}
