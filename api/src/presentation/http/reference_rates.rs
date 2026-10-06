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
use crate::presentation::pages::{self, SessionReader};

const ORIGIN_REJECTED: &str = "ORIGIN_REJECTED";
const SERVER_ERROR: &str = "เซิร์ฟเวอร์ขัดข้อง";

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

fn check_origin(req: &HttpRequest) -> bool {
    let marker = req
        .headers()
        .get("X-Xchange-Request")
        .and_then(|v| v.to_str().ok());
    if marker != Some("1") {
        return false;
    }

    let Some(origin) = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };

    let Some(host_hdr) = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };

    let scheme = match req
        .headers()
        .get("X-Forwarded-Proto")
        .and_then(|v| v.to_str().ok())
    {
        Some("https") => "https",
        _ => "http",
    };

    let host_lower = host_hdr.to_ascii_lowercase();
    let host = match host_lower.rfind(':') {
        Some(colon_idx) => {
            let port = &host_lower[colon_idx + 1..];
            if (scheme == "http" && port == "80") || (scheme == "https" && port == "443") {
                &host_lower[..colon_idx]
            } else {
                &host_lower
            }
        }
        None => &host_lower,
    };

    let expected_origin = format!("{scheme}://{host}");
    origin == expected_origin
}

fn policy_refusal_response(err: &PolicyError) -> HttpResponse {
    let status = actix_web::http::StatusCode::from_u16(err.status())
        .unwrap_or(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR);
    let msg = err.to_string();
    HttpResponse::build(status)
        .content_type("application/json")
        .body(format!(r#"{{"error":"{msg}"}}"#))
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
    if !check_origin(&req) {
        return HttpResponse::BadRequest()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .content_type("application/json")
            .body(format!(r#"{{"error":"{ORIGIN_REJECTED}"}}"#));
    }

    // 4: Ask the rate source
    let Some(source_data) = req.app_data::<web::Data<Arc<dyn RateSource>>>() else {
        return HttpResponse::InternalServerError()
            .content_type("application/json")
            .body(format!(r#"{{"error":"{SERVER_ERROR}"}}"#));
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
        Err(AppError::Domain(sentence)) => HttpResponse::BadRequest()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .content_type("application/json")
            .body(format!(r#"{{"error":"{sentence}"}}"#)),
        Err(_) => HttpResponse::InternalServerError()
            .content_type("application/json")
            .body(format!(r#"{{"error":"{SERVER_ERROR}"}}"#)),
    }
}
