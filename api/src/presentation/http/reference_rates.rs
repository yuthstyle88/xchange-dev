//! Reference rates routes.

use std::sync::Arc;

use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse, web};
use pos_error::AppError;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;

use crate::bootstrap::AppState;
use crate::presentation::extractors::actor::{answer, refuse, server_error};
use crate::presentation::extractors::origin::same_origin;
use crate::presentation::pages::{self, SessionReader};

const ORIGIN_REJECTED: &str = "ORIGIN_REJECTED";

/// Routes of this module; registered through the one route table.
pub fn configure(cfg: &mut web::ServiceConfig, _state: &AppState) {
    cfg.app_data(web::Data::new(
        Arc::new(BotRateSource::bot()) as Arc<dyn RateSource>
    ));
    cfg.service(
        web::resource("/api/reference-rates")
            .route(web::post().to(handler))
            .default_service(web::to(pages::fallback)),
    );
}

/// The 401 and 403 of this route: the sentence, and no `Cache-Control`.
fn policy_refusal_response(err: &PolicyError) -> HttpResponse {
    answer(
        err.status(),
        serde_json::json!({ "error": err.to_string() }),
        false,
    )
}

async fn handler(req: HttpRequest) -> HttpResponse {
    // 1 & 2: Session and policy check
    let actor = if let Some(reader) = req.app_data::<web::Data<dyn SessionReader>>() {
        let sessions = reader.sessions(&req).await;
        sessions.owner.or(sessions.staff)
    } else {
        None
    };

    let action = Action::new("/api/reference-rates");
    if let Err(refusal) = Policy::allow(actor.as_ref(), EntryPath::OwnerRoute, &action) {
        return policy_refusal_response(&refusal);
    }

    // 3: Same-origin check
    if !same_origin(&req) {
        return refuse(400, ORIGIN_REJECTED);
    }

    // 4: Ask the rate source
    let Some(source_data) = req.app_data::<web::Data<Arc<dyn RateSource>>>() else {
        return server_error();
    };

    match source_data.fetch().await {
        Ok(rates) => {
            let body = serde_json::json!({
                "source": rates.source,
                "fetchedAt": rates.fetched_at,
                "kind": rates.kind,
                "rows": rates.rows,
            });
            HttpResponse::Ok()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .content_type("application/json")
                .body(serde_json::to_string(&body).unwrap_or_default())
        }
        Err(AppError::Domain(sentence)) => refuse(400, &sentence),
        Err(other) => {
            tracing::error!(error = ?other, "the reference-rate source failed");
            server_error()
        }
    }
}
