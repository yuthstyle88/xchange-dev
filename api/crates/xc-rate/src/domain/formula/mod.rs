//! The owner's pricing formula (pricing-formula.md §2, §6): a small arithmetic language over named inputs.
//!
//! SKELETON (pricing-formula): types and signatures are fixed by `docs/design/blueprint/pricing-formula.md` §6;
//! every `todo!()` body is one function ticket.

pub mod error;
pub mod expr;
pub mod input;
pub mod price_formula;
pub mod text;
