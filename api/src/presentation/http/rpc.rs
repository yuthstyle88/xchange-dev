//! RPC routes: `POST /api/rpc` and `POST /api/staff/rpc`.
//!
//! The two doors of the unchanged v2 screens. This file reads who is calling
//! and what was sent, in the order v2 does on each route, and hands the rest to
//! `CompatRpc`:
//!
//! - `/api/rpc`: who is signed in (401, said before anything else) → the
//!   same-origin headers → the body → the name against the role → the payload;
//! - `/api/staff/rpc`: the same-origin headers → the body → the staff session
//!   (401, or the sentence about the first password) → the name against the
//!   role.
//!
//! The families of part two (`application/compat/actions/`) are built once, in
//! [`configure`], and read by a request as `web::Data<RpcActions>`.
//!
//! Every answer is JSON, a refusal `{"error": sentence}`; the body goes out as
//! `V2Json` prints it, keys in v2's order.

use std::sync::Arc;

use actix_web::error::PayloadError;
use actix_web::http::{StatusCode, header};
use actix_web::{HttpRequest, HttpResponse, web};
use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use sqlx::PgPool;
use xc_auth::domain::account::entities::session::SignedIn;
use xc_exchange::application::customer::service::CustomerService;
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_rate::application::branch::service::BranchService;
use xc_rate::application::rate::service::RateService;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use crate::application::compat::abandon_request::AbandonRequest;
use crate::application::compat::actions::RpcActions;
use crate::application::compat::actions::custody::CustodyAction;
use crate::application::compat::actions::documents::DocumentsAction;
use crate::application::compat::actions::save::SaveAction;
use crate::application::compat::envelope::{
    MAX_BODY_UTF16_UNITS, RpcRefusal, RpcRequest, RpcResponse, RpcRoute,
};
use crate::application::compat::rpc::{CompatRpc, RpcCaller, RpcFailure, RpcServices};
use crate::application::compat::v2_json::V2Json;
use crate::application::workflows::documents::DocumentsLookup;
use crate::application::workflows::save_transaction::SaveTransactionWorkflow;
use crate::bootstrap::AppState;
use crate::infrastructure::compat::snapshot_sources::ServiceSnapshotSources;
use crate::infrastructure::custody::custody_ports::PgCustodyPorts;
use crate::infrastructure::files::file_service::FileService;
use crate::presentation::extractors::actor::{
    Accounts, OWNER_COOKIE, STAFF_COOKIE, accounts, actor_of, cookie, same_origin, server_error,
    signed_in, staff_actor,
};
use crate::presentation::pages;

/// The most bytes a body of 120,000 UTF-16 units can be: three per unit, and a
/// byte-order mark that is not counted. Anything longer is over v2's limit
/// whatever it holds, so it is not read.
const MAX_BODY_BYTES: usize = MAX_BODY_UTF16_UNITS * 3 + 3;

/// The clock a request is stamped with: `syncedAt` of a snapshot and the time
/// of every write. Looked up as `web::Data<dyn RpcClock>`; when none is
/// registered the routes read the system clock.
pub trait RpcClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

/// Routes of this module; registered through the one route table.
pub fn configure(cfg: &mut web::ServiceConfig, state: &AppState) {
    cfg.app_data(web::Data::new(registered_actions(state)));
    for (path, route) in [
        ("/api/rpc", RpcRoute::Rpc),
        ("/api/staff/rpc", RpcRoute::StaffRpc),
    ] {
        cfg.service(
            web::resource(path)
                .app_data(web::PayloadConfig::new(MAX_BODY_BYTES))
                .route(web::post().to(
                    move |req: HttpRequest, body: Result<web::Bytes, actix_web::Error>| {
                        handle(req, body, route)
                    },
                ))
                .default_service(web::to(pages::fallback)),
        );
    }
}

