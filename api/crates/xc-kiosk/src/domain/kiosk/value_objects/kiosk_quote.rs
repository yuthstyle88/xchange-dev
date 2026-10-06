use xc_kernel::money::{CurrencyCode, Money, Rate, Side};

use crate::domain::kiosk::value_objects::payment_method::PaymentMethod;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KioskQuote {
    pub side: Side,
    pub currency: CurrencyCode,
    pub rate: Rate,
    pub foreign: Money,
    pub thb: Money,
    pub input_amount: Money,
    pub payment_method: PaymentMethod,
}
