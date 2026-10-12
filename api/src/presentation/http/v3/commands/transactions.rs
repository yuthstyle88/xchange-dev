//! TX-4 #672: the transaction commands (design `quote-transaction.md` §4.2): `POST
//! /api/transactions` (a DRAFT from a quote), `POST /api/transactions/{id}/confirm` and `POST
//! /api/transactions/{id}/cancel`. Each inherits `(Rpc, "saveTransaction")`, the row of
//! `ExchangeTransact`, as `POST /api/quotes` does (lane controller ruling on #672).
//!
//! ⚠️ Until every TX-8 reader has merged, the desk must not call these routes (design §9); the lane
//! controller gates DK-C #539 on TX-8.

use std::path::Path;
use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use actix_web::web;
use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use ts_rs::TS;
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role; // OR-7 bridge, removed by #1024
use xc_exchange::application::booking::service::BookingService;
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::repository::TransactionStore;
use xc_exchange::domain::transaction::transaction::TransactionId;
use xc_exchange::infrastructure::booking::booking_ids::UuidBookingIds;
use xc_exchange::infrastructure::booking::booking_repository::PgBookingRepository;
use xc_exchange::infrastructure::booking::member_repository::PgMemberRepository;
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Money;
use xc_kiosk::application::kiosk::service::KioskService;
use xc_rate::infrastructure::branch::branch_repository::PgBranchRepository;
use xc_rate::infrastructure::rate::bot_rate_source::BotRateSource;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use super::commands::make_approval_service;
use crate::application::settings::PaymentSettingsService;
use crate::application::workflows::booking_save_step::BookingSaveStep;
use crate::application::workflows::broker_feed_save_step::BrokerFeedSaveStep;
use crate::application::workflows::custody_save_step::CustodySaveStep;
use crate::application::workflows::device_save_step::DeviceSaveStep;
use crate::application::workflows::exchange::cancel::CancelTransaction;
use crate::application::workflows::exchange::confirm::ConfirmTransaction;
use crate::application::workflows::exchange::create::{
    CreatePayment, CreateRequest, CreateTransaction,
};
use crate::application::workflows::kiosk_save_step::KioskSaveStep;
use crate::application::workflows::payment_save_step::PaymentSaveStep;
use crate::application::workflows::save_step::{SaveActor, SaveExtras, SaveStep};
use crate::bootstrap::AppState;
use crate::infrastructure::bookings::booking_ports::PgBookingPorts;
use crate::infrastructure::custody::custody_service::pg_custody_service;
use crate::infrastructure::devices::device_service::DeviceService;
use crate::infrastructure::devices::job_ids::UuidJobIds;
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::devices::pg_saved_exchanges::PgSavedExchanges;
use crate::infrastructure::devices::printer_models::PrinterModels;
use crate::infrastructure::files::file_service::FileService;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::infrastructure::kiosk::kiosk_ports::{PgKioskAuditLog, PgKioskPorts};
use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;
use crate::presentation::dto::v3::transaction::TransactionDto;
use crate::presentation::extractors::actor::legacy_actor;
use crate::presentation::http::v3::command::{
    role_not_allowed_here,
    CommandAction, CommandEndpoint, out_of_scope, server_error,
};
use crate::presentation::http::v3::error::V3Error;

// ------------------------------------------------------------ inputs

