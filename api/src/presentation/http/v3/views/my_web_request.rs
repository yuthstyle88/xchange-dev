//! `GET /api/v3/views/my_web_request?branch=&id=&since=` (design `kiosk-shared-phone.md` §4.2 D8;
//! §4.3a W4, W4a, W5; §4.4 B11, B11a, B11b): the signed-in member's own web request, before and
//! after the staff scan. Registers its own `GET`, as `my_queue_ticket` does, because its version
//! is not the seam's integer.
//!
//! `[required]` wire:
//! - `branch` = the branch **id** (as `my_queue_ticket`); `id` = `kiosk_requests.id`.
//! - `200 {version, changed: true, data}` with `data` = `{requestId, status, expiresAt,
//!   branch: {code, name}, type, currency, amount, displayNumber: string|null,
//!   ticketsAhead: number|null}`. **No counter** (R-7). `amount` is the foreign amount as a string.
//! - `version` (W4a) = `"<queue_version>.<f>"`, `f` = the first 8 bytes (16 hex digits) of
//!   `SHA-256(status ‖ "|" ‖ coalesce(cancel_token_hash, "") ‖ "|" ‖ coalesce(cancelled_at as
//!   RFC 3339, ""))` of the request row. `since` equal to it ⇒ `{version, changed: false}`.
//! - `status` (W5): `QUOTED` (no ticket) · `WAITING` · `CALLED` · `SERVING` (ticket `SERVING`,
//!   or request `ACCEPTED`/`ID_CHECKED`) · `COMPLETED` · `CANCELLED` · `NO_SHOW` (the ticket's).
//! - `expiresAt` = created + `KioskMachine::WEB_REQUEST_LIFE_MS` for `QUOTED`, `null` otherwise.
//! - `displayNumber`/`ticketsAhead` only once a ticket exists; `ticketsAhead` only while it waits.
//! - Signed out ⇒ `401 signIn`. Not the member's request, or no such row (purged, B11b) ⇒
//!   `404 notFound` **whatever `since` says**. Unknown branch ⇒ `require_branch`'s answer.
//!   `Cache-Control: no-store`.

use std::fmt::Write;
use std::path::Path;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use ts_rs::TS;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kiosk::domain::kiosk::entities::kiosk_machine::KioskMachine;
use xc_kiosk::domain::kiosk::value_objects::web_request_status::derive_status;
use xc_queue::application::status::status_service::TicketRef;

use crate::bootstrap::AppState;
use crate::presentation::dto::v3::my_web_request::{MyWebRequestBranch, MyWebRequestData};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::view::{require_branch, ticket_wait};
use crate::presentation::pages;

pub const PUBLIC_ROUTE: &str = "/api/v3/views/my_web_request";

#[derive(sqlx::FromRow)]
struct WebRequestRow {
    id: String,
    status: String,
    side: String,
    currency: String,
    foreign_amount: String,
    created_at: DateTime<Utc>,
    cancel_token_hash: Option<String>,
    cancelled_at: Option<DateTime<Utc>>,
    branch_code: String,
    branch_name: String,
    queue_version: i64,
    ticket_id: Option<Uuid>,
    display_number: Option<String>,
    service_type: Option<String>,
    sequence: Option<i32>,
    ticket_status: Option<String>,
}

fn refuse(code: &'static str, field: Option<&'static str>, status: StatusCode) -> HttpResponse {
    let message = match code {
        "signIn" => "กรุณาเข้าสู่ระบบ",
        "serverError" => "เซิร์ฟเวอร์ขัดข้อง",
        "notFound" => "ไม่พบรายการ",
        _ => "คำขอไม่ถูกต้อง",
    };
    V3Error::new(code, field, message, status).error_response()
}

