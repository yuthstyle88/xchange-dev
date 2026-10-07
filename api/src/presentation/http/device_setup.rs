//! `POST /api/device-setup` and `GET /_xc/device-agents`, filled by row AG-4 (design §5.4c).

use actix_web::web;

use crate::bootstrap::AppState;

/// Routes of this module; filled in by row AG-4. Until then they answer 404.
pub fn configure(_cfg: &mut web::ServiceConfig, _state: &AppState) {}
