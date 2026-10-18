//! Bookings routes: `/api/bookings` and `/api/staff/bookings`.

use std::sync::Arc;

use actix_web::{HttpRequest, HttpResponse, web};
use chrono::{DateTime, Utc};
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_exchange::application::booking::service::BookingService;
use xc_exchange::domain::booking::commands::confirm_booking::ConfirmBooking;
use xc_exchange::domain::booking::commands::create_booking::CreateBooking;
use xc_exchange::domain::booking::commands::issue_member::IssueMember;
use xc_exchange::domain::booking::value_objects::booking_action::BookingAction;
use xc_exchange::domain::booking::value_objects::booking_error::BookingError;
use xc_exchange::domain::booking::value_objects::booking_input::{
    contact_phone, member_secret, required_text,
};
use xc_exchange::domain::booking::value_objects::booking_rules::BookingRules;
use xc_exchange::infrastructure::booking::booking_ids::UuidBookingIds;
use xc_exchange::infrastructure::booking::booking_repository::PgBookingRepository;
use xc_exchange::infrastructure::booking::member_repository::PgMemberRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CompatRate, CurrencyCode, Money, NumberError, Side};

use crate::application::bookings::booking_rules_service::BookingRulesService;
use crate::bootstrap::AppState;
use crate::infrastructure::bookings::booking_ports::PgBookingPorts;
use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;
use crate::presentation::dto::booking::{booking_json, booking_view_json, list_json};
use crate::presentation::extractors::actor::{answer, refuse, server_error};
use crate::presentation::extractors::origin::same_origin;
use crate::presentation::http::rpc::RpcClock;
use crate::presentation::pages::{self, SessionReader};

const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
const OWNER_ONLY: &str = "เฉพาะเจ้าของระบบเท่านั้น";
const ORIGIN_REJECTED: &str = "ORIGIN_REJECTED";
const TOO_MUCH: &str = "ข้อมูลมากเกินไป";
const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
const BAD_BRANCH: &str = "สาขาไม่ถูกต้อง";
const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";

const FILL_IN: &str = "กรอกข้อมูลให้ครบและไม่เกินกำหนด";
const BAD_PHONE: &str = "เบอร์ติดต่อไม่ถูกต้อง";
const INCOMPLETE: &str = "ข้อมูลรายการไม่ครบ";
const BAD_NUMBER: &str = "จำนวนเงินหรือเรทไม่ถูกต้อง";
const TOO_MANY_DECIMALS: &str = "ทศนิยมเกินกำหนด";
const NOT_IDENTIFIED: &str = "ต้องเลือกลูกค้าและยืนยันตรวจตัวตนหน้าร้านก่อน";
const BAD_MEMBER_CODE: &str = "รหัสสมาชิกไม่ถูกต้อง";
const BAD_RULES: &str = "กำหนดเวลา 5\u{2013}1440 นาที";
const NO_REGISTRATION: &str = "ไม่พบการลงทะเบียนของสาขานี้";

/// Configures booking routes into the service config.
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let pool = state.db.clone();
    let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
    let store = BranchSettingStore::new(pool.clone());
    let rules_service = BookingRulesService::new(branch_tx.clone(), store);
    let ports = Arc::new(PgBookingPorts::new(pool.clone()));
    let bookings = Arc::new(PgBookingRepository::new(pool.clone()));
    let members = Arc::new(PgMemberRepository::new(pool.clone()));
    let ids = Arc::new(UuidBookingIds);
    let booking_service = BookingService::new(branch_tx, bookings, members, ports, ids);

    cfg.app_data(web::Data::new(booking_service));
    cfg.app_data(web::Data::new(rules_service));

    let body = || web::PayloadConfig::new(1 << 20);

    cfg.service(
        web::resource("/api/v2/bookings")
            .app_data(body())
            .route(web::post().to(owner_bookings))
            .default_service(web::to(pages::fallback)),
    );
    cfg.service(
        web::resource("/api/staff/bookings")
            .app_data(body())
            .route(web::post().to(staff_bookings))
            .default_service(web::to(pages::fallback)),
    );
}

