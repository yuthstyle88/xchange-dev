//! Stock delta.

use std::collections::HashMap;

use xc_kernel::money::{CurrencyCode, Money, MoneyDelta, Quote, Side};

/// Direction of a manual stock movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

/// Errors that can occur during stock changes or shift reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StockError {
    #[error("THB is not a stock currency")]
    NotAStockCurrency,
    #[error("insufficient stock for {0}")]
    Insufficient(CurrencyCode),
}

/// A signed amount per currency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockDelta {
    map: HashMap<CurrencyCode, MoneyDelta>,
}

impl StockDelta {
    /// An empty delta with zero changes.
    pub fn none() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Constructs a delta from pairs. Pairs for the same currency add up;
    /// entries that come to zero are not kept.
    pub fn of(pairs: &[(CurrencyCode, MoneyDelta)]) -> Self {
        let mut map = HashMap::new();
        for &(currency, delta) in pairs {
            let current = map.get(&currency).copied().unwrap_or(MoneyDelta::ZERO);
            let updated = current.add(delta);
            if updated.is_zero() {
                map.remove(&currency);
            } else {
                map.insert(currency, updated);
            }
        }
        Self { map }
    }

    /// Creates a delta for an exchange transaction.
    /// Foreign: `+amount` on Buy, `-amount` on Sell.
    /// Baht moves only when `cash`: `+quote.received` on Sell, `-quote.paid` on Buy.
    /// Returns `Err(NotAStockCurrency)` when `currency` is THB.
    pub fn for_tx(
        side: Side,
        currency: CurrencyCode,
        amount: Money,
        quote: &Quote,
        cash: bool,
    ) -> Result<Self, StockError> {
        if currency.is_thb() {
            return Err(StockError::NotAStockCurrency);
        }

        let mut pairs = Vec::with_capacity(2);
        let foreign_delta = match side {
            Side::Buy => MoneyDelta::from(amount),
            Side::Sell => MoneyDelta::ZERO.minus(amount),
        };
        pairs.push((currency, foreign_delta));

        if cash {
            let thb_delta = match side {
                Side::Buy => MoneyDelta::ZERO.minus(quote.paid),
                Side::Sell => MoneyDelta::from(quote.received),
            };
            pairs.push((CurrencyCode::THB, thb_delta));
        }

        Ok(Self::of(&pairs))
    }

    /// Creates a delta for a manual movement.
    /// `+amount` for `In`, `-amount` for `Out`; baht is allowed here.
    pub fn for_movement(currency: CurrencyCode, direction: Direction, amount: Money) -> Self {
        let delta = match direction {
            Direction::In => MoneyDelta::from(amount),
            Direction::Out => MoneyDelta::ZERO.minus(amount),
        };
        Self::of(&[(currency, delta)])
    }

    /// Gets the delta for a given currency (zero if absent).
    pub fn get(&self, currency: CurrencyCode) -> MoneyDelta {
        self.map.get(&currency).copied().unwrap_or(MoneyDelta::ZERO)
    }

    /// Returns non-zero entries in register order (`CurrencyCode::all()`, THB first).
    pub fn non_zero(&self) -> Vec<(CurrencyCode, MoneyDelta)> {
        let mut result = Vec::new();
        for &code in CurrencyCode::all() {
            if let Some(&delta) = self.map.get(&code) {
                if !delta.is_zero() {
                    result.push((code, delta));
                }
            }
        }
        result
    }
}
