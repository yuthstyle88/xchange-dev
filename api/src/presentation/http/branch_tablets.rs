//! The branch tablet's routes (design `kiosk-shared-phone.md` §7.7):
//! - `POST /api/branch-tablets` — staff with `SystemSettings` at the named branch
//!   (`EntryPath::DeviceSetup`), same-origin, authorised exactly as `device_setup.rs:97-131`:
//!   `{action: "list" | "add" | "newCode" | "revoke", payload}`;
//! - `POST /api/branch-tablet/pair` — public, same-origin: `{code}` ⇒ `200 {tabletId, branchCode,
//!   name}` and the tablet cookie, the token never in the body;
//! - `GET /api/branch-tablet/me` — public, reads the cookie: `{paired: false}` or
//!   `{paired: true, tabletId, branchCode, name}`, `Cache-Control: no-store`.
//!
//! Filled by BT-4, which also adds `branch_tablets::configure(cfg, state);` to the one route table
//! (`http/mod.rs` `configure`). Not mounted until then: no route answers today, so
//! `api/tests/branch_tablet_http_test.rs` fails on the missing routes.

use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::{SecondsFormat, Utc};
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use super::client_address::client_address;
use super::rpc::RpcClock;
use crate::bootstrap::AppState;
use crate::infrastructure::branch_tablets::tablet_service::BranchTabletService;
use crate::infrastructure::branch_tablets::{ListedTablet, TabletMe, TabletSeen};
use crate::infrastructure::paired_devices::pairing::{PairOutcome, PairingCode, PairingError};
use crate::presentation::extractors::actor::{
    BAD_REQUEST, answer, authorize, failure, payload_of, read, refuse, server_error,
};
use crate::presentation::extractors::origin::same_origin;
use crate::presentation::extractors::tablet_cookie::{
    clear_tablet_cookie, set_tablet_cookie, tablet_token,
};
use crate::presentation::pages;

const ORIGIN_REJECTED: &str = "ORIGIN_REJECTED";
const PAIRING_CODE_INVALID: &str = "PAIRING_CODE_INVALID";
const RATE_LIMITED: &str = "RATE_LIMITED";
const TOO_LARGE: &str = "ข้อมูลมากเกินไป";
const MAX_BODY_UNITS: usize = 20_000;
const SPECIFY_TABLET_NAME: &str = "ระบุชื่อเครื่อง";
const NO_SUCH_TABLET: &str = "ไม่พบแท็บเล็ต";

/// Routes of this module; registered through the one route table.
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let service = BranchTabletService::new(branch_tx);
    cfg.service(
        web::resource("/api/branch-tablets")
            .app_data(web::PayloadConfig::new(1 << 20))
            .app_data(web::Data::new(service.clone()))
            .route(web::post().to(branch_tablets_handler))
            .default_service(web::to(pages::fallback)),
    )
    .service(
        web::resource("/api/branch-tablet/pair")
            .app_data(web::PayloadConfig::new(1 << 20))
            .app_data(web::Data::new(service.clone()))
            .route(web::post().to(pair_handler))
            .default_service(web::to(pages::fallback)),
    )
    .service(
        web::resource("/api/branch-tablet/me")
            .app_data(web::Data::new(service))
            .route(web::get().to(me_handler))
            .default_service(web::to(pages::fallback)),
    );
}

fn pairing_code_json(code: &PairingCode) -> Value {
    json!({
        "tabletId": code.device_id,
        "code": code.code,
        "codeExpiresAt": code.code_expires_at.to_rfc3339_opts(SecondsFormat::Millis, true),
    })
}

fn tablet_json(tablet: &ListedTablet) -> Value {
    json!({
        "tabletId": tablet.tablet_id,
        "branchId": tablet.branch_id,
        "name": tablet.name,
        "state": tablet.state.as_str(),
        "lastSeenAt": tablet.last_seen_at.map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true)),
        "createdAt": tablet.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        "pairedAt": tablet.paired_at.map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true)),
        "revokedAt": tablet.revoked_at.map(|t| t.to_rfc3339_opts(SecondsFormat::Millis, true)),
    })
}

fn pairing_failure(error: PairingError) -> HttpResponse {
    match error {
        PairingError::NameRequired => refuse(400, SPECIFY_TABLET_NAME),
        PairingError::BranchHasOne | PairingError::NoSuchDevice => refuse(400, NO_SUCH_TABLET),
        PairingError::App(e) => failure(e),
    }
}

