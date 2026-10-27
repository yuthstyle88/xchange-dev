//! CustomerDetails value object and validation.

use chrono::NaiveDate;
use pos_error::{AppError, AppResult};
use xc_kernel::input::{InputError, calendar_date, text};
use xc_kernel::mode::Mode;

use crate::domain::customer::commands::reserve_customer::ReserveCustomer;
use crate::domain::customer::value_objects::customer_field::{CustomerField, CustomerRefusal};

pub const INVALID_NATIONALITY: &str = "สัญชาติไม่ถูกต้อง";
pub const NOT_THIS_PASSPORT: &str = "รหัสลูกค้าไม่ตรงกับพาสปอร์ต กรุณายืนยันข้อมูลลูกค้าใหม่";
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
    /// The details, or the first fault in payload order with the field it is about
    /// (`rpc-refusal-field.md` RF-D2, RF-D6).
    pub fn check(cmd: &ReserveCustomer) -> Result<Self, CustomerRefusal> {
        let name = parse_text(&cmd.name, 100, true, CustomerField::Name)?.to_owned();
        let passport_no =
            parse_text(&cmd.passport_no, 30, true, CustomerField::PassportNo)?.to_uppercase();
        let nationality =
            parse_text(&cmd.nationality, 60, true, CustomerField::Nationality)?.to_uppercase();
        let birth_date = parse_date(&cmd.birth_date, CustomerField::BirthDate)?;
        let passport_expiry = parse_date(&cmd.passport_expiry, CustomerField::PassportExpiry)?;
        let mode = match cmd.mode.as_str() {
            "LIVE" => Mode::Live,
            "DEMO" => Mode::Demo,
            _ => {
                return Err(CustomerRefusal {
                    field: CustomerField::Mode,
                    sentence: BAD_MODE,
                });
            }
        };

        if nationality.len() != 3 || !nationality.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err(CustomerRefusal {
                field: CustomerField::Nationality,
                sentence: INVALID_NATIONALITY,
            });
        }

        Ok(Self {
            name,
            passport_no,
            nationality,
            birth_date,
            passport_expiry,
            mode,
        })
    }

    pub fn parse(cmd: &ReserveCustomer) -> AppResult<Self> {
        Self::check(cmd).map_err(AppError::from)
    }
}

fn parse_text(
    raw: &str,
    max_utf16: usize,
    required: bool,
    field: CustomerField,
) -> Result<&str, CustomerRefusal> {
    text(raw, max_utf16, required).map_err(|e| CustomerRefusal {
        field,
        sentence: e.sentence(),
    })
}

fn parse_date(raw: &str, field: CustomerField) -> Result<Option<NaiveDate>, CustomerRefusal> {
    let trimmed = raw.trim();
    if trimmed.encode_utf16().count() > 10 {
        return Err(CustomerRefusal {
            field,
            sentence: InputError::Text.sentence(),
        });
    }
    if trimmed.is_empty() {
        return Ok(None);
    }
    calendar_date(trimmed)
        .map(Some)
        .map_err(|e| CustomerRefusal {
            field,
            sentence: e.sentence(),
        })
}
