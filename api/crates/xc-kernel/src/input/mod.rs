//! The input rules every context reads a request with, and their sentences (ticket #88).

mod date;
mod input_error;
mod number;
mod text;

pub use date::{DATE_FLOOR, calendar_date};
pub use input_error::InputError;
pub use number::number_error;
pub use text::text;
