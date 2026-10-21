//! The cash commands (CA-8 #624; design `cash-shift-transfer.md` §4.4, §4.9): counts, hand-overs and transfers,
//! each a `CommandAction` on `EntryPath::Rpc` whose area row is in `xc-auth/.../policy/areas/cash.rs` (G2, G2a).
//! Each calls the xc-cash service's CA-7c `_in` form on the seam's transaction, so the posting, the state change and
//! the idempotency row commit together.
//!
//! This file is the flat registration file `build.rs` reads (`build.rs:249-251` reads only files under `commands/`;
//! lane controller's ruling A on #624). The count commands and the shared helpers are here; the hand-over and
//! transfer commands live in `commands/cash/`. Every file declares `INHERITS` on a `CommandAction` (`or4_routes_guard_test`).

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::{DateTime, Utc};
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_booking::infrastructure::lifecycle_factory::pg_booking_lifecycle;
use xc_cash::application::count::count_service::CountVerified;
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::error::CashError;
use xc_cash::domain::handover::HandoverError;
use xc_cash::domain::transfer::cash_transfer::TransferError;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, NumberError};
use xc_kernel::shift_hook::{ShiftLifecycleHook, ShiftLifecycleHooks};
use xc_queue::application::counter::counter_service::CounterService;
use xc_queue::application::day::queue_day_hook::QueueDayHook;
use xc_queue::application::status::status_service::StatusService;
use xc_queue::domain::repository::QueueRepository;
use xc_queue::infrastructure::repository::PgQueueRepository;

use crate::bootstrap::AppState;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::dto::v3::cash::{
    CashLineInput, CashReceivedLineInput, CashRecordInput, CashRecordOutput, CountSubmitInput,
    CountSubmitOutput, CountVerifyOutput, HandoverStartInput, TransferCreateInput,
    TransferReceiveInput, TransferReconcileInput,
};
use crate::presentation::dto::v3::cash_transfer_policy::{
    CashTransferPolicyData, TransferPolicySaveInput,
};
use crate::presentation::http::v3::command::{CommandAction, CommandEndpoint, server_error};
use crate::presentation::http::v3::error::V3Error;

#[path = "cash/handovers.rs"]
mod handovers;
#[path = "cash/transfers.rs"]
mod transfers;

pub(crate) fn validation_failed(field: &'static str, message: impl Into<String>) -> V3Error {
    V3Error::new(
        "validationFailed",
        Some(field),
        message.into(),
        StatusCode::BAD_REQUEST,
    )
}

/// `user_roles`: role code, branch, valid from, valid to.
type RoleRow = (String, Option<String>, DateTime<Utc>, Option<DateTime<Utc>>);

/// The caller the seam authorized, as the Blueprint actor the cash services check: their organisation and their
/// roles valid at `now`, read on the seam's transaction. The seam's actor id is the caller's `users.id`.
pub(crate) async fn caller_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    who: &User,
    now: DateTime<Utc>,
) -> Result<User, V3Error> {
    let id = who.user_id.clone();
    let org: Option<String> =
        sqlx::query_scalar("SELECT organization_id::text FROM users WHERE id::text = $1")
            .bind(&id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "failed to read the caller's organisation");
                server_error()
            })?;
    let Some(org) = org else {
        return Err(V3Error::new(
            "roleNotAllowedHere",
            None,
            "บัญชีนี้ไม่มีสิทธิ์ทำรายการเงินสด",
            StatusCode::FORBIDDEN,
        ));
    };
    let rows: Vec<RoleRow> = sqlx::query_as(
        "SELECT role_code, branch_id, valid_from, valid_to FROM user_roles \
         WHERE user_id::text = $1 AND valid_from <= $2 AND (valid_to IS NULL OR valid_to > $2) \
         ORDER BY valid_from ASC, id ASC",
    )
    .bind(&id)
    .bind(now)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "failed to read the caller's roles");
        server_error()
    })?;
    let roles = rows
        .into_iter()
        .filter_map(|(code, branch, from, to)| {
            Some(ScopedRole::new(
                Role::parse(&code)?,
                branch.map(BranchId::new),
                from,
                to,
            ))
        })
        .collect();
    Ok(User::new(&id, &org, roles, None))
}

