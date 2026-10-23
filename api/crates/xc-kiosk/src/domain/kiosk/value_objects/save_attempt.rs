use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveAttempt {
    pub live: bool,
    pub side: Side,
    pub currency: CurrencyCode,
    pub amount: ForeignAmount,
    pub method: String,
    pub submission_id: String,
}
