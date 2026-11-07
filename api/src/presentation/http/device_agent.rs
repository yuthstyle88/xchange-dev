//! `/api/device-agent/*` (pair, claim, report), filled by row AG-2 (design §5.4c); `info` and `desk-binding`
//! for XC Counter (`docs/design/xc-counter-app.md` §8.2).

use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::{DateTime, Utc};
use pos_infra::transactions::Transactor;
use xc_auth::infrastructure::attempts::PgAttemptCounter;
use xc_kernel::branch_tx::BranchTx;

use super::client_address::client_address;
use super::rpc::RpcClock;
use crate::bootstrap::AppState;
use crate::infrastructure::device_agents::agent_repository::DrawerTarget;
use crate::infrastructure::device_agents::agent_service::{
    AgentService, AuthOutcome, PairOutcome, PairParams,
};
use crate::infrastructure::device_agents::claim::{ClaimOutcome, ClaimParams};
use crate::infrastructure::device_agents::counter_policy::Capabilities;
use crate::infrastructure::device_agents::desk_binding::DeskNonce;
use crate::infrastructure::device_agents::pg_agent_repository::PgAgentRepository;
use crate::infrastructure::device_agents::report::{ReportOutcome, ReportParams};
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::devices::printer_models::PrinterModels;
use crate::presentation::dto::device_agent::{
    CapabilitiesDto, ClaimRequest, ClaimResponse, DeskBindingResponse, InfoResponse, PairRequest,
    PairResponse, ReportRequest, ReportResponse,
};
use crate::presentation::extractors::agent_auth::{AgentAuth, check_browser, extract_bearer_token};
use crate::presentation::pages;

/// Routes of this module; registered through the one route table.
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let printer_models = PrinterModels::new(state.settings.devices.printer_models.clone())
        .unwrap_or_else(|_| PrinterModels::built_in());
    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let agent_repo = Arc::new(PgAgentRepository::new(state.db.clone()));
    let device_repo = Arc::new(PgDeviceRepository::new(state.db.clone()));
    let attempts = PgAttemptCounter::new(state.db.clone());

    let service = AgentService::new(agent_repo, device_repo, attempts, branch_tx, printer_models);

    cfg.service(
        web::resource("/api/device-agent/pair")
            .app_data(web::PayloadConfig::new(1 << 20))
            .app_data(web::Data::new(service.clone()))
            .route(web::post().to(pair_handler))
            .default_service(web::to(pages::fallback)),
    );
    cfg.service(
        web::resource("/api/device-agent/claim")
            .app_data(web::PayloadConfig::new(1 << 20))
            .app_data(web::Data::new(service.clone()))
            .route(web::post().to(claim_handler))
            .default_service(web::to(pages::fallback)),
    );
    cfg.service(
        web::resource("/api/device-agent/report")
            .app_data(web::PayloadConfig::new(1 << 20))
            .app_data(web::Data::new(service.clone()))
            .route(web::post().to(report_handler))
            .default_service(web::to(pages::fallback)),
    );
    // XC Counter (docs/design/xc-counter-app.md §8.2): who this counter is, and the desk nonce.
    cfg.service(
        web::resource("/api/device-agent/info")
            .app_data(web::Data::new(service.clone()))
            .route(web::get().to(info_handler))
            .default_service(web::to(pages::fallback)),
    );
    cfg.service(
        web::resource("/api/device-agent/desk-binding")
            .app_data(web::PayloadConfig::new(1 << 10))
            .app_data(web::Data::new(service))
            .route(web::post().to(desk_binding_handler))
            .default_service(web::to(pages::fallback)),
    );
}

/// `GET /api/device-agent/info` (§8.2): `InfoResponse` for the token's counter; 401 as `claim` for a revoked or
/// unknown token. CA-1a.
async fn info_handler(req: HttpRequest, service: web::Data<AgentService>) -> HttpResponse {
    if let Some(refusal) = check_browser(&req) {
        return refusal;
    }

    let clock = req.app_data::<web::Data<dyn RpcClock>>().cloned();
    let now = clock.as_ref().map_or_else(Utc::now, |c| c.now());
    let address = client_address(&req);

    let token_str = extract_bearer_token(&req);
    let agent = match service.authenticate(token_str, address, now).await {
        Ok(AuthOutcome::Ok(agent)) => *agent,
        Ok(AuthOutcome::RateLimited) => {
            return HttpResponse::build(StatusCode::TOO_MANY_REQUESTS)
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({ "error": "RATE_LIMITED" }));
        }
        Ok(AuthOutcome::InvalidToken) => {
            return HttpResponse::Unauthorized()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({ "error": "AGENT_TOKEN_INVALID" }));
        }
        Err(e) => {
            tracing::error!(error = %e, "info auth failed");
            return server_error();
        }
    };

    let caps = Capabilities::of(&agent);
    HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(InfoResponse {
            agent_id: agent.id,
            counter_name: agent.counter_name,
            branch_id: agent.branch_id,
            capabilities: CapabilitiesDto {
                cash_drawer: caps.cash_drawer,
                scanner: caps.scanner,
                kiosk_lock: caps.kiosk_lock,
            },
        })
}