/// A file uploaded before the command, named by its key.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transactions.ts")]
pub struct UploadRef {
    pub upload_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transactions.ts")]
pub struct TransactionPaymentInput {
    /// `PaymentMethod::as_str`, e.g. `"Cash"`.
    pub method: String,
    #[serde(default)]
    pub reference: String,
    #[serde(default)]
    pub account: String,
    pub slip: Option<UploadRef>,
    pub bank_fee_thb: String,
    pub paid_at: String,
}

/// §4.2's `[required]` create input. No figure is read from it: every figure is the quote's.
#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transactions.ts")]
pub struct TransactionCreateInput {
    pub quote_id: Option<String>,
    pub customer_id: String,
    pub booking_id: Option<String>,
    pub kiosk_request_id: Option<String>,
    pub staff_draft_id: Option<String>,
    pub passport: UploadRef,
    pub payment: TransactionPaymentInput,
    pub value_date: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub count_confirmed: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transactions.ts")]
pub struct TransactionConfirmInput {
    /// The path's `{id}`.
    pub id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "transactions.ts")]
pub struct TransactionCancelInput {
    /// The path's `{id}`.
    pub id: String,
    #[serde(default)]
    pub reason: String,
}

// ------------------------------------------------------------ helpers

fn invalid(field: &'static str, message: &str) -> V3Error {
    V3Error::new(
        "validationFailed",
        Some(field),
        message.to_owned(),
        StatusCode::BAD_REQUEST,
    )
}

/// The v3 answer of a workflow refusal: `Err(AppError::Domain(m))` whose `m` starts with its code.
/// A step's or today's sentence without a code is answered `validationFailed` with the sentence.
pub(crate) fn refusal(e: AppError) -> V3Error {
    let AppError::Domain(m) = e else {
        tracing::error!(error = %e, "transaction command failed");
        return server_error();
    };
    let code = m
        .split(|c: char| c == ':' || c.is_whitespace())
        .next()
        .unwrap_or("");
    let (code, status) = match code {
        "outOfScope" => ("outOfScope", StatusCode::FORBIDDEN),
        "staleVersion" => ("staleVersion", StatusCode::CONFLICT),
        "quoteExpired" => ("quoteExpired", StatusCode::CONFLICT),
        "idempotencyConflict" => ("idempotencyConflict", StatusCode::CONFLICT),
        "alreadyPending" => ("alreadyPending", StatusCode::CONFLICT),
        "countFirst" => ("countFirst", StatusCode::CONFLICT),
        _ => ("validationFailed", StatusCode::BAD_REQUEST),
    };
    V3Error::new(code, None, m, status)
}

/// `PaymentMethod::as_str` read back.
fn payment_method(text: &str) -> Option<PaymentMethod> {
    [
        PaymentMethod::Cash,
        PaymentMethod::ScbTransfer,
        PaymentMethod::BankTransfer,
        PaymentMethod::CreditCard,
        PaymentMethod::Alipay,
        PaymentMethod::PromptPay,
    ]
    .into_iter()
    .find(|m| m.as_str() == text)
}

/// The signed-in caller as the exchange workflows are told who acts.
fn save_actor(who: &Actor) -> Result<SaveActor, V3Error> {
    // OR-7 bridge, removed by #1024
    let legacy = legacy_actor(who, Utc::now()).ok_or_else(role_not_allowed_here)?;
    Ok(SaveActor {
        id: legacy.id().to_owned(),
        username: legacy.id().to_owned(),
        employee_id: (legacy.role() != Role::Owner).then(|| legacy.id().to_owned()),
        role: legacy.role(),
    })
}

fn transaction_id(id: &str) -> Result<TransactionId, V3Error> {
    Uuid::parse_str(id)
        .map(TransactionId)
        .map_err(|_| invalid("id", "invalid transaction id"))
}

/// The stored exchange's save extras (Q3: stored at create, rebuilt at confirm), after checking it
/// is this branch's.
async fn extras_of(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
    id: TransactionId,
) -> Result<SaveExtras, V3Error> {
    let row: Option<(String, Option<String>, Option<String>, bool)> = sqlx::query_as(
        "SELECT branch_id, kiosk_request_id, staff_draft_id, count_confirmed \
         FROM transactions WHERE id = $1",
    )
    .bind(id.0)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| server_error())?;
    let Some((owner, kiosk_request_id, staff_draft_id, count_confirmed)) = row else {
        return Err(V3Error::new(
            "notFound",
            Some("id"),
            "no such transaction",
            StatusCode::NOT_FOUND,
        ));
    };
    if owner != branch.as_str() {
        return Err(out_of_scope());
    }
    Ok(SaveExtras {
        staff_draft_id,
        count_confirmed,
        kiosk_request_id,
    })
}

