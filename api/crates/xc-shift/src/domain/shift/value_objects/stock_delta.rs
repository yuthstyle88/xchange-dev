//! Stock delta.

use std::collections::HashMap;

use xc_kernel::money::{CurrencyCode, ForeignAmount, QuantityDelta, Quote, Side};

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
    #[error("an amount is past what the type holds")]
    OutOfRange,
}

/// A signed amount per currency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockDelta {
    map: HashMap<CurrencyCode, QuantityDelta>,
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
    pub fn of(pairs: &[(CurrencyCode, QuantityDelta)]) -> Self {
        let mut map = HashMap::new();
        for &(currency, delta) in pairs {
            let current = map.get(&currency).copied().unwrap_or(QuantityDelta::ZERO);
            let updated = current
                .checked_add(delta)
                .expect("StockDelta::of overflowed QuantityDelta");
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
        amount: ForeignAmount,
        quote: &Quote,
        cash: bool,
    ) -> Result<Self, StockError> {
        if currency.is_thb() {
            return Err(StockError::NotAStockCurrency);
        }

        let mut pairs = Vec::with_capacity(2);
        let foreign_delta = match side {
            Side::Buy => QuantityDelta::from(amount),
            Side::Sell => QuantityDelta::ZERO
                .checked_minus(amount)
                .map_err(|_| StockError::OutOfRange)?,
        };
        pairs.push((currency, foreign_delta));

        if cash {
            let thb_delta = match side {
                Side::Buy => QuantityDelta::ZERO
                    .checked_minus(ForeignAmount::from_decimal(
                        rust_decimal::Decimal::from_i128_with_scale(quote.paid.minor_i128(), 2),
                    ))
                    .map_err(|_| StockError::OutOfRange)?,
                Side::Sell => QuantityDelta::from(ForeignAmount::from_decimal(
                    rust_decimal::Decimal::from_i128_with_scale(quote.received.minor_i128(), 2),
                )),
            };
            pairs.push((CurrencyCode::THB, thb_delta));
        }

        Ok(Self::of(&pairs))
    }

    /// Creates a delta for a manual movement.
    /// `+amount` for `In`, `-amount` for `Out`; baht is allowed here.
    pub fn for_movement(
        currency: CurrencyCode,
        direction: Direction,
        amount: ForeignAmount,
    ) -> Self {
        let delta = match direction {
            Direction::In => QuantityDelta::from(amount),
            Direction::Out => QuantityDelta::ZERO
                .checked_minus(amount)
                .expect("ForeignAmount fits in negative QuantityDelta"),
        };
        Self::of(&[(currency, delta)])
    }

    /// Gets the delta for a given currency (zero if absent).
    pub fn get(&self, currency: CurrencyCode) -> QuantityDelta {
        self.map
            .get(&currency)
            .copied()
            .unwrap_or(QuantityDelta::ZERO)
    }

    /// Returns non-zero entries in register order (`CurrencyCode::all()`, THB first).
    pub fn non_zero(&self) -> Vec<(CurrencyCode, QuantityDelta)> {
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
