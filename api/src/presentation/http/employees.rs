//! Employee routes: `/api/employees`, the owner's employee screen.

use std::future::Future;
use std::pin::Pin;
use std::time::SystemTime;

use actix_web::{HttpRequest, HttpResponse, web};
use pos_error::AppResult;
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use xc_auth::application::account::service::EmployeesAnswer;
use xc_auth::domain::account::entities::employee::Employee;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;

use crate::bootstrap::AppState;
use crate::presentation::extractors::actor::{
    BAD_REQUEST, OWNER_COOKIE, OWNER_ID, STAFF_COOKIE, accounts, actor_of, answer, cookie, failure,
    payload_of, read, refuse, same_origin, server_error, signed_in,
};
use crate::presentation::http::auth::employee_json;
use crate::presentation::pages;

const HOLDS_CASH: &str = "ส่งมอบเงินและงานค้างก่อนปิดบัญชี หากเข้าระบบไม่ได้ให้รีเซ็ตรหัสผ่าน";
const SIX_ROLES: &str =
    "สร้างบัญชีให้ครบ Front, Backend, Manager, Rates, Monitor และ Finance ในสาขาก่อน";

/// What the employee screen needs from custody. Looked up as `web::Data<dyn EmployeeCustody>`;
/// until one is registered nobody holds cash and the two custody actions are bad requests.
pub trait EmployeeCustody: Send + Sync {
    fn holds_cash_or_pending<'a>(
        &'a self,
        branch: &'a BranchId,
        employee_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = AppResult<bool>> + 'a>>;
    fn set_entity<'a>(
        &'a self,
        payload: &'a Value,
        actor: &'a Actor,
    ) -> Pin<Box<dyn Future<Output = AppResult<Value>> + 'a>>;
    fn enable<'a>(
        &'a self,
        payload: &'a Value,
        actor: &'a Actor,
    ) -> Pin<Box<dyn Future<Output = AppResult<Value>> + 'a>>;
}

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branches = BranchService::new(BranchTx::new(Transactor::new(state.db.clone())));
    cfg.service(
        web::resource("/api/employees")
            .app_data(web::PayloadConfig::new(1 << 20))
            .app_data(web::Data::new(branches))
            .route(web::post().to(employees))
            .default_service(web::to(pages::fallback)),
    );
}

async fn everyone(req: &HttpRequest) -> Result<Vec<Employee>, HttpResponse> {
    let accounts = accounts(req).ok_or_else(server_error)?;
    match accounts
        .employees("list", &json!({}), OWNER_ID, SystemTime::now().into())
        .await
    {
        Ok(EmployeesAnswer::List(people)) => Ok(people),
        Ok(EmployeesAnswer::Saved) => Err(server_error()),
        Err(e) => Err(failure(e)),
    }
}

async fn employees(req: HttpRequest, bytes: web::Bytes) -> HttpResponse {
    let Some(accounts) = accounts(&req) else {
        return server_error();
    };
    let now = || SystemTime::now().into();
    let who = match signed_in(
        accounts,
        &cookie(&req, OWNER_COOKIE),
        &cookie(&req, STAFF_COOKIE),
        SystemTime::now(),
    )
    .await
    {
        Ok(who) => who,
        Err(e) => return failure(e),
    };
    let actor = who.as_ref().and_then(actor_of);
    if let Err(refusal) = Policy::allow(
        actor.as_ref(),
        EntryPath::OwnerRoute,
        &Action::new("employees"),
    ) {
        return answer(
            refusal.status(),
            json!({ "error": refusal.to_string() }),
            false,
        );
    }
    let Some(actor) = actor else {
        return server_error();
    };
    if !same_origin(&req) {
        return refuse(400, "ORIGIN_REJECTED");
    }
    let body = match read(&bytes, 6000, "ข้อมูลมากเกินไป") {
        Ok(body) => body,
        Err(response) => return response,
    };
    let payload = payload_of(&body);
    let action = body["action"].as_str().unwrap_or("");
    let custody = req.app_data::<web::Data<dyn EmployeeCustody>>();

    match action {
        "disable" => {
            let people = match everyone(&req).await {
                Ok(people) => people,
                Err(response) => return response,
            };
            let found = payload["id"]
                .as_str()
                .and_then(|id| people.iter().find(|e| e.id == id));
            if let (Some(employee), Some(custody)) = (found, custody) {
                match custody
                    .holds_cash_or_pending(&BranchId::new(employee.branch_id.clone()), &employee.id)
                    .await
                {
                    Ok(false) => {}
                    Ok(true) => return refuse(400, HOLDS_CASH),
                    Err(e) => return failure(e),
                }
            }
        }
        "create" => {
            if let Some(branch) = payload["branchId"].as_str().filter(|b| !b.is_empty()) {
                let Some(branches) = req.app_data::<web::Data<BranchService>>() else {
                    return server_error();
                };
                if let Err(e) = branches.require(&BranchId::new(branch)).await {
                    return failure(e);
                }
            }
        }
        "entity" => {
            return match custody {
                None => refuse(400, BAD_REQUEST),
                Some(custody) => match custody.set_entity(&payload, &actor).await {
                    Ok(body) => answer(200, body, false),
                    Err(e) => failure(e),
                },
            };
        }
        "enable" => {
            let people = match everyone(&req).await {
                Ok(people) => people,
                Err(response) => return response,
            };
            let branch = payload["branchId"].as_str();
            for role in ["FRONT", "BACKEND", "MANAGER", "RATES", "MONITOR", "FINANCE"] {
                if !people
                    .iter()
                    .any(|e| Some(e.branch_id.as_str()) == branch && e.active && e.role == role)
                {
                    return refuse(400, SIX_ROLES);
                }
            }
            return match custody {
                None => refuse(400, BAD_REQUEST),
                Some(custody) => match custody.enable(&payload, &actor).await {
                    Ok(body) => answer(200, body, false),
                    Err(e) => failure(e),
                },
            };
        }
        _ => {}
    }

    match accounts
        .employees(action, &payload, actor.id(), now())
        .await
    {
        Ok(EmployeesAnswer::List(people)) => answer(
            200,
            json!({ "employees": people.iter().map(employee_json).collect::<Vec<_>>() }),
            true,
        ),
        Ok(EmployeesAnswer::Saved) => answer(200, json!({ "saved": true }), true),
        Err(e) => failure(e),
    }
}
