//! Kiosk routes: POST /api/kiosk.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use actix_web::error::PayloadError;
use actix_web::http::header;
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::{DateTime, Utc};
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;
use xc_auth::domain::access::Role;
use xc_auth::domain::account::entities::session::SignedIn;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
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
use xc_rate::domain::publication::freshness::Freshness;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;

use xc_booking::infrastructure::lifecycle_factory::pg_booking_lifecycle;
use xc_kernel::queue_tickets::QueueTickets;
use xc_queue::application::issue::ticket_issuer::TicketIssuer;
use xc_queue::infrastructure::repository::PgQueueRepository;

use crate::application::compat::v2_json::V2Json;
use crate::bootstrap::AppState;
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::kiosk::kiosk_ports::{KioskAuditLog, PgKioskAuditLog, PgKioskPorts};
use crate::presentation::dto::kiosk as dto_kiosk;
use crate::presentation::dto::v3::common::RateFreshnessDto;
use crate::presentation::extractors::actor::{
    accounts, authorize, callers, identity_session, password_session, refuse, same_origin,
};
use crate::presentation::http::client_address::client_address;
use crate::presentation::http::rpc::RpcClock;
use crate::presentation::pages;
use crate::presentation::pages::Callers;

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

fn kiosk_actor_from_callers(callers: &Callers) -> KioskActor {
    if callers.owner_standing().is_some() {
        KioskActor::Staff {
            username: "owner".to_string(),
            role: KioskRole::Owner,
        }
    } else if let Some(staff) = callers.staff_standing() {
        let role = match staff.staff_role() {
            Some(Role::Teller) => KioskRole::Front,
            Some(Role::BranchManager) => KioskRole::Manager,
            Some(Role::RateManager) => KioskRole::Rates,
            _ => KioskRole::Other,
        };
        KioskActor::Staff {
            username: staff.user_id,
            role,
        }
    } else {
        KioskActor::Customer
    }
}

fn kiosk_server_error() -> HttpResponse {
    HttpResponse::InternalServerError()
        .insert_header((header::CACHE_CONTROL, "no-store"))
        .content_type("application/json")
        .body(json!({ "error": "เซิร์ฟเวอร์ขัดข้อง" }).to_string())
}

/// Whether the caller may claim a request: the policy's answer for `accept`.
fn claims_requests(
    actor: Option<&xc_auth::domain::access::Actor>,
    now: chrono::DateTime<Utc>,
) -> bool {
    Policy::authorize(actor, EntryPath::Kiosk, &Action::new("accept"), None, now).is_ok()
}

/// The usernames of the given holder ids, read in one query. A holder is stored by id;
/// `owner` is the owner's id.
pub(crate) async fn usernames<'a>(
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
        tracing::error!(error = %err, "kiosk usernames failed");
        Vec::new()
    });
    rows.into_iter().collect()
}

