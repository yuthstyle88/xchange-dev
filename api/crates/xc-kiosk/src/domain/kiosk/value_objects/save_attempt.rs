use xc_kernel::money::{CurrencyCode, Money, Side};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveAttempt {
    pub live: bool,
    pub side: Side,
    pub currency: CurrencyCode,
    pub amount: Money,
    pub method: String,
    pub submission_id: String,
}
