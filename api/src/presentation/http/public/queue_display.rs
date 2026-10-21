//! `GET /api/public/orgs/{org}/branches/{code}/queue-display?since=` (design
//! `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D10, §4.3a W9, §4.4 B17): the public,
//! numbers-only calling display. No session; nothing per user. Number only, by the owner's rule
//! (R-7, Q8): never a name, phone, amount, teller or counter.
//!
//! Answer (`[required]`, W9): the seam's envelope on `branch_service_status.queue_version`:
//! `{version, changed: true, data: {called, recent}}`, or `{version, changed: false}` when
//! `since` equals the current version (🔁). `called` = today's `CALLED`/`SERVING` tickets
//! `{displayNumber, status, calledAt}`; `recent` = today's last 5 called tickets
//! `{displayNumber, calledAt}`; both newest `calledAt` first. "Today" = the branch's
//! `service_date` in its own timezone. Unknown organisation or branch ⇒ `not_found_response()`;
//! a `since` that is not an integer ⇒ `400 invalidRequest` (field `since`).
//! `Cache-Control: no-store`.

use std::collections::HashMap;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::bootstrap::AppState;
use crate::presentation::http::public::not_found_response;
use crate::presentation::http::v3::error::V3Error;

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    cfg.app_data(web::Data::new(state.db.clone())).route(
        "/api/public/orgs/{org}/branches/{code}/queue-display",
        web::get().to(handle),
    );
}

fn server_error() -> HttpResponse {
    V3Error::new(
        "serverError",
        None,
        "เซิร์ฟเวอร์ขัดข้อง",
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .error_response()
}

fn answer(body: Value) -> HttpResponse {
    HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(body)
}

async fn handle(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (org_code, branch_code) = path.into_inner();

    let branch_id: String = match sqlx::query_scalar(
        "SELECT b.id \
           FROM branches b \
           JOIN organizations o ON o.id = b.organization_id \
          WHERE o.code = $1 AND b.code = $2",
    )
    .bind(&org_code)
    .bind(&branch_code)
    .fetch_optional(pool.get_ref())
    .await
    {
        Ok(Some(id)) => id,
        Ok(None) => return not_found_response(),
        Err(e) => {
            tracing::error!("failed to query branch for public queue display: {e}");
            return server_error();
        }
    };

    let query = web::Query::<HashMap<String, String>>::from_query(req.query_string())
        .map(|q| q.into_inner())
        .unwrap_or_default();
    let since = match query.get("since").map(|s| s.parse::<i64>()) {
        None => None,
        Some(Ok(v)) => Some(v),
        Some(Err(_)) => {
            return V3Error::new(
                "invalidRequest",
                Some("since"),
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };

    let version: i64 = match sqlx::query_scalar(
        "SELECT queue_version FROM branch_service_status WHERE branch_id = $1",
    )
    .bind(&branch_id)
    .fetch_optional(pool.get_ref())
    .await
    {
        Ok(v) => v.unwrap_or(0),
        Err(e) => {
            tracing::error!("failed to query queue_version for public queue display: {e}");
            return server_error();
        }
    };

    if since == Some(version) {
        return answer(json!({ "version": version, "changed": false }));
    }

    let rows: Vec<(String, String, DateTime<Utc>)> = match sqlx::query_as(
        "SELECT t.display_number, t.status, t.called_at \
           FROM queue_tickets t \
           JOIN branches b ON b.id = t.branch_id \
          WHERE t.branch_id = $1 \
            AND t.service_date = (now() AT TIME ZONE b.timezone)::date \
            AND t.called_at IS NOT NULL \
          ORDER BY t.called_at DESC, t.sequence DESC",
    )
    .bind(&branch_id)
    .fetch_all(pool.get_ref())
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("failed to query tickets for public queue display: {e}");
            return server_error();
        }
    };

    let called: Vec<Value> = rows
        .iter()
        .filter(|(_, status, _)| status == "CALLED" || status == "SERVING")
        .map(|(display_number, status, called_at)| {
            json!({
                "displayNumber": display_number,
                "status": status,
                "calledAt": called_at.to_rfc3339(),
            })
        })
        .collect();

    let recent: Vec<Value> = rows
        .iter()
        .take(5)
        .map(|(display_number, _, called_at)| {
            json!({
                "displayNumber": display_number,
                "calledAt": called_at.to_rfc3339(),
            })
        })
        .collect();

    answer(json!({
        "version": version,
        "changed": true,
        "data": {
            "called": called,
            "recent": recent,
        }
    }))
}
