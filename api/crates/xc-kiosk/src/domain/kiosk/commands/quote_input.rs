use xc_kernel::money::{CurrencyCode, Money, Side};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteInput {
    pub side: Option<Side>,
    pub currency: Option<CurrencyCode>,
    pub amount: Option<Money>,
    pub payment_method: Option<String>,
}
