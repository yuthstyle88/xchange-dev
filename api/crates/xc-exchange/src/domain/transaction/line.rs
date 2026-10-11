//! TransactionLine entity.

use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, MoneyDelta, Rate, Side, UnitSize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionLine {
    pub id: Uuid,
    pub quote_line_id: Option<Uuid>,
    pub shop_side: Side,
    pub currency: CurrencyCode,
    pub quantity: ForeignAmount,
    pub unit_size: UnitSize,
    pub rate_snapshot: Rate,
    pub reference_rate: Option<Rate>,
    pub thb_amount: Money,
    pub fee_thb: Money,
    pub settle_thb: Money,
    pub cost_basis_thb: Option<Money>,
    /// Signed: a SELL below cost has a negative margin (TX-5 #873, ruling Q5).
    pub realized_margin_thb: Option<MoneyDelta>,
    pub line_no: i32,
}