fn version_of(row: &WebRequestRow) -> String {
    let mut hasher = Sha256::new();
    hasher.update(row.status.as_bytes());
    hasher.update(b"|");
    if let Some(h) = row.cancel_token_hash.as_deref() {
        hasher.update(h.as_bytes());
    }
    hasher.update(b"|");
    if let Some(c) = row.cancelled_at {
        hasher.update(c.to_rfc3339().as_bytes());
    }
    let digest = hasher.finalize();
    let mut f = String::with_capacity(16);
    for b in &digest[..8] {
        let _ = write!(&mut f, "{b:02x}");
    }
    format!("{}.{f}", row.queue_version)
}

async fn handle(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let query =
        web::Query::<std::collections::HashMap<String, String>>::from_query(req.query_string())
            .map(|q| q.into_inner())
            .unwrap_or_default();
    let branch = query.get("branch").cloned().unwrap_or_default();
    if let Err(e) = require_branch(&req, &BranchId::new(branch.as_str())).await {
        return e.error_response();
    }

    let callers = pages::get_callers(&req).await;
    let Some(customer) = callers.user.and_then(|u| u.customer_id) else {
        return refuse("signIn", None, StatusCode::UNAUTHORIZED);
    };
    let id = query.get("id").cloned().unwrap_or_default();
    let since = query.get("since").map(String::as_str);

    let pool = state.db.clone();
    let row: Option<WebRequestRow> = match sqlx::query_as(
        "SELECT r.id, r.status, r.side, btrim(r.currency::text) AS currency, \
                r.foreign_amount::text AS foreign_amount, r.created_at, \
                r.cancel_token_hash, r.cancelled_at, \
                COALESCE(b.code, b.id) AS branch_code, b.name AS branch_name, \
                COALESCE(s.queue_version, 0) AS queue_version, \
                t.id AS ticket_id, t.display_number, t.service_type, t.sequence, \
                t.status AS ticket_status \
           FROM kiosk_requests r \
           JOIN branches b ON b.id = r.branch_id \
           LEFT JOIN branch_service_status s ON s.branch_id = r.branch_id \
           LEFT JOIN queue_tickets t ON t.branch_id = r.branch_id AND t.kiosk_request_id = r.id \
          WHERE r.branch_id = $1 AND r.id = $2 AND r.customer_id = $3",
    )
    .bind(&branch)
    .bind(&id)
    .bind(&customer)
    .fetch_optional(&pool)
    .await
    {
        Ok(r) => r,
        Err(_) => return refuse("serverError", None, StatusCode::INTERNAL_SERVER_ERROR),
    };
    let Some(row) = row else {
        return refuse("notFound", None, StatusCode::NOT_FOUND);
    };

    let version = version_of(&row);
    if since == Some(version.as_str()) {
        return answer(json!({ "version": version, "changed": false }));
    }

    let status = derive_status(&row.status, row.ticket_status.as_deref());
    let expires_at = (status == "QUOTED").then(|| {
        (row.created_at + Duration::milliseconds(KioskMachine::WEB_REQUEST_LIFE_MS)).to_rfc3339()
    });

    let branch_id = BranchId::new(&branch);
    let tickets_ahead = match (
        status == "WAITING",
        row.ticket_id,
        row.service_type.as_deref(),
        row.sequence,
        row.ticket_status.as_deref(),
    ) {
        (true, Some(ticket_id), Some(service_type), Some(sequence), Some(ticket_status)) => {
            let ticket = TicketRef {
                id: ticket_id,
                branch: &branch_id,
                service_type,
                sequence,
                status: ticket_status,
            };
            match ticket_wait(&pool, ticket, Utc::now()).await {
                Ok((ahead, _)) => Some(ahead),
                Err(_) => return refuse("serverError", None, StatusCode::INTERNAL_SERVER_ERROR),
            }
        }
        _ => None,
    };

    let data = MyWebRequestData {
        request_id: row.id,
        status: status.to_string(),
        expires_at,
        branch: MyWebRequestBranch {
            code: row.branch_code,
            name: row.branch_name,
        },
        kind: row.side,
        currency: row.currency,
        amount: row.foreign_amount,
        display_number: row.display_number,
        tickets_ahead,
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
    MyWebRequestData::export_all(&cfg)?;
    Ok(())
}
