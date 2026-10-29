//! Which formula prices a line, and the save check that prices every affected currency first
//! (pricing-formula.md §3, §4, §6).
//!
//! SKELETON (pricing-formula): types and signatures are fixed by `docs/design/blueprint/pricing-formula.md` §6;
//! every `todo!()` body is one function ticket.

pub mod formula_record;
pub mod formula_resolver;
pub mod spread_check;
