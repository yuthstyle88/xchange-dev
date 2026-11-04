//! Backdated keying (`docs/design/backdated-keying.md` §6; owner rule R4): one line priced at the rate the shop
//! really used, as typed, never the published rate. Pure.

use xc_kernel::money::{ExchangeArithmetic, ForeignAmount, Money, Rate, Side, UnitSize};

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
        match self {
            Self::Overflow => "quantity",
            Self::SettleNotPositive => "feeThb",
            Self::SettleMismatch { .. } => "expectedSettleThb",
        }
    }

    /// `[required]` The sentence: for `SettleMismatch` it contains the computed figure as `Money` prints it
    /// (e.g. `16075.00`).
    pub fn message(self) -> String {
        match self {
            Self::Overflow => "amount overflows".to_string(),
            Self::SettleNotPositive => "settle amount must be positive".to_string(),
            Self::SettleMismatch { computed } => {
                format!("expected settle does not match computed {computed}")
            }
        }
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
        let thb_amount = ExchangeArithmetic::try_thb(quantity, rate, unit_size)
            .map_err(|_| PricingRefusal::Overflow)?;
        let settle_minor = match side {
            Side::Buy => thb_amount
                .minor_i128()
                .checked_sub(fee_thb.minor_i128())
                .ok_or(PricingRefusal::Overflow)?,
            Side::Sell => thb_amount
                .minor_i128()
                .checked_add(fee_thb.minor_i128())
                .ok_or(PricingRefusal::Overflow)?,
        };
        if settle_minor <= 0 {
            return Err(PricingRefusal::SettleNotPositive);
        }
        let settle_thb =
            Money::try_from_minor_i128(settle_minor).map_err(|_| PricingRefusal::Overflow)?;
        if let Some(expected) = expected_settle_thb
            && expected != settle_thb
        {
            return Err(PricingRefusal::SettleMismatch {
                computed: settle_thb,
            });
        }
        Ok(PricedLine {
            thb_amount,
            fee_thb,
            settle_thb,
        })
    }
}
