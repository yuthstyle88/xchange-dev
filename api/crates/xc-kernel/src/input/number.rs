//! Mapping number parse errors to input refusals (ticket #88).

use super::InputError;
use crate::money::NumberError;

/// The refusal of a number that `Money` or `Rate` could not read.
pub fn number_error(e: NumberError) -> InputError {
    match e {
        NumberError::TooManyDecimals => InputError::TooManyDecimals,
        NumberError::Invalid => InputError::Number,
    }
}
