//! `POST /api/drawer-passport`, filled by row PD-0 (design §5.5.5, §5.5.8).

use actix_web::error::PayloadError;
use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::Utc;
use serde_json::Value;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use crate::application::workflows::passport_drawer::{
    PassportDrawerRefusal, PassportDrawerWorkflow,
};
use crate::bootstrap::AppState;
use crate::infrastructure::devices::passport_drawer_device::passport_drawer_device;
use crate::presentation::dto::drawer_passport::{PassportDrawerAnswer, PassportDrawerInput};
use crate::presentation::extractors::actor::{authorize, callers, failure, same_origin};
use crate::presentation::http::rpc::RpcClock;
use crate::presentation::pages;

/// Routes of this module; registered through the one route table.
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let device = passport_drawer_device(&state.settings, &state.db);
    let workflow = PassportDrawerWorkflow::new(device);
    cfg.app_data(web::Data::new(workflow)).service(
        web::resource("/api/drawer-passport")
            .app_data(web::PayloadConfig::new(8_000_008))
            .route(web::post().to(handle_drawer_passport))
            .default_service(web::to(pages::fallback)),
    );
}

fn refuse_401(sentence: &str) -> HttpResponse {
    HttpResponse::Unauthorized()
        .content_type("application/json")
        .json(serde_json::json!({ "error": sentence }))
}

fn refuse(status: u16, sentence: &str) -> HttpResponse {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST);
    HttpResponse::build(code)
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .content_type("application/json")
        .json(serde_json::json!({ "error": sentence }))
}

fn body_text(body: Result<web::Bytes, actix_web::Error>) -> Result<String, HttpResponse> {
    let bytes = body.map_err(|error| match error.as_error::<PayloadError>() {
        Some(PayloadError::Overflow) => refuse(400, "Request too large"),
        _ => refuse(400, "คำขอไม่ถูกต้อง"),
    })?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if text.encode_utf16().count() > 2000 {
        return Err(refuse(400, "Request too large"));
    }
    Ok(text.to_string())
}

async fn handle_drawer_passport(
    req: HttpRequest,
    body: Result<web::Bytes, actix_web::Error>,
    workflow: web::Data<PassportDrawerWorkflow>,
) -> HttpResponse {
    let callers = callers(&req).await;
    if callers.user.is_none() {
        return refuse_401("กรุณาเข้าสู่ระบบ");
    }

    let now = req
        .app_data::<web::Data<dyn RpcClock>>()
        .map_or_else(Utc::now, |c| c.now());

    // 2. Origin: Origin / X-Xchange-Request
    if !same_origin(&req) {
        return refuse(400, "Request rejected");
    }

    // 3. Size: body longer than 2,000 characters
    let text = match body_text(body) {
        Ok(t) => t,
        Err(res) => return res,
    };

    let trimmed = text.trim();
    let body_val: Value = if trimmed.is_empty() {
        return refuse(400, "คำขอไม่ถูกต้อง");
    } else {
        match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => return refuse(400, "คำขอไม่ถูกต้อง"),
        }
    };
    if body_val.is_null() || !body_val.is_object() {
        return refuse(400, "คำขอไม่ถูกต้อง");
    }

    // 4 & 5. Named branch & Policy::authorize
    let named_str = match body_val.get("branchId").and_then(Value::as_str) {
        Some(s) if !s.is_empty() => s,
        _ => "MAIN",
    };
    let named_branch = BranchId::new(named_str);

    let empty_action = Action::new("");
    let (grant, callers) = match authorize(
        &req,
        EntryPath::DrawerPassport,
        &empty_action,
        Some(&named_branch),
    )
    .await
    {
        Ok(pair) => pair,
        Err(refusal) => return refusal,
    };

    // 6. Shape: branch shape check
    let branch = grant.branch;
    if !BranchId::is_well_formed(branch.as_str()) {
        return refuse(400, "สาขาไม่ถูกต้อง");
    }

    // 7. Workflow
    let input = PassportDrawerInput::from_value(&body_val);
    let actor_id = callers
        .user
        .as_ref()
        .map(|u| u.user_id.as_str())
        .unwrap_or("");

    match workflow.enter(&branch, &input, actor_id, now).await {
        Ok(reply) => HttpResponse::Ok()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .content_type("application/json")
            .json(PassportDrawerAnswer::with_duplicate(
                reply.state,
                reply.duplicate,
            )),
        Err(PassportDrawerRefusal::InvalidBranch) => refuse(400, "สาขาไม่ถูกต้อง"),
        Err(PassportDrawerRefusal::InvalidOrderId) => refuse(400, "รหัสออเดอร์ไม่ถูกต้อง"),
        Err(PassportDrawerRefusal::KioskNotHeld) => refuse(400, "ต้องเป็นผู้รับเรื่องของคิวนี้ก่อนเปิดลิ้นชัก"),
        Err(PassportDrawerRefusal::OrderHeldByOther) => refuse(400, "ออเดอร์นี้เป็นของพนักงานคนอื่น"),
        Err(PassportDrawerRefusal::Database(err)) => failure(err),
    }
}
