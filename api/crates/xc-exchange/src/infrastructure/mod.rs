//! Infrastructure layer for exchange.
pub mod booking;
pub mod exchange;
pub mod quote;
pub mod reversal;
pub mod settings;
pub mod transaction;

use pos_error::{AppError, AppResult};
use xc_kernel::money::{Money, plain_decimal};

/// Reads a stored numeric THB money figure back from the database without
/// enforcing the 100M input cap.
pub(crate) fn parse_stored_money(text: &str) -> AppResult<Money> {
    let mut d = plain_decimal(text)
        .ok_or_else(|| AppError::Internal(format!("invalid stored money: {text}")))?;
    if d.scale() > 2 {
        return Err(AppError::Internal(format!(
            "invalid stored money scale: {text}"
        )));
    }
    d.rescale(2);
    Money::try_from_minor_i128(d.mantissa())
        .map_err(|e| AppError::Internal(format!("invalid stored money amount: {e}")))
}
