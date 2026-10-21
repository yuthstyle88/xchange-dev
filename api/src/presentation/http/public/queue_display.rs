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

use actix_web::{HttpRequest, HttpResponse, web};
use sqlx::PgPool;

use crate::bootstrap::AppState;

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    cfg.app_data(web::Data::new(state.db.clone())).route(
        "/api/public/orgs/{org}/branches/{code}/queue-display",
        web::get().to(handle),
    );
}

async fn handle(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let _ = (req, pool, path);
    todo!("KWQ-B1: the numbers-only calling display")
}
