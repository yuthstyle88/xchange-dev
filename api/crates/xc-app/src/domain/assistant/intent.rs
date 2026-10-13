//! Intent parser and intent representation (design §7, §9.1 APP-6).

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
    pub fn parse(_text: &str, _locale: &str) -> Intent {
        Intent::unknown()
    }
}