/// G1: a quantity is a decimal string of a whole number ≥ 0 (`"0"` included).
pub(crate) fn whole_number(field: &'static str, text: &str) -> Result<i32, V3Error> {
    let t = text.trim();
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
        return Err(validation_failed(
            field,
            format!("{field} must be a whole number ≥ 0"),
        ));
    }
    t.parse::<i32>()
        .map_err(|_| validation_failed(field, format!("{field} is too large")))
}

/// An xc-cash service error on the wire. `Forbidden` (a transfer received at a branch that is not its destination,
/// G9) and the receiver rule are `outOfScope` / 403; a hand-over that is no longer PENDING is `staleVersion` / 409
/// (Q-B); a move beyond the stock is `stockInsufficient` / 409; every other refusal is `validationFailed`.
pub(crate) fn from_cash_error(e: AppError) -> V3Error {
    match e {
        AppError::Forbidden => V3Error::new(
            "outOfScope",
            None,
            "รายการนี้ไม่อยู่ในสาขาที่เลือก",
            StatusCode::FORBIDDEN,
        ),
        AppError::Domain(message) => {
            if message == CashError::NotReceiver.to_string() {
                V3Error::new("outOfScope", None, message, StatusCode::FORBIDDEN)
            } else if message == HandoverError::NotPending.to_string() {
                V3Error::new("staleVersion", None, message, StatusCode::CONFLICT)
            } else if message.starts_with("stockInsufficient")
                || message == TransferError::StockInsufficient.to_string()
            {
                V3Error::new("stockInsufficient", None, message, StatusCode::CONFLICT)
            } else {
                V3Error::new("validationFailed", None, message, StatusCode::BAD_REQUEST)
            }
        }
        AppError::Validation(message) => {
            V3Error::new("validationFailed", None, message, StatusCode::BAD_REQUEST)
        }
        AppError::NotFound { entity, id } => V3Error::new(
            "notFound",
            None,
            format!("{entity} {id} not found"),
            StatusCode::NOT_FOUND,
        ),
        AppError::Conflict(message) => {
            V3Error::new("staleVersion", None, message, StatusCode::CONFLICT)
        }
        other => {
            tracing::error!(error = %other, "cash command failed");
            server_error()
        }
    }
}

/// The lines of a count or a hand-over, with each `qty` parsed as a whole number ≥ 0 (G1).
pub(crate) fn count_lines(lines: &[CashLineInput]) -> Result<Vec<CountLine>, V3Error> {
    lines
        .iter()
        .map(|l| {
            Ok(CountLine::new(
                &l.currency,
                l.denomination_id,
                whole_number("qty", &l.qty)?,
            ))
        })
        .collect()
}

pub(crate) fn shift_lifecycle_hook(pool: &PgPool) -> Arc<dyn ShiftLifecycleHook> {
    let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
    let queue: Arc<dyn QueueRepository> = Arc::new(PgQueueRepository::new(pool.clone()));
    let status = StatusService::new(branch_tx.clone(), queue.clone());
    let counters: Arc<dyn ShiftLifecycleHook> = Arc::new(CounterService::new(
        branch_tx,
        queue.clone(),
        status.clone(),
    ));
    let day: Arc<dyn ShiftLifecycleHook> = Arc::new(
        QueueDayHook::new(queue, status).with_booking_events(pg_booking_lifecycle(pool.clone())),
    );
    Arc::new(ShiftLifecycleHooks::new(vec![counters, day]))
}

pub struct CountSubmit {
    counts: CashCountService,
}

