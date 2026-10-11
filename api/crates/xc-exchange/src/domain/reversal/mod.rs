//! RV-1 (#682): the reversal of a completed exchange (design `docs/design/blueprint/reversal.md`).

pub mod repository;
#[allow(clippy::module_inception)]
pub mod reversal;
pub mod window;
