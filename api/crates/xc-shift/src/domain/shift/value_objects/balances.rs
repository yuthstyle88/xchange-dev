//! Shift balances.

use std::collections::HashMap;

use xc_kernel::money::{CurrencyCode, ForeignAmount};

/// A non-negative amount per currency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Balances {
    map: HashMap<CurrencyCode, ForeignAmount>,
}

impl Balances {
    /// An empty set of balances (zero of everything).
    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Constructs balances from pairs. A later pair for the same currency replaces the earlier one.
    /// Entries with an amount of zero are not kept.
    pub fn of(pairs: &[(CurrencyCode, ForeignAmount)]) -> Self {
        let mut map = HashMap::new();
        for &(currency, amount) in pairs {
            if amount.is_zero() {
                map.remove(&currency);
            } else {
                map.insert(currency, amount);
            }
        }
        Self { map }
    }

    /// Gets the balance for a given currency (zero when absent).
    pub fn get(&self, currency: CurrencyCode) -> ForeignAmount {
        self.map
            .get(&currency)
            .copied()
            .unwrap_or(ForeignAmount::ZERO)
    }

    /// Returns non-zero entries in register order (`CurrencyCode::all()`, THB first).
    pub fn non_zero(&self) -> Vec<(CurrencyCode, ForeignAmount)> {
        let mut result = Vec::new();
        for &code in CurrencyCode::all() {
            if let Some(&amount) = self.map.get(&code) {
                if !amount.is_zero() {
                    result.push((code, amount));
                }
            }
        }
        result
    }
}