fn current_time(req: &HttpRequest) -> DateTime<Utc> {
    req.app_data::<web::Data<dyn RpcClock>>()
        .map_or_else(Utc::now, |clock| clock.now())
}

/// Handler for the owner route `POST /api/bookings`.
async fn owner_bookings(
    req: HttpRequest,
    bytes: web::Bytes,
    booking_service: web::Data<BookingService>,
    rules_service: web::Data<BookingRulesService>,
) -> HttpResponse {
    let now = current_time(&req);

    // 1. Session and policy check (no Cache-Control)
    let callers = if let Some(reader) = req.app_data::<web::Data<dyn SessionReader>>() {
        reader.callers(&req).await
    } else {
        pages::Callers {
            user: None,
            staff_user: None,
        }
    };

    let action_dummy = Action::new("/api/v2/bookings");
    match Policy::authorize(
        callers.user.as_ref(),
        EntryPath::OwnerRoute,
        &action_dummy,
        None,
        now,
    ) {
        Ok(_) => {}
        Err(PolicyError::SignIn) => {
            return answer(401, json!({ "error": SIGN_IN }), false);
        }
        Err(PolicyError::OwnerOnly) => {
            return answer(403, json!({ "error": OWNER_ONLY }), false);
        }
        Err(err) => {
            return answer(err.status(), json!({ "error": err.to_string() }), false);
        }
    }

    // 2. Same-origin check
    if !same_origin(&req) {
        return refuse(400, ORIGIN_REJECTED);
    }

    // 3. Body size check (<= 8,000 UTF-16 units)
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return refuse(400, BAD_REQUEST);
    };
    if text.encode_utf16().count() > 8000 {
        return refuse(400, TOO_MUCH);
    }

    // 4. JSON & payload shape
    if text.is_empty() {
        return refuse(400, BAD_REQUEST);
    }
    let body: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return refuse(400, BAD_REQUEST),
    };
    if !body.is_object() {
        return refuse(400, BAD_REQUEST);
    }
    let payload = match body.get("payload") {
        None | Some(Value::Null) => json!({}),
        Some(v) if v.is_object() => v.clone(),
        _ => return refuse(400, BAD_REQUEST),
    };

    // 5. Branch validation
    let branch = match payload.get("branchId") {
        None | Some(Value::Null) => BranchId::new("MAIN"),
        Some(Value::String(s)) if s.is_empty() => BranchId::new("MAIN"),
        Some(Value::String(s)) => {
            let count = s.encode_utf16().count();
            if count == 0 || count > 70 || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                return refuse(400, BAD_BRANCH);
            }
            BranchId::new(s)
        }
        _ => return refuse(400, BAD_BRANCH),
    };

    // 6. Action validation
    let Some(action_str) = body.get("action").and_then(|a| a.as_str()) else {
        return refuse(400, BAD_REQUEST);
    };
    let Some(action) = BookingAction::parse(action_str) else {
        return refuse(400, BAD_REQUEST);
    };

    // 7. Branch existence check
    match booking_service.branch_exists(&branch).await {
        Ok(true) => {}
        Ok(false) => return refuse(400, NO_SUCH_BRANCH),
        Err(AppError::Database(_)) | Err(AppError::Internal(_)) => return server_error(),
        Err(AppError::Domain(msg)) => return refuse(400, &msg),
        Err(_) => return server_error(),
    }

    let actor_id = owner_route_actor(&callers);
    execute_action(
        action,
        &branch,
        &payload,
        &actor_id,
        now,
        &booking_service,
        &rules_service,
    )
    .await
}

