//! Redeem booking command.

use xc_kernel::money::{CurrencyCode, Money, Rate, Side};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedeemBooking {
    pub booking_id: String,
    pub submission_id: String,
    pub live: bool,
    pub side: Side,
    pub currency: CurrencyCode,
    pub amount: Money,
    pub rate: Rate,
    pub form_booking: String,
    pub customer_id: String,
    pub customer_confirmed: bool,
}
