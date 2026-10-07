//! `POST /api/device-setup` and `GET /_xc/device-agents`, filled by row AG-4 (design §5.4c).

use actix_web::{HttpRequest, HttpResponse, web};
use chrono::Utc;
use pos_infra::transactions::Transactor;
use serde_json::json;
use xc_auth::domain::account::entities::session::SignedIn;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;

use crate::bootstrap::AppState;
use crate::infrastructure::device_agents::agent_admin::AgentAdmin;
use crate::presentation::dto::device_setup::{list_response_json, pairing_code_json};
use crate::presentation::extractors::actor::{
    BAD_REQUEST, OWNER_COOKIE, STAFF_COOKIE, accounts, actor_of, answer, cookie, failure,
    payload_of, read, refuse, server_error,
};
use crate::presentation::extractors::origin::same_origin;
use crate::presentation::http::rpc::RpcClock;
use crate::presentation::pages;

const ORIGIN_REJECTED: &str = "ORIGIN_REJECTED";
const TOO_LARGE: &str = "ข้อมูลมากเกินไป";
const MAX_BODY_UNITS: usize = 20_000;

const PAGE_HTML: &str = include_str!("../../../static/xc-device-agents.html");

/// Routes of this module; registered through the one route table.
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let admin = AgentAdmin::new(branch_tx);
    cfg.service(
        web::resource("/api/device-setup")
            .app_data(web::PayloadConfig::new(1 << 20))
            .app_data(web::Data::new(admin))
            .route(web::post().to(device_setup_handler))
            .default_service(web::to(pages::fallback)),
    )
    .service(
        web::resource("/_xc/device-agents")
            .route(web::get().to(device_agents_page_handler))
            .default_service(web::to(pages::fallback)),
    );
}

fn redirect_origin(req: &HttpRequest) -> String {
    let scheme = match req
        .headers()
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
    {
        Some("https") => "https",
        _ => "http",
    };
    let host = req
        .headers()
        .get(actix_web::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    format!("{scheme}://{host}")
}

async fn device_agents_page_handler(req: HttpRequest) -> HttpResponse {
    let Some(accounts) = accounts(&req) else {
        return server_error();
    };

    let now = req
        .app_data::<web::Data<dyn RpcClock>>()
        .map_or_else(Utc::now, |c| c.now());

    let is_owner = matches!(
        accounts.status(&cookie(&req, OWNER_COOKIE), "", now).await,
        Ok(Some(SignedIn::Owner(_)))
    );

    if !is_owner {
        let origin = redirect_origin(&req);
        return HttpResponse::SeeOther()
            .insert_header((actix_web::http::header::LOCATION, format!("{origin}/login")))
            .finish();
    }

    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"))
        .body(PAGE_HTML)
}

async fn device_setup_handler(req: HttpRequest, bytes: web::Bytes) -> HttpResponse {
    let Some(admin) = req.app_data::<web::Data<AgentAdmin>>() else {
        return server_error();
    };
    let Some(accounts) = accounts(&req) else {
        return server_error();
    };

    let now = req
        .app_data::<web::Data<dyn RpcClock>>()
        .map_or_else(Utc::now, |c| c.now());

    let body = match read(&bytes, MAX_BODY_UNITS, TOO_LARGE) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let action = body["action"].as_str().unwrap_or("");
    let payload = payload_of(&body);

    let who = match accounts
        .status(
            &cookie(&req, OWNER_COOKIE),
            &cookie(&req, STAFF_COOKIE),
            now,
        )
        .await
    {
        Ok(who) => who,
        Err(e) => return failure(e),
    };
    let actor = who.as_ref().and_then(actor_of);

    if let Err(refusal) =
        Policy::allow(actor.as_ref(), EntryPath::DeviceSetup, &Action::new(action))
    {
        return answer(
            refusal.status(),
            json!({ "error": refusal.to_string() }),
            true,
        );
    }
    let Some(actor) = actor else {
        return server_error();
    };

    if !same_origin(&req) {
        return refuse(400, ORIGIN_REJECTED);
    }

    match action {
        "list" => match admin.list(now).await {
            Ok(agents) => answer(200, list_response_json(&agents), true),
            Err(e) => failure(e),
        },
        "add" => {
            let branch_id = payload
                .get("branchId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("");
            match admin.add(branch_id, name, actor.id(), now).await {
                Ok(added) => answer(200, pairing_code_json(&added), true),
                Err(e) => failure(e),
            }
        }
        "newCode" => {
            let agent_id = payload
                .get("agentId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match admin.new_code(agent_id, actor.id(), now).await {
                Ok(renewed) => answer(200, pairing_code_json(&renewed), true),
                Err(e) => failure(e),
            }
        }
        "revoke" => {
            let agent_id = payload
                .get("agentId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match admin.revoke(agent_id, actor.id(), now).await {
                Ok(()) => answer(200, json!({ "saved": true }), true),
                Err(e) => failure(e),
            }
        }
        _ => refuse(400, BAD_REQUEST),
    }
}
