//! `GET /api/public/host` (RD-0, #1255): answers which organisation the request's
//! host belongs to, from `organization_hosts`.

use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse, web};
use serde::Serialize;
use sqlx::PgPool;

use crate::bootstrap::AppState;
use crate::presentation::extractors::request_host::request_host;
use crate::presentation::http::public::not_found_response;

#[derive(Serialize)]
struct PublicHostResponse {
    org: String,
}

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    cfg.app_data(web::Data::new(state.db.clone()))
        .route("/api/public/host", web::get().to(handle));
}

async fn handle(req: HttpRequest, pool: web::Data<PgPool>) -> HttpResponse {
    let Some(host) = request_host(&req) else {
        return not_found_response();
    };

    let row = sqlx::query_scalar::<_, String>(
        "SELECT o.code \
         FROM organization_hosts h \
         JOIN organizations o ON o.id = h.organization_id \
         WHERE h.host = $1",
    )
    .bind(&host)
    .fetch_optional(pool.get_ref())
    .await;

    match row {
        Ok(Some(org)) => HttpResponse::Ok()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(PublicHostResponse { org }),
        Ok(None) => not_found_response(),
        Err(e) => {
            tracing::error!("failed to lookup organization host: {e}");
            HttpResponse::InternalServerError().finish()
        }
    }
}
