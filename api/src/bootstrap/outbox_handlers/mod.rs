//! PL-0a (#910, design `broker-seam.md` §11.1 row "Outbox handlers"): one file per destination,
//! `bootstrap/outbox_handlers/<destination>.rs`, each with
//! `pub fn handler(state: &AppState) -> Option<Arc<dyn OutboxHandler>>` (`None` = not configured).
//! `build.rs` lists them in `OUTBOX_HANDLERS`; `Application::build` turns the list into the
//! worker's registry.

use std::sync::Arc;

use pos_error::AppResult;
use xc_outbox::application::registry::OutboxRegistry;
use xc_outbox::domain::handler::OutboxHandler;

use crate::bootstrap::AppState;

/// What every destination file's `handler` is.
pub type HandlerFn = fn(&AppState) -> Option<Arc<dyn OutboxHandler>>;

include!(concat!(env!("OUT_DIR"), "/outbox_handlers.rs"));

/// The registry of the handlers found: `None` is skipped; two of one `event_type` fail.
pub fn collect(
    found: impl IntoIterator<Item = Option<Arc<dyn OutboxHandler>>>,
) -> AppResult<OutboxRegistry> {
    let handlers: Vec<Arc<dyn OutboxHandler>> = found.into_iter().flatten().collect();
    OutboxRegistry::new(handlers)
}

/// `collect` over `OUTBOX_HANDLERS`, each asked with `state`.
pub fn registry(state: &AppState) -> AppResult<OutboxRegistry> {
    collect(OUTBOX_HANDLERS.iter().map(|f| f(state)))
}