/// Handler for the staff route `POST /api/staff/bookings`.
async fn staff_bookings(
    req: HttpRequest,
    bytes: web::Bytes,
    booking_service: web::Data<BookingService>,
    rules_service: web::Data<BookingRulesService>,
) -> HttpResponse {
    let now = current_time(&req);

    // 1. Same-origin check (answers 400 คำขอไม่ถูกต้อง, before session)
    if !same_origin(&req) {
        return refuse(400, BAD_REQUEST);
    }

    // 2. Body size check (<= 8,000 UTF-16 units, before session)
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return refuse(400, BAD_REQUEST);
    };
    if text.encode_utf16().count() > 8000 {
        return refuse(400, TOO_MUCH);
    }

    // 3. JSON & payload shape (before session)
    if text.is_empty() {
        return refuse(400, BAD_REQUEST);
    }
    let body: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return refuse(400, BAD_REQUEST),
    };
    if !body.is_object() {
        return refuse(400, BAD_REQUEST);
    }
    let payload = match body.get("payload") {
        None | Some(Value::Null) => json!({}),
        Some(v) if v.is_object() => v.clone(),
        _ => return refuse(400, BAD_REQUEST),
    };

    // 4. Session and policy check
    let callers = if let Some(reader) = req.app_data::<web::Data<dyn SessionReader>>() {
        reader.callers(&req).await
    } else {
        pages::Callers {
            user: None,
            staff_user: None,
        }
    };

    let action_str = body.get("action").and_then(|a| a.as_str()).unwrap_or("");
    let action = Action::new(action_str);

    let actor_user = callers.staff_user.as_ref().or(callers.user.as_ref());
    let grant = match Policy::authorize(actor_user, EntryPath::StaffBookings, &action, None, now) {
        Ok(grant) => grant,
        Err(refusal) => return refuse(refusal.status(), &refusal.to_string()),
    };

    // 5. Branch: the employee's own branch
    let branch = grant.branch;

    // 6. Branch existence check
    match booking_service.branch_exists(&branch).await {
        Ok(true) => {}
        Ok(false) => return refuse(400, NO_SUCH_BRANCH),
        Err(AppError::Database(_)) | Err(AppError::Internal(_)) => return server_error(),
        Err(AppError::Domain(msg)) => return refuse(400, &msg),
        Err(_) => return server_error(),
    }

    // 7. Parse BookingAction (safe because Policy::allow passed)
    let booking_action = match BookingAction::parse(action_str) {
        Some(a) => a,
        None => return refuse(400, BAD_REQUEST),
    };

    // 8. Execute action
    let staff_id = callers.staff_standing().map(|s| s.user_id);
    let actor_id = staff_id.as_deref().unwrap_or("owner");
    execute_action(
        booking_action,
        &branch,
        &payload,
        actor_id,
        now,
        &booking_service,
        &rules_service,
    )
    .await
}

fn owner_route_actor(callers: &pages::Callers) -> String {
    let caller = callers
        .owner_standing()
        .or_else(|| callers.staff_standing());
    caller.map_or_else(|| "owner".to_string(), |s| s.user_id)
}

