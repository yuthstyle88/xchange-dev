//! Which way the foreign currency moves, seen from the customer (design `customer-app.md` P4).
//!
//! The customer app, rate alerts (#561) and bookings (#555) speak `CUSTOMER_BUY` / `CUSTOMER_SELL`.
//! [`CustomerSide::to_shop`] is the only place that turns it into the shop's [`Side`]; no other code
//! converts, and no customer-side value reaches a desk or rate type.

use super::Side;

/// The customer's side of an exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CustomerSide {
    /// The customer buys foreign currency (the shop sells it). Wire value `CUSTOMER_BUY`.
    CustomerBuy,
    /// The customer sells foreign currency (the shop buys it). Wire value `CUSTOMER_SELL`.
    CustomerSell,
}

impl CustomerSide {
    /// The shop's side of the same exchange: `CUSTOMER_BUY → Side::Sell`, `CUSTOMER_SELL → Side::Buy`.
    pub fn to_shop(self) -> Side {
        match self {
            Self::CustomerBuy => Side::Sell,
            Self::CustomerSell => Side::Buy,
        }
    }

    /// The wire value, `CUSTOMER_BUY` or `CUSTOMER_SELL` (the `planner_drafts.customer_side` CHECK).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CustomerBuy => "CUSTOMER_BUY",
            Self::CustomerSell => "CUSTOMER_SELL",
        }
    }

    /// Reads the wire value exactly; anything else (`BUY`, `customer_buy`, …) is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "CUSTOMER_BUY" => Some(Self::CustomerBuy),
            "CUSTOMER_SELL" => Some(Self::CustomerSell),
            _ => None,
        }
    }
}
