//! Kiosk routes: POST /api/kiosk.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use actix_web::error::PayloadError;
use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::Utc;
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use xc_auth::domain::account::entities::employee::Employee;
use xc_auth::domain::account::entities::owner::Owner;
use xc_auth::domain::account::entities::session::SignedIn;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, Side};
use xc_kiosk::application::kiosk::service::KioskService;
use xc_kiosk::domain::kiosk::commands::configure_currency::ConfigureCurrency;
use xc_kiosk::domain::kiosk::commands::new_quote::NewQuote;
use xc_kiosk::domain::kiosk::commands::quote_input::QuoteInput;
use xc_kiosk::domain::kiosk::value_objects::accept_outcome::AcceptOutcome;
use xc_kiosk::domain::kiosk::value_objects::kiosk_action::KioskAction;
use xc_kiosk::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use xc_kiosk::domain::kiosk::value_objects::kiosk_role::KioskRole;
use xc_kiosk::domain::kiosk::value_objects::queue_filter::QueueFilter;
use xc_kiosk::domain::kiosk::value_objects::queue_page::QueuePage;
use xc_rate::application::rate::service::RateService;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;

use crate::application::compat::v2_json::V2Json;
use crate::bootstrap::AppState;
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::kiosk::kiosk_ports::{KioskAuditLog, PgKioskAuditLog, PgKioskPorts};
use crate::presentation::dto::kiosk as dto_kiosk;
use crate::presentation::extractors::actor::{
    OWNER_COOKIE, OWNER_ID, STAFF_COOKIE, accounts, cookie, refuse, same_origin, staff_actor,
};
use crate::presentation::http::client_address::client_address;
use crate::presentation::http::rpc::RpcClock;
use crate::presentation::pages;

/// The ids the kiosk route makes for a new request. Looked up as `web::Data<dyn KioskIds>`, so a
/// test can register its own after the route table (ticket #172; contract of
/// `api/tests/kiosk_http_common/mod.rs`).
pub trait KioskIds: Send + Sync {
    fn new_id(&self) -> String;
}

#[derive(Clone, Default)]
pub struct RandomKioskIds;

impl KioskIds for RandomKioskIds {
    fn new_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

/// Routes of this module; registered through the one route table.
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    let default_ids: Arc<dyn KioskIds> = Arc::new(RandomKioskIds);
    let default_audit: Arc<dyn KioskAuditLog> = Arc::new(PgKioskAuditLog::new(
        PgDeviceRepository::new(state.db.clone()),
    ));

    cfg.app_data(web::Data::from(default_ids))
        .app_data(web::Data::from(default_audit))
        .service(
            web::resource("/api/kiosk")
                .app_data(web::PayloadConfig::new(8_000_008))
                .route(web::post().to(handle_kiosk))
                .default_service(web::to(pages::fallback)),
        );
}

enum Caller {
    Customer,
    Owner(Owner),
    Employee(Employee),
}

fn kiosk_server_error() -> HttpResponse {
    HttpResponse::InternalServerError()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .content_type("application/json")
        .body(json!({ "error": "เซิร์ฟเวอร์ขัดข้อง" }).to_string())
}

/// Whether the caller may claim a request: the policy's answer for `accept`.
fn claims_requests(actor: Option<&Actor>) -> bool {
    Policy::allow(actor, EntryPath::Kiosk, &Action::new("accept")).is_ok()
}

/// The usernames of the given holder ids, read in one query. A holder is stored by id;
/// `owner` is the owner's id.
async fn usernames<'a>(
    pool: &sqlx::PgPool,
    ids: impl Iterator<Item = &'a str>,
) -> HashMap<String, String> {
    let ids: Vec<String> = ids
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    if ids.is_empty() {
        return HashMap::new();
    }
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id::text, username FROM employees WHERE id::text = ANY($1) \
         UNION ALL \
         SELECT 'owner', username FROM (SELECT username FROM owners LIMIT 1) o \
         WHERE 'owner' = ANY($1)",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await
    .unwrap_or_else(|err| {
        tracing::error!(error = ?err, "kiosk usernames failed");
        Vec::new()
    });
    rows.into_iter().collect()
}

