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

    /// The customer's side of a stored shop side, the reverse of [`CustomerSide::to_shop`]:
    /// `Side::Sell → CUSTOMER_BUY`, `Side::Buy → CUSTOMER_SELL` (design `marketing-campaigns.md` §6.4 E-10).
    pub fn from_shop(side: Side) -> Self {
        match side {
            Side::Sell => Self::CustomerBuy,
            Side::Buy => Self::CustomerSell,
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

// Written before the implementation by the ticket author claude/e2926a49 (sub-controller
// XC-BOOKING-ESTIMATE, design `marketing-campaigns.md` §6.4.5 row 20). Do not edit, delete or weaken.
#[cfg(test)]
mod from_shop_tests {
    use super::*;

    #[test]
    fn from_shop_reverses_to_shop_for_both_sides() {
        for side in [CustomerSide::CustomerBuy, CustomerSide::CustomerSell] {
            assert_eq!(CustomerSide::from_shop(side.to_shop()), side, "{side:?}");
        }
    }

    #[test]
    fn from_shop_maps_the_shop_side_to_the_customer_side() {
        assert_eq!(
            CustomerSide::from_shop(Side::Sell),
            CustomerSide::CustomerBuy
        );
        assert_eq!(
            CustomerSide::from_shop(Side::Buy),
            CustomerSide::CustomerSell
        );
    }
}
