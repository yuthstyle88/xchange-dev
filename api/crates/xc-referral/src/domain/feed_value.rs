//! `FeedValue` (design §6.2, condition 1): the one place that computes `value_satang`, the THB
//! amount of an exchange in satang, fixed once at completion and never re-priced.

use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;

pub struct FeedValue;

impl FeedValue {
    /// Until #551: `transactions.principal × 100`, exact because `principal` has 2 places
    /// (`20261007190000_transactions.sql:19`). #551 replaces this body with the sum of the
    /// completed lines' THB value.
    ///
    /// `Err(AppError::Validation(..))` when `principal` is not > 0, has more than 2 decimal places,
    /// or does not fit an `i64` of satang.
    pub fn of(principal: Decimal) -> AppResult<i64> {
        if principal <= Decimal::ZERO {
            return Err(AppError::Validation("principal must be positive".into()));
        }
        let mult = match principal.scale() {
            0 => 100i128,
            1 => 10i128,
            2 => 1i128,
            _ => {
                return Err(AppError::Validation(
                    "principal cannot have more than 2 decimal places".into(),
                ));
            }
        };
        let satang = principal
            .mantissa()
            .checked_mul(mult)
            .ok_or_else(|| AppError::Validation("principal satang overflow".into()))?;
        if satang <= 0 {
            return Err(AppError::Validation("principal must be positive".into()));
        }
        i64::try_from(satang).map_err(|_| AppError::Validation("principal satang overflow".into()))
    }
}