/// The username of a holder, or the id as it is when nobody has it.
pub(crate) fn username_or_id(names: &HashMap<String, String>, id: &str) -> String {
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

async fn publication_freshness(
    pool: &sqlx::PgPool,
    branch: &BranchId,
) -> Result<RateFreshnessDto, AppError> {
    let pub_row = sqlx::query(
        "SELECT rp.id, rp.published_at \
         FROM branches b \
         JOIN rate_publications rp ON rp.id = b.current_publication_id \
         WHERE b.id = $1",
    )
    .bind(branch.as_str())
    .fetch_optional(pool)
    .await
    .map_err(AppError::Database)?;

    let Some(r) = pub_row else {
        return Ok(RateFreshnessDto {
            stale: false,
            source_date: None,
            published_at: None,
        });
    };

    let pub_id: Uuid = r.get(0);
    let published_at: DateTime<Utc> = r.get(1);

    let raw_lines =
        sqlx::query("SELECT source_snapshot FROM published_rate_lines WHERE publication_id = $1 AND tier_lower_exclusive_thb IS NULL")
            .bind(pub_id)
            .fetch_all(pool)
            .await
            .map_err(AppError::Database)?;

    let now = Utc::now();
    let mut freshnesses = Vec::with_capacity(raw_lines.len() * 2);
    for row in raw_lines {
        let source_snapshot: Value = row.get(0);
        freshnesses.push(Freshness::of_side(&source_snapshot, Side::Buy, now));
        freshnesses.push(Freshness::of_side(&source_snapshot, Side::Sell, now));
    }

    let pub_freshness = Freshness::of_publication(freshnesses);
    Ok(RateFreshnessDto {
        stale: pub_freshness.stale,
        source_date: pub_freshness
            .source_date
            .map(|d| d.format("%Y-%m-%d").to_string()),
        published_at: Some(published_at.to_rfc3339()),
    })
}

fn handle_service_error(err: AppError) -> HttpResponse {
    match err {
        AppError::Domain(msg) => refuse(400, &msg),
        AppError::Unauthenticated => refuse(401, "EMPLOYEE_AUTH_REQUIRED"),
        other => {
            tracing::error!(error = %other, "kiosk service error");
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
    let who = match password_session(accounts_service, &req, now).await {
        Ok(w) => w,
        Err(err) => {
            tracing::error!(error = %err, "status failed");
            return kiosk_server_error();
        }
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

    let action_str = body_val
        .as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(Value::as_str)
        .unwrap_or("");

    let action_vo = Action::new(action_str);

    let callers_peek = callers(&req).await;
    let named_branch = match payload.get("branchId") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            if s.is_empty() {
                None
            } else {
                Some(BranchId::new(s))
            }
        }
        Some(Value::Number(n)) if n.as_i64() == Some(0) => None,
        _ => Some(BranchId::new("")),
    };

    // D-4 for a Blueprint session (`__Host-xc_session`): the named branch is authorised. Only a
    // password-session employee keeps v2's own-branch answer, as on `/api/rpc` (`rpc.rs`), which
    // the provided tests pin; `callers.staff` is set for both, so it cannot decide this alone.
    let blueprint_session = identity_session(&req, now).await.is_some();
    let named = if !blueprint_session && callers_peek.staff_standing().is_some() {
        None
    } else {
        named_branch
    };

    let (grant, callers) = match authorize(&req, EntryPath::Kiosk, &action_vo, named.as_ref()).await
    {
        Ok(pair) => pair,
        Err(refusal) => return refusal,
    };

    let branch = grant.branch;
    if !BranchId::is_well_formed(branch.as_str()) {
        return refuse(400, "Invalid branch.");
    }

    let kiosk_actor = kiosk_actor_from_callers(&callers);

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
            Ok(public_rates) => {
                let freshness = match publication_freshness(pool, &branch).await {
                    Ok(f) => f,
                    Err(err) => return handle_service_error(err),
                };
                match dto_kiosk::rates_json(&public_rates, &freshness) {
                    Ok(json) => json_response(json),
                    Err(err) => handle_service_error(err),
                }
            }
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
                    match dto_kiosk::quote_json(
                        &identified.request,
                        &identified.identity,
                        &branch_name,
                    ) {
                        Ok(json) => json_response(json),
                        Err(err) => handle_service_error(err),
                    }
                }
                Err(err) => handle_service_error(err),
            }
        }
        KioskAction::Confirm => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("");
            // §4.5 (QU-7 #1266): the confirm issues the request's walk-in ticket in its own
            // transaction, through the queue's port.
            let booking_events = pg_booking_lifecycle(pool.clone());
            let queue: Arc<dyn QueueTickets> = Arc::new(
                TicketIssuer::new(
                    BranchTx::new(Transactor::new(pool.clone())),
                    Arc::new(PgQueueRepository::new(pool.clone())),
                )
                .with_booking_events(booking_events),
            );
            match service.confirm_with_queue(&branch, id, now, queue).await {
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
                let emp_info = match who {
                    Some(SignedIn::Owner(owner)) => dto_kiosk::EmployeeInfo {
                        id: "owner".to_string(),
                        username: owner.username.clone(),
                        name: owner.name.clone(),
                        role: "OWNER".to_string(),
                    },
                    Some(SignedIn::Employee(emp)) => dto_kiosk::EmployeeInfo {
                        id: emp.id.clone(),
                        username: emp.username.clone(),
                        name: emp.name.clone(),
                        role: emp.role.clone(),
                    },
                    None => {
                        if callers.owner_standing().is_some() {
                            dto_kiosk::EmployeeInfo {
                                id: "owner".to_string(),
                                username: "owner".to_string(),
                                name: "Owner".to_string(),
                                role: "OWNER".to_string(),
                            }
                        } else if let Some(staff) = callers.staff_standing() {
                            dto_kiosk::EmployeeInfo {
                                id: staff.user_id.clone(),
                                username: staff.user_id.clone(),
                                name: staff.user_id,
                                role: staff.kind.audit_text().to_string(),
                            }
                        } else {
                            return kiosk_server_error();
                        }
                    }
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
                        claims: claims_requests(callers.user.as_ref(), now),
                    };
                    match dto_kiosk::list_json(
                        &branch,
                        &queue_page,
                        &branch_name,
                        now,
                        &list_caller,
                        &|id| username_or_id(&names, id),
                    ) {
                        Ok(json) => json_response(json),
                        Err(err) => handle_service_error(err),
                    }
                }
                Err(err) => handle_service_error(err),
            }
        }
        KioskAction::Accept => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("");
            // §4.5 (QU-6 #1354): accept starts the request's ticket as the accepting teller, through
            // the queue's port, in the same transaction; the queue's refusals keep it WAITING.
            let issuer = TicketIssuer::new(
                BranchTx::new(Transactor::new(pool.clone())),
                Arc::new(PgQueueRepository::new(pool.clone())),
            );
            let queue: Arc<dyn QueueTickets> = Arc::new(match callers.user.clone() {
                Some(user) => issuer.acting_as(user),
                None => issuer,
            });
            match service
                .accept_with_queue(&branch, &kiosk_actor, id, now, queue)
                .await
            {
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
                    match dto_kiosk::get_json(&kiosk_req, &branch_name) {
                        Ok(json) => json_response(json),
                        Err(err) => handle_service_error(err),
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use xc_auth::domain::access::{Actor, ScopedRole};

    /// A TELLER caller is `Front` at the kiosk (OR-7 R10 #3170).
    #[test]
    fn a_teller_is_front_at_the_kiosk() {
        let teller = ScopedRole::new(
            Role::Teller,
            Some(BranchId::new("B1")),
            Default::default(),
            None,
        );
        let user = Some(Actor::new("u1", "org", vec![teller], None));
        let callers = Callers {
            owner: None,
            staff: None,
            user: user.clone(),
            staff_user: user,
        };
        let role = KioskRole::Front;
        let username = "u1".to_string();
        assert_eq!(
            kiosk_actor_from_callers(&callers),
            KioskActor::Staff { username, role }
        );
    }
}
