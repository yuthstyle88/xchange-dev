//! Shift ledger.

use xc_kernel::money::{CurrencyCode, MoneyDelta};

use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::stock_delta::{StockDelta, StockError};

/// Calculates expected shift balances and difference reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShiftLedger;

impl ShiftLedger {
    /// Calculates the expected balances after applying every delta to opening balances.
    /// Only the end balance is judged: if any currency ends negative, returns
    /// `Err(Insufficient(code))` naming the first such currency in register order.
    /// A currency whose sum, or end balance, is past what the types hold is
    /// `Err(OutOfRange)` — never left out of the result.
    pub fn expected(opening: &Balances, deltas: &[StockDelta]) -> Result<Balances, StockError> {
        let mut final_pairs = Vec::new();
        for &code in CurrencyCode::all() {
            let mut current = MoneyDelta::from(opening.get(code));
            for delta in deltas {
                current = current
                    .checked_add(delta.get(code))
                    .map_err(|_| StockError::OutOfRange)?;
            }
            if current.is_negative() {
                return Err(StockError::Insufficient(code));
            }
            let money = current.to_money().map_err(|_| StockError::OutOfRange)?;
            if !money.is_zero() {
                final_pairs.push((code, money));
            }
        }
        Ok(Balances::of(&final_pairs))
    }

    /// Verifies that balance + delta >= 0 for every currency.
    /// Returns `Ok(())` if non-negative; otherwise `Err(Insufficient(first in register order))`.
    pub fn ensure_non_negative(balance: &Balances, delta: &StockDelta) -> Result<(), StockError> {
        for &code in CurrencyCode::all() {
            let bal = balance.get(code);
            let d = delta.get(code);
            let sum = MoneyDelta::from(bal)
                .checked_add(d)
                .map_err(|_| StockError::OutOfRange)?;
            if sum.is_negative() {
                return Err(StockError::Insufficient(code));
            }
        }
        Ok(())
    }

    /// Computes `counted - expected` per currency.
    pub fn difference(expected: &Balances, counted: &Balances) -> StockDelta {
        let mut pairs = Vec::new();
        for &code in CurrencyCode::all() {
            let exp = expected.get(code);
            let cnt = counted.get(code);
            let diff = MoneyDelta::from(cnt).minus(exp);
            if !diff.is_zero() {
                pairs.push((code, diff));
            }
        }
        StockDelta::of(&pairs)
    }
}
