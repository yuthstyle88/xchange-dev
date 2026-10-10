//! `GET /api/public/orgs/{org}/branches` (#995): the organisation's branches, with an `ETag`
//! and `304` when unchanged.

use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::bootstrap::AppState;
use crate::presentation::dto::public_branches::{PublicBranchDto, PublicBranchesData};
use crate::presentation::http::public::not_found_response;

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    cfg.app_data(web::Data::new(state.db.clone()))
        .route("/api/public/orgs/{org}/branches", web::get().to(handle));
}

async fn handle(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<String>,
) -> HttpResponse {
    let org_code = path.into_inner();

    let etag_row = sqlx::query(
        "SELECT \
            o.id, \
            count(b.id)::bigint AS count, \
            coalesce(sum(v.version), 0)::bigint AS sum_version, \
            max(b.created_at) AS max_created_at \
         FROM organizations o \
         LEFT JOIN branches b ON b.organization_id = o.id \
         LEFT JOIN branch_versions v ON v.branch_id = b.id \
         WHERE o.code = $1 \
         GROUP BY o.id",
    )
    .bind(&org_code)
    .fetch_optional(pool.get_ref())
    .await;

    let (org_id, count, sum_version, max_created_at) = match etag_row {
        Ok(Some(r)) => {
            let id: Uuid = r.get("id");
            let c: i64 = r.get("count");
            let s: i64 = r.get("sum_version");
            let m: Option<DateTime<Utc>> = r.get("max_created_at");
            (id, c, s, m)
        }
        Ok(None) => return not_found_response(),
        Err(e) => {
            tracing::error!("failed to compute etag for branches: {e}");
            return HttpResponse::InternalServerError().finish();
        }
    };

    let max_ts = max_created_at.map_or(0, |t| {
        t.timestamp_nanos_opt()
            .unwrap_or_else(|| t.timestamp_micros())
    });
    let etag = format!("\"c{count}-v{sum_version}-t{max_ts}\"");

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

    let rows = match sqlx::query(
        "SELECT \
            b.code, \
            b.name, \
            b.address, \
            b.opening_hours, \
            b.timezone, \
            b.booking_enabled, \
            COALESCE(s.queue_open, false) AS queue_open \
         FROM branches b \
         LEFT JOIN branch_service_status s ON s.branch_id = b.id AND s.organization_id = b.organization_id \
         WHERE b.organization_id = $1 \
         ORDER BY b.code ASC",
    )
    .bind(org_id)
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("failed to fetch branches: {e}");
            return HttpResponse::InternalServerError().finish();
        }
    };

    let branches: Vec<PublicBranchDto> = rows
        .into_iter()
        .map(|r| PublicBranchDto {
            code: r.get("code"),
            name: r.get("name"),
            address: r.get("address"),
            opening_hours: r.get("opening_hours"),
            timezone: r.get("timezone"),
            booking_enabled: r.get("booking_enabled"),
            queue_open: r.get("queue_open"),
        })
        .collect();

    HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-cache"))
        .insert_header((header::ETAG, etag))
        .json(PublicBranchesData { branches })
}
