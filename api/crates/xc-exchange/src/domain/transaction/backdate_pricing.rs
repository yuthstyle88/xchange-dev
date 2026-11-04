//! Backdated keying (`docs/design/backdated-keying.md` §6; owner rule R4): one line priced at the rate the shop
//! really used, as typed, never the published rate. Pure.

use xc_kernel::money::{ForeignAmount, Money, Rate, Side, UnitSize};

/// `[required]` The line's THB figures (`transaction_lines.thb_amount`, `fee_thb`, `settle_thb`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PricedLine {
    pub thb_amount: Money,
    pub fee_thb: Money,
    pub settle_thb: Money,
}

/// `[required]` Why a typed line is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PricingRefusal {
    /// `quantity × rate ÷ unit_size` cannot be held (`ExchangeArithmetic::try_thb`'s overflow).
    Overflow,
    /// The settle figure is zero or below (a fee as large as the amount).
    SettleNotPositive,
    /// `expected_settle_thb` was sent and differs from the computed settle, named here.
    SettleMismatch { computed: Money },
}

impl PricingRefusal {
    /// `[required]` The input field it names: `"quantity"` for `Overflow`, `"feeThb"` for `SettleNotPositive`,
    /// `"expectedSettleThb"` for `SettleMismatch`.
    pub fn field(self) -> &'static str {
        todo!("BK-3: backdated-keying.md §6")
    }

    /// `[required]` The sentence: for `SettleMismatch` it contains the computed figure as `Money` prints it
    /// (e.g. `16075.00`).
    pub fn message(self) -> String {
        todo!("BK-3: backdated-keying.md §6")
    }
}

pub struct BackdatePricing;

impl BackdatePricing {
    /// `[required]` `thb_amount = ExchangeArithmetic::try_thb(quantity, rate, unit_size)` (HALF_UP, 2 dp, once).
    /// BUY (the shop buys foreign): `settle_thb = thb_amount − fee_thb`. SELL: `settle_thb = thb_amount + fee_thb`.
    /// `settle_thb <= 0` is `SettleNotPositive`. `expected_settle_thb`, when given, must equal `settle_thb`, else
    /// `SettleMismatch { computed: settle_thb }`. No cash rounding is applied.
    pub fn price(
        side: Side,
        quantity: ForeignAmount,
        rate: Rate,
        unit_size: UnitSize,
        fee_thb: Money,
        expected_settle_thb: Option<Money>,
    ) -> Result<PricedLine, PricingRefusal> {
        let _ = (
            side,
            quantity,
            rate,
            unit_size,
            fee_thb,
            expected_settle_thb,
        );
        todo!("BK-3: backdated-keying.md §6")
    }
}
