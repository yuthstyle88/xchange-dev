//! `ledger_postings` and `ledger_entries` (design §3.1). Scaffold by the ticket author: `is_balanced`
//! is `todo!()`.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use super::account::CashAccountId;

/// A posting's id (`ledger_postings.id`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PostingId(pub Uuid);

/// `ledger_postings.source_type` (design §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceType {
    Exchange,
    Transfer,
    Count,
    Opening,
    Reversal,
    Adjustment,
}

impl SourceType {
    /// The stored value: `EXCHANGE`, `TRANSFER`, `COUNT`, `OPENING`, `REVERSAL`, `ADJUSTMENT`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exchange => "EXCHANGE",
            Self::Transfer => "TRANSFER",
            Self::Count => "COUNT",
            Self::Opening => "OPENING",
            Self::Reversal => "REVERSAL",
            Self::Adjustment => "ADJUSTMENT",
        }
    }
}

/// What a posting records: one posting per `(organization, source_type, source_id, source_key)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceRef {
    pub source_type: SourceType,
    pub source_id: String,
    pub source_key: String,
}

/// One `ledger_entries` row: `quantity_signed` is never 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub account: CashAccountId,
    pub currency: CurrencyCode,
    pub quantity_signed: Decimal,
    pub valuation_thb: Decimal,
}

/// A posting with its entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posting {
    pub organization_id: String,
    pub branch: BranchId,
    pub source: SourceRef,
    pub posted_at: DateTime<Utc>,
    pub posted_by: String,
    pub entries: Vec<Entry>,
}

impl Posting {
    /// Whether, for every currency, the entries' `quantity_signed` sum to 0 (design §3.1).
    pub fn is_balanced(&self) -> bool {
        let mut sums: std::collections::HashMap<CurrencyCode, Decimal> =
            std::collections::HashMap::new();
        for entry in &self.entries {
            *sums.entry(entry.currency).or_insert(Decimal::ZERO) += entry.quantity_signed;
        }
        sums.values().all(|sum| sum.is_zero())
    }
}
