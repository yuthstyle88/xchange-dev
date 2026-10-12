//! XS-0 #1375 (design `membership-points.md` §6.2 "Registration", `broker-seam.md` §11.1): one file per
//! destination, `bootstrap/exchange_effects/<destination>.rs`, each with
//! `pub fn effect(state: &AppState) -> Option<Arc<dyn ExchangeEffect>>` (`None` = not configured). `build.rs`
//! lists them in `EXCHANGE_EFFECTS`, in file-name order; `effects` turns the list into an `ExchangeEffects`.
//!
//! The same mechanism as `outbox_handlers` (PL-0a #910).

use std::sync::Arc;

use crate::application::workflows::exchange::effects::{ExchangeEffect, ExchangeEffects};
use crate::bootstrap::AppState;

/// What every destination file's `effect` is.
pub type EffectFn = fn(&AppState) -> Option<Arc<dyn ExchangeEffect>>;

include!(concat!(env!("OUT_DIR"), "/exchange_effects.rs"));

/// The effects found, in order: `None` (not configured) is skipped.
pub fn collect(
    found: impl IntoIterator<Item = Option<Arc<dyn ExchangeEffect>>>,
) -> ExchangeEffects {
    ExchangeEffects::new(found.into_iter().flatten().collect())
}

/// `collect` over `EXCHANGE_EFFECTS`, each asked with `state`.
pub fn effects(state: &AppState) -> ExchangeEffects {
    collect(EXCHANGE_EFFECTS.iter().map(|f| f(state)))
}
