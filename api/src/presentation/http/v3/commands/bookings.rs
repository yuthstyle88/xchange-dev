//! Booking commands runner and registration (§6, §4.4 BK-6 #1388).

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::{HttpRequest, HttpResponse, ResponseError, web};
use chrono::{NaiveDate, Utc};
use pos_infra::transactions::Transactor;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;

use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_booking::application::booking_service::BookingService;
use xc_booking::application::check_in::CheckInService;
use xc_booking::application::lifecycle::BookingLifecycle;
use xc_booking::application::slot_service::SlotService;
use xc_booking::infrastructure::booking_repository::PgBookingRepository;
use xc_booking::infrastructure::slot_repository::PgSlotRepository;
use xc_customer::application::customer::contact_reserve::ContactReserve;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_queue::application::issue::ticket_issuer::TicketIssuer;
use xc_queue::infrastructure::repository::PgQueueRepository;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::pg_store::PgIdempotencyStore;
use crate::infrastructure::idempotency::store::{Claim, IdempotencyKey, IdempotencyStore};
use crate::presentation::http::v3::command::{server_error, CommandAction};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::pages;

#[path = "bookings/cancel.rs"]
pub mod cancel;
#[path = "bookings/check_in.rs"]
pub mod check_in;
#[path = "bookings/closure_add.rs"]
pub mod closure_add;
#[path = "bookings/confirm.rs"]
pub mod confirm;
#[path = "bookings/create.rs"]
pub mod create;
#[path = "bookings/kiosk_check_in.rs"]
pub mod kiosk_check_in;
#[path = "bookings/policy.rs"]
pub mod policy;
#[path = "bookings/qr.rs"]
pub mod qr;
#[path = "bookings/reschedule.rs"]
pub mod reschedule;
#[path = "bookings/slot_capacity.rs"]
pub mod slot_capacity;
#[path = "bookings/slot_close.rs"]
pub mod slot_close;
#[path = "bookings/slot_open.rs"]
pub mod slot_open;
#[path = "bookings/slot_rule.rs"]
pub mod slot_rule;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingOkOutput {
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct ClosureRemoveInput {
    pub id: String,
    #[ts(type = "string")]
    pub date: NaiveDate,
}

pub struct ClosureRemove;

impl CommandAction for ClosureRemove {
    const NAME: &'static str = "closureRemove";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "closureRemove");
    type Input = ClosureRemoveInput;
    type Output = BookingOkOutput;
    const ROUTE: &'static str = "/api/branches/{id}/closures/{date}/remove";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        _who: &Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        Err(server_error())
    }
}

fn register_closure_remove(cfg: &mut web::ServiceConfig, services: web::Data<BookingServices>) {
    cfg.route(
        ClosureRemove::ROUTE,
        web::post().to(move |req: HttpRequest, payload: web::Payload| {
            let services = services.clone();
            async move {
                handle_booking_command(
                    req,
                    payload,
                    services,
                    ClosureRemove::NAME,
                    ClosureRemove::INHERITS,
                    false,
                    |_, value, user, services, tx| {
                        Box::pin(async move {
                            let input: ClosureRemoveInput =
                                serde_json::from_value(value).map_err(|_| {
                                    V3Error::new(
                                        "invalidRequest",
                                        None,
                                        "คำขอไม่ถูกต้อง",
                                        StatusCode::BAD_REQUEST,
                                    )
                                })?;

                            let _ = user.ok_or_else(|| V3Error::from(PolicyError::SignIn))?;

                            let branch = BranchId::new(&input.id);
                            BranchTx::lock_also(tx, &branch).await.map_err(|e| {
                                V3Error::new(
                                    "serverError",
                                    None,
                                    e.to_string(),
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                )
                            })?;

                            services
                                .slot_service
                                .remove_closure_in(tx, &branch, input.date)
                                .await
                                .map_err(|e| {
                                    V3Error::new(
                                        "badRequest",
                                        None,
                                        e.to_string(),
                                        StatusCode::BAD_REQUEST,
                                    )
                                })?;

                            let resp = serde_json::to_value(BookingOkOutput { ok: true })
                                .map_err(|_| server_error())?;
                            Ok(BookingCommandResult::identical(resp))
                        })
                    },
                )
                .await
            }
        }),
    );
}

#[derive(Clone)]
pub struct BookingServices {
    pub pool: PgPool,
    pub branch_tx: BranchTx,
    pub booking_service: Arc<BookingService>,
    pub check_in_service: Arc<CheckInService>,
    pub slot_service: Arc<SlotService>,
}

