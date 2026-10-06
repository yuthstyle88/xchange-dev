//! Pages presentation.

use actix_web::{HttpRequest, HttpResponse, web};

use crate::bootstrap::AppState;

/// Page routes; filled in by ticket T1c.
pub fn configure(_cfg: &mut web::ServiceConfig, _state: &AppState) {}

/// What answers when no route matched. T1c serves static files here before the 404.
pub async fn fallback(_req: HttpRequest) -> HttpResponse {
    HttpResponse::NotFound()
        .content_type("text/plain")
        .body("Not found")
}
