//! Backdated keying (`docs/design/backdated-keying.md` §2, §4–§7; the owner's rules R1–R4 and guard G of
//! 2026-11-04): one COMPLETED, backdated exchange keyed at its real past time, with no shift, at the typed rate, its
//! cash in the branch SAFE. Runs in the caller's transaction (the command seam's, under the branch lock).

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_customer::application::customer::service::CustomerService;
use xc_exchange::domain::exchange::value_objects::transaction_form::{
    FILE_MISSING, NO_PAYMENT_REF,
};
use xc_exchange::domain::transaction::backdate_mark::BackdateMark;
use xc_exchange::domain::transaction::backdate_pricing::BackdatePricing;
use xc_exchange::domain::transaction::backdate_window::{BackdateRefusal, BackdateWindow};
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::{StatusMove, TransactionStore};
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{
    CurrencyCode, CurrencyUnits, ForeignAmount, MinorUnit, Money, Rate, Side, UnitSize,
};

use super::complete::{CompleteExchange, ExchangeEffects};
use super::create::CreatePayment;
use crate::application::workflows::save_step::SaveActor;
use crate::application::workflows::save_transaction::ATTACHMENT_NOT_FOUND;
use crate::infrastructure::files::file_service::FileService;
use crate::infrastructure::finance::closed_days::ClosedDays;

const WALK_IN: &str = "WALK-IN";

/// `[required]` What the command parsed (§4). Every figure is the caller's; none is read from a quote.
pub struct BackdateRequest {
    pub branch_id: BranchId,
    pub customer_id: String,
    pub side: Side,
    pub currency: CurrencyCode,
    pub quantity: ForeignAmount,
    /// THB per the branch's `unit_size` for `currency`, as the shop used it (R4).
    pub rate: Rate,
    pub fee_thb: Money,
    pub expected_settle_thb: Option<Money>,
    /// `payment.paid_at` is the deal's real time (R2).
    pub payment: CreatePayment,
    pub value_date: NaiveDate,
    /// The passport scan's upload key. Optional for a backdated sale (owner, §11 Q1 = B, 2026-11-04): `None` writes
    /// no PASSPORT row; `Some` is checked as `CreateTransaction` checks it.
    pub passport_key: Option<String>,
    pub notes: String,
}

pub struct BackdateSale {
    pool: PgPool,
    files: FileService,
}

impl BackdateSale {
    pub fn new(pool: PgPool, files: FileService) -> Self {
        Self { pool, files }
    }

    async fn attachment(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        key: &str,
        who: &SaveActor,
        branch: &BranchId,
    ) -> AppResult<()> {
        if self
            .files
            .attachment_in(tx, key, &who.id, branch)
            .await?
            .is_none()
        {
            return Err(AppError::Domain(ATTACHMENT_NOT_FOUND.to_owned()));
        }
        Ok(())
    }

