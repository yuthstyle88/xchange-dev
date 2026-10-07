//! `POST /api/payment-methods`, filled by row PM-API (design §5.5).

use actix_web::error::PayloadError;
use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::Utc;
use pos_infra::transactions::Transactor;
use serde_json::Value;
use xc_auth::domain::account::entities::session::SignedIn;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::infrastructure::branch::branch_repository::PgBranchRepository;

use crate::application::settings::payment_input::{parse_methods, parse_version};
use crate::application::settings::payment_settings::{PaymentSaveError, PaymentSettingsService};
use crate::bootstrap::AppState;
use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;
use crate::presentation::extractors::actor::{
    OWNER_COOKIE, STAFF_COOKIE, accounts, actor_of, cookie, same_origin,
};
use crate::presentation::http::rpc::RpcClock;

const MAX_BODY_BYTES: usize = 256 * 1024;
const DB_ERROR: &str = "โหลดการตั้งค่าไม่ได้ กรุณาตรวจการเชื่อมต่อและติดตั้งตารางวิธีชำระเงิน";

/// Routes of this module; registered through the one route table.
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let store = BranchSettingStore::new(state.db.clone());
    let branches = PgBranchRepository::new(state.db.clone());
    let service = PaymentSettingsService::new(branch_tx, store, branches);
    cfg.app_data(web::Data::new(service));

    cfg.service(
        web::resource("/api/payment-methods")
            .app_data(web::PayloadConfig::new(MAX_BODY_BYTES))
            .route(web::post().to(handle)),
    );
}

fn ok(body: Value) -> HttpResponse {
    HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .content_type("application/json")
        .json(body)
}

fn refuse(status: u16, sentence: &str) -> HttpResponse {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
    HttpResponse::build(code)
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .content_type("application/json")
        .json(serde_json::json!({ "error": sentence }))
}

async fn handle(
    req: HttpRequest,
    body: Result<web::Bytes, actix_web::Error>,
    service: web::Data<PaymentSettingsService>,
) -> HttpResponse {
    let Some(accounts) = accounts(&req) else {
        return refuse(500, DB_ERROR);
    };

    let now = req
        .app_data::<web::Data<dyn RpcClock>>()
        .map_or_else(Utc::now, |clock| clock.now());

    // Row 1: Nobody signed in
    let signed_in = match accounts
        .status(
            &cookie(&req, OWNER_COOKIE),
            &cookie(&req, STAFF_COOKIE),
            now,
        )
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return refuse(401, "กรุณาเข้าสู่ระบบ"),
        Err(_) => return refuse(500, DB_ERROR),
    };

    let Some(actor) = actor_of(&signed_in) else {
        return refuse(401, "กรุณาเข้าสู่ระบบ");
    };

    let username = match &signed_in {
        SignedIn::Owner(o) => o.username.clone(),
        SignedIn::Employee(e) => e.username.clone(),
    };

    // Row 2: Origin / X-Xchange-Request
    if !same_origin(&req) {
        return refuse(403, "Request rejected.");
    }

    // Row 3: Body length > 12,000 UTF-16 units
    let bytes = match body {
        Ok(b) => b,
        Err(e) => {
            if e.as_error::<PayloadError>()
                .is_some_and(|p| matches!(p, PayloadError::Overflow))
            {
                return refuse(400, "คำขอใหญ่เกินกำหนด");
            }
            return refuse(400, "คำขอไม่ถูกต้อง");
        }
    };

    let Ok(text) = std::str::from_utf8(&bytes) else {
        return refuse(400, "คำขอไม่ถูกต้อง");
    };

    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.encode_utf16().count() > 12_000 {
        return refuse(400, "คำขอใหญ่เกินกำหนด");
    }

    // Row 4: Non-JSON body / not an object
    let body_val: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return refuse(400, "คำขอไม่ถูกต้อง"),
    };
    if !body_val.is_object() {
        return refuse(400, "คำขอไม่ถูกต้อง");
    }

    let payload = match body_val.get("payload") {
        Some(p) => {
            if !p.is_object() {
                return refuse(400, "คำขอไม่ถูกต้อง");
            }
            p
        }
        None => &Value::Null,
    };

    // Row 5: Branch validation
    let named = match payload.get("branchId") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => (!s.is_empty()).then(|| BranchId::new(s)),
        _ => Some(BranchId::new("")),
    };
    let branch = Policy::branch(Some(&actor), named.as_ref());
    if !BranchId::is_well_formed(branch.as_str()) {
        return refuse(400, "สาขาไม่ถูกต้อง");
    }

    // Row 6 & 7: Action and Policy check
    let action_str = body_val
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let action = Action::new(action_str);
    if let Err(policy_err) = Policy::allow(Some(&actor), EntryPath::PaymentMethods, &action) {
        return refuse(policy_err.status(), &policy_err.to_string());
    }

    // Row 8: Branch must exist
    match service.branch_exists(&branch).await {
        Ok(true) => {}
        Ok(false) => return refuse(400, "ไม่พบสาขา"),
        Err(_) => return refuse(500, DB_ERROR),
    }

    // Process list / save
    if action_str == "list" {
        match service.view(&branch, &actor).await {
            Ok(view) => ok(serde_json::to_value(view).unwrap()),
            Err(_) => refuse(500, DB_ERROR),
        }
    } else {
        // action_str == "save"
        // Rows 9–14
        let methods_val = payload.get("methods").unwrap_or(&Value::Null);
        let rules = match parse_methods(methods_val) {
            Ok(r) => r,
            Err((status, sentence)) => return refuse(status, sentence),
        };

        // Row 15: version
        let version_val = payload.get("version");
        let expected_version = match parse_version(version_val) {
            Ok(v) => v,
            Err((status, sentence)) => return refuse(status, sentence),
        };

        match service
            .save(&branch, expected_version, rules, &username, now)
            .await
        {
            Ok(view) => ok(serde_json::to_value(view).unwrap()),
            Err(PaymentSaveError::Conflict(_)) => {
                refuse(409, "มีผู้แก้ไขการตั้งค่านี้แล้ว กรุณาโหลดล่าสุดก่อนบันทึก")
            }
            Err(PaymentSaveError::Database(_)) => refuse(500, DB_ERROR),
        }
    }
}