/// The exchange as stored, answered as `TransactionDto`.
async fn answer(
    store: &PgTransactionStore,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: TransactionId,
) -> Result<TransactionDto, V3Error> {
    match store.find_in(tx, id).await {
        Ok(Some(t)) => Ok(TransactionDto::from_domain(&t)),
        _ => Err(server_error()),
    }
}

/// The save steps confirm runs, in the order and with the services the compat save registers them
/// (`rpc.rs::registered_actions`): device, kiosk, custody, payment, booking, broker feed (D-7).
pub(crate) fn save_steps(state: &AppState) -> Vec<Arc<dyn SaveStep>> {
    let pool = &state.db;
    let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
    let shifts = ShiftService::new(
        Arc::new(PgShiftRepository::new(pool.clone())),
        branch_tx.clone(),
    );
    let custody = pg_custody_service(pool, &shifts, &branch_tx);
    let kiosk = KioskService::new(
        branch_tx.clone(),
        Arc::new(PgKioskPorts::new(
            pool.clone(),
            Arc::new(PgKioskAuditLog::new(PgDeviceRepository::new(pool.clone()))),
            Arc::new(BotRateSource::bot()),
        )),
    );
    let transactions = TransactionService::new(
        Arc::new(PgTransactionRepository::new(pool.clone())),
        branch_tx.clone(),
    );
    let printer_models = Arc::new(
        PrinterModels::new(state.settings.devices.printer_models.clone())
            .unwrap_or_else(|_| PrinterModels::built_in()),
    );
    let devices = DeviceService::new(
        branch_tx.clone(),
        Arc::new(PgSavedExchanges::new(transactions)),
        Arc::new(UuidJobIds),
    )
    .with_printer_models(printer_models);
    let payments = PaymentSettingsService::new(
        branch_tx.clone(),
        BranchSettingStore::new(pool.clone()),
        PgBranchRepository::new(pool.clone()),
    );
    let bookings = BookingService::new(
        branch_tx,
        Arc::new(PgBookingRepository::new(pool.clone())),
        Arc::new(PgMemberRepository::new(pool.clone())),
        Arc::new(PgBookingPorts::new(pool.clone())),
        Arc::new(UuidBookingIds),
    );
    vec![
        Arc::new(DeviceSaveStep::new(devices)),
        Arc::new(KioskSaveStep::new(kiosk)),
        Arc::new(CustodySaveStep::new(custody)),
        Arc::new(PaymentSaveStep::new(payments)),
        Arc::new(BookingSaveStep::new(bookings)),
        Arc::new(BrokerFeedSaveStep::new()),
    ]
}

// ------------------------------------------------------------ create

pub struct TransactionCreateCommand {
    service: CreateTransaction,
    store: PgTransactionStore,
}

impl TransactionCreateCommand {
    pub fn new(pool: PgPool, files: FileService) -> Self {
        Self {
            service: CreateTransaction::new(pool.clone(), files),
            store: PgTransactionStore::new(pool),
        }
    }
}

impl CommandAction for TransactionCreateCommand {
    const NAME: &'static str = "transactionCreate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "saveTransaction");
    type Input = TransactionCreateInput;
    type Output = TransactionDto;
    const ROUTE: &'static str = "/api/transactions";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let quote_id = input
            .quote_id
            .as_deref()
            .ok_or_else(|| invalid("quoteId", "quoteId is required"))?;
        let quote_id =
            Uuid::parse_str(quote_id).map_err(|_| invalid("quoteId", "invalid quoteId"))?;
        let method = payment_method(&input.payment.method)
            .ok_or_else(|| invalid("payment.method", "invalid payment method"))?;
        let bank_fee_thb = Money::parse(&input.payment.bank_fee_thb)
            .map_err(|_| invalid("payment.bankFeeThb", "invalid bank fee"))?;
        let paid_at = DateTime::parse_from_rfc3339(&input.payment.paid_at)
            .map(|at| at.with_timezone(&Utc))
            .map_err(|_| invalid("payment.paidAt", "invalid paidAt"))?;
        let value_date = NaiveDate::parse_from_str(&input.value_date, "%Y-%m-%d")
            .map_err(|_| invalid("valueDate", "invalid valueDate"))?;
        let req = CreateRequest {
            branch_id: branch.clone(),
            quote_id,
            customer_id: input.customer_id,
            booking_id: input.booking_id,
            kiosk_request_id: input.kiosk_request_id,
            staff_draft_id: input.staff_draft_id,
            passport_key: input.passport.upload_key,
            payment: CreatePayment {
                method,
                reference: input.payment.reference,
                account: input.payment.account,
                slip_key: input.payment.slip.map(|s| s.upload_key),
                bank_fee_thb,
                paid_at,
            },
            value_date,
            notes: input.notes,
            count_confirmed: input.count_confirmed,
        };
        let made = self
            .service
            .run_in(tx, req, &save_actor(who)?, Utc::now())
            .await
            .map_err(refusal)?;
        answer(&self.store, tx, made.id).await
    }
}

