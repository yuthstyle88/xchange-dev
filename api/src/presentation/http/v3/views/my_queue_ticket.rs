//! QU-7 (#1266): `GET /api/v3/views/my_queue_ticket?branch=&ticketId=` (design `queue-tickets.md`
//! §4.4, #558): one ticket, its place in the queue and its wait, **only to the ticket's own
//! customer** (`queue_tickets.customer_id` = the session's `customer_id`).
//!
//! The view seam serves only signed-in staff, so this view registers its own `GET` and answers the
//! seam's envelope, `{version, changed, data}`, with `version` = the branch's
//! `branch_service_status.queue_version` and `since` → `{changed: false}`. Signed out → 401
//! `signIn`; anyone but the ticket's customer → 403 `outOfScope` (no oracle for other tickets).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::{DateTime, Utc};
use pos_infra::transactions::Transactor;
use serde_json::json;
use ts_rs::TS;
use uuid::Uuid;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_queue::application::status::status_service::{StatusService, TicketRef};
use xc_queue::domain::repository::QueueRepository;
use xc_queue::infrastructure::repository::PgQueueRepository;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::my_queue_ticket::MyQueueTicketData;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::require_branch;
use crate::presentation::pages;

/// Outside the staff policy table (ruling (b) on #767; the lane controller's ruling 3 on PR 1336), but
/// **not open**: its own handler requires the ticket's customer session. No session answers 401
/// `signIn`, another customer's 403 `outOfScope`. It moves onto the table once the `customerSelf`
/// row (#654) exists.
pub const PUBLIC_ROUTE: &str = "/api/v3/views/my_queue_ticket";

type TicketRow = (String, String, String, i32, Option<String>);

fn refuse(code: &'static str, field: Option<&'static str>, status: StatusCode) -> HttpResponse {
    let message = match code {
        "signIn" => "กรุณาเข้าสู่ระบบ",
        "serverError" => "เซิร์ฟเวอร์ขัดข้อง",
        "outOfScope" => "ไม่มีสิทธิ์ดูคิวนี้",
        _ => "คำขอไม่ถูกต้อง",
    };
    V3Error::new(code, field, message, status).error_response()
}

async fn handle(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let query =
        web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string())
            .map(|q| q.into_inner())
            .unwrap_or_default();
    // The branch first, for every caller: an unknown branch is the core's `noSuchBranch` with
    // `no-store` (`view::require_branch`; the lane controller's ruling 2 on PR 1336 round 2). It
    // tells nothing `branch_service_status` does not already answer publicly.
    let branch = query.get("branch").cloned().unwrap_or_default();
    if let Err(e) = require_branch(&req, &BranchId::new(branch.as_str())).await {
        return e.error_response();
    }

    let callers = pages::get_callers(&req).await;
    let Some(customer) = callers.user.and_then(|u| u.customer_id) else {
        return refuse("signIn", None, StatusCode::UNAUTHORIZED);
    };
    let Some(ticket_id) = query.get("ticketId").and_then(|t| Uuid::parse_str(t).ok()) else {
        return refuse(
            "validationFailed",
            Some("ticketId"),
            StatusCode::BAD_REQUEST,
        );
    };
    let since = match query.get("since").map(|s| s.parse::<i64>()) {
        None => None,
        Some(Ok(v)) => Some(v),
        Some(Err(_)) => return refuse("invalidRequest", Some("since"), StatusCode::BAD_REQUEST),
    };

    let pool = state.db.clone();
    let row: Option<TicketRow> = match sqlx::query_as(
        "SELECT display_number, service_type, status, sequence, customer_id \
           FROM queue_tickets WHERE branch_id = $1 AND id = $2",
    )
    .bind(&branch)
    .bind(ticket_id)
    .fetch_optional(&pool)
    .await
    {
        Ok(r) => r,
        Err(_) => return refuse("serverError", None, StatusCode::INTERNAL_SERVER_ERROR),
    };
    let Some((display_number, service_type, status, sequence, owner)) = row else {
        return refuse("outOfScope", None, StatusCode::FORBIDDEN);
    };
    if owner.as_deref() != Some(customer.as_str()) {
        return refuse("outOfScope", None, StatusCode::FORBIDDEN);
    }

    let version: i64 = match sqlx::query_scalar(
        "SELECT queue_version FROM branch_service_status WHERE branch_id = $1",
    )
    .bind(&branch)
    .fetch_optional(&pool)
    .await
    {
        Ok(v) => v.unwrap_or(0),
        Err(_) => return refuse("serverError", None, StatusCode::INTERNAL_SERVER_ERROR),
    };
    if since == Some(version) {
        return answer(json!({ "version": version, "changed": false }));
    }

    let now = Utc::now();
    let branch_id = BranchId::new(&branch);
    let ticket = TicketRef {
        id: ticket_id,
        branch: &branch_id,
        service_type: &service_type,
        sequence,
        status: &status,
    };
    let (tickets_ahead, estimate) = match ticket_wait(&pool, ticket, now).await {
        Ok(w) => w,
        Err(_) => return refuse("serverError", None, StatusCode::INTERNAL_SERVER_ERROR),
    };
    let as_of = (status == "WAITING").then_some(now);

    let data = MyQueueTicketData {
        ticket_id: ticket_id.to_string(),
        branch,
        display_number,
        service_type,
        status,
        tickets_ahead,
        estimate_seconds: estimate.map(|e| e.seconds),
        estimate_low_seconds: estimate.map(|e| e.low_seconds),
        estimate_high_seconds: estimate.map(|e| e.high_seconds),
        estimate_as_of: as_of.map(|t: DateTime<Utc>| t.to_rfc3339()),
    };
    answer(json!({ "version": version, "changed": true, "data": data }))
}

/// `StatusService::ticket_wait_in` on a read-only transaction of its own: the same ahead count and
/// estimate the branch's queue computes for `ticket`.
async fn ticket_wait(
    pool: &sqlx::PgPool,
    ticket: TicketRef<'_>,
    now: DateTime<Utc>,
) -> pos_error::AppResult<(
    i32,
    Option<xc_queue::domain::status::wait_estimator::WaitEstimate>,
)> {
    let repo: Arc<dyn QueueRepository> = Arc::new(PgQueueRepository::new(pool.clone()));
    let status = StatusService::new(BranchTx::new(Transactor::new(pool.clone())), repo);
    let mut tx = pool.begin().await?;
    let wait = status.ticket_wait_in(&mut tx, ticket, now).await?;
    tx.rollback().await?;
    Ok(wait)
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
    MyQueueTicketData::export_all(&cfg)?;
    Ok(())
}
