//! `GET /api/public/orgs/{org}/branches/{code}/rates` (#995, design P16): the branch's current
//! publication in the `published_rates` view's shape (`rates.md:454`).

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use sqlx::{PgPool, Row};
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::http::public::not_found_response;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::published_rates::published_rates_data;

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    cfg.app_data(web::Data::new(state.db.clone())).route(
        "/api/public/orgs/{org}/branches/{code}/rates",
        web::get().to(handle),
    );
}

async fn handle(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (org_code, branch_code) = path.into_inner();

    let row = sqlx::query(
        "SELECT b.id \
         FROM branches b \
         JOIN organizations o ON o.id = b.organization_id \
         WHERE o.code = $1 AND b.code = $2",
    )
    .bind(&org_code)
    .bind(&branch_code)
    .fetch_optional(pool.get_ref())
    .await;

    let branch_id_str: String = match row {
        Ok(Some(r)) => r.get(0),
        Ok(None) => return not_found_response(),
        Err(e) => {
            tracing::error!("failed to query branch for public rates: {e}");
            return HttpResponse::InternalServerError().finish();
        }
    };

    let branch_id = BranchId::new(branch_id_str);
    match published_rates_data(pool.get_ref(), &branch_id).await {
        Ok(data) => {
            let etag = match data.publication {
                Some(ref pub_dto) => format!("\"p{}-s{}\"", pub_dto.id, u8::from(data.stale)),
                None => "\"none\"".to_string(),
            };

            if let Some(if_none_match) = req
                .headers()
                .get(header::IF_NONE_MATCH)
                .and_then(|v| v.to_str().ok())
                && (if_none_match == etag || if_none_match.trim() == etag.trim())
            {
                return HttpResponse::NotModified()
                    .insert_header((header::ETAG, etag))
                    .finish();
            }

            HttpResponse::Ok()
                .insert_header((header::ETAG, etag))
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(data)
        }
        Err(_) => V3Error::new(
            "serverError",
            None,
            "เซิร์ฟเวอร์ขัดข้อง",
            StatusCode::INTERNAL_SERVER_ERROR,
        )
        .error_response(),
    }
}
