//! `CompletedFacts` / `ReversedFacts` (design §6): what the loyalty events are built from.
//! No personal data: the phone is read by the handler when it sends (D-2), never held here.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_kernel::Mode;
use xc_kernel::money::Side;

/// One line of a completed exchange, as the wire body sends it (design §4.5, D-6).
#[derive(Debug, Clone, PartialEq)]
pub struct CompletedLine {
    pub currency: String,
    pub shop_side: Side,
    pub quantity: Decimal,
    pub unit_size: Decimal,
    pub thb_amount: Decimal,
    pub settle_thb: Decimal,
    /// `None` until #553.
    pub realized_margin_thb: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletedFacts {
    pub organization_id: Uuid,
    pub transaction_id: Uuid,
    pub document_ref: String,
    pub customer_id: Option<String>,
    pub mode: Mode,
    pub branch_id: String,
    pub completed_at: DateTime<Utc>,
    pub business_date: NaiveDate,
    pub reversible_until: DateTime<Utc>,
    pub lines: Vec<CompletedLine>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReversedFacts {
    pub organization_id: Uuid,
    pub reversal_id: Uuid,
    /// The **original** transaction (D-4).
    pub transaction_id: Uuid,
    pub customer_id: Option<String>,
    pub posted_at: DateTime<Utc>,
    /// Cumulative: the total reversed so far for that transaction (design §4.5).
    pub amount_thb: Decimal,
    pub full: bool,
}
