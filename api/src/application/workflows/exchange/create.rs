//! TX-4 #672: `CreateTransaction` (design `quote-transaction.md` §4.4, §7): a DRAFT made from an
//! ACTIVE quote of this branch. Every figure comes from the quote, never from the body.

use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;
use xc_exchange::application::customer::service::CustomerService;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::quote::quote::{Quote, QuoteStatus};
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::TransactionStore;
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{Money, Rate};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use crate::application::workflows::save_step::SaveActor;
use crate::application::workflows::save_transaction::ATTACHMENT_NOT_FOUND;
use crate::infrastructure::files::file_service::FileService;
use serde_json::json;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;

/// The booking number of an exchange that names no booking.
const WALK_IN: &str = "WALK-IN";

pub struct CreatePayment {
    pub method: PaymentMethod,
    pub reference: String,
    pub account: String,
    pub slip_key: Option<String>,
    pub bank_fee_thb: Money,
    pub paid_at: DateTime<Utc>,
}

pub struct CreateRequest {
    pub branch_id: BranchId,
    pub quote_id: Uuid,
    pub customer_id: String,
    pub booking_id: Option<String>,
    pub kiosk_request_id: Option<String>,
    pub staff_draft_id: Option<String>,
    pub passport_key: String,
    pub payment: CreatePayment,
    pub value_date: NaiveDate,
    pub notes: String,
    pub count_confirmed: bool,
}

pub struct CreateTransaction {
    store: PgTransactionStore,
    quotes: PgQuoteRepository,
    shifts: ShiftService,
    files: FileService,
    audit: PgAuditLog,
}

fn refuse<T>(m: &str) -> AppResult<T> {
    Err(AppError::Domain(m.to_owned()))
}

/// The branch's current rate publication: the second half of `Quote::is_valid` (D-5).
pub(crate) async fn current_publication_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
) -> AppResult<Option<Uuid>> {
    let row: Option<Option<Uuid>> =
        sqlx::query_scalar("SELECT current_publication_id FROM branches WHERE id = $1")
            .bind(branch.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;
    Ok(row.flatten())
}

/// D-5 on a stored quote: `now < expires_at` and its publication is the branch's current one.
pub(crate) async fn check_valid_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    q: &Quote,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let current = current_publication_in(tx, &q.branch_id).await?;
    if current.is_some_and(|c| q.is_valid(now, c)) {
        Ok(())
    } else {
        Err(AppError::Domain(format!(
            "quoteExpired: quote {} is no longer valid",
            q.id
        )))
    }
}

impl CreateTransaction {
    pub fn new(pool: PgPool, files: FileService) -> Self {
        let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
        Self {
            store: PgTransactionStore::new(pool.clone()),
            quotes: PgQuoteRepository::new(pool.clone()),
            shifts: ShiftService::new(Arc::new(PgShiftRepository::new(pool.clone())), branch_tx),
            files,
            audit: PgAuditLog::new(pool),
        }
    }

