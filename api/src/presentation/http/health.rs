//! Health check routes.

use std::time::Duration;

use actix_web::{HttpRequest, HttpResponse, web};

use crate::bootstrap::AppState;

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadyResponse<'a> {
    status: &'static str,
    database: bool,
    version: &'a str,
    build: &'a str,
    built_at: &'a str,
    channel: &'a str,
    ui: &'static str,
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/healthz",
        web::to(|| async {
            HttpResponse::Ok()
                .content_type("application/json")
                .body(r#"{"ok":true}"#)
        }),
    )
    .route("/health/ready", web::get().to(ready_handler));
}

async fn ready_handler(state: web::Data<AppState>, req: HttpRequest) -> HttpResponse {
    let database_ok = tokio::time::timeout(PROBE_TIMEOUT, state.database_probe.ping())
        .await
        .unwrap_or_default();

    let status = if database_ok { "ok" } else { "degraded" };
    let http_status = if database_ok {
        actix_web::http::StatusCode::OK
    } else {
        actix_web::http::StatusCode::SERVICE_UNAVAILABLE
    };

    let ui = match req.app_data::<web::Data<crate::presentation::screens::ScreenTable>>() {
        Some(table) if table.built() => "built",
        _ => "missing",
    };

    let body = ReadyResponse {
        status,
        database: database_ok,
        version: &state.build.version,
        build: &state.build.build,
        built_at: &state.build.built_at,
        channel: state.settings.channel.as_str(),
        ui,
    };

    HttpResponse::build(http_status).json(body)
}
