//! Input parsing and validation for payment methods (PM-API, design §5.5.2).

use serde_json::Value;
use xc_kernel::payment::{ChannelRule, METHODS, PaymentRules};

/// Forbidden control characters: U+0000..=U+0008, U+000B, U+000C, U+000E..=U+001F.
fn has_forbidden_control(s: &str) -> bool {
    s.chars().any(|c| {
        let u = c as u32;
        (u <= 0x0008) || u == 0x000B || u == 0x000C || (0x000E..=0x001F).contains(&u)
    })
}

/// Parses and validates `methods` JSON value according to rows 9–14 of §5.5.2.
pub fn parse_methods(value: &Value) -> Result<PaymentRules, (u16, &'static str)> {
    let Some(items) = value.as_array() else {
        return Err((400, "กรุณาตั้งค่าให้ครบทั้ง 5 ช่องทาง"));
    };
    if items.len() != 5 {
        return Err((400, "กรุณาตั้งค่าให้ครบทั้ง 5 ช่องทาง"));
    }

    // Check for duplicate string IDs (row 9).
    let mut seen_ids = Vec::with_capacity(5);
    for item in items {
        if let Some(id_str) = item
            .as_object()
            .and_then(|o| o.get("id"))
            .and_then(|v| v.as_str())
        {
            if seen_ids.contains(&id_str) {
                return Err((400, "กรุณาตั้งค่าให้ครบทั้ง 5 ช่องทาง"));
            }
            seen_ids.push(id_str);
        }
    }

    // Validate each channel in METHODS order:
    let mut channel_rules = Vec::with_capacity(5);
    for &method_name in &METHODS {
        // Find matching item
        let Some(obj) = items.iter().find_map(|item| {
            let o = item.as_object()?;
            if o.get("id").and_then(|v| v.as_str()) == Some(method_name) {
                Some(o)
            } else {
                None
            }
        }) else {
            return Err((400, "ข้อมูลช่องทางชำระเงินไม่ถูกต้อง"));
        };

        // Check enabled, buy, sell are booleans (row 10)
        let (Some(enabled), Some(buy), Some(sell)) = (
            obj.get("enabled").and_then(|v| v.as_bool()),
            obj.get("buy").and_then(|v| v.as_bool()),
            obj.get("sell").and_then(|v| v.as_bool()),
        ) else {
            return Err((400, "ข้อมูลช่องทางชำระเงินไม่ถูกต้อง"));
        };

        // Row 12: enabled on, with neither direction
        if enabled && !buy && !sell {
            return Err((400, "ช่องทางที่เปิดต้องเลือกการรับเงินหรือจ่ายเงินอย่างน้อยหนึ่งประเภท"));
        }

        // Row 11: buy on a channel other than Cash / Bank Transfer
        if buy && method_name != "Cash" && method_name != "Bank Transfer" {
            return Err((400, "การจ่ายเงินบาทให้ลูกค้ารองรับเงินสดและโอนธนาคาร"));
        }

        // Row 13: account & instruction string, length, control chars
        let Some(account_raw) = obj.get("account").and_then(|v| v.as_str()) else {
            return Err((400, "ข้อความยาวเกินกำหนดหรือไม่ถูกต้อง"));
        };
        if account_raw.encode_utf16().count() > 100 || has_forbidden_control(account_raw) {
            return Err((400, "ข้อความยาวเกินกำหนดหรือไม่ถูกต้อง"));
        }

        let Some(instruction_raw) = obj.get("instruction").and_then(|v| v.as_str()) else {
            return Err((400, "ข้อความยาวเกินกำหนดหรือไม่ถูกต้อง"));
        };
        if instruction_raw.encode_utf16().count() > 400 || has_forbidden_control(instruction_raw) {
            return Err((400, "ข้อความยาวเกินกำหนดหรือไม่ถูกต้อง"));
        }

        channel_rules.push(ChannelRule {
            id: method_name.to_string(),
            enabled,
            buy,
            sell,
            account: account_raw.trim().to_string(),
            instruction: instruction_raw.trim().to_string(),
        });
    }

    // Row 14: at least one channel enabled for BUY, and at least one for SELL
    if !channel_rules.iter().any(|c| c.enabled && c.buy)
        || !channel_rules.iter().any(|c| c.enabled && c.sell)
    {
        return Err((
            400,
            "ต้องเหลือช่องทางรับเงินและจ่ายเงินบาทอย่างน้อยประเภทละ 1 ช่องทาง",
        ));
    }

    let rules = PaymentRules::from_rows(channel_rules)
        .expect("validated channel rules construct PaymentRules");
    Ok(rules)
}

/// Parses and validates `version` according to row 15 of §5.5.2.
pub fn parse_version(value: Option<&Value>) -> Result<i64, (u16, &'static str)> {
    let Some(val) = value else {
        return Err((400, "เวอร์ชันข้อมูลไม่ถูกต้อง"));
    };

    let Some(num) = val.as_number() else {
        return Err((400, "เวอร์ชันข้อมูลไม่ถูกต้อง"));
    };

    const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991; // 2^53 - 1

    if let Some(i) = num.as_i64() {
        if (0..=MAX_SAFE_INTEGER).contains(&i) {
            return Ok(i);
        }
        return Err((400, "เวอร์ชันข้อมูลไม่ถูกต้อง"));
    }

    if let Some(u) = num.as_u64() {
        if u <= MAX_SAFE_INTEGER as u64 {
            return Ok(u as i64);
        }
        return Err((400, "เวอร์ชันข้อมูลไม่ถูกต้อง"));
    }

    if let Some(f) = num.as_f64() {
        if f >= 0.0 && f <= MAX_SAFE_INTEGER as f64 && f.fract() == 0.0 {
            return Ok(f as i64);
        }
        return Err((400, "เวอร์ชันข้อมูลไม่ถูกต้อง"));
    }

    Err((400, "เวอร์ชันข้อมูลไม่ถูกต้อง"))
}