impl CountSubmit {
    pub fn new(pool: PgPool) -> Self {
        let hook = shift_lifecycle_hook(&pool);
        Self {
            counts: CashCountService::pg(pool).with_shift_lifecycle_hook(hook),
        }
    }
}

impl CommandAction for CountSubmit {
    const NAME: &'static str = "countSubmit";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "countSubmit");
    type Input = CountSubmitInput;
    type Output = CountSubmitOutput;
    const ROUTE: &'static str = "/api/cash-counts";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &User,
        input: Self::Input,
        key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let kind = CountKind::parse(&input.kind)
            .ok_or_else(|| validation_failed("kind", "unknown count kind"))?;
        let lines = count_lines(&input.lines)?;
        let now = Utc::now();
        let user = caller_in(tx, who, now).await?;
        let count = self
            .counts
            .submit_in(
                tx,
                &user,
                SubmitCount {
                    branch: branch.clone(),
                    shift_id: input.shift_id,
                    kind,
                    lines,
                    note: input.note,
                    idempotency_key: key.as_str().to_owned(),
                },
                now,
            )
            .await
            .map_err(from_cash_error)?;
        Ok(CountSubmitOutput { id: count.id })
    }
}

pub struct CountVerify {
    counts: CashCountService,
}

impl CountVerify {
    pub fn new(pool: PgPool) -> Self {
        let hook = shift_lifecycle_hook(&pool);
        Self {
            counts: CashCountService::pg(pool).with_shift_lifecycle_hook(hook),
        }
    }
}

impl CommandAction for CountVerify {
    const NAME: &'static str = "countVerify";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "countVerify");
    type Input = CashRecordInput;
    type Output = CountVerifyOutput;
    const ROUTE: &'static str = "/api/cash-counts/{id}/verify";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &User,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let now = Utc::now();
        let user = caller_in(tx, who, now).await?;
        // §4.9: awaiting FINANCE is an answer, not an error, so its approval request commits with the seam
        match self
            .counts
            .verify_in(tx, &user, &input.id, now)
            .await
            .map_err(from_cash_error)?
        {
            CountVerified::Verified(count) => Ok(CountVerifyOutput {
                id: count.id,
                status: "VERIFIED".to_owned(),
                approval_request_id: None,
            }),
            CountVerified::AwaitingFinance(request) => Ok(CountVerifyOutput {
                id: input.id,
                status: "AWAITING_FINANCE".to_owned(),
                approval_request_id: Some(request),
            }),
        }
    }
}

pub struct TransferPolicySave;

impl TransferPolicySave {
    pub fn new(_pool: PgPool) -> Self {
        Self
    }
}

impl CommandAction for TransferPolicySave {
    const NAME: &'static str = "transferPolicySave";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "transferPolicySave");
    type Input = TransferPolicySaveInput;
    type Output = CashTransferPolicyData;
    const ROUTE: &'static str = "/api/cash-transfer-policy";
    const METHOD: Method = Method::PUT;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &User,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let org_uuid = Uuid::parse_str(&who.organization_id).map_err(|_| server_error())?;
        let user_uuid = Uuid::parse_str(&who.user_id).map_err(|_| server_error())?;

        for (curr, amount_str) in &input.limits {
            let code = CurrencyCode::parse(curr)
                .map_err(|_| validation_failed("currency", "unknown currency"))?;

            Money::parse(amount_str).map_err(|e| match e {
                NumberError::TooManyDecimals => {
                    validation_failed("limits", CashError::TooManyDecimals.to_string())
                }
                NumberError::Invalid => validation_failed("limits", e.to_string()),
            })?;

            let minor_unit: Option<i16> =
                sqlx::query_scalar("SELECT minor_unit FROM currencies WHERE code = $1")
                    .bind(code.as_str())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|_| server_error())?;

            let minor = match minor_unit {
                Some(m) => m,
                None => return Err(validation_failed("currency", "unknown currency")),
            };

            let dec = Decimal::from_str(amount_str)
                .map_err(|e| validation_failed("limits", e.to_string()))?;