async fn execute_action(
    action: BookingAction,
    branch: &BranchId,
    payload: &Value,
    actor_id: &str,
    now: DateTime<Utc>,
    booking_service: &BookingService,
    rules_service: &BookingRulesService,
) -> HttpResponse {
    match action {
        BookingAction::List => match booking_service.list(branch, now).await {
            Ok(list) => match list_json(branch, &list, now) {
                Ok(val) => answer(200, val, true),
                Err(_) => server_error(),
            },
            Err(AppError::Domain(sentence)) => refuse(400, &sentence),
            Err(AppError::Database(_)) | Err(AppError::Internal(_)) => server_error(),
            Err(_) => server_error(),
        },
        BookingAction::Get => {
            let id = match required_text(&payload["id"], 60) {
                Ok(id) => id,
                Err(_) => return refuse(400, FILL_IN),
            };
            match booking_service.get(branch, &id).await {
                Ok(view) => match booking_view_json(&view, now) {
                    Ok(val) => answer(200, val, true),
                    Err(_) => server_error(),
                },
                Err(AppError::Domain(sentence)) => refuse(400, &sentence),
                Err(AppError::Database(_)) | Err(AppError::Internal(_)) => server_error(),
                Err(_) => server_error(),
            }
        }
        BookingAction::Create => {
            let request_id = match required_text(&payload["requestId"], 60) {
                Ok(r) => r,
                Err(_) => return refuse(400, FILL_IN),
            };
            let name = match required_text(&payload["name"], 100) {
                Ok(n) => n,
                Err(_) => return refuse(400, FILL_IN),
            };
            let phone = match contact_phone(&payload["phone"]) {
                Ok(p) => p,
                Err(_) => return refuse(400, BAD_PHONE),
            };
            let side = match payload.get("type").and_then(|v| v.as_str()) {
                Some("BUY") => Side::Buy,
                Some("SELL") => Side::Sell,
                _ => return refuse(400, INCOMPLETE),
            };
            let currency = match payload.get("currency").and_then(|v| v.as_str()) {
                Some(c) => match CurrencyCode::parse(c) {
                    Ok(code) if code != CurrencyCode::THB => code,
                    _ => return refuse(400, INCOMPLETE),
                },
                None => return refuse(400, INCOMPLETE),
            };
            let amount_str = match payload.get("amount") {
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::String(s)) => s.clone(),
                _ => return refuse(400, BAD_NUMBER),
            };
            let amount = match Money::parse(&amount_str) {
                Ok(m) if !m.is_zero() => m,
                Ok(_) => return refuse(400, BAD_NUMBER),
                Err(NumberError::TooManyDecimals) => return refuse(400, TOO_MANY_DECIMALS),
                Err(NumberError::Invalid) => return refuse(400, BAD_NUMBER),
            };
            let rate_str = match payload.get("expectedRate") {
                Some(Value::Number(n)) => n.to_string(),
                Some(Value::String(s)) => s.clone(),
                _ => return refuse(400, BAD_NUMBER),
            };
            let expected_rate = match CompatRate::parse_per_one(&rate_str) {
                Ok(r) => r,
                Err(NumberError::TooManyDecimals) => return refuse(400, TOO_MANY_DECIMALS),
                Err(NumberError::Invalid) => return refuse(400, BAD_NUMBER),
            };
            let customer_id = payload
                .get("customerId")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            let cmd = CreateBooking {
                request_id,
                name,
                phone,
                side,
                currency,
                amount,
                expected_rate,
                customer_id,
            };

            match booking_service.create(branch, cmd, actor_id, now).await {
                Ok(booking) => match booking_json(&booking, now) {
                    Ok(b) => answer(
                        200,
                        json!({
                            "saved": true,
                            "booking": b,
                        }),
                        true,
                    ),
                    Err(_) => server_error(),
                },
                Err(AppError::Domain(sentence)) => refuse(400, &sentence),
                Err(AppError::Database(_)) | Err(AppError::Internal(_)) => server_error(),
                Err(_) => server_error(),
            }
        }
        BookingAction::Confirm => {
            let id = match required_text(&payload["id"], 60) {
                Ok(id) => id,
                Err(_) => return refuse(400, FILL_IN),
            };
            let expires_at = match payload.get("expiresAt").and_then(|v| v.as_str()) {
                Some(s) => DateTime::parse_from_rfc3339(s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc)),
                None => None,
            };
            let expected_rate = match payload.get("expectedRate") {
                Some(Value::Number(n)) => CompatRate::parse_per_one(&n.to_string()).ok(),
                Some(Value::String(s)) => CompatRate::parse_per_one(s).ok(),
                _ => None,
            };

            let cmd = ConfirmBooking {
                id,
                expires_at,
                expected_rate,
            };

            match booking_service.confirm(branch, cmd, actor_id, now).await {
                Ok(()) => answer(200, json!({ "saved": true }), true),
                Err(AppError::Domain(sentence)) => refuse(400, &sentence),
                Err(AppError::Database(_)) | Err(AppError::Internal(_)) => server_error(),
                Err(_) => server_error(),
            }
        }
        BookingAction::Cancel => {
            let id = match required_text(&payload["id"], 60) {
                Ok(id) => id,
                Err(_) => return refuse(400, FILL_IN),
            };
            match booking_service.cancel(branch, &id).await {
                Ok(()) => answer(200, json!({ "saved": true }), true),
                Err(AppError::Domain(sentence)) => refuse(400, &sentence),
                Err(AppError::Database(_)) | Err(AppError::Internal(_)) => server_error(),
                Err(_) => server_error(),
            }
        }
        BookingAction::Member => {
            let confirmed = payload.get("identityConfirmed") == Some(&Value::Bool(true));
            let customer_id = payload
                .get("customerId")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            let (true, Some(cid)) = (confirmed, customer_id) else {
                return refuse(400, NOT_IDENTIFIED);
            };

            let phone = match contact_phone(&payload["phone"]) {
                Ok(p) => p,
                Err(_) => return refuse(400, BAD_PHONE),
            };

            let secret = match member_secret(&payload["memberSecret"]) {
                Ok(s) => s,
                Err(BookingError::FillIn) => return refuse(400, FILL_IN),
                Err(BookingError::BadMemberCode) => return refuse(400, BAD_MEMBER_CODE),
                Err(e) => return refuse(400, &e.to_string()),
            };

            let cmd = IssueMember {
                customer_id: cid.to_string(),
                identity_confirmed: true,
                phone,
                secret: secret.clone(),
            };

            match booking_service
                .issue_member(branch, cmd, actor_id, now)
                .await
            {
                Ok(member) => answer(
                    200,
                    json!({
                        "saved": true,
                        "name": member.name,
                        "memberCode": secret,
                    }),
                    true,
                ),
                Err(AppError::Domain(sentence)) => refuse(400, &sentence),
                Err(AppError::Database(_)) | Err(AppError::Internal(_)) => server_error(),
                Err(_) => server_error(),
            }
        }
        BookingAction::Settings => {
            let enabled = payload.get("enabled").unwrap_or(&Value::Null);
            let minutes = payload.get("minutes").unwrap_or(&Value::Null);
            let rules = match BookingRules::parse(enabled, minutes) {
                Ok(r) => r,
                Err(_) => return refuse(400, BAD_RULES),
            };
            match rules_service.save(branch, rules, actor_id, now).await {
                Ok(()) => answer(200, json!({ "saved": true }), true),
                Err(AppError::Domain(sentence)) => refuse(400, &sentence),
                Err(AppError::Database(_)) | Err(AppError::Internal(_)) => server_error(),
                Err(_) => server_error(),
            }
        }
        BookingAction::Registrations => {
            let page = match payload.get("page") {
                Some(Value::Number(n)) => {
                    if let Some(f) = n.as_f64() {
                        if f.is_finite() {
                            (f.floor() as i64).clamp(0, 100_000)
                        } else {
                            0
                        }
                    } else {
                        0
                    }
                }
                Some(Value::String(s)) => {
                    if let Ok(f) = s.parse::<f64>() {
                        if f.is_finite() {
                            (f.floor() as i64).clamp(0, 100_000)
                        } else {
                            0
                        }
                    } else {
                        0
                    }
                }
                _ => 0,
            };
            answer(
                200,
                json!({
                    "items": [],
                    "hasMore": false,
                    "page": page,
                }),
                true,
            )
        }
        BookingAction::ApproveRegistration => match required_text(&payload["id"], 60) {
            Ok(_) => refuse(400, NO_REGISTRATION),
            Err(_) => refuse(400, FILL_IN),
        },
    }
}

#[cfg(test)]
mod tests {
    use xc_auth::domain::access::{Actor, Role, ScopedRole};
    #[test]
    fn a_blueprint_owner_is_recorded_under_their_id() {
        let org = |role| ScopedRole::new(role, None, Default::default(), None);
        let roles = vec![org(Role::Executive), org(Role::Admin)];
        let user = Some(Actor::new("u-own", "org", roles, None));
        let callers = super::pages::Callers {
            user,
            staff_user: None,
        };
        assert_eq!(super::owner_route_actor(&callers), "u-own");
    }
}
