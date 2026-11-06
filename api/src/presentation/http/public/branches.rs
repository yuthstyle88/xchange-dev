//! `GET /api/public/orgs/{org}/branches` (#995): the organisation's branches, with an `ETag`
//! and `304` when unchanged.

use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;
use xc_rate::domain::branch::branch_hours::BranchHours;

use crate::bootstrap::AppState;
use crate::presentation::dto::public_branches::{PublicBranchDto, PublicBranchesData};
use crate::presentation::http::public::etag::if_none_match_hits;
use crate::presentation::http::public::not_found_response;
use crate::presentation::http::rpc::RpcClock;

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
    let now = req
        .app_data::<web::Data<dyn RpcClock>>()
        .map_or_else(Utc::now, |c| c.now());

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

    let rows = match sqlx::query(
        "SELECT \
            b.id, \
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
        .map(|r| {
            let opening_hours: Option<serde_json::Value> = r.get("opening_hours");
            let timezone: String = r.get("timezone");
            let stored_queue_open: bool = r.get("queue_open");
            let queue_open = stored_queue_open
                && BranchHours::new(&timezone, opening_hours.as_ref())
                    .is_ok_and(|h| h.is_open_at(now));
            PublicBranchDto {
                id: r.get("id"),
                code: r.get("code"),
                name: r.get("name"),
                address: r.get("address"),
                opening_hours,
                timezone,
                booking_enabled: r.get("booking_enabled"),
                queue_open,
            }
        })
        .collect();

    let max_ts = max_created_at.map_or(0, |t| {
        t.timestamp_nanos_opt()
            .unwrap_or_else(|| t.timestamp_micros())
    });
    let open_bits: String = branches
        .iter()
        .map(|b| if b.queue_open { '1' } else { '0' })
        .collect();
    let etag = format!("\"c{count}-v{sum_version}-t{max_ts}-q{open_bits}\"");

    if if_none_match_hits(req.headers(), &etag) {
        return HttpResponse::NotModified()
            .insert_header((header::ETAG, etag))
            .finish();
    }

    HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-cache"))
        .insert_header((header::ETAG, etag))
        .json(PublicBranchesData { branches })
}
