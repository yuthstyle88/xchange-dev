//! `TtFeed`: TT Currency Exchange, Pattaya head office (V.9 `src/pricing/market.mjs:7,15-28`).
//!
//! SKELETON (XC-PRICING PX-0): `parse` is PX-2's. URLs, `requests` and signatures stay.

use std::str::FromStr;

use rust_decimal::Decimal;
use serde_json::Value;

use crate::domain::pricing::market_feed::{
    MarketAccept, MarketBody, MarketFeed, MarketFeedError, MarketRequest,
};
use crate::domain::pricing::market_quote::MarketSource;
use crate::domain::pricing::market_snapshot::{MarketRow, MarketSnapshot};

pub const TT_RATES_URL: &str =
    "https://api.software.ttexchange.com/currencies?is_main=false&branch_id=1";
pub const TT_UPDATED_URL: &str =
    "https://api.software.ttexchange.com/currencies/updated_at?branch_id=1";

fn parse_name(raw: &str) -> Option<(&str, &str)> {
    let s = raw.trim();
    let b = s.as_bytes();
    if b.len() < 3 || !b[..3].iter().all(u8::is_ascii_uppercase) {
        return None;
    }
    match &b[3..] {
        b"" => Some((&s[..3], "")),
        [b'(', b'L' | b'M' | b'S', b')'] => Some((&s[..3], &s[4..5])),
        _ => None,
    }
}

fn parse_positive_decimal(val: Option<&Value>) -> Option<Decimal> {
    let d = match val? {
        Value::Number(n) => Decimal::from_str(&n.to_string()).ok()?,
        Value::String(s) => Decimal::from_str(s.trim()).ok()?,
        _ => return None,
    };
    (d > Decimal::ZERO).then_some(d)
}

fn parse_updated_at(bodies: &[MarketBody]) -> Option<String> {
    let raw = bodies.get(1)?.body.as_deref()?;
    let val: Value = serde_json::from_str(raw).ok()?;
    val.get("updated_at")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Two requests: the rates (required, JSON array) and the update time (optional, `{"updated_at": "…"}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtFeed {
    pub rates_url: String,
    pub updated_url: String,
}

impl TtFeed {
    pub fn live() -> Self {
        Self::at(TT_RATES_URL, TT_UPDATED_URL)
    }

    pub fn at(rates_url: &str, updated_url: &str) -> Self {
        Self {
            rates_url: rates_url.to_string(),
            updated_url: updated_url.to_string(),
        }
    }
}

impl MarketFeed for TtFeed {
    fn source(&self) -> MarketSource {
        MarketSource::Tt
    }

    fn requests(&self) -> Vec<MarketRequest> {
        vec![
            MarketRequest {
                url: self.rates_url.clone(),
                accept: MarketAccept::Json,
                optional: false,
            },
            MarketRequest {
                url: self.updated_url.clone(),
                accept: MarketAccept::Json,
                optional: true,
            },
        ]
    }

    /// V.9 `parseTT`: `bodies[0]` the rates array, `bodies[1]` the update time (its body may be `None`).
    /// A row is `{name: "USD(L)", description, current_buy_rate, current_sell_rate, decimals}`; a name that is not
    /// `^[A-Z]{3}(\([LMS]\))?$` or a row with neither price > 0 is skipped. Errors: `TT: unexpected response`
    /// (not an array), `TT: no rates` (nothing left).
    fn parse(&self, bodies: &[MarketBody]) -> Result<MarketSnapshot, MarketFeedError> {
        let raw = bodies
            .first()
            .and_then(|b| b.body.as_deref())
            .ok_or_else(|| MarketFeedError("TT: unexpected response".to_string()))?;
        let val: Value = serde_json::from_str(raw)
            .map_err(|_| MarketFeedError("TT: unexpected response".to_string()))?;
        let items = val
            .as_array()
            .ok_or_else(|| MarketFeedError("TT: unexpected response".to_string()))?;

        let mut rows = Vec::new();
        for item in items {
            let Some(name) = item.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some((code, band)) = parse_name(name) else {
                continue;
            };
            let buy = parse_positive_decimal(item.get("current_buy_rate"));
            let sell = parse_positive_decimal(item.get("current_sell_rate"));
            if buy.is_none() && sell.is_none() {
                continue;
            }
            let denoms = item
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let decimals = item
                .get("decimals")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .filter(|&n| n > 0);
            rows.push(MarketRow {
                code: code.to_string(),
                band: band.to_string(),
                denoms,
                buy,
                sell,
                decimals,
            });
        }

        if rows.is_empty() {
            return Err(MarketFeedError("TT: no rates".to_string()));
        }

        Ok(MarketSnapshot {
            source: MarketSource::Tt,
            source_updated_at: parse_updated_at(bodies),
            rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numeric_strings_one_sided_rates_and_non_positive_decimals() {
        let bodies = vec![MarketBody {
            url: TT_RATES_URL.to_string(),
            body: Some(
                r#"[{"name":" GBP(M) ","current_buy_rate":"43.5","current_sell_rate":0,"decimals":0}]"#
                    .to_string(),
            ),
        }];
        let snap = TtFeed::live().parse(&bodies).expect("parsed");
        assert_eq!(
            snap.rows,
            vec![MarketRow {
                code: "GBP".to_string(),
                band: "M".to_string(),
                denoms: String::new(),
                buy: Some(Decimal::from_str("43.5").unwrap()),
                sell: None,
                decimals: None,
            }]
        );
    }
}
