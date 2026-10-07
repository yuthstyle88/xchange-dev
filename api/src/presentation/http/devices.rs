//! Devices routes: POST /api/devices.

use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, web};
use chrono::Utc;
use pos_infra::transactions::Transactor;
use serde_json::json;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::bootstrap::AppState;
use crate::infrastructure::devices::device_commands::QueueJob;
use crate::infrastructure::devices::device_service::{BAD_PRINTER, DeviceService};
use crate::infrastructure::devices::job_ids::UuidJobIds;
use crate::infrastructure::devices::pg_saved_exchanges::PgSavedExchanges;
use crate::infrastructure::devices::printer_settings::{DEFAULT_PORT, PrinterKind};
use crate::presentation::dto::device::{job_json, overview_json};
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

/// Routes of this module; registered through the one route table.
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let transactions = TransactionService::new(
        Arc::new(PgTransactionRepository::new(state.db.clone())),
        branch_tx.clone(),
    );
    let devices = DeviceService::new(
        branch_tx,
        Arc::new(PgSavedExchanges::new(transactions)),
        Arc::new(UuidJobIds),
    );
    cfg.service(
        web::resource("/api/devices")
            .app_data(web::PayloadConfig::new(1 << 20))
            .app_data(web::Data::new(devices))
            .route(web::post().to(devices_handler))
            .default_service(web::to(pages::fallback)),
    );
}

async fn devices_handler(req: HttpRequest, bytes: web::Bytes) -> HttpResponse {
    let Some(devices) = req.app_data::<web::Data<DeviceService>>() else {
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

    if let Err(refusal) = Policy::allow(actor.as_ref(), EntryPath::Devices, &Action::new(action)) {
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

    let named = payload
        .get("branchId")
        .and_then(|v| v.as_str())
        .map(BranchId::new);
    let branch = Policy::branch(Some(&actor), named.as_ref());

    match action {
        "snapshot" => match devices.overview(&branch).await {
            Ok(overview) => match devices.drawer_settings(&branch).await {
                Ok(drawer) => answer(200, overview_json(&overview, &drawer), true),
                Err(e) => failure(e),
            },
            Err(e) => failure(e),
        },
        "queue" => {
            let kind = payload
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let reference = payload
                .get("ref")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let request_id = payload
                .get("requestId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let reason = payload
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            let cmd = QueueJob {
                kind,
                reference,
                request_id,
                reason,
            };
            match devices.queue(&branch, cmd, actor.id(), now).await {
                Ok(job) => answer(200, job_json(&job), true),
                Err(e) => failure(e),
            }
        }
        "process" => {
            let id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
            match devices.process(&branch, id, actor.id(), now).await {
                Ok(job) => answer(200, job_json(&job), true),
                Err(e) => failure(e),
            }
        }
        "retry" => {
            let id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
            match devices.retry(&branch, id, actor.id(), now).await {
                Ok(()) => answer(200, json!({ "saved": true }), true),
                Err(e) => failure(e),
            }
        }
        "printerOnline" => {
            let online = payload
                .get("online")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            match devices
                .set_printer_online(&branch, online, actor.id(), now)
                .await
            {
                Ok(()) => answer(200, json!({ "saved": true }), true),
                Err(e) => failure(e),
            }
        }
        "drawerObservation" => {
            let reference = payload.get("ref").and_then(|v| v.as_str()).unwrap_or("");
            let state = payload.get("state").and_then(|v| v.as_str()).unwrap_or("");
            match devices
                .drawer_observation(&branch, reference, state, actor.id(), now)
                .await
            {
                Ok(()) => answer(200, json!({ "saved": true }), true),
                Err(e) => failure(e),
            }
        }
        "setPrinter" => {
            let kind_str = match payload.get("kind").and_then(|v| v.as_str()) {
                Some(k) => k,
                None => return refuse(400, BAD_PRINTER),
            };
            let kind = match kind_str {
                "SIMULATED" => PrinterKind::Simulated,
                "NETWORK" => {
                    let host = payload
                        .get("host")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let port = match payload.get("port") {
                        None => DEFAULT_PORT,
                        Some(v) => match v.as_i64() {
                            Some(p) if (0..=65535).contains(&p) => p as u16,
                            _ => return refuse(400, BAD_PRINTER),
                        },
                    };
                    PrinterKind::Network(host, port)
                }
                _ => return refuse(400, BAD_PRINTER),
            };
            match devices.set_printer(&branch, kind, &actor, now).await {
                Ok(()) => answer(200, json!({ "saved": true }), true),
                Err(e) => failure(e),
            }
        }
        "setDrawer" => {
            let model = payload
                .get("printerModel")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let has_drawer = match payload.get("hasDrawer").and_then(|v| v.as_bool()) {
                Some(b) => b,
                None => return refuse(400, BAD_PRINTER),
            };
            match devices
                .set_drawer(&branch, model, has_drawer, &actor, now)
                .await
            {
                Ok(()) => answer(200, json!({ "saved": true }), true),
                Err(e) => failure(e),
            }
        }
        _ => refuse(400, BAD_REQUEST),
    }
}