            if dec < Decimal::ZERO {
                return Err(validation_failed("limits", "amount must be non-negative"));
            }

            if dec.scale() > (minor as u32) {
                return Err(validation_failed(
                    "limits",
                    CashError::TooManyDecimals.to_string(),
                ));
            }
        }

        let version = Uuid::new_v4().to_string();
        let limits_val = serde_json::to_value(&input.limits).map_err(|_| server_error())?;

        let created_at: DateTime<Utc> = sqlx::query_scalar(
            "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by, created_at) \
             VALUES ($1, $2, $3, $4, now()) \
             RETURNING created_at",
        )
        .bind(&version)
        .bind(org_uuid)
        .bind(limits_val)
        .bind(user_uuid)
        .fetch_one(&mut **tx)
        .await
        .map_err(|_| server_error())?;

        Ok(CashTransferPolicyData {
            version,
            limits: input.limits,
            valid: true,
            invalid_entries: Vec::new(),
            set_by: who.user_id.clone(),
            created_at: created_at.to_rfc3339(),
        })
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let pool = state.db.clone();
    CommandEndpoint::register(cfg, Arc::new(CountSubmit::new(pool.clone())));
    CommandEndpoint::register(cfg, Arc::new(CountVerify::new(pool.clone())));
    CommandEndpoint::register(cfg, Arc::new(handovers::HandoverStart::new(pool.clone())));
    CommandEndpoint::register(cfg, Arc::new(handovers::HandoverAccept::new(pool.clone())));
    CommandEndpoint::register(cfg, Arc::new(handovers::HandoverReject::new(pool.clone())));
    CommandEndpoint::register(cfg, Arc::new(transfers::TransferCreate::new(pool.clone())));
    CommandEndpoint::register(cfg, Arc::new(transfers::TransferSubmit::new(pool.clone())));
    CommandEndpoint::register(
        cfg,
        Arc::new(transfers::TransferDispatch::new(pool.clone())),
    );
    CommandEndpoint::register(cfg, Arc::new(transfers::TransferReceive::new(pool.clone())));
    CommandEndpoint::register(
        cfg,
        Arc::new(transfers::TransferReconcile::new(pool.clone())),
    );
    CommandEndpoint::register(cfg, Arc::new(TransferPolicySave::new(pool)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    CashLineInput::export_all(&cfg)?;
    CashReceivedLineInput::export_all(&cfg)?;
    CountSubmitInput::export_all(&cfg)?;
    CashRecordInput::export_all(&cfg)?;
    HandoverStartInput::export_all(&cfg)?;
    TransferCreateInput::export_all(&cfg)?;
    TransferReceiveInput::export_all(&cfg)?;
    TransferReconcileInput::export_all(&cfg)?;
    CountSubmitOutput::export_all(&cfg)?;
    CountVerifyOutput::export_all(&cfg)?;
    CashRecordOutput::export_all(&cfg)?;
    TransferPolicySaveInput::export_all(&cfg)?;
    Ok(())
}

// Written before the implementation by the ticket author claude/5edf7bb5. Do not edit, delete or weaken.
#[cfg(test)]
mod stock_insufficient_mapping_tests {
    use super::*;

    #[test]
    fn a_ledger_stock_refusal_is_a_409_stock_insufficient() {
        let e = from_cash_error(AppError::Domain(
            "stockInsufficient: the posting takes a drawer, a safe or a position below zero".into(),
        ));
        assert_eq!(e.code, "stockInsufficient");
        assert_eq!(e.status, StatusCode::CONFLICT);
    }

    #[test]
    fn a_transfer_stock_refusal_stays_a_409_stock_insufficient() {
        let e = from_cash_error(AppError::Domain(
            TransferError::StockInsufficient.to_string(),
        ));
        assert_eq!(e.code, "stockInsufficient");
        assert_eq!(e.status, StatusCode::CONFLICT);
    }
}
