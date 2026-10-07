//! `POST /api/drawer-passport`, filled by row PD-0 (design §5.5.8).

use actix_web::web;

use crate::bootstrap::AppState;

/// Routes of this module; filled in by its own row. Until then the path falls through to
/// `pages::fallback`.
pub fn configure(_cfg: &mut web::ServiceConfig, _state: &AppState) {}