    /// `[required]` In this order, every refusal before the first write (§4, §5, §6, §7):
    /// 1. `BackdateWindow::check(paid_at, value_date, now, <branch timezone>, ClosedDays::is_closed_in(..))`;
    ///    a refusal answers `AppError::Domain("<code>:<field>: …")` as `transactions.rs::refusal` reads it
    ///    (`dayClosed: …` for a closed day).
    /// 2. The currency: not THB, in the branch's register (`PgCurrencyRegister`), for `unit_size`.
    /// 3. `BackdatePricing::price(side, quantity, rate, unit_size, fee_thb, expected_settle_thb)`.
    /// 4. The customer: `CustomerService::exchange_customer_in`; DEMO is `validationFailed:customerId`.
    /// 5. The passport and, for a non-cash method, the reference and slip, as `CreateTransaction` checks them.
    /// 6. `insert_backdated_draft_in` (LIVE, no shift, no quote, `recorded_at = now`, `recorded_by = who.id`,
    ///    `staff_id = who.username`, the line at `rate_snapshot = rate`) and the audit event `transaction.backdated`.
    /// 7. `set_status_in` DRAFT → CONFIRMED (no approval, §11 Q2).
    /// 8. `CompleteExchange::run_backdated_in(tx, id, who, paid_at, BackdateMark { entered_at: now, entered_by })`.
    ///
    /// Answers the COMPLETED exchange.
    pub async fn run_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        req: BackdateRequest,
        who: &SaveActor,
        now: DateTime<Utc>,
    ) -> AppResult<ExchangeTransaction> {
        // 1. Branch timezone + closed day + BackdateWindow::check
        let row: Option<(Uuid, String)> =
            sqlx::query_as("SELECT organization_id, timezone FROM branches WHERE id = $1")
                .bind(req.branch_id.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)?;
        let Some((organization_id, timezone)) = row else {
            return Err(AppError::Internal(format!(
                "branch {} does not exist",
                req.branch_id.as_str()
            )));
        };
        let tz: chrono_tz::Tz = timezone
            .parse()
            .map_err(|_| AppError::Internal(format!("branch timezone {timezone}")))?;
        let closed = ClosedDays::is_closed_in(tx, &req.branch_id, req.value_date).await?;
        if let Err(r) = BackdateWindow::check(req.payment.paid_at, req.value_date, now, tz, closed)
        {
            let msg = match r {
                BackdateRefusal::PaidAtInFuture => format!(
                    "{}:{}: paidAt is in the future",
                    r.code(),
                    r.field().unwrap_or("")
                ),
                BackdateRefusal::ValueDateNotPaidDate => format!(
                    "{}:{}: valueDate must match paidAt's branch date",
                    r.code(),
                    r.field().unwrap_or("")
                ),
                BackdateRefusal::ValueDateInFuture => format!(
                    "{}:{}: valueDate is in the future",
                    r.code(),
                    r.field().unwrap_or("")
                ),
                BackdateRefusal::DayClosed => {
                    format!("{}: business date {} is closed", r.code(), req.value_date)
                }
            };
            return Err(AppError::Domain(msg));
        }

        // 2. The currency: not THB, in the branch's register (PgCurrencyRegister), for unit_size
        if req.currency == CurrencyCode::THB {
            return Err(AppError::Domain(
                "validationFailed:currency: THB is not a foreign currency".to_owned(),
            ));
        }
        // Read in the caller's transaction (it holds the branch lock), as `PgCurrencyRegister::units` reads.
        let row: Option<(rust_decimal::Decimal, i16)> =
            sqlx::query_as("SELECT unit_size, minor_unit FROM currencies WHERE code = $1")
                .bind(req.currency.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)?;
        let Some((unit_size, minor_unit)) = row else {
            return Err(AppError::Domain(format!(
                "validationFailed:currency: unknown currency {}",
                req.currency
            )));
        };
        let unit_size = UnitSize::from_decimal(unit_size).map_err(|e| {
            AppError::Internal(format!("invalid unit_size for {}: {e}", req.currency))
        })?;
        let minor_unit = MinorUnit::new(minor_unit as u8).map_err(|e| {
            AppError::Internal(format!("invalid minor_unit for {}: {e}", req.currency))
        })?;
        let units = CurrencyUnits::new(req.currency, unit_size, minor_unit);
        if req.quantity.value() <= rust_decimal::Decimal::ZERO {
            return Err(AppError::Domain(
                "validationFailed:quantity: quantity must be greater than zero".to_owned(),
            ));
        }
        if req.quantity.value().normalize().scale() > u32::from(units.minor_unit().get()) {
            return Err(AppError::Domain(
                "validationFailed:quantity: quantity has more decimals than the currency's minor unit".to_owned(),
            ));
        }

        // 3. BackdatePricing::price
        let priced = BackdatePricing::price(
            req.side,
            req.quantity,
            req.rate,
            units.unit_size(),
            req.fee_thb,
            req.expected_settle_thb,
        )
        .map_err(|r| {
            AppError::Domain(format!("validationFailed:{}: {}", r.field(), r.message()))
        })?;

        // 4. Customer: CustomerService::exchange_customer_in; Mode::Demo -> validationFailed:customerId
        let customer =
            CustomerService::exchange_customer_in(tx, &req.branch_id, &req.customer_id).await?;
        if customer.mode == Mode::Demo {
            return Err(AppError::Domain(
                "validationFailed:customerId: backdated keying is LIVE only".to_owned(),
            ));
        }

        // 5. Passport (optional) and, for a non-cash method, reference and slip
        if !req.payment.method.is_cash() {
            if req.payment.reference.trim().is_empty() {
                return Err(AppError::Domain(NO_PAYMENT_REF.to_owned()));
            }
            if req.payment.slip_key.is_none() {
                return Err(AppError::Domain(FILE_MISSING.to_owned()));
            }
        }
        if let Some(passport) = req.passport_key.as_deref() {
            self.attachment(tx, passport, who, &req.branch_id).await?;
        }
        if let Some(slip) = req.payment.slip_key.as_deref() {
            self.attachment(tx, slip, who, &req.branch_id).await?;
        }

        // 6. Build ExchangeTransaction, insert_backdated_draft_in, and audit transaction.backdated
        let id = TransactionId(Uuid::new_v4());
        let t = ExchangeTransaction {
            queue_ticket_id: None,
            id,
            organization_id,
            branch_id: req.branch_id.clone(),
            mode: Mode::Live,
            status: TransactionStatus::Draft,
            document_ref: None,
            quote_id: None,
            booking_id: None,
            shift_id: None,
            submission_id: id.0.to_string(),
            payload_hash: payload_hash(&req),
            customer_id: customer.id,
            customer_name: customer.display_name,
            nationality: customer.nationality,
            staff_id: who.username.clone(),
            booking_no: WALK_IN.to_owned(),
            notes: req.notes.clone(),
            value_date: req.value_date,
            recorded_at: now,
            recorded_by: who.id.clone(),
            completed_at: None,
            cancelled_at: None,
            cancel_reason: None,
            approval_request_id: None,
            lines: vec![TransactionLine {
                id: Uuid::new_v4(),
                quote_line_id: None,
                shop_side: req.side,
                currency: req.currency,
                quantity: req.quantity,
                unit_size: units.unit_size(),
                rate_snapshot: req.rate,
                reference_rate: None,
                thb_amount: priced.thb_amount,
                fee_thb: priced.fee_thb,
                settle_thb: priced.settle_thb,
                cost_basis_thb: None,
                realized_margin_thb: None,
                line_no: 1,
            }],
            payment: TransactionPayment {
                id: Uuid::new_v4(),
                payment_no: 1,
                method: req.payment.method,
                reference: req.payment.reference.clone(),
                account: req.payment.account.clone(),
                slip_file_key: req.payment.slip_key.clone(),
                amount_thb: priced.settle_thb,
                bank_fee_thb: req.payment.bank_fee_thb,
                paid_at: req.payment.paid_at,
            },
            receipts: vec![],
            passport_file_key: req.passport_key.clone().unwrap_or_default(),
            kiosk_request_id: None,
            staff_draft_id: None,
            count_confirmed: false,
        };
        let store = PgTransactionStore::new(self.pool.clone());
        store.insert_backdated_draft_in(tx, &t).await?;
        PgAuditLog::new(self.pool.clone())
            .append_in(
                tx,
                &organization_id.to_string(),
                &AuditEvent {
                    actor_id: who.id.clone(),
                    actor_role: who.audit_role().to_string(),
                    scope: "branch".to_string(),
                    action: "transaction.backdated".to_string(),
                    object_ref: id.0.to_string(),
                    branch_id: Some(t.branch_id.as_str().to_string()),
                    before: None,
                    after: Some(json!({
                        "paidAt": req.payment.paid_at.to_rfc3339(),
                        "valueDate": req.value_date.to_string(),
                        "rate": req.rate.to_string(),
                        "enteredAt": now.to_rfc3339(),
                        "enteredBy": who.id.clone(),
                    })),
                },
            )
            .await?;

        // 7. set_status_in(DRAFT -> CONFIRMED) with no approval request (§11 Q2 = A)
        store
            .set_status_in(
                tx,
                id,
                StatusMove {
                    from: TransactionStatus::Draft,
                    to: TransactionStatus::Confirmed,
                    reason: None,
                    approval_request_id: None,
                },
                now,
            )
            .await?;

        // 8. CompleteExchange::run_backdated_in
        let completed = CompleteExchange::new(
            self.pool.clone(),
            self.files.clone(),
            Arc::new(LedgerCashPostings::new()),
            vec![],
            Arc::new(ExchangeEffects::new(vec![])),
        )
        .run_backdated_in(
            tx,
            id,
            who,
            req.payment.paid_at,
            BackdateMark {
                entered_at: now,
                entered_by: who.id.clone(),
            },
        )
        .await?;

        Ok(completed.transaction)
    }
}

fn payload_hash(req: &BackdateRequest) -> String {
    let parts = [
        req.branch_id.as_str().to_owned(),
        req.customer_id.clone(),
        match req.side {
            Side::Buy => "BUY".to_owned(),
            Side::Sell => "SELL".to_owned(),
        },
        req.currency.as_str().to_owned(),
        req.quantity.to_string(),
        req.rate.to_string(),
        req.fee_thb.to_string(),
        req.expected_settle_thb
            .map(|m| m.to_string())
            .unwrap_or_default(),
        req.passport_key.clone().unwrap_or_default(),
        req.payment.method.as_str().to_owned(),
        req.payment.reference.clone(),
        req.payment.account.clone(),
        req.payment.slip_key.clone().unwrap_or_default(),
        req.payment.bank_fee_thb.to_string(),
        req.payment.paid_at.to_rfc3339(),
        req.value_date.to_string(),
        req.notes.clone(),
    ];
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}
