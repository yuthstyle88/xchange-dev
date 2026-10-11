//! The BoT row validation and mapping shared by every BoT source (design §8.3).

use std::collections::HashSet;

use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;

use crate::infrastructure::rate::bot_rate_source::BOT_UNAVAILABLE;

const ALLOWED_CURRENCIES: [&str; 6] = ["USD", "EUR", "GBP", "JPY", "CNY", "AUD"];

fn parse_rate(val: Option<&serde_json::Value>) -> Option<Decimal> {
    let text = match val? {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let d = xc_kernel::money::plain_decimal(&text)?;
    if d <= Decimal::ZERO {
        return None;
    }
    Some(d)
}

fn decimal_to_number(d: Decimal) -> serde_json::Number {
    let normalized = d.normalize();
    serde_json::from_str::<serde_json::Number>(&normalized.to_string())
        .expect("normalized decimal is valid json number")
}

/// The rows `BotRateSource` emits for the bank's table rows `detail`.
pub fn bot_rows(detail: &[serde_json::Value]) -> AppResult<Vec<serde_json::Value>> {
    let mut kept_rows = Vec::new();
    for row_val in detail {
        let Some(currency_id) = row_val.get("currency_id").and_then(|v| v.as_str()) else {
            continue;
        };
        if !ALLOWED_CURRENCIES.contains(&currency_id) {
            continue;
        }

        let Some(period) = row_val.get("period").and_then(|v| v.as_str()) else {
            return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
        };
        if period.len() != 10 {
            return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
        }
        let bytes = period.as_bytes();
        let is_date_shape = bytes.iter().enumerate().all(|(i, &b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
        if !is_date_shape {
            return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
        }

        let bs_val = row_val.get("buying_sight");
        let bt_val = row_val.get("buying_transfer");
        let selling_val = row_val.get("selling");

        let (Some(bs_dec), Some(bt_dec), Some(selling_dec)) = (
            parse_rate(bs_val),
            parse_rate(bt_val),
            parse_rate(selling_val),
        ) else {
            return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
        };

        let unit: u64 = if currency_id == "JPY" { 100 } else { 1 };

        let bs_rate = decimal_to_number(bs_dec);
        let bt_rate = decimal_to_number(bt_dec);
        let selling_rate = decimal_to_number(selling_dec);

        let original = serde_json::json!({
            "buyingSight": bs_val.unwrap().clone(),
            "buyingTransfer": bt_val.unwrap().clone(),
            "selling": selling_val.unwrap().clone(),
        });

        let row = serde_json::json!({
            "code": currency_id,
            "date": period,
            "unit": unit,
            "buyingSight": bs_rate,
            "buyingTransfer": bt_rate,
            "selling": selling_rate,
            "original": original,
        });

        kept_rows.push((currency_id.to_string(), row));
    }

    if kept_rows.len() != 6 {
        return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
    }

    let mut seen_currencies = HashSet::new();
    for (code, _) in &kept_rows {
        if !seen_currencies.insert(code) {
            return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
        }
    }
    if seen_currencies.len() != 6 {
        return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
    }

    let rows: Vec<serde_json::Value> = kept_rows.into_iter().map(|(_, r)| r).collect();
    Ok(rows)
}