/// `POST /api/device-agent/desk-binding` (§8.2, §8.3): `DeskBindingResponse` with a nonce sealed by
/// `desk_binding::DeskNonce`; 401 as `claim`. CA-1a.
async fn desk_binding_handler(
    req: HttpRequest,
    _bytes: web::Bytes,
    service: web::Data<AgentService>,
) -> HttpResponse {
    if let Some(refusal) = check_browser(&req) {
        return refusal;
    }

    let clock = req.app_data::<web::Data<dyn RpcClock>>().cloned();
    let now = clock.as_ref().map_or_else(Utc::now, |c| c.now());
    let address = client_address(&req);

    let token_str = extract_bearer_token(&req);
    let agent = match service.authenticate(token_str, address, now).await {
        Ok(AuthOutcome::Ok(agent)) => *agent,
        Ok(AuthOutcome::RateLimited) => {
            return HttpResponse::build(StatusCode::TOO_MANY_REQUESTS)
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({ "error": "RATE_LIMITED" }));
        }
        Ok(AuthOutcome::InvalidToken) => {
            return HttpResponse::Unauthorized()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({ "error": "AGENT_TOKEN_INVALID" }));
        }
        Err(e) => {
            tracing::error!(error = %e, "desk-binding auth failed");
            return server_error();
        }
    };

    let Ok(keys) = xc_customer::get_pii_keys() else {
        return server_error();
    };
    let issued = DeskNonce::issue(&agent.id, &agent.branch_id, now);
    let Some(exp) = DateTime::from_timestamp(issued.exp, 0) else {
        return server_error();
    };

    HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(DeskBindingResponse {
            nonce: issued.seal(&keys),
            expires_at: exp.to_rfc3339(),
        })
}

fn bad_request() -> HttpResponse {
    HttpResponse::BadRequest()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(serde_json::json!({ "error": "BAD_REQUEST" }))
}

fn server_error() -> HttpResponse {
    HttpResponse::InternalServerError()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .json(serde_json::json!({ "error": "เซิร์ฟเวอร์ขัดข้อง" }))
}

async fn pair_handler(
    req: HttpRequest,
    bytes: web::Bytes,
    service: web::Data<AgentService>,
) -> HttpResponse {
    if let Some(refusal) = check_browser(&req) {
        return refusal;
    }

    let Ok(body) = serde_json::from_slice::<PairRequest>(&bytes) else {
        return bad_request();
    };

    let clock = req.app_data::<web::Data<dyn RpcClock>>().cloned();
    let now = clock.as_ref().map_or_else(Utc::now, |c| c.now());
    let address = client_address(&req);

    let params = PairParams {
        code: &body.code,
        host: &body.host,
        os: &body.os,
        version: &body.version,
        build: &body.build,
    };

    match service.pair(&params, address, now).await {
        Ok(PairOutcome::Success {
            agent_id,
            token,
            branch_id,
            agent_name,
        }) => HttpResponse::Ok()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(PairResponse {
                agent_id,
                token,
                branch_id,
                agent_name,
            }),
        Ok(PairOutcome::RateLimited) => HttpResponse::build(StatusCode::TOO_MANY_REQUESTS)
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(serde_json::json!({ "error": "RATE_LIMITED" })),
        Ok(PairOutcome::InvalidCode) => HttpResponse::BadRequest()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(serde_json::json!({ "error": "PAIRING_CODE_INVALID" })),
        Err(e) => {
            tracing::error!(error = %e, "pair failed");
            server_error()
        }
    }
}

