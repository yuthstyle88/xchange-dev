//! `xc-alert` — rate alerts (design `docs/design/blueprint/rate-alerts.md`, #561).
//!
//! Depends on `xc-rate` (lines, freshness, register) and `xc-kernel`. `xc-rate` never depends on this crate (§5).

pub mod application;
pub mod domain;
pub mod infrastructure;
