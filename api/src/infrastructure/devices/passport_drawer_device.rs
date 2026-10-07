//! Builds the passport drawer the server talks to (design §5.5.8). Row S0 lands the builder
//! answering `None`; row AG-PD reads the switch in `Settings` and builds the device on the pool.

use std::sync::Arc;

use sqlx::PgPool;

use crate::application::workflows::passport_drawer_device::PassportDrawerDevice;
use crate::config::Settings;

/// The branch-independent passport drawer, or `None` while there is none to build.
pub fn passport_drawer_device(
    _settings: &Settings,
    _db: &PgPool,
) -> Option<Arc<dyn PassportDrawerDevice>> {
    None
}
