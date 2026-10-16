//! Intent parser and intent representation (design §7, §9.1 APP-6).

use rust_decimal::Decimal;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, CustomerSide, ForeignAmount, Money};

/// The kinds of intent recognized by the assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IntentKind {
    Quote,
    Queue,
    Alert,
    Branch,
    Unknown,
}

/// A parsed intent extracted from customer text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub kind: IntentKind,
    pub currency: Option<CurrencyCode>,
    pub customer_side: Option<CustomerSide>,
    pub quantity: Option<ForeignAmount>,
    pub budget_thb: Option<Money>,
    pub branch: Option<BranchId>,
}

impl Intent {
    pub fn unknown() -> Self {
        Self {
            kind: IntentKind::Unknown,
            currency: None,
            customer_side: None,
            quantity: None,
            budget_thb: None,
            branch: None,
        }
    }
}

/// Deterministic parser turning text into an Intent (design §7, P11).
pub struct IntentParser;

impl IntentParser {
    /// Parses customer input in the given locale (`th`, `en`, `zh`).
    pub fn parse(text: &str, locale: &str) -> Intent {
        let redacted = xc_kernel::pii::redact_text(text);
        Self::parse_in(text, &redacted, locale)
    }

    /// Parses customer input given both raw and redacted texts.
    ///
    /// Per ruling 1 amended: the amount is taken from the raw text, while the branch
    /// is taken from the redacted text so phone numbers, OTPs, or IDs never leak into
    /// `Intent.branch`.
    pub fn parse_in(raw_text: &str, redacted_text: &str, locale: &str) -> Intent {
        let raw_trimmed = raw_text.trim();
        let redacted_trimmed = redacted_text.trim();
        if raw_trimmed.is_empty() {
            return Intent::unknown();
        }

        // 1. QUEUE intent: check queue keywords
        if is_queue(raw_trimmed) {
            let branch = extract_branch(redacted_trimmed, None, locale);
            return Intent {
                kind: IntentKind::Queue,
                currency: None,
                customer_side: None,
                quantity: None,
                budget_thb: None,
                branch,
            };
        }

        // 2. ALERT intent: check alert keywords
        if is_alert(raw_trimmed) {
            let currency = find_currency(raw_trimmed);
            return Intent {
                kind: IntentKind::Alert,
                currency,
                customer_side: None,
                quantity: None,
                budget_thb: None,
                branch: None,
            };
        }

        // 3. QUOTE intent: requires currency and quantity (quantity from raw_text!)
        if let Some(currency) = find_currency(raw_trimmed) {
            if let Some(quantity) = find_quantity(raw_trimmed, currency) {
                let customer_side =
                    extract_customer_side(raw_trimmed).unwrap_or(CustomerSide::CustomerBuy);
                let branch = extract_branch(redacted_trimmed, Some(currency), locale);
                return Intent {
                    kind: IntentKind::Quote,
                    currency: Some(currency),
                    customer_side: Some(customer_side),
                    quantity: Some(quantity),
                    budget_thb: None,
                    branch,
                };
            }
        }

        // 4. Default to Unknown
        Intent::unknown()
    }
}

fn is_queue(text: &str) -> bool {
    text.contains("คิว") || text.to_ascii_lowercase().contains("queue") || text.contains("排队")
}

fn is_alert(text: &str) -> bool {
    text.contains("เตือน")
        || text.to_ascii_lowercase().contains("alert")
        || text.to_ascii_lowercase().contains("notify")
        || text.contains("提醒")
}

