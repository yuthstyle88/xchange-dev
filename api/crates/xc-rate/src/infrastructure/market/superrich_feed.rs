//! `SuperRichFeed`: Super Rich Thailand, head office H01 (V.9 `src/pricing/market.mjs:8,30-63`).
//!
//! SKELETON (XC-PRICING PX-0): `parse` is PX-3's. URL, `requests` and signatures stay.

use rust_decimal::Decimal;
use serde_json::Value;

use crate::domain::pricing::market_feed::{
    MarketAccept, MarketBody, MarketFeed, MarketFeedError, MarketRequest,
};
use crate::domain::pricing::market_quote::MarketSource;
use crate::domain::pricing::market_snapshot::{MarketRow, MarketSnapshot};

pub const SR_URL: &str = "https://www.superrichthailand.com/exchange-rate";

/// One request: the HTML page, whose rates are server-rendered into Next.js flight chunks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuperRichFeed {
    pub url: String,
}

impl SuperRichFeed {
    pub fn live() -> Self {
        Self::at(SR_URL)
    }

    pub fn at(url: &str) -> Self {
        Self {
            url: url.to_string(),
        }
    }
}

impl MarketFeed for SuperRichFeed {
    fn source(&self) -> MarketSource {
        MarketSource::SuperRich
    }

    fn requests(&self) -> Vec<MarketRequest> {
        vec![MarketRequest {
            url: self.url.clone(),
            accept: MarketAccept::Html,
            optional: false,
        }]
    }

    /// V.9 `parseSuperRich`: concatenate every decodable `self.__next_f.push([1,"…"])` string, cut the object after
    /// `"exchangeList":` by brace depth (quotes and escapes respected), read `"date":"YYYY-MM-DD"`. One row per
    /// (code, list index), codes `^[A-Z]{3,4}$`, `denomRem`/`buyText`/`sellText`; a row with neither price > 0 is
    /// dropped. Errors: `SR: exchangeList not found`, `SR: exchangeList is cut off`, `SR: no rates`.
    fn parse(&self, bodies: &[MarketBody]) -> Result<MarketSnapshot, MarketFeedError> {
        let text = bodies
            .first()
            .and_then(|b| b.body.as_deref())
            .unwrap_or_default();
        let flight = collect_flight(text);

        let Some(key_pos) = flight.find("\"exchangeList\":") else {
            return Err(MarketFeedError("SR: exchangeList not found".into()));
        };
        let after_key = &flight[key_pos..];
        let Some(brace_offset) = after_key.find('{') else {
            return Err(MarketFeedError("SR: exchangeList is cut off".into()));
        };
        let from_brace = &after_key[brace_offset..];
        let Some(end) = find_object_end(from_brace) else {
            return Err(MarketFeedError("SR: exchangeList is cut off".into()));
        };
        let list: serde_json::Map<String, Value> = serde_json::from_str(&from_brace[..end])
            .map_err(|_| MarketFeedError("SR: exchangeList is cut off".into()))?;

        let source_updated_at = find_first_date(&flight);
        let mut rows = Vec::new();

        for (code, value) in &list {
            if !(3..=4).contains(&code.len()) || !code.bytes().all(|b| b.is_ascii_uppercase()) {
                continue;
            }
            let Some(items) = value.as_array() else {
                continue;
            };
            for (idx, item) in items.iter().enumerate() {
                let buy = parse_positive_decimal(item.get("buyText"));
                let sell = parse_positive_decimal(item.get("sellText"));
                if buy.is_none() && sell.is_none() {
                    continue;
                }
                let denoms = match item.get("denomRem") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Number(n)) => n.to_string(),
                    _ => String::new(),
                };
                rows.push(MarketRow {
                    code: code.clone(),
                    band: idx.to_string(),
                    denoms,
                    buy,
                    sell,
                    decimals: None,
                });
            }
        }

        if rows.is_empty() {
            return Err(MarketFeedError("SR: no rates".into()));
        }

        Ok(MarketSnapshot {
            source: MarketSource::SuperRich,
            source_updated_at,
            rows,
        })
    }
}

fn collect_flight(mut rest: &str) -> String {
    const PREFIX: &str = "self.__next_f.push([1,";
    let mut flight = String::new();
    while let Some(pos) = rest.find(PREFIX) {
        rest = &rest[pos + PREFIX.len()..];
        if !rest.starts_with('"') {
            continue;
        }
        let mut escaped = false;
        let mut literal_end = None;
        for (idx, ch) in rest.char_indices().skip(1) {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                literal_end = Some(idx + 1);
                break;
            }
        }
        let Some(end) = literal_end else {
            break;
        };
        if rest[end..].starts_with("])") {
            if let Ok(decoded) = serde_json::from_str::<String>(&rest[..end]) {
                flight.push_str(&decoded);
            }
            rest = &rest[end + 2..];
        } else {
            rest = &rest[end..];
        }
    }
    flight
}

fn find_object_end(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut quoted = false;
    let mut escaped = false;
    for (idx, ch) in s.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
            continue;
        }
        match ch {
            '"' => quoted = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_first_date(flight: &str) -> Option<String> {
    const KEY: &str = "\"date\":\"";
    let mut rest = flight;
    while let Some(pos) = rest.find(KEY) {
        let after = &rest[pos + KEY.len()..];
        if after.len() >= 11 {
            let b = after.as_bytes();
            let valid = b[0..4].iter().all(u8::is_ascii_digit)
                && b[4] == b'-'
                && b[5..7].iter().all(u8::is_ascii_digit)
                && b[7] == b'-'
                && b[8..10].iter().all(u8::is_ascii_digit)
                && b[10] == b'"';
            if valid {
                return Some(after[..10].to_string());
            }
        }
        rest = after;
    }
    None
}

fn parse_positive_decimal(v: Option<&Value>) -> Option<Decimal> {
    let text = match v? {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let d = text.parse::<Decimal>().ok()?;
    (d > Decimal::ZERO).then_some(d)
}
