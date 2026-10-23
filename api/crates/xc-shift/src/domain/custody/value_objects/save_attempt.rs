//! Save attempt value object for live transactions.

use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorSave {
    None,
    Unfinished,
    SavedByActor,
    SavedByOther,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveAttempt {
    pub customer_id: String,
    pub draft_id: Option<String>,
    pub side: Side,
    pub currency: CurrencyCode,
    pub amount: ForeignAmount,
    pub prior: PriorSave,
}
