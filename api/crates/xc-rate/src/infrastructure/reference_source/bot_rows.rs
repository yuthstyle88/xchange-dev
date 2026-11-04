//! The BoT row validation and mapping shared by every BoT source (design §8.3).

use std::collections::HashSet;

use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use xc_kernel::money::CurrencyCode;

use crate::infrastructure::rate::bot_rate_source::BOT_UNAVAILABLE;
use crate::infrastructure::reference_source::bot_table::{BOT_TABLE_SENTINELS, bot_unit};

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
        if CurrencyCode::parse(currency_id).is_err() {
            continue;
        }
        let is_sentinel = BOT_TABLE_SENTINELS.contains(&currency_id);

        let valid_date = row_val
            .get("period")
            .and_then(|v| v.as_str())
            .filter(|period| {
                period.len() == 10
                    && period.as_bytes().iter().enumerate().all(|(i, &b)| {
                        if i == 4 || i == 7 {
                            b == b'-'
                        } else {
                            b.is_ascii_digit()
                        }
                    })
            });
        let Some(period) = valid_date else {
            if is_sentinel {
                return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
            }
            continue;
        };

        let bs_val = row_val.get("buying_sight");
        let bt_val = row_val.get("buying_transfer");
        let selling_val = row_val.get("selling");

        let (bs_dec, bt_dec, selling_dec) = (
            parse_rate(bs_val),
            parse_rate(bt_val),
            parse_rate(selling_val),
        );
        if is_sentinel && (bs_dec.is_none() || bt_dec.is_none() || selling_dec.is_none()) {
            return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
        }

        let unit = bot_unit(currency_id);

        let bs_rate = bs_dec.map(decimal_to_number);
        let bt_rate = bt_dec.map(decimal_to_number);
        let selling_rate = selling_dec.map(decimal_to_number);

        let original = serde_json::json!({
            "buyingSight": bs_val.cloned().unwrap_or(serde_json::Value::Null),
            "buyingTransfer": bt_val.cloned().unwrap_or(serde_json::Value::Null),
            "selling": selling_val.cloned().unwrap_or(serde_json::Value::Null),
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

    let mut seen_currencies = HashSet::new();
    for (code, _) in &kept_rows {
        if !seen_currencies.insert(code.as_str()) {
            return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
        }
    }
    if !BOT_TABLE_SENTINELS
        .iter()
        .all(|s| seen_currencies.contains(s))
    {
        return Err(AppError::Domain(BOT_UNAVAILABLE.to_string()));
    }

    let rows: Vec<serde_json::Value> = kept_rows.into_iter().map(|(_, r)| r).collect();
    Ok(rows)
}
