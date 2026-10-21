//! `GET /api/public/orgs/{org}/branches/{code}/queue-rates` (`webRequestRates`, design
//! `kiosk-shared-phone.md` §4.3a W2): the rates the web request page offers — the kiosk core's
//! `KioskService::public_rates` (the branch's kiosk currency switches and payment rules, exactly
//! what `quote` enforces), not the public `branch_rates`.
//!
//! `[required]` wire: the same JSON `POST /api/kiosk {action:"rates"}` answers for that branch
//! (`dto_kiosk::rates_json` with the branch's publication freshness, as `kiosk.rs` builds it),
//! plus nothing. Unknown organisation or branch ⇒ `not_found_response()` (byte-identical to
//! `/rates`). No session. `Cache-Control: no-store`.

use actix_web::{HttpResponse, web};
use sqlx::PgPool;

use crate::bootstrap::AppState;

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    cfg.app_data(web::Data::new(state.db.clone())).route(
        "/api/public/orgs/{org}/branches/{code}/queue-rates",
        web::get().to(handle),
    );
}

async fn handle(pool: web::Data<PgPool>, path: web::Path<(String, String)>) -> HttpResponse {
    let _ = (pool, path);
    todo!("KWQ-C9: the kiosk core's rates for the web page")
}