async fn handle(
    req: HttpRequest,
    body: Result<web::Bytes, actix_web::Error>,
    route: RpcRoute,
) -> HttpResponse {
    match answer(&req, body, route).await {
        Ok(result) => HttpResponse::Ok()
            .insert_header((header::CACHE_CONTROL, "no-store"))
            .content_type("application/json")
            .body(result.to_text()),
        Err(RpcFailure::Refused(refusal)) => refuse(&refusal.response()),
        Err(RpcFailure::Failed(error)) => {
            tracing::error!(error = ?error, "rpc request failed");
            server_error()
        }
    }
}

fn refuse(response: &RpcResponse) -> HttpResponse {
    let status = StatusCode::from_u16(response.status).unwrap_or(StatusCode::BAD_REQUEST);
    let mut builder = HttpResponse::build(status);
    for (name, value) in &response.headers {
        builder.insert_header((*name, *value));
    }
    builder.body(response.body.clone())
}

fn not_wired(what: &str) -> RpcFailure {
    RpcFailure::Failed(AppError::Internal(format!("{what} is not registered")))
}

async fn answer(
    req: &HttpRequest,
    body: Result<web::Bytes, actix_web::Error>,
    route: RpcRoute,
) -> Result<V2Json, RpcFailure> {
    let accounts = accounts(req).ok_or_else(|| not_wired("the account service"))?;
    let state = req
        .app_data::<web::Data<AppState>>()
        .ok_or_else(|| not_wired("the application state"))?;
    let now = req
        .app_data::<web::Data<dyn RpcClock>>()
        .map_or_else(Utc::now, |clock| clock.now());

    let (request, caller) = match route {
        RpcRoute::Rpc => {
            let caller = caller_of_rpc(accounts, req, now)
                .await?
                .ok_or_else(|| RpcRefusal::not_signed_in(route))?;
            if !same_origin(req) {
                return Err(RpcRefusal::invalid_request().into());
            }
            let text = body_text(body, route)?;
            let staff = caller.actor.branch_id().is_some();
            (RpcRequest::parse(route, &text, staff)?, caller)
        }
        RpcRoute::StaffRpc => {
            if !same_origin(req) {
                return Err(RpcRefusal::invalid_request().into());
            }
            let text = body_text(body, route)?;
            let request = RpcRequest::parse(route, &text, true)?;
            (request, caller_of_staff_rpc(accounts, req, now).await?)
        }
    };

    // v2 judges the name before it looks at the payload
    CompatRpc::allow(route, request.action(), &caller)?;
    let (action, payload) = request.into_parts()?;

    let source = req
        .app_data::<web::Data<Arc<dyn RateSource>>>()
        .map_or_else(
            || Arc::new(BotRateSource::bot()) as Arc<dyn RateSource>,
            |source| source.get_ref().clone(),
        );
    let actions = req
        .app_data::<web::Data<RpcActions>>()
        .ok_or_else(|| not_wired("the rpc actions"))?;
    compat_rpc(&state.db, source, accounts.get_ref().clone())
        .with_actions(actions.get_ref().clone())
        .dispatch(route, &action, payload, &caller, now)
        .await
}

/// The text of the body as v2 reads it (`request.text()`): bytes that are not
/// UTF-8 become U+FFFD and a leading byte-order mark is dropped. A body too
/// long to be within the limit is refused in the route's own sentence.
fn body_text(
    body: Result<web::Bytes, actix_web::Error>,
    route: RpcRoute,
) -> Result<String, RpcRefusal> {
    let bytes = body.map_err(|error| match error.as_error::<PayloadError>() {
        Some(PayloadError::Overflow) => RpcRefusal::too_large(route),
        _ => RpcRefusal::invalid_request(),
    })?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
}

fn caller_of(who: &SignedIn) -> Option<RpcCaller> {
    let actor = actor_of(who)?;
    let (username, name) = match who {
        SignedIn::Owner(owner) => (owner.username.clone(), owner.name.clone()),
        SignedIn::Employee(employee) => (employee.username.clone(), employee.name.clone()),
    };
    Some(RpcCaller {
        actor,
        username,
        name,
    })
}

