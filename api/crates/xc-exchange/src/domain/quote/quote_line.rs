//! Quote line domain entity.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};

#[derive(Debug, Clone, PartialEq)]
pub struct QuoteLine {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub quote_id: Uuid,
    pub published_rate_line_id: Uuid,
    pub shop_side: Side,
    pub currency: CurrencyCode,
    pub denomination_group: String,
    pub quantity: ForeignAmount,
    pub unit_size: UnitSize,
    pub base_rate: Rate,
    pub final_rate: Rate,
    pub thb_amount: Money,
    pub fee_thb: Money,
    pub settle_thb: Money,
    pub line_no: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl QuoteLine {
    pub fn rounding_thb(&self) -> Decimal {
        let thb = Decimal::from_i128_with_scale(self.thb_amount.minor_i128(), 2);
        let fee = Decimal::from_i128_with_scale(self.fee_thb.minor_i128(), 2);
        let settle = Decimal::from_i128_with_scale(self.settle_thb.minor_i128(), 2);
        let u = match self.shop_side {
            Side::Buy => thb - fee,
            Side::Sell => thb + fee,
        };
        let mut r = settle - u;
        r.rescale(2);
        r
    }
}