// ------------------------------------------------------------ confirm

pub struct TransactionConfirmCommand {
    service: ConfirmTransaction,
    store: PgTransactionStore,
}

impl TransactionConfirmCommand {
    pub fn new(pool: PgPool, steps: Vec<Arc<dyn SaveStep>>) -> Self {
        Self {
            service: ConfirmTransaction::new(
                pool.clone(),
                make_approval_service(pool.clone()),
                steps,
            ),
            store: PgTransactionStore::new(pool),
        }
    }
}

impl CommandAction for TransactionConfirmCommand {
    const NAME: &'static str = "transactionConfirm";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "saveTransaction");
    type Input = TransactionConfirmInput;
    type Output = TransactionDto;
    const ROUTE: &'static str = "/api/transactions/{id}/confirm";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let id = transaction_id(&input.id)?;
        let extras = extras_of(tx, branch, id).await?;
        self.service
            .run_in(tx, id, &save_actor(who)?, &extras, Utc::now())
            .await
            .map_err(refusal)?;
        answer(&self.store, tx, id).await
    }
}

// ------------------------------------------------------------ cancel

pub struct TransactionCancelCommand {
    service: CancelTransaction,
}

impl TransactionCancelCommand {
    pub fn new(pool: PgPool) -> Self {
        Self {
            service: CancelTransaction::new(pool),
        }
    }
}

impl CommandAction for TransactionCancelCommand {
    const NAME: &'static str = "transactionCancel";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "saveTransaction");
    type Input = TransactionCancelInput;
    type Output = TransactionDto;
    const ROUTE: &'static str = "/api/transactions/{id}/cancel";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let id = transaction_id(&input.id)?;
        extras_of(tx, branch, id).await?;
        let cancelled = self
            .service
            .run_in(tx, id, &input.reason, &save_actor(who)?, Utc::now())
            .await
            .map_err(refusal)?;
        Ok(TransactionDto::from_domain(&cancelled))
    }
}

pub fn register(cfg: &mut web::ServiceConfig, state: &AppState) {
    let pool = state.db.clone();
    let files = FileService::new(pool.clone(), state.settings.files.dir.clone());
    CommandEndpoint::register(
        cfg,
        Arc::new(TransactionCreateCommand::new(pool.clone(), files)),
    );
    CommandEndpoint::register(
        cfg,
        Arc::new(TransactionConfirmCommand::new(
            pool.clone(),
            save_steps(state),
        )),
    );
    CommandEndpoint::register(cfg, Arc::new(TransactionCancelCommand::new(pool)));
}

pub fn export_types(dir: &Path) -> Result<(), ts_rs::ExportError> {
    let cfg = ts_rs::Config::new().with_out_dir(dir);
    UploadRef::export_all(&cfg)?;
    TransactionPaymentInput::export_all(&cfg)?;
    TransactionCreateInput::export_all(&cfg)?;
    TransactionConfirmInput::export_all(&cfg)?;
    TransactionCancelInput::export_all(&cfg)?;
    TransactionDto::export_all(&cfg)?;
    Ok(())
}
