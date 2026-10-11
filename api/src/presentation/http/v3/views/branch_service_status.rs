//! QU-7 (#1266): `GET /api/v3/views/branch_service_status?branch=` (design `queue-tickets.md` §4.4,
//! Q14, C08): the queue's state and the wait estimate, **public**.
//!
//! The view seam serves only signed-in staff (`view::authenticate_and_authorize`), so this view
//! registers its own `GET` and answers the seam's envelope, `{version, changed, data}`, with
//! `version` = `branch_service_status.queue_version` and `since` → `{changed: false}`. It carries
//! no customer field.

use std::path::Path;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::{DateTime, Utc};
use serde_json::json;
use ts_rs::TS;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::branch_service_status::{
    BranchServiceStatusData, BranchServiceWaiting,
};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::require_branch;

/// Public by design (C08): no session is asked, so this view is outside the staff policy table
/// (ruling (b) on #767; the lane controller's ruling 2 on PR 1336).
pub const PUBLIC_ROUTE: &str = "/api/v3/views/branch_service_status";

type StatusRow = (
    bool,
    i32,
    i64,
    Option<i32>,
    Option<i32>,
    Option<i32>,
    Option<DateTime<Utc>>,
);

fn server_error() -> HttpResponse {
    V3Error::new(
        "serverError",
        None,
        "เซิร์ฟเวอร์ขัดข้อง",
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .error_response()
}

async fn handle(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let query = web::Query::<std::collections::HashMap<String, String>>::from_query(
        req.query_string(),
    )
    .map(|q| q.into_inner())
    .unwrap_or_default();
    // An unknown or missing branch is the core's refusal, `noSuchBranch` with `no-store`, as every
    // view answers it (`view::require_branch`; the lane controller's ruling 1 on PR 1336 round 2).
    let branch = query.get("branch").cloned().unwrap_or_default();
    if let Err(e) = require_branch(&req, &BranchId::new(branch.as_str())).await {
        return e.error_response();
    }
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
    let pool = &state.db;

    let row: Option<StatusRow> = match sqlx::query_as(
        "SELECT queue_open, active_counters, queue_version, estimate_seconds, \
                estimate_low_seconds, estimate_high_seconds, estimate_as_of \
           FROM branch_service_status WHERE branch_id = $1",
    )
    .bind(&branch)
    .fetch_optional(pool)
    .await
    {
        Ok(r) => r,
        Err(_) => return server_error(),
    };
    let (queue_open, active_counters, version, est, low, high, as_of) =
        row.unwrap_or((false, 0, 0, None, None, None, None));

    if since == Some(version) {
        return answer(json!({ "version": version, "changed": false }));
    }

    let waiting: Vec<(String, i64)> = match sqlx::query_as(
        "SELECT service_type, count(*) FROM queue_tickets \
          WHERE branch_id = $1 AND status = 'WAITING' GROUP BY service_type",
    )
    .bind(&branch)
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(_) => return server_error(),
    };
    let count = |t: &str| {
        waiting
            .iter()
            .find(|(s, _)| s == t)
            .map_or(0, |(_, n)| *n as i32)
    };
    let data = BranchServiceStatusData {
        branch,
        queue_open,
        active_counters,
        waiting: BranchServiceWaiting {
            walk_in: count("WALK_IN"),
            booking: count("BOOKING"),
        },
        estimate_seconds: est,
        estimate_low_seconds: low,
        estimate_high_seconds: high,
        estimate_as_of: as_of.map(|t| t.to_rfc3339()),
    };
    answer(json!({ "version": version, "changed": true, "data": data }))
}

fn answer(body: serde_json::Value) -> HttpResponse {
    HttpResponse::Ok()
        .insert_header((header::CONTENT_TYPE, "application/json"))
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(body)
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    cfg.route(PUBLIC_ROUTE, web::get().to(handle));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    BranchServiceStatusData::export_all(&cfg)?;
    Ok(())
}
