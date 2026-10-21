//! APP-5 (#995): public reads. No session, no actor, nothing per user, like `/healthz`
//! (lane controller's ruling 4 on #995): no policy row, and an `Authorization` header or a session
//! cookie is ignored.

pub mod branch_rates;
pub mod branches;
pub mod consent_texts;
pub mod copy_link;
pub mod host;
pub mod queue_display;

use actix_web::http::StatusCode;
use actix_web::{HttpResponse, ResponseError, web};

use crate::bootstrap::AppState;
use crate::presentation::http::v3::error::V3Error;

/// The public routes; called once from the route table (`http/mod.rs`).
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    branches::configure(cfg, state);
    branch_rates::configure(cfg, state);
    consent_texts::configure(cfg, state);
    copy_link::configure(cfg, state);
    host::configure(cfg, state);
    queue_display::configure(cfg, state);
}

/// Byte-identical 404 response for both routes when organisation or branch is unknown/mismatched (ruling 2).
pub fn not_found_response() -> HttpResponse {
    V3Error::new("notFound", None, "not found", StatusCode::NOT_FOUND).error_response()
}
