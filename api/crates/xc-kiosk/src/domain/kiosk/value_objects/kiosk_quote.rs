use xc_kernel::money::{CurrencyCode, Money, Rate, Side, UnitSize};

use crate::domain::kiosk::value_objects::counter_facts::LineId;
use crate::domain::kiosk::value_objects::payment_method::PaymentMethod;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KioskQuote {
    pub side: Side,
    pub currency: CurrencyCode,
    pub rate: Rate,
    pub unit_size: UnitSize,
    pub line_id: Option<LineId>,
    pub foreign: Money,
    pub thb: Money,
    pub input_amount: Money,
    pub payment_method: PaymentMethod,
}
