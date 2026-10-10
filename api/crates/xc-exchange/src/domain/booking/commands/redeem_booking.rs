//! Redeem booking command.

use rust_decimal::Decimal;
use xc_kernel::money::{CurrencyCode, Money, Side};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedeemBooking {
    pub booking_id: String,
    pub submission_id: String,
    pub live: bool,
    pub side: Side,
    pub currency: CurrencyCode,
    pub amount: Money,
    pub rate: Decimal,
    pub form_booking: String,
    pub customer_id: String,
    pub customer_confirmed: bool,
}
