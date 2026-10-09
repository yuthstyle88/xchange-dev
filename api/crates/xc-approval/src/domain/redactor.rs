//! `Redactor`: masks and drops sensitive values before they are stored (rates design §5.5).

use serde_json::{Map, Value};

/// Redacts sensitive fields from payloads and audit records.
pub struct Redactor;

impl Redactor {
    /// Recursively redacts sensitive fields in `value`.
    ///
    /// - Keys in `secret`, `token`, `password`, `otp` are dropped completely.
    /// - Keys in `passport`, `passportNo`, `phone`, `idNumber` are masked:
    ///   - Strings, numbers, and bools keep only their last 4 characters.
    ///   - Objects and arrays under a masked key are fully masked.
    ///   - Null stays as null.
    /// - All other fields are preserved as-is.
    pub fn redact(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut out = Map::new();
                for (k, v) in map {
                    if is_dropped_key(k) {
                        continue;
                    }
                    if is_masked_key(k) {
                        out.insert(k.clone(), mask_value(v));
                        continue;
                    }
                    out.insert(k.clone(), Self::redact(v));
                }
                Value::Object(out)
            }
            Value::Array(arr) => Value::Array(arr.iter().map(Self::redact).collect()),
            _ => value.clone(),
        }
    }
}

fn mask_value(v: &Value) -> Value {
    match v {
        Value::Null => Value::Null,
        Value::String(s) => {
            let count = s.chars().count();
            let last4: String = s.chars().skip(count.saturating_sub(4)).collect();
            Value::String(format!("***{last4}"))
        }
        Value::Number(n) => {
            let s = n.to_string();
            let count = s.chars().count();
            let last4: String = s.chars().skip(count.saturating_sub(4)).collect();
            Value::String(format!("***{last4}"))
        }
        Value::Bool(b) => {
            let s = b.to_string();
            let count = s.chars().count();
            let last4: String = s.chars().skip(count.saturating_sub(4)).collect();
            Value::String(format!("***{last4}"))
        }
        Value::Object(_) | Value::Array(_) => Value::String("***".to_string()),
    }
}

fn is_dropped_key(k: &str) -> bool {
    matches!(k, "secret" | "token" | "password" | "otp")
}

fn is_masked_key(k: &str) -> bool {
    matches!(
        k,
        "passport" | "passportNo" | "passport_no" | "phone" | "idNumber" | "id_number"
    )
}
