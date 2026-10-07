//! CustomerDetails value object and validation.

use chrono::NaiveDate;
use pos_error::{AppError, AppResult};
use xc_kernel::input::{InputError, calendar_date, text};

use crate::domain::customer::commands::reserve_customer::ReserveCustomer;
use crate::domain::customer::value_objects::mode::Mode;

pub const BAD_TEXT: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";
pub const BAD_DATE_FORMAT: &str = "รูปแบบวันที่ไม่ถูกต้อง";
pub const BAD_DATE: &str = "วันที่ไม่ถูกต้อง";
pub const BAD_MODE: &str = "กรุณาเลือกประเภทรายการ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerDetails {
    pub name: String,
    pub passport_no: String,
    pub nationality: String,
    pub birth_date: Option<NaiveDate>,
    pub passport_expiry: Option<NaiveDate>,
    pub mode: Mode,
}

impl CustomerDetails {
    pub fn parse(cmd: &ReserveCustomer) -> AppResult<Self> {
        let name = parse_text(&cmd.name, 100, true)?.to_owned();
        let passport_no = parse_text(&cmd.passport_no, 30, true)?.to_uppercase();
        let nationality = parse_text(&cmd.nationality, 60, true)?.to_uppercase();
        let birth_date = parse_date(&cmd.birth_date)?;
        let passport_expiry = parse_date(&cmd.passport_expiry)?;
        let mode = match cmd.mode.as_str() {
            "LIVE" => Mode::Live,
            "DEMO" => Mode::Demo,
            _ => return Err(AppError::Domain(BAD_MODE.to_owned())),
        };

        Ok(Self {
            name,
            passport_no,
            nationality,
            birth_date,
            passport_expiry,
            mode,
        })
    }
}

fn parse_text(raw: &str, max_utf16: usize, required: bool) -> AppResult<&str> {
    Ok(text(raw, max_utf16, required)?)
}

fn parse_date(raw: &str) -> AppResult<Option<NaiveDate>> {
    let trimmed = raw.trim();
    if trimmed.encode_utf16().count() > 10 {
        return Err(InputError::Text.into());
    }
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(calendar_date(trimmed)?))
}
