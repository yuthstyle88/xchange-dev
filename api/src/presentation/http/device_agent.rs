//! `/api/device-agent/*` (pair, claim, report), filled by row AG-2 (design §5.4c).

use actix_web::web;

use crate::bootstrap::AppState;

/// Routes of this module; filled in by row AG-2. Until then they answer 404.
pub fn configure(_cfg: &mut web::ServiceConfig, _state: &AppState) {}
