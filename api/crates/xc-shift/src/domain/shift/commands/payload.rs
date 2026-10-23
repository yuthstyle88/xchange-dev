//! Payload reader helpers for shift commands.

use rust_decimal::Decimal;
use serde_json::Value;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money};

use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::shift_error::ShiftError;

pub fn read_text(p: &Value, key: &str, max: usize, required: bool) -> Result<String, ShiftError> {
    let raw = match p.get(key) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(_) => return Err(ShiftError::TextInvalid),
    };
    xc_kernel::input::text(&raw, max, required)
        .map(str::to_owned)
        .map_err(|_| ShiftError::TextInvalid)
}

pub fn read_request_id(p: &Value, key: &str) -> Result<String, ShiftError> {
    let text = read_text(p, key, 60, true)?;
    xc_kernel::input::request_id(&text).map_err(|_| ShiftError::RequestIdInvalid)?;
    Ok(text)
}

pub fn read_amount(p: &Value, key: &str, positive: bool) -> Result<Money, ShiftError> {
    match p.get(key) {
        None | Some(Value::Null) => Err(ShiftError::AmountMissing),
        Some(val) => read_amount_from_val(val, positive),
    }
}

pub fn read_amount_from_val(val: &Value, positive: bool) -> Result<Money, ShiftError> {
    let text = match val {
        Value::Null => return Err(ShiftError::AmountMissing),
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err(ShiftError::AmountMissing);
            }
            trimmed.to_owned()
        }
        Value::Number(n) => n.to_string(),
        _ => return Err(ShiftError::AmountInvalid),
    };
    let m = Money::parse(&text).map_err(|_| ShiftError::AmountInvalid)?;
    if positive && m.is_zero() {
        return Err(ShiftError::AmountInvalid);
    }
    Ok(m)
}

pub fn read_balances(p: &Value, key: &str) -> Result<Balances, ShiftError> {
    let obj = match p.get(key) {
        None | Some(Value::Null) => return Ok(Balances::empty()),
        Some(Value::Object(map)) => map,
        Some(_) => return Ok(Balances::empty()),
    };

    let mut pairs = Vec::new();
    for &code in CurrencyCode::all() {
        let code_str = code.as_str();
        if let Some(val) = obj.get(code_str) {
            if val.is_null() {
                continue;
            }
            let amount = read_amount_from_val(val, false)?;
            if !amount.is_zero() {
                pairs.push((
                    code,
                    ForeignAmount::from_decimal(Decimal::from_i128_with_scale(
                        amount.minor_i128(),
                        2,
                    )),
                ));
            }
        }
    }
    Ok(Balances::of(&pairs))
}