/// The username of a holder, or the id as it is when nobody has it.
fn username_or_id(names: &HashMap<String, String>, id: &str) -> String {
    names.get(id).cloned().unwrap_or_else(|| id.to_owned())
}

fn json_response(v2_json: V2Json) -> HttpResponse {
    HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .content_type("application/json")
        .body(v2_json.to_text())
}

fn saved_response() -> HttpResponse {
    HttpResponse::Ok()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .content_type("application/json")
        .body(r#"{"saved":true}"#)
}

fn handle_service_error(err: AppError) -> HttpResponse {
    match err {
        AppError::Domain(msg) => refuse(400, &msg),
        AppError::Unauthenticated => refuse(401, "EMPLOYEE_AUTH_REQUIRED"),
        other => {
            tracing::error!(error = ?other, "kiosk service error");
            kiosk_server_error()
        }
    }
}

fn body_text(body: Result<web::Bytes, actix_web::Error>) -> Result<String, HttpResponse> {
    let bytes = body.map_err(|error| match error.as_error::<PayloadError>() {
        Some(PayloadError::Overflow) => refuse(400, "Request too large."),
        _ => refuse(400, "Invalid request."),
    })?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if text.encode_utf16().count() > 2_000_000 {
        return Err(refuse(400, "Request too large."));
    }
    Ok(text.to_string())
}

async fn handle_kiosk(
    req: HttpRequest,
    body: Result<web::Bytes, actix_web::Error>,
    state: web::Data<AppState>,
) -> HttpResponse {
    if !same_origin(&req) {
        return refuse(400, "Request rejected.");
    }

    let text = match body_text(body) {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    let trimmed = text.trim();
    let body_val: Value = if trimmed.is_empty() {
        json!({})
    } else {
        match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => return refuse(400, "Invalid request."),
        }
    };

    if body_val.is_null() {
        return refuse(400, "Invalid request.");
    }

    let now = req
        .app_data::<web::Data<dyn RpcClock>>()
        .map_or_else(Utc::now, |c| c.now());

    let Some(accounts_service) = accounts(&req) else {
        return kiosk_server_error();
    };
    let owner_cookie = cookie(&req, OWNER_COOKIE);
    let staff_cookie = cookie(&req, STAFF_COOKIE);
    let who = match accounts_service
        .status(&owner_cookie, &staff_cookie, now)
        .await
    {
        Ok(w) => w,
        Err(err) => {
            tracing::error!(error = ?err, "status failed");
            return kiosk_server_error();
        }
    };

    // A first-password session never reaches here: `AccountService::status` answers `None` for
    // it, which is where the must-change rule lives.
    let policy_actor = match &who {
        Some(SignedIn::Owner(_)) => Some(Actor::owner(OWNER_ID)),
        Some(SignedIn::Employee(emp)) => staff_actor(emp),
        None => None,
    };

    let (kiosk_actor, caller) = match who {
        Some(SignedIn::Owner(owner)) => (
            KioskActor::Staff {
                username: "owner".to_string(),
                role: KioskRole::Owner,
            },
            Caller::Owner(owner),
        ),
        Some(SignedIn::Employee(emp)) => {
            let role = match emp.role.as_str() {
                "FRONT" => KioskRole::Front,
                "BACKEND" => KioskRole::Backend,
                "MANAGER" => KioskRole::Manager,
                "RATES" => KioskRole::Rates,
                _ => KioskRole::Other,
            };
            (
                KioskActor::Staff {
                    username: emp.id.clone(),
                    role,
                },
                Caller::Employee(emp),
            )
        }
        None => (KioskActor::Customer, Caller::Customer),
    };

    let action_str = body_val
        .as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(Value::as_str)
        .unwrap_or("");

    let policy_action = Action::new(action_str);
    if let Err(err) = Policy::allow(policy_actor.as_ref(), EntryPath::Kiosk, &policy_action) {
        return refuse(err.status(), &err.to_string());
    }

    let action = match action_str {
        "rates" => KioskAction::Rates,
        "quote" => KioskAction::Quote,
        "confirm" => KioskAction::Confirm,
        "status" => KioskAction::Status,
        "customerReceipt" => KioskAction::CustomerReceipt,
        "notifications" => KioskAction::Notifications,
        "list" => KioskAction::List,
        "accept" => KioskAction::Accept,
        "identityChecked" => KioskAction::IdentityChecked,
        "get" => KioskAction::Get,
        "publishReceipt" => KioskAction::PublishReceipt,
        "manage" => KioskAction::Manage,
        "configure" => KioskAction::Configure,
        _ => return kiosk_server_error(),
    };

    let empty_payload = json!({});
    let payload = match body_val.as_object() {
        Some(obj) => match obj.get("payload") {
            None => &empty_payload,
            Some(Value::Object(_)) => obj.get("payload").unwrap(),
            Some(_) => return refuse(400, "Invalid request."),
        },
        None => &empty_payload,
    };

    let branch = match &caller {
        Caller::Employee(emp) => BranchId::new(emp.branch_id.clone()),
        Caller::Owner(_) | Caller::Customer => match payload.get("branchId") {
            None | Some(Value::Null) => BranchId::new("MAIN"),
            Some(Value::String(s)) => {
                if s.is_empty() {
                    BranchId::new("MAIN")
                } else if BranchId::is_well_formed(s) {
                    BranchId::new(s)
                } else {
                    return refuse(400, "Invalid branch.");
                }
            }
            Some(Value::Number(n)) if n.as_i64() == Some(0) => BranchId::new("MAIN"),
            _ => return refuse(400, "Invalid branch."),
        },
    };

    let pool = &state.db;
    let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
    let audit: Arc<dyn KioskAuditLog> = req
        .app_data::<web::Data<dyn KioskAuditLog>>()
        .map(|d| d.clone().into_inner())
        .unwrap_or_else(|| Arc::new(PgKioskAuditLog::new(PgDeviceRepository::new(pool.clone()))));
    let source: Arc<dyn RateSource> = req
        .app_data::<web::Data<Arc<dyn RateSource>>>()
        .map_or_else(
            || Arc::new(BotRateSource::bot()) as Arc<dyn RateSource>,
            |s| s.get_ref().clone(),
        );
    let ports = Arc::new(PgKioskPorts::new(
        pool.clone(),
        audit.clone(),
        source.clone(),
    ));
    let service = KioskService::new(branch_tx, ports);

    match action {
        KioskAction::Rates => match service.public_rates(&branch).await {
            Ok(public_rates) => json_response(dto_kiosk::rates_json(&public_rates)),
            Err(err) => handle_service_error(err),
        },
        KioskAction::Quote => {
            let quote_type = payload.get("type").and_then(Value::as_str);
            let side = match quote_type {
                Some("BUY") => Some(Side::Buy),
                Some("SELL") => Some(Side::Sell),
                _ => None,
            };
            let currency = payload
                .get("currency")
                .and_then(Value::as_str)
                .and_then(|s| CurrencyCode::parse(s).ok());
            let amount = match payload.get("amount") {
                Some(Value::Number(n)) => Money::parse(&n.to_string()).ok(),
                Some(Value::String(s)) => Money::parse(s).ok(),
                _ => None,
            };
            let payment_method = match payload.get("paymentMethod") {
                Some(Value::String(s)) => Some(s.clone()),
                Some(Value::Null) | None => None,
                Some(_) => Some("invalid".to_string()),
            };

            let input = QuoteInput {
                side,
                currency,
                amount,
                payment_method,
            };

            let default_ids = RandomKioskIds;
            let ids: &dyn KioskIds = req
                .app_data::<web::Data<dyn KioskIds>>()
                .map(|d| d.as_ref())
                .unwrap_or(&default_ids);
            let new_id = ids.new_id();
            let client_ip = client_address(&req).to_string();

            let cmd = NewQuote {
                input,
                new_id,
                client_ip,
            };

            match service.quote(&branch, cmd, now).await {
                Ok(identified) => {
                    let branch_name: String =
                        sqlx::query_scalar("SELECT name FROM branches WHERE id = $1")
                            .bind(branch.as_str())
                            .fetch_optional(pool)
                            .await
                            .ok()
                            .flatten()
                            .unwrap_or_default();
                    json_response(dto_kiosk::quote_json(
                        &identified.request,
                        &identified.identity,
                        &branch_name,
                    ))
                }
                Err(err) => handle_service_error(err),
            }
        }
        KioskAction::Confirm => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("");
            match service.confirm(&branch, id, now).await {
                Ok(identity) => json_response(dto_kiosk::identity_json(&identity)),
                Err(err) => handle_service_error(err),
            }
        }
        KioskAction::Status => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("");
            match service.status(&branch, id, now).await {
                Ok(identity) => json_response(dto_kiosk::identity_json(&identity)),
                Err(err) => handle_service_error(err),
            }
        }
        KioskAction::CustomerReceipt => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("");
            match service.customer_receipt(&branch, id, now).await {
                Ok(receipt) => json_response(dto_kiosk::customer_receipt_json(&receipt)),
                Err(err) => handle_service_error(err),
            }
        }
        KioskAction::Notifications => match service.notifications(&branch, &kiosk_actor).await {
            Ok(notifications) => {
                let emp_info = match &caller {
                    Caller::Owner(owner) => dto_kiosk::EmployeeInfo {
                        id: "owner".to_string(),
                        username: owner.username.clone(),
                        name: owner.name.clone(),
                        role: "OWNER".to_string(),
                    },
                    Caller::Employee(emp) => dto_kiosk::EmployeeInfo {
                        id: emp.id.clone(),
                        username: emp.username.clone(),
                        name: emp.name.clone(),
                        role: emp.role.clone(),
                    },
                    Caller::Customer => unreachable!(),
                };
                let names =
                    usernames(pool, notifications.working.iter().map(|w| w.staff.as_str())).await;
                json_response(dto_kiosk::notifications_json(
                    &branch,
                    &notifications,
                    &emp_info,
                    now,
                    &|id| username_or_id(&names, id),
                ))
            }
            Err(err) => handle_service_error(err),
        },
        KioskAction::List => {
            // v2 judges the page before the filter (`queue-work.mjs:15-16`).
            let page = match QueuePage::parse(payload.get("page")) {
                Ok(page) => page,
                Err(err) => return refuse(400, &err.to_string()),
            };
            let filter = match QueueFilter::parse(payload.get("filter")) {
                Ok(filter) => filter,
                Err(err) => return refuse(400, &err.to_string()),
            };
            match service.list(&branch, &kiosk_actor, filter, page, now).await {
                Ok(queue_page) => {
                    let names = usernames(
                        pool,
                        queue_page
                            .items
                            .iter()
                            .filter_map(|item| item.request.staff.as_deref()),
                    )
                    .await;
                    let branch_name: String =
                        sqlx::query_scalar("SELECT name FROM branches WHERE id = $1")
                            .bind(branch.as_str())
                            .fetch_optional(pool)
                            .await
                            .ok()
                            .flatten()
                            .unwrap_or_default();
                    let caller_id = match &kiosk_actor {
                        KioskActor::Staff { username, .. } => username.as_str(),
                        KioskActor::Customer => "",
                    };
                    let list_caller = dto_kiosk::ListCaller {
                        id: caller_id,
                        claims: claims_requests(policy_actor.as_ref()),
                    };
                    json_response(dto_kiosk::list_json(
                        &branch,
                        &queue_page,
                        &branch_name,
                        now,
                        &list_caller,
                        &|id| username_or_id(&names, id),
                    ))
                }
                Err(err) => handle_service_error(err),
            }
        }
        KioskAction::Accept => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("");
            match service.accept(&branch, &kiosk_actor, id, now).await {
                Ok(AcceptOutcome::Claimed(times)) => {
                    json_response(dto_kiosk::accept_claimed_json(&times))
                }
                Ok(AcceptOutcome::AlreadyMine(times)) => {
                    json_response(dto_kiosk::accept_mine_json(&times))
                }
                Ok(AcceptOutcome::HeldBy(holder)) => {
                    let names = usernames(pool, std::iter::once(holder.as_str())).await;
                    refuse(
                        400,
                        &format!(
                            "ออเดอร์นี้มี {} กำลังทำรายการแล้ว",
                            username_or_id(&names, &holder)
                        ),
                    )
                }
                Ok(AcceptOutcome::Unavailable) => refuse(400, "คิวนี้ไม่พร้อมรับเรื่อง กรุณาโหลดล่าสุด"),
                Err(err) => handle_service_error(err),
            }
        }
        KioskAction::IdentityChecked => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("");
            let confirmed = payload
                .get("confirmed")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            match service
                .identity_checked(&branch, &kiosk_actor, id, confirmed, now)
                .await
            {
                Ok(identity) => json_response(dto_kiosk::identity_json(&identity)),
                Err(err) => handle_service_error(err),
            }
        }
        KioskAction::Get => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("");
            match service.get(&branch, &kiosk_actor, id).await {
                Ok(kiosk_req) => {
                    let branch_name: String =
                        sqlx::query_scalar("SELECT name FROM branches WHERE id = $1")
                            .bind(branch.as_str())
                            .fetch_optional(pool)
                            .await
                            .ok()
                            .flatten()
                            .unwrap_or_default();
                    json_response(dto_kiosk::get_json(&kiosk_req, &branch_name))
                }
                Err(err) => handle_service_error(err),
            }
        }
        KioskAction::PublishReceipt => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("");
            let image = payload.get("image").and_then(Value::as_str).unwrap_or("");
            match service
                .publish_receipt(&branch, &kiosk_actor, id, image, now)
                .await
            {
                Ok(()) => saved_response(),
                Err(err) => handle_service_error(err),
            }
        }
        KioskAction::Manage => match service.manage(&branch, &kiosk_actor).await {
            Ok(switches) => {
                let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
                let rate_service = RateService::new(branch_tx, source.clone());
                let current_rates = match rate_service.current(&branch).await {
                    Ok(rates) => rates,
                    Err(err) => return handle_service_error(err),
                };
                json_response(dto_kiosk::manage_json(&switches, &current_rates))
            }
            Err(err) => handle_service_error(err),
        },
        KioskAction::Configure => {
            let code = payload
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let enabled = match payload.get("enabled").and_then(Value::as_bool) {
                Some(b) => b,
                None => return refuse(400, "Invalid currency settings."),
            };
            let parse_str = |v: Option<&Value>| match v {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Number(n)) => n.to_string(),
                _ => String::new(),
            };
            let buy = parse_str(payload.get("buy"));
            let sell = parse_str(payload.get("sell"));
            let previous_updated = payload
                .get("previousUpdated")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let partner_name = payload
                .get("partnerName")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let partner_cost = match payload.get("partnerCost") {
                Some(Value::String(s)) => Some(s.clone()),
                Some(Value::Number(n)) => Some(n.to_string()),
                _ => None,
            };

            let default_ids = RandomKioskIds;
            let ids: &dyn KioskIds = req
                .app_data::<web::Data<dyn KioskIds>>()
                .map(|d| d.as_ref())
                .unwrap_or(&default_ids);
            let request_id = ids.new_id();

            let cmd = ConfigureCurrency {
                code,
                enabled,
                buy,
                sell,
                previous_updated,
                partner_name,
                partner_cost,
                request_id,
            };

            match service.configure(&branch, &kiosk_actor, cmd, now).await {
                Ok(()) => saved_response(),
                Err(err) => handle_service_error(err),
            }
        }
    }
}
