//! Employee routes: `/api/employees`, the owner's employee screen.

use std::sync::Arc;
use std::time::SystemTime;

use actix_web::{HttpRequest, HttpResponse, web};
use pos_infra::transactions::Transactor;
use serde_json::json;
use xc_auth::application::account::service::EmployeesAnswer;
use xc_auth::domain::account::entities::employee::Employee;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_rate::application::branch::service::BranchService;

use crate::application::workflows::disable_employee::{DisableEmployee, DisableOutcome};
use crate::bootstrap::AppState;
use crate::infrastructure::custody::employee_custody::PgEmployeeCustody;
use crate::presentation::extractors::actor::{
    OWNER_ID, accounts, answer, failure, password_session, payload_of, read, refuse, same_origin,
    server_error,
};
use crate::presentation::http::auth::employee_json;
use crate::presentation::pages;

const HOLDS_CASH: &str = "ส่งมอบเงินและงานค้างก่อนปิดบัญชี หากเข้าระบบไม่ได้ให้รีเซ็ตรหัสผ่าน";

pub use crate::application::custody::employee_custody::EmployeeCustody;

pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let branch_tx = BranchTx::new(Transactor::new(state.db.clone()));
    let branches = BranchService::new(branch_tx.clone());
    let disable = DisableEmployee::new(branch_tx);
    // For the whole app, not for this resource alone: a test that registers its own port
    // after `.configure(...)` replaces this one.
    let custody: Arc<dyn EmployeeCustody> = Arc::new(PgEmployeeCustody::new(state.db.clone()));
    cfg.app_data(web::Data::from(custody));
    cfg.service(
        web::resource("/api/employees")
            .app_data(web::PayloadConfig::new(1 << 20))
            .app_data(web::Data::new(branches))
            .app_data(web::Data::new(disable))
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

    // A database failure reading the password session is v2's 500 sentence, before the rule.
    if let Err(e) = password_session(accounts, &req, now()).await {
        return failure(e);
    }

    let callers = pages::get_callers(&req).await;
    let action = Action::new("employees");
    if let Err(refusal) = Policy::authorize(
        callers.user.as_ref(),
        EntryPath::OwnerRoute,
        &action,
        None,
        chrono::Utc::now(),
    ) {
        return answer(
            refusal.status(),
            json!({ "error": refusal.to_string() }),
            false,
        );
    }

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
    let standing = callers
        .owner_standing()
        .or_else(|| callers.staff_standing());
    let by = standing
        .as_ref()
        .map(|s| s.user_id.as_str())
        .or_else(|| callers.user.as_ref().map(|u| u.user_id.as_str()))
        .unwrap_or(OWNER_ID);

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
            // The answer above is a quick refusal on the pool; the one that counts is asked
            // again under the employee's branch lock, together with the closing (#157). An id
            // that names nobody goes on to the accounts' own answer below.
            if let Some(employee) = found {
                let Some(disable) = req.app_data::<web::Data<DisableEmployee>>() else {
                    return server_error();
                };
                return match disable
                    .run(
                        accounts,
                        custody.cloned().map(web::Data::into_inner),
                        &BranchId::new(employee.branch_id.clone()),
                        &employee.id,
                        by,
                        now(),
                    )
                    .await
                {
                    Ok(DisableOutcome::Closed) => answer(200, json!({ "saved": true }), true),
                    Ok(DisableOutcome::HoldsCash) => refuse(400, HOLDS_CASH),
                    Err(e) => failure(e),
                };
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
        _ => {}
    }

    match accounts.employees(action, &payload, by, now()).await {
        Ok(EmployeesAnswer::List(people)) => answer(
            200,
            json!({ "employees": people.iter().map(employee_json).collect::<Vec<_>>() }),
            true,
        ),
        Ok(EmployeesAnswer::Saved) => answer(200, json!({ "saved": true }), true),
        Err(e) => failure(e),
    }
}
