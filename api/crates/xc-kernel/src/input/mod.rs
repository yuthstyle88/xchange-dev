//! The input rules every context reads a request with, and their sentences (ticket #88).
//!
//! STUB of the first commit: the signatures of the class diagram (`docs/design/xchange-v3.md`,
//! row IN-K) with wrong bodies, so `tests/input_test.rs` compiles and fails on wrong answers.

mod date;
mod input_error;
mod number;
mod text;

pub use date::{DATE_FLOOR, calendar_date};
pub use input_error::InputError;
pub use number::number_error;
pub use text::text;
