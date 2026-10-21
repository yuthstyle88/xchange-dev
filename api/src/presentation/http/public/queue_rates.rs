//! `GET /api/public/orgs/{org}/branches/{code}/queue-rates` (`webRequestRates`, design
//! `kiosk-shared-phone.md` §4.3a W2): the rates the web request page offers — the kiosk core's
//! `KioskService::public_rates` (the branch's kiosk currency switches and payment rules, exactly
//! what `quote` enforces), not the public `branch_rates`.
//!
//! `[required]` wire: the same JSON `POST /api/kiosk {action:"rates"}` answers for that branch
//! (`dto_kiosk::rates_json` with the branch's publication freshness, as `kiosk.rs` builds it),
//! plus nothing. Unknown organisation or branch ⇒ `not_found_response()` (byte-identical to
//! `/rates`). No session. `Cache-Control: no-store`.

use std::sync::Arc;

use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse, web};
use pos_infra::transactions::Transactor;
use sqlx::{PgPool, Row};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kiosk::application::kiosk::service::KioskService;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;

use crate::bootstrap::AppState;
use crate::infrastructure::kiosk::kiosk_ports::{KioskAuditLog, PgKioskAuditLog, PgKioskPorts};
use crate::presentation::dto::kiosk as dto_kiosk;
use crate::presentation::http::kiosk::publication_freshness;
use crate::presentation::http::public::not_found_response;

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    cfg.app_data(web::Data::new(state.db.clone())).route(
        "/api/public/orgs/{org}/branches/{code}/queue-rates",
        web::get().to(handle),
    );
}

async fn handle(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (org_code, branch_code) = path.into_inner();
    let pool_ref = pool.get_ref();

    let row = sqlx::query(
        "SELECT b.id \
         FROM branches b \
         JOIN organizations o ON o.id = b.organization_id \
         WHERE o.code = $1 AND b.code = $2",
    )
    .bind(&org_code)
    .bind(&branch_code)
    .fetch_optional(pool_ref)
    .await;

    let branch_id_str: String = match row {
        Ok(Some(r)) => r.get(0),
        Ok(None) => return not_found_response(),
        Err(e) => {
            tracing::error!("failed to query branch for public queue-rates: {e}");
            return HttpResponse::InternalServerError().finish();
        }
    };

    let branch_id = BranchId::new(branch_id_str);
    let branch_tx = BranchTx::new(Transactor::new(pool_ref.clone()));
    let audit: Arc<dyn KioskAuditLog> = req
        .app_data::<web::Data<dyn KioskAuditLog>>()
        .map(|d| d.clone().into_inner())
        .unwrap_or_else(|| Arc::new(PgKioskAuditLog::with_pool(pool_ref.clone())));
    let source: Arc<dyn RateSource> = req
        .app_data::<web::Data<Arc<dyn RateSource>>>()
        .map_or_else(
            || Arc::new(BotRateSource::bot()) as Arc<dyn RateSource>,
            |s| s.get_ref().clone(),
        );
    let ports = Arc::new(PgKioskPorts::new(pool_ref.clone(), audit, source));
    let service = KioskService::new(branch_tx, ports);

    let rates = match service.public_rates(&branch_id).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("failed to read kiosk public_rates for queue-rates: {e}");
            return HttpResponse::InternalServerError().finish();
        }
    };

    let freshness = match publication_freshness(pool_ref, &branch_id).await {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("failed to read publication freshness for queue-rates: {e}");
            return HttpResponse::InternalServerError().finish();
        }
    };

    let json = match dto_kiosk::rates_json(&rates, &freshness) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!("failed to format queue-rates json: {e}");
            return HttpResponse::InternalServerError().finish();
        }
    };

    HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .content_type("application/json")
        .body(json.to_text())
}