    /// A file the saver uploaded at this branch (its `files` row), refused with today's sentence.
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
            return refuse(ATTACHMENT_NOT_FOUND);
        }
        Ok(())
    }

    /// §4.4 steps 1–4. Every refusal comes before the one write.
    pub async fn run_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        req: CreateRequest,
        who: &SaveActor,
        now: DateTime<Utc>,
    ) -> AppResult<ExchangeTransaction> {
        self.run_inner(tx, req, who, None, None, None, None, None, None, None, now)
            .await
    }

    /// TX-6: adapter run with optional teller_user_id and caller-supplied submission_id / payload_hash.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_for_save_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        req: CreateRequest,
        who: &SaveActor,
        teller_user_id: Option<Uuid>,
        submission_id: Option<String>,
        payload_hash: Option<String>,
        mode: Mode,
        customer_name: String,
        nationality: String,
        reference_rate: Option<Rate>,
        now: DateTime<Utc>,
    ) -> AppResult<ExchangeTransaction> {
        self.run_inner(
            tx,
            req,
            who,
            teller_user_id,
            submission_id,
            payload_hash,
            Some(mode),
            Some(customer_name),
            Some(nationality),
            reference_rate,
            now,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_inner(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        req: CreateRequest,
        who: &SaveActor,
        teller_user_id: Option<Uuid>,
        custom_submission_id: Option<String>,
        custom_payload_hash: Option<String>,
        custom_mode: Option<Mode>,
        custom_customer_name: Option<String>,
        custom_nationality: Option<String>,
        custom_reference_rate: Option<Rate>,
        now: DateTime<Utc>,
    ) -> AppResult<ExchangeTransaction> {
        // 1. The quote: this branch's, ACTIVE, valid, and the named customer's
        let Some(q) = self.quotes.find_in(tx, req.quote_id).await? else {
            return Err(AppError::Domain(format!(
                "staleVersion: quote {} does not exist",
                req.quote_id
            )));
        };
        if q.branch_id != req.branch_id {
            return Err(AppError::Domain(format!(
                "outOfScope: quote {} is another branch's",
                q.id
            )));
        }
        if q.status != QuoteStatus::Active {
            return Err(AppError::Domain(format!(
                "staleVersion: quote {} is {}",
                q.id,
                q.status.as_str()
            )));
        }
        check_valid_in(tx, &q, now).await?;
        if q.customer_id
            .as_ref()
            .is_some_and(|c| *c != req.customer_id)
        {
            return Err(AppError::Domain(format!(
                "validationFailed: quote {} is another customer's",
                q.id
            )));
        }
        // G7: a kiosk request quoted since TX-7 is completed with its own quote only
        if let Some(request) = req.kiosk_request_id.as_deref() {
            let kiosk_quote: Option<Option<Uuid>> = sqlx::query_scalar(
                "SELECT quote_id FROM kiosk_requests WHERE branch_id = $1 AND id = $2",
            )
            .bind(req.branch_id.as_str())
            .bind(request)
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;
            if let Some(Some(kiosk_quote)) = kiosk_quote
                && kiosk_quote != q.id
            {
                return Err(AppError::Domain(format!(
                    "validationFailed: kiosk request {request} was quoted as {kiosk_quote}"
                )));
            }
        }
        let Some(line) = q.lines.first() else {
            return Err(AppError::Internal(format!("quote {} has no line", q.id)));
        };

        // G6, Q1 = B: the customer's mode, name and nationality, read without decrypting
        let customer =
            CustomerService::exchange_customer_in(tx, &req.branch_id, &req.customer_id).await?;

        let mode = custom_mode.unwrap_or(customer.mode);
        let customer_name = custom_customer_name.unwrap_or(customer.display_name);
        let nationality = custom_nationality.unwrap_or(customer.nationality);

        // 2. LIVE needs the branch's open shift; DEMO has none
        let shift_id = match mode {
            Mode::Live => {
                let shift = if let Some(teller_id) = teller_user_id {
                    sqlx::query_scalar(
                        "SELECT id FROM shifts WHERE branch_id = $1 AND teller_id = $2 AND status = 'OPEN'",
                    )
                    .bind(req.branch_id.as_str())
                    .bind(teller_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(AppError::Database)?
                } else {
                    self.shifts
                        .active_in(tx, &req.branch_id)
                        .await?
                        .map(|s| s.id)
                };
                match shift {
                    Some(id) => Some(id),
                    None => return Err(ShiftError::NoOpenShift.into()),
                }
            }
            Mode::Demo => None,
        };

        // 3. The passport and slip files, as today
        self.attachment(tx, &req.passport_key, who, &req.branch_id)
            .await?;
        if let Some(slip) = req.payment.slip_key.as_deref() {
            self.attachment(tx, slip, who, &req.branch_id).await?;
        }

        // 4. The DRAFT, its line copied from the quote line, and its one payment (D-9)
        let id = TransactionId(Uuid::new_v4());
        let quote_line_id = Some(line.id);
        let t = ExchangeTransaction {
            id,
            organization_id: q.organization_id,
            branch_id: req.branch_id.clone(),
            mode,
            status: TransactionStatus::Draft,
            document_ref: None,
            quote_id: Some(q.id),
            booking_id: req.booking_id.clone(),
            shift_id,
            submission_id: custom_submission_id.unwrap_or_else(|| id.0.to_string()),
            payload_hash: custom_payload_hash.unwrap_or_else(|| payload_hash(&req)),
            customer_id: customer.id,
            customer_name,
            nationality,
            staff_id: who.username.clone(),
            booking_no: req.booking_id.clone().unwrap_or_else(|| WALK_IN.to_owned()),
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
                quote_line_id,
                shop_side: line.shop_side,
                currency: line.currency,
                quantity: line.quantity,
                unit_size: line.unit_size,
                rate_snapshot: line.final_rate,
                reference_rate: custom_reference_rate,
                thb_amount: line.thb_amount,
                fee_thb: line.fee_thb,
                settle_thb: line.settle_thb,
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
                amount_thb: line.settle_thb,
                bank_fee_thb: req.payment.bank_fee_thb,
                paid_at: req.payment.paid_at,
            },
            receipts: vec![],
            passport_file_key: req.passport_key.clone(),
            kiosk_request_id: req.kiosk_request_id.clone(),
            staff_draft_id: req.staff_draft_id.clone(),
            count_confirmed: req.count_confirmed,
        };
        self.store.insert_draft_in(tx, &t).await?;
        self.audit
            .append(
                tx,
                &AuditEvent {
                    actor_id: who.id.clone(),
                    actor_role: who.role.as_str().to_string(),
                    scope: "branch".to_string(),
                    action: "transaction.created".to_string(),
                    object_ref: id.0.to_string(),
                    branch_id: Some(t.branch_id.as_str().to_string()),
                    before: None,
                    after: Some(json!({
                        "status": "DRAFT",
                        "quoteId": q.id.to_string(),
                    })),
                },
            )
            .await?;
        self.store
            .find_in(tx, id)
            .await?
            .ok_or_else(|| AppError::Internal(format!("draft {} was not stored", id.0)))
    }
}

/// What the body asked for, hashed: the draft's `payload_hash`.
fn payload_hash(req: &CreateRequest) -> String {
    let parts = [
        req.branch_id.as_str().to_owned(),
        req.quote_id.to_string(),
        req.customer_id.clone(),
        req.booking_id.clone().unwrap_or_default(),
        req.kiosk_request_id.clone().unwrap_or_default(),
        req.staff_draft_id.clone().unwrap_or_default(),
        req.passport_key.clone(),
        req.payment.method.as_str().to_owned(),
        req.payment.reference.clone(),
        req.payment.account.clone(),
        req.payment.slip_key.clone().unwrap_or_default(),
        req.payment.bank_fee_thb.to_string(),
        req.payment.paid_at.to_rfc3339(),
        req.value_date.to_string(),
        req.notes.clone(),
        req.count_confirmed.to_string(),
    ];
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}
