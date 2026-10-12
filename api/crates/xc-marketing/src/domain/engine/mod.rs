//! Benefit engine, stacking, rate floor, and snapshot (design §6, §7).

pub mod active_rules;
pub mod consent_gate;
#[allow(clippy::module_inception)]
pub mod engine;
pub mod rate_floor;
pub mod referral_source;
pub mod snapshot;
pub mod stacking;
pub mod tier_source;
