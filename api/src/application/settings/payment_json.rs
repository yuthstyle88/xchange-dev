//! JSON formatting for payment rules (PM-API, design §5.5.2).

use serde_json::{Value, json};
use xc_kernel::payment::{ChannelRule, PaymentRules};

use crate::application::compat::v2_json::V2Json;

/// Converts payment rules to V2Json tree (dropping account if `!full`).
pub fn rules_v2_json(rules: &PaymentRules, full: bool) -> V2Json {
    let rows = rules
        .rows()
        .iter()
        .map(|r| {
            let mut pairs = vec![
                ("id", V2Json::text(&r.id)),
                ("enabled", V2Json::Bool(r.enabled)),
                ("buy", V2Json::Bool(r.buy)),
                ("sell", V2Json::Bool(r.sell)),
            ];
            if full {
                pairs.push(("account", V2Json::text(&r.account)));
            }
            pairs.push(("instruction", V2Json::text(&r.instruction)));
            V2Json::object(pairs)
        })
        .collect();
    V2Json::Array(rows)
}

/// Converts payment rules to serde_json::Value (dropping account if `!full`).
pub fn rules_json(rules: &PaymentRules, full: bool) -> Value {
    let list: Vec<Value> = rules
        .rows()
        .iter()
        .map(|r| {
            if full {
                json!({
                    "id": r.id,
                    "enabled": r.enabled,
                    "buy": r.buy,
                    "sell": r.sell,
                    "account": r.account,
                    "instruction": r.instruction,
                })
            } else {
                json!({
                    "id": r.id,
                    "enabled": r.enabled,
                    "buy": r.buy,
                    "sell": r.sell,
                    "instruction": r.instruction,
                })
            }
        })
        .collect();
    Value::Array(list)
}

/// Parses payment rules from stored JSON array in database.
pub fn rules_from_json(value: &Value) -> Option<PaymentRules> {
    let arr = value.as_array()?;
    if arr.len() != 5 {
        return None;
    }
    let mut channel_rules = Vec::with_capacity(5);
    for item in arr {
        let id = item.get("id")?.as_str()?.to_string();
        let enabled = item.get("enabled")?.as_bool()?;
        let buy = item.get("buy")?.as_bool()?;
        let sell = item.get("sell")?.as_bool()?;
        let account = item
            .get("account")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let instruction = item
            .get("instruction")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        channel_rules.push(ChannelRule {
            id,
            enabled,
            buy,
            sell,
            account,
            instruction,
        });
    }
    PaymentRules::from_rows(channel_rules)
}