/// `POST /api/branch-tablets`.
pub async fn branch_tablets_handler(req: HttpRequest, bytes: web::Bytes) -> HttpResponse {
    let Some(service) = req.app_data::<web::Data<BranchTabletService>>() else {
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

    let named = match action {
        "add" => Some(BranchId::new(
            payload
                .get("branchId")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
        )),
        "newCode" | "revoke" => {
            let tablet_id = payload
                .get("tabletId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match service.pairing().branch_of(tablet_id).await {
                Ok(branch) => branch,
                Err(e) => return failure(e),
            }
        }
        _ => None,
    };

    let (_grant, callers) = match authorize(
        &req,
        EntryPath::DeviceSetup,
        &Action::new(action),
        named.as_ref(),
    )
    .await
    {
        Ok(ok) => ok,
        Err(resp) => return resp,
    };

    if !same_origin(&req) {
        return refuse(400, ORIGIN_REJECTED);
    }

    let actor_id = callers
        .user
        .as_ref()
        .map(|u| u.user_id.as_str())
        .unwrap_or("");

    match action {
        "list" => match service.list(now).await {
            Ok(tablets) => {
                let act = Action::new(action);
                let visible: Vec<_> = tablets
                    .iter()
                    .filter(|t| {
                        let branch = BranchId::new(&t.branch_id);
                        Policy::authorize(
                            callers.user.as_ref(),
                            EntryPath::DeviceSetup,
                            &act,
                            Some(&branch),
                            Utc::now(),
                        )
                        .is_ok()
                    })
                    .map(tablet_json)
                    .collect();
                answer(200, json!({ "tablets": visible }), true)
            }
            Err(e) => failure(e),
        },
        "add" => {
            let branch_id = payload
                .get("branchId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let branch = BranchId::new(branch_id);
            match service.pairing().issue(&branch, name, actor_id, now).await {
                Ok(added) => answer(200, pairing_code_json(&added), true),
                Err(e) => pairing_failure(e),
            }
        }
        "newCode" => {
            let tablet_id = payload
                .get("tabletId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match service.pairing().renew(tablet_id, actor_id, now).await {
                Ok(renewed) => answer(200, pairing_code_json(&renewed), true),
                Err(e) => pairing_failure(e),
            }
        }
        "revoke" => {
            let tablet_id = payload
                .get("tabletId")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match service.pairing().revoke(tablet_id, actor_id, now).await {
                Ok(()) => answer(200, json!({ "saved": true }), true),
                Err(e) => pairing_failure(e),
            }
        }
        _ => refuse(400, BAD_REQUEST),
    }
}

/// `POST /api/branch-tablet/pair`.
pub async fn pair_handler(req: HttpRequest, bytes: web::Bytes) -> HttpResponse {
    if !same_origin(&req) {
        return refuse(400, ORIGIN_REJECTED);
    }

    let Some(service) = req.app_data::<web::Data<BranchTabletService>>() else {
        return server_error();
    };

    let body = match read(&bytes, MAX_BODY_UNITS, TOO_LARGE) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let Some(code) = body.get("code").and_then(|v| v.as_str()) else {
        return refuse(400, PAIRING_CODE_INVALID);
    };

    let now = req
        .app_data::<web::Data<dyn RpcClock>>()
        .map_or_else(Utc::now, |c| c.now());
    let address = client_address(&req);
    let user_agent = req
        .headers()
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let seen = TabletSeen { user_agent };

    match service.pairing().pair(code, &seen, address, now).await {
        Ok(PairOutcome::Success { device, token }) => {
            let mut builder = HttpResponse::Ok();
            builder.insert_header((header::CACHE_CONTROL, "no-store"));
            set_tablet_cookie(&mut builder, &token);
            builder.json(json!({
                "tabletId": device.id,
                "branchCode": device.branch_id,
                "name": device.name,
            }))
        }
        Ok(PairOutcome::RateLimited) => refuse(429, RATE_LIMITED),
        Ok(PairOutcome::InvalidCode) => refuse(400, PAIRING_CODE_INVALID),
        Err(e) => failure(e),
    }
}

/// `GET /api/branch-tablet/me`.
pub async fn me_handler(req: HttpRequest) -> HttpResponse {
    let Some(service) = req.app_data::<web::Data<BranchTabletService>>() else {
        return server_error();
    };

    let now = req
        .app_data::<web::Data<dyn RpcClock>>()
        .map_or_else(Utc::now, |c| c.now());
    let address = client_address(&req);
    let user_agent = req
        .headers()
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let seen = TabletSeen { user_agent };
    let token = tablet_token(&req);

    match service.me(token.as_deref(), &seen, address, now).await {
        Ok(TabletMe::Paired {
            tablet_id,
            branch_code,
            name,
        }) => answer(
            200,
            json!({
                "paired": true,
                "tabletId": tablet_id,
                "branchCode": branch_code,
                "name": name,
            }),
            true,
        ),
        Ok(TabletMe::Unpaired { clear_cookie }) => {
            let mut builder = HttpResponse::Ok();
            builder.insert_header((header::CACHE_CONTROL, "no-store"));
            if clear_cookie {
                clear_tablet_cookie(&mut builder);
            }
            builder.json(json!({ "paired": false }))
        }
        Err(e) => failure(e),
    }
}
