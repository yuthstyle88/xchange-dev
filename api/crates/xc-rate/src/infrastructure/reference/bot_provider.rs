//! BotProvider: Bank of Thailand reference provider (§2, §3.1).

use std::str::FromStr;

use rust_decimal::Decimal;
use serde_json::Value;

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::rate::entities::rate_source::ReferenceRates;
use crate::domain::reference::reference_provider::{ParsedBatch, ParsedLine, ReferenceProvider};

const BOT_CURRENCIES: [&str; 6] = ["USD", "EUR", "GBP", "JPY", "CNY", "AUD"];

fn parse_decimal(val: &Value) -> Option<Decimal> {
    match val {
        Value::Number(n) => Decimal::from_str(&n.to_string()).ok(),
        Value::String(s) => Decimal::from_str(s).ok(),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BotProvider;

impl ReferenceProvider for BotProvider {
    fn provider(&self) -> &'static str {
        "BOT"
    }

    fn reference_type(&self) -> &'static str {
        "BOT"
    }

    fn row_key(&self, row: &Value) -> String {
        row.get("code")
            .or_else(|| row.get("currency_code"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_uppercase())
            .unwrap_or_default()
    }

    fn parse(&self, rates: &ReferenceRates) -> Result<ParsedBatch, RateEngineError> {
        let mut source_date = None;
        for r in &rates.rows {
            if let Some(d_str) = r
                .get("date")
                .or_else(|| r.get("period"))
                .and_then(|v| v.as_str())
            {
                if let Ok(nd) = chrono::NaiveDate::parse_from_str(d_str, "%Y-%m-%d") {
                    source_date = Some(nd);
                    break;
                }
            }
        }

        let fetched_at = chrono::DateTime::parse_from_rfc3339(&rates.fetched_at)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());

        let source_date = source_date.unwrap_or_else(|| fetched_at.date_naive());

        let mut lines = Vec::new();
        let mut seen_line_currencies = std::collections::HashSet::new();

        for r in &rates.rows {
            let Some(code_raw) = r
                .get("code")
                .or_else(|| r.get("currency_code"))
                .and_then(|v| v.as_str())
            else {
                continue;
            };
            let code = code_raw.trim().to_uppercase();
            if code.is_empty() || !seen_line_currencies.insert(code.clone()) {
                continue;
            }

            let unit_size = r
                .get("unit")
                .or_else(|| r.get("unit_size"))
                .and_then(|v| {
                    if let Some(u) = v.as_u64() {
                        Some(Decimal::from(u))
                    } else if let Some(s) = v.as_str() {
                        Decimal::from_str(s).ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(Decimal::ONE);

            let buy = r
                .get("buyingTransfer")
                .or_else(|| r.get("buying_transfer"))
                .or_else(|| r.get("buy"))
                .and_then(parse_decimal);

            let sell = r
                .get("selling")
                .or_else(|| r.get("sell"))
                .and_then(parse_decimal);

            lines.push(ParsedLine {
                currency: code,
                unit_size,
                buy,
                sell,
                raw: r.clone(),
                ..Default::default()
            });
        }

        let is_complete = BOT_CURRENCIES
            .iter()
            .all(|c| seen_line_currencies.contains(*c));

        Ok(ParsedBatch {
            source_date,
            source_published_at: None,
            complete: is_complete,
            lines,
        })
    }
}