/// Who `/api/rpc` is answering: the owner session first, else the staff
/// session. An employee whose first password is unchanged is nobody here.
async fn caller_of_rpc(
    accounts: &Accounts,
    req: &HttpRequest,
    now: DateTime<Utc>,
) -> AppResult<Option<RpcCaller>> {
    let who = signed_in(
        accounts,
        &cookie(req, OWNER_COOKIE),
        &cookie(req, STAFF_COOKIE),
        now.into(),
    )
    .await?;
    Ok(who.as_ref().and_then(caller_of))
}

/// Who `/api/staff/rpc` is answering: the staff session only — an owner
/// session counts for nothing — and only once the first password is changed.
async fn caller_of_staff_rpc(
    accounts: &Accounts,
    req: &HttpRequest,
    now: DateTime<Utc>,
) -> Result<RpcCaller, RpcFailure> {
    let route = RpcRoute::StaffRpc;
    let employee = match accounts
        .employee_session(&cookie(req, STAFF_COOKIE), true, now)
        .await
    {
        Ok(employee) => employee,
        Err(AppError::Unauthenticated) => return Err(RpcRefusal::not_signed_in(route).into()),
        Err(other) => return Err(other.into()),
    };
    if employee.must_change {
        return Err(RpcRefusal::first_password_unchanged(route).into());
    }
    let actor = staff_actor(&employee).ok_or_else(|| RpcRefusal::not_signed_in(route))?;
    Ok(RpcCaller {
        actor,
        username: employee.username,
        name: employee.name,
    })
}

fn shift_service(pool: &PgPool, branch_tx: &BranchTx) -> ShiftService {
    ShiftService::new(
        Arc::new(PgShiftRepository::new(pool.clone())),
        branch_tx.clone(),
    )
}

fn custody_service(pool: &PgPool, shifts: &ShiftService, branch_tx: &BranchTx) -> CustodyService {
    CustodyService::new(
        Arc::new(PgCustodyRepository::new(pool.clone())),
        shifts.clone(),
        Arc::new(PgCustodyPorts),
        branch_tx.clone(),
    )
}

fn transaction_service(pool: &PgPool, branch_tx: &BranchTx) -> TransactionService {
    TransactionService::new(
        Arc::new(PgTransactionRepository::new(pool.clone())),
        branch_tx.clone(),
    )
}

/// The families of part two the server registers, in the order the dispatcher
/// asks them: documents, custody, the save. Each is built here once, from the
/// parts its own file needs; wiring a family does not change this function.
pub fn registered_actions(state: &AppState) -> RpcActions {
    let pool = &state.db;
    let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
    let files = FileService::new(pool.clone(), state.settings.files.dir.clone());
    let shifts = shift_service(pool, &branch_tx);
    let custody = custody_service(pool, &shifts, &branch_tx);
    let documents = DocumentsLookup::new(
        transaction_service(pool, &branch_tx),
        BranchService::new(branch_tx.clone()),
        files.clone(),
    );
    let save = SaveTransactionWorkflow::new(branch_tx, shifts, files);
    vec![
        Arc::new(DocumentsAction::new(documents)),
        Arc::new(CustodyAction::new(custody.clone())),
        Arc::new(SaveAction::new(save, custody)),
    ]
}

/// The dispatcher over the services of every context it calls.
fn compat_rpc(pool: &PgPool, source: Arc<dyn RateSource>, accounts: Accounts) -> CompatRpc {
    let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
    let shifts = shift_service(pool, &branch_tx);
    let custody = custody_service(pool, &shifts, &branch_tx);
    let transactions = transaction_service(pool, &branch_tx);
    let sources = Arc::new(ServiceSnapshotSources::new(transactions, accounts.clone()));
    CompatRpc::new(
        RpcServices {
            branches: BranchService::new(branch_tx.clone()),
            rates: RateService::new(branch_tx.clone(), source),
            customers: CustomerService::new(branch_tx.clone()),
            shifts,
            custody,
            abandon: AbandonRequest::new(branch_tx),
            accounts,
        },
        sources,
    )
}