impl BookingServices {
    pub fn new(pool: PgPool) -> Self {
        let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
        let booking_repo = Arc::new(PgBookingRepository::new(pool.clone()));
        let slot_repo = Arc::new(PgSlotRepository::new(pool.clone()));
        let queue_repo = Arc::new(PgQueueRepository::new(pool.clone()));

        let booking_events: Arc<dyn xc_kernel::booking_queue_events::BookingQueueEvents> =
            Arc::new(BookingLifecycle::new(booking_repo.clone()).with_slots(slot_repo.clone()));

        let issuer = Arc::new(
            TicketIssuer::new(branch_tx.clone(), queue_repo).with_booking_events(booking_events),
        );

        let pii_keys1 =
            xc_kernel::pii::PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
                .unwrap();

        let pii_keys2 =
            xc_kernel::pii::PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=")
                .unwrap();

        let contact_reserve = Arc::new(ContactReserve::new(branch_tx.clone()));

        let booking_service = Arc::new(
            BookingService::new(
                pool.clone(),
                booking_repo.clone(),
                slot_repo.clone(),
                contact_reserve,
                None,
                pii_keys1,
            )
            .with_queue(issuer.clone()),
        );

        let check_in_service = Arc::new(CheckInService::new(
            pool.clone(),
            branch_tx.clone(),
            booking_repo,
            slot_repo.clone(),
            issuer,
            pii_keys2,
        ));

        let slot_service = Arc::new(SlotService::new(pool.clone(), slot_repo));

        Self {
            pool,
            branch_tx,
            booking_service,
            check_in_service,
            slot_service,
        }
    }
}

pub struct BookingCommandResult {
    pub response_val: Value,
    pub replay_val: Value,
}

impl BookingCommandResult {
    pub fn identical(val: Value) -> Self {
        Self {
            response_val: val.clone(),
            replay_val: val,
        }
    }
}

