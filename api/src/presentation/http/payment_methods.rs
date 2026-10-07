//! `POST /api/payment-methods`, filled by row PM-API (design §5.5).

use actix_web::web;

use crate::bootstrap::AppState;

/// Routes of this module; filled in by its own row. Until then the path falls through to
/// `pages::fallback`.
pub fn configure(_cfg: &mut web::ServiceConfig, _state: &AppState) {}
