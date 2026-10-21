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

use std::path::Path;

use actix_web::{HttpRequest, HttpResponse, web};

use crate::bootstrap::AppState;

pub const PUBLIC_ROUTE: &str = "/api/v3/views/my_web_request";

async fn handle(req: HttpRequest, state: web::Data<AppState>) -> HttpResponse {
    let _ = (req, state);
    todo!("KWQ-C8: the member's own web request (W4, W4a, W5)")
}

pub fn register(cfg: &mut web::ServiceConfig, _state: &AppState) {
    cfg.route(PUBLIC_ROUTE, web::get().to(handle));
}

pub fn export_types(_dir: &Path) -> Result<(), ts_rs::ExportError> {
    Ok(())
}