pub async fn handle_booking_command<F>(
    req: HttpRequest,
    payload: web::Payload,
    services: web::Data<BookingServices>,
    op_name: &'static str,
    inherits: (EntryPath, &'static str),
    is_public: bool,
    f: F,
) -> HttpResponse
where
    F: for<'a> FnOnce(
        HttpRequest,
        Value,
        Option<Arc<Actor>>,
        web::Data<BookingServices>,
        &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<BookingCommandResult, V3Error>> + Send + 'a>,
    >,
{
    let callers = pages::get_callers(&req).await;
    let user = callers.user.map(Arc::new);
    let now = Utc::now();
    let action = Action::new(inherits.1);

    if !is_public {
        match &user {
            Some(u) => {
                if let Err(e) = Policy::authorize(Some(u.as_ref()), inherits.0, &action, None, now)
                {
                    return V3Error::from(e).error_response();
                }
            }
            None => {
                return V3Error::from(PolicyError::SignIn).error_response();
            }
        }
    }

    let raw_key = match req.headers().get("idempotency-key") {
        Some(val) => match val.to_str() {
            Ok(s) if !s.trim().is_empty() => s.to_string(),
            _ => {
                return V3Error::new(
                    "validationFailed",
                    Some("idempotencyKey"),
                    "Idempotency-Key is required",
                    StatusCode::BAD_REQUEST,
                )
                .error_response();
            }
        },
        None => {
            return V3Error::new(
                "validationFailed",
                Some("idempotencyKey"),
                "Idempotency-Key is required",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };
    let idem_key = IdempotencyKey::new(raw_key.clone());

    const MAX_BODY: usize = 65_536;
    let bytes = match payload.to_bytes_limited(MAX_BODY).await {
        Ok(Ok(b)) => b,
        _ => {
            return V3Error::new(
                "invalidRequest",
                None,
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };

    let mut value: Value = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(_) => {
                return V3Error::new(
                    "invalidRequest",
                    None,
                    "คำขอไม่ถูกต้อง",
                    StatusCode::BAD_REQUEST,
                )
                .error_response();
            }
        }
    };

    let obj = match value.as_object_mut() {
        Some(o) => o,
        None => {
            return V3Error::new(
                "invalidRequest",
                None,
                "คำขอไม่ถูกต้อง",
                StatusCode::BAD_REQUEST,
            )
            .error_response();
        }
    };

    for (name, val) in req.match_info().iter() {
        obj.insert(name.to_string(), Value::String(val.to_string()));
    }

    let branch_str = obj
        .get("branchId")
        .or_else(|| obj.get("branch"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&value).unwrap()));

    let org_id = if let Some(u) = &user {
        u.organization_id.to_string()
    } else {
        match sqlx::query_scalar::<_, Uuid>("SELECT organization_id FROM branches WHERE id = $1")
            .bind(&branch_str)
            .fetch_optional(&services.pool)
            .await
        {
            Ok(Some(org)) => org.to_string(),
            _ => {
                return V3Error::new(
                    "outOfScope",
                    None,
                    "BRANCH_NOT_FOUND",
                    StatusCode::FORBIDDEN,
                )
                .error_response();
            }
        }
    };

    let store = Arc::new(PgIdempotencyStore::new(services.pool.clone(), org_id));
    let claim = match store.claim(op_name, &idem_key, &hash).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "idempotency claim failed");
            return V3Error::new(
                "serverError",
                None,
                "INTERNAL_SERVER_ERROR",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response();
        }
    };

    let claim_token = match claim {
        Claim::Conflict => {
            return V3Error::new(
                "idempotencyConflict",
                None,
                "IDEMPOTENCY_CONFLICT",
                StatusCode::CONFLICT,
            )
            .error_response();
        }
        Claim::Replay(answer) => {
            let mut resp = HttpResponse::Ok();
            resp.insert_header((actix_web::http::header::CONTENT_TYPE, "application/json"))
                .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"));
            return resp.json(answer);
        }
        Claim::New(token) => token,
    };

    let mut tx = match services.pool.begin().await {
        Ok(tx) => tx,
        Err(_) => {
            return V3Error::new(
                "serverError",
                None,
                "INTERNAL_SERVER_ERROR",
                StatusCode::INTERNAL_SERVER_ERROR,
            )
            .error_response();
        }
    };

    let result = f(req.clone(), value.clone(), user, services.clone(), &mut tx).await;

    match result {
        Ok(cmd_result) => {
            if let Err(e) = store
                .finish_in(
                    &mut tx,
                    op_name,
                    &idem_key,
                    &claim_token,
                    cmd_result.replay_val,
                )
                .await
            {
                tracing::error!(error = %e, "idempotency finish_in failed");
                return V3Error::new(
                    "serverError",
                    None,
                    "INTERNAL_SERVER_ERROR",
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
                .error_response();
            }

            if let Err(e) = tx.commit().await {
                tracing::error!(error = %e, "commit failed");
                return V3Error::new(
                    "serverError",
                    None,
                    "INTERNAL_SERVER_ERROR",
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
                .error_response();
            }

            let mut resp = HttpResponse::Ok();
            resp.insert_header((actix_web::http::header::CONTENT_TYPE, "application/json"))
                .insert_header((actix_web::http::header::CACHE_CONTROL, "no-store"));
            resp.json(cmd_result.response_val)
        }
        Err(v3_err) => v3_err.error_response(),
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let services = web::Data::new(BookingServices::new(state.db.clone()));
    cfg.app_data(services.clone());

    create::register(cfg, services.clone());
    confirm::register(cfg, services.clone());
    cancel::register(cfg, services.clone());
    reschedule::register(cfg, services.clone());
    qr::register(cfg, services.clone());
    check_in::register(cfg, services.clone());
    kiosk_check_in::register(cfg, services.clone());

    slot_rule::register(cfg, services.clone());
    closure_add::register(cfg, services.clone());
    register_closure_remove(cfg, services.clone());
    slot_capacity::register(cfg, services.clone());
    slot_close::register(cfg, services.clone());
    slot_open::register(cfg, services.clone());
    policy::register(cfg, services);
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    create::BookingCreateInput::export_all(&cfg)?;
    create::BookingCreateOutput::export_all(&cfg)?;
    confirm::BookingConfirmInput::export_all(&cfg)?;
    confirm::BookingConfirmOutput::export_all(&cfg)?;
    cancel::BookingCancelInput::export_all(&cfg)?;
    cancel::BookingCancelOutput::export_all(&cfg)?;
    reschedule::BookingRescheduleInput::export_all(&cfg)?;
    reschedule::BookingRescheduleOutput::export_all(&cfg)?;
    qr::BookingQrReissueInput::export_all(&cfg)?;
    qr::BookingQrReissueOutput::export_all(&cfg)?;
    check_in::BookingCheckInInput::export_all(&cfg)?;
    check_in::BookingCheckInOutput::export_all(&cfg)?;
    kiosk_check_in::KioskBookingCheckInInput::export_all(&cfg)?;
    kiosk_check_in::KioskBookingCheckInOutput::export_all(&cfg)?;
    slot_rule::SlotRuleSetInput::export_all(&cfg)?;
    closure_add::ClosureAddInput::export_all(&cfg)?;
    ClosureRemoveInput::export_all(&cfg)?;
    slot_capacity::SlotCapacitySetInput::export_all(&cfg)?;
    slot_capacity::SlotCapacitySetOutput::export_all(&cfg)?;
    slot_close::SlotCloseInput::export_all(&cfg)?;
    slot_close::SlotCloseOutput::export_all(&cfg)?;
    slot_open::SlotOpenInput::export_all(&cfg)?;
    slot_open::SlotOpenOutput::export_all(&cfg)?;
    policy::BookingPolicySetInput::export_all(&cfg)?;
    BookingOkOutput::export_all(&cfg)?;
    Ok(())
}