async fn claim_handler(
    req: HttpRequest,
    bytes: web::Bytes,
    service: web::Data<AgentService>,
) -> HttpResponse {
    if let Some(refusal) = check_browser(&req) {
        return refusal;
    }

    let Ok(body) = serde_json::from_slice::<ClaimRequest>(&bytes) else {
        return bad_request();
    };
    if let Some(ref drawer) = body.drawer
        && crate::presentation::http::v3::input::one_of(
            "drawer.transport",
            &drawer.transport,
            &["CUPS", "TCP"],
        )
        .is_err()
    {
        return bad_request();
    }

    let clock = req.app_data::<web::Data<dyn RpcClock>>().cloned();
    let now = clock.as_ref().map_or_else(Utc::now, |c| c.now());
    let address = client_address(&req);

    let token_str = extract_bearer_token(&req);
    let agent = match service.authenticate(token_str, address, now).await {
        Ok(AuthOutcome::Ok(agent)) => *agent,
        Ok(AuthOutcome::RateLimited) => {
            return HttpResponse::build(StatusCode::TOO_MANY_REQUESTS)
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({ "error": "RATE_LIMITED" }));
        }
        Ok(AuthOutcome::InvalidToken) => {
            return HttpResponse::Unauthorized()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({ "error": "AGENT_TOKEN_INVALID" }));
        }
        Err(e) => {
            tracing::error!(error = %e, "claim auth failed");
            return server_error();
        }
    };
    let auth = AgentAuth::from_agent(&agent);

    let params = ClaimParams {
        wait_ms: body.wait_ms,
        version: &body.version,
        build: &body.build,
        os: &body.os,
        host: &body.host,
        drawer: body.drawer.as_ref().map(|d| DrawerTarget {
            transport: &d.transport,
            target: &d.target,
        }),
        branch_id: body.branch_id.as_deref(),
    };

    let clock_for_poll = clock.clone();
    let now_fn = move || clock_for_poll.as_ref().map_or_else(Utc::now, |c| c.now());

    match service.claim(&auth, &params, now_fn).await {
        Ok(ClaimOutcome::Leased {
            job_id,
            lease_id,
            bytes,
            send_within_ms,
        }) => HttpResponse::Ok()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(ClaimResponse {
                job_id,
                lease_id,
                bytes,
                send_within_ms,
            }),
        Ok(ClaimOutcome::NoContent) => HttpResponse::NoContent()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .finish(),
        Ok(ClaimOutcome::RateLimited { retry_after }) => {
            HttpResponse::build(StatusCode::TOO_MANY_REQUESTS)
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .insert_header((header::RETRY_AFTER, retry_after.to_string()))
                .json(serde_json::json!({ "error": "RATE_LIMITED" }))
        }
        Ok(ClaimOutcome::BranchMismatch) => HttpResponse::Forbidden()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(serde_json::json!({ "error": "AGENT_BRANCH_MISMATCH" })),
        Ok(ClaimOutcome::Revoked) => HttpResponse::Unauthorized()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(serde_json::json!({ "error": "AGENT_TOKEN_INVALID" })),
        Err(e) => {
            tracing::error!(error = %e, "claim failed");
            server_error()
        }
    }
}

async fn report_handler(
    req: HttpRequest,
    bytes: web::Bytes,
    service: web::Data<AgentService>,
) -> HttpResponse {
    if let Some(refusal) = check_browser(&req) {
        return refusal;
    }

    let Ok(body) = serde_json::from_slice::<ReportRequest>(&bytes) else {
        return bad_request();
    };

    let clock = req.app_data::<web::Data<dyn RpcClock>>().cloned();
    let now = clock.as_ref().map_or_else(Utc::now, |c| c.now());
    let address = client_address(&req);

    let token_str = extract_bearer_token(&req);
    let agent = match service.authenticate(token_str, address, now).await {
        Ok(AuthOutcome::Ok(agent)) => *agent,
        Ok(AuthOutcome::RateLimited) => {
            return HttpResponse::build(StatusCode::TOO_MANY_REQUESTS)
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({ "error": "RATE_LIMITED" }));
        }
        Ok(AuthOutcome::InvalidToken) => {
            return HttpResponse::Unauthorized()
                .insert_header((header::CACHE_CONTROL, "no-store"))
                .json(serde_json::json!({ "error": "AGENT_TOKEN_INVALID" }));
        }
        Err(e) => {
            tracing::error!(error = %e, "report auth failed");
            return server_error();
        }
    };
    let auth = AgentAuth::from_agent(&agent);

    let params = ReportParams {
        job_id: &body.job_id,
        lease_id: &body.lease_id,
        outcome: &body.outcome,
        reason: &body.reason,
        detail: &body.detail,
    };

    match service.report(&auth, &params, now).await {
        Ok(ReportOutcome::Recorded) | Ok(ReportOutcome::AlreadyRecorded) => HttpResponse::Ok()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(ReportResponse { recorded: true }),
        Ok(ReportOutcome::InvalidOutcome) => bad_request(),
        Ok(ReportOutcome::NotFound) => HttpResponse::NotFound()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(serde_json::json!({ "error": "NOT_FOUND" })),
        Ok(ReportOutcome::Conflict) => HttpResponse::Conflict()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .json(serde_json::json!({ "error": "LEASE_MISMATCH" })),
        Err(e) => {
            tracing::error!(error = %e, "report failed");
            server_error()
        }
    }
}