fn find_currency(text: &str) -> Option<CurrencyCode> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            let word = &text[start..i];
            if word.len() == 3 && word.chars().all(|c| c.is_ascii_uppercase()) {
                if let Ok(code) = CurrencyCode::parse(word) {
                    return Some(code);
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

fn find_quantity(text: &str, currency: CurrencyCode) -> Option<ForeignAmount> {
    let curr_str = currency.as_str();
    if let Some(pos) = text.find(curr_str) {
        // 1. Check digits immediately before the currency
        let before = text[..pos].trim_end();
        let num_rev: String = before
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
            .collect();
        let num_str: String = num_rev.chars().rev().collect();
        if let Some(amt) = parse_amount(&num_str) {
            return Some(amt);
        }

        // 2. Check digits immediately after the currency
        let after = text[pos + curr_str.len()..].trim_start();
        let num_str: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
            .collect();
        if let Some(amt) = parse_amount(&num_str) {
            return Some(amt);
        }
    }
    None
}

fn parse_amount(s: &str) -> Option<ForeignAmount> {
    if s.is_empty() {
        return None;
    }
    let clean: String = s.chars().filter(|c| *c != ',').collect();
    if let Ok(dec) = clean.parse::<Decimal>() {
        if dec > Decimal::ZERO && dec <= Decimal::from(Money::MAX_INPUT_UNITS) {
            return Some(ForeignAmount::from_decimal(dec));
        }
    }
    None
}

fn extract_customer_side(text: &str) -> Option<CustomerSide> {
    // Sell keywords checked first
    if text.contains("ขาย") || text.to_ascii_lowercase().contains("sell") || text.contains("卖")
    {
        return Some(CustomerSide::CustomerSell);
    }

    // Buy/want keywords
    if text.contains("ซื้อ")
        || text.contains("อยาก")
        || text.contains("ต้องการ")
        || text.to_ascii_lowercase().contains("buy")
        || text.to_ascii_lowercase().contains("want")
        || text.to_ascii_lowercase().contains("need")
        || text.to_ascii_lowercase().contains("get")
        || text.to_ascii_lowercase().contains("exchange")
        || text.contains("买")
        || text.contains("想")
    {
        return Some(CustomerSide::CustomerBuy);
    }

    None
}

fn extract_branch(text: &str, currency: Option<CurrencyCode>, locale: &str) -> Option<BranchId> {
    match locale {
        "th" => extract_branch_th(text, currency)
            .or_else(|| extract_branch_en(text, currency))
            .or_else(|| extract_branch_zh(text, currency)),
        "zh" => extract_branch_zh(text, currency)
            .or_else(|| extract_branch_en(text, currency))
            .or_else(|| extract_branch_th(text, currency)),
        _ => extract_branch_en(text, currency)
            .or_else(|| extract_branch_th(text, currency))
            .or_else(|| extract_branch_zh(text, currency)),
    }
}

fn extract_branch_en(text: &str, currency: Option<CurrencyCode>) -> Option<BranchId> {
    let lower = text.to_ascii_lowercase();
    for (idx, _) in lower.match_indices("at") {
        let before_ok = idx == 0 || !text[..idx].chars().next_back().unwrap().is_alphanumeric();
        let after_idx = idx + 2;
        let after_ok =
            after_idx < text.len() && text[after_idx..].chars().next().unwrap().is_whitespace();
        if before_ok && after_ok {
            let remainder = text[after_idx..].trim_start();
            let token: String = remainder
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            if is_valid_branch_token(&token, currency) {
                return Some(BranchId::new(token));
            }
        }
    }
    None
}

fn extract_branch_th(text: &str, currency: Option<CurrencyCode>) -> Option<BranchId> {
    for prefix in &["ที่สาขา", "สาขา", "ที่"] {
        if let Some(pos) = text.find(prefix) {
            let remainder = text[pos + prefix.len()..].trim_start();
            let token: String = remainder
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            if is_valid_branch_token(&token, currency) {
                return Some(BranchId::new(token));
            }
        }
    }
    None
}

fn extract_branch_zh(text: &str, currency: Option<CurrencyCode>) -> Option<BranchId> {
    if let Some(pos) = text.find('在') {
        let remainder = text[pos + '在'.len_utf8()..].trim_start();
        let token: String = remainder
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        if is_valid_branch_token(&token, currency) {
            return Some(BranchId::new(token));
        }
    }
    None
}

fn is_valid_branch_token(token: &str, currency: Option<CurrencyCode>) -> bool {
    BranchId::is_well_formed(token)
        && !token.contains('*')
        && xc_kernel::pii::redact_text(token) == token
        && currency.is_none_or(|c| c.as_str() != token)
}
