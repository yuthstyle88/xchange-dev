//! Save transaction workflow.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_customer::application::customer::service::CustomerService;
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_exchange::domain::exchange::entities::transaction::Transaction;
use xc_exchange::domain::exchange::value_objects::transaction_form::{
    FILE_MISSING, TransactionForm,
};
use xc_exchange::domain::quote::quote::QuoteChannel;
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CompatRate, CurrencyCode, ForeignAmount, Rate, Side};
use xc_rate::application::branch::service::BranchService;
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;

use crate::application::workflows::exchange::complete::{
    CompleteExchange, CompletedExchange, ExchangeEffects,
};
use crate::application::workflows::exchange::confirm::{ConfirmOutcome, ConfirmTransaction};
use crate::application::workflows::exchange::create::{
    CreatePayment, CreateRequest, CreateTransaction,
};
use crate::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use crate::application::workflows::save_step::{
    BookingClaim, SaveActor, SaveContext, SaveExtras, SaveStep,
};
use crate::infrastructure::files::file_service::{Attachment, FileService};

pub const BAD_BRANCH: &str = "สาขาไม่ถูกต้อง";
pub const BAD_ATTACHMENT: &str = "ไฟล์แนบไม่ถูกต้อง";
pub const ATTACHMENT_NOT_FOUND: &str = "ไม่พบไฟล์แนบของสาขานี้";
pub const ATTACHMENT_NOT_READY: &str = "ไฟล์แนบยังไม่พร้อม";
pub const CHANGED: &str = "รายการเดิมมีข้อมูลเปลี่ยน ห้ามใช้รหัสส่งซ้ำกับข้อมูลใหม่";
pub const RATE_MISMATCH: &str = "เรทไม่ตรงกับเรทหน้าร้านล่าสุด กรุณาตรวจสอบเรทก่อนยืนยันรายการ";
pub const TOO_MANY_DECIMALS: &str = "ทศนิยมเกินกำหนด";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedTransaction {
    pub mode: Mode,
    pub document_ref: String,
    pub passport_url: String,
    pub slip_url: String,
    pub receipt_url: String,
}

#[derive(Clone)]
pub struct SaveTransactionWorkflow {
    branch_tx: BranchTx,
    shifts: ShiftService,
    files: FileService,
    branches: BranchService,
    transactions: TransactionService,
    steps: Vec<Arc<dyn SaveStep>>,
}

fn refuse<T>(sentence: &str) -> AppResult<T> {
    Err(AppError::Domain(sentence.to_owned()))
}

/// What makes two sendings of one submission the same exchange: every parsed
/// value of the form and the content of its files. Kept with the exchange and
/// compared as text.
fn digest(form: &TransactionForm, passport_sha256: &str, slip_sha256: Option<&str>) -> String {
    let date = |d: Option<chrono::NaiveDate>| d.map(|d| d.to_string()).unwrap_or_default();
    let passport_part = if let Ok(keys) = xc_customer::get_pii_keys() {
        let normalized = xc_kernel::pii::normalize(
            xc_kernel::pii::IdentityType::Passport,
            &form.customer.passport_no,
            Some(&form.customer.nationality),
        )
        .unwrap_or_else(|_| form.customer.passport_no.trim().to_uppercase());
        let hash = xc_kernel::pii::LookupHasher::new(&keys)
            .hash(xc_kernel::pii::IdentityType::Passport, &normalized);
        hash.iter().map(|b| format!("{b:02x}")).collect::<String>()
    } else {
        form.customer.passport_no.clone()
    };
    let parts = [
        form.customer.name.clone(),
        passport_part,
        form.customer.nationality.clone(),
        date(form.customer.birth_date),
        date(form.customer.passport_expiry),
        form.customer.mode.as_str().to_owned(),
        form.submission_id.clone(),
        form.customer_id.clone(),
        match form.side {
            Side::Buy => "BUY".to_owned(),
            Side::Sell => "SELL".to_owned(),
        },
        form.currency.clone(),
        form.amount.to_string(),
        form.rate.to_string(),
        form.fee.to_string(),
        form.bank_fee.to_string(),
        form.method.as_str().to_owned(),
        form.payment_ref.clone(),
        form.account.clone(),
        form.staff.clone(),
        form.booking.clone(),
        form.notes.clone(),
        form.value_date.to_string(),
        form.paid_at.timestamp_millis().to_string(),
        form.reference_rate
            .map(|r| r.to_string())
            .unwrap_or_default(),
        passport_sha256.to_owned(),
        slip_sha256.unwrap_or_default().to_owned(),
    ];
    let mut hasher = Sha256::new();
    for part in parts {
        // length first, so no two lists of texts run together into the same bytes
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn answer(t: &Transaction) -> SavedTransaction {
    SavedTransaction {
        mode: t.mode,
        document_ref: t.document_ref.clone(),
        passport_url: format!("/files/{}", t.passport_key),
        slip_url: t
            .slip_key
            .as_ref()
            .map(|k| format!("/files/{k}"))
            .unwrap_or_default(),
        receipt_url: format!("/files/{}", t.receipt_key),
    }
}

fn answer_from_completed(completed: &CompletedExchange) -> SavedTransaction {
    SavedTransaction {
        mode: completed.transaction.mode,
        document_ref: completed
            .transaction
            .document_ref
            .clone()
            .unwrap_or_default(),
        passport_url: format!("/files/{}", completed.transaction.passport_file_key),
        slip_url: completed
            .transaction
            .payment
            .slip_file_key
            .as_ref()
            .map(|k| format!("/files/{k}"))
            .unwrap_or_default(),
        receipt_url: format!("/files/{}", completed.receipt.document_ref),
    }
}

impl SaveTransactionWorkflow {
    pub fn new(branch_tx: BranchTx, shifts: ShiftService, files: FileService) -> Self {
        let transactions = TransactionService::new(
            Arc::new(PgTransactionRepository::new(branch_tx.pool().clone())),
            branch_tx.clone(),
        );
        Self {
            branches: BranchService::new(branch_tx.clone()),
            branch_tx,
            shifts,
            files,
            transactions,
            steps: Vec::new(),
        }
    }

    async fn attachment(
        &self,
        key: &str,
        actor_id: &str,
        branch: &BranchId,
    ) -> AppResult<Attachment> {
        if key.len() != 36
            || !key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
        {
            return refuse(BAD_ATTACHMENT);
        }
        let Some(found) = self.files.attachment(key, actor_id, branch).await? else {
            return refuse(ATTACHMENT_NOT_FOUND);
        };
        if !self.files.on_disk(key) {
            return refuse(ATTACHMENT_NOT_READY);
        }
        Ok(found)
    }

    /// The steps the other contexts add around a save, in the order they run.
    /// They take part in `save_as` only.
    pub fn with_steps(mut self, steps: Vec<Arc<dyn SaveStep>>) -> Self {
        self.steps = steps;
        self
    }

    /// The save itself, with no step of another context around it and the
    /// form's `staff` as it was sent. Callers that act for a signed-in person
    /// use `save_as`.
    pub async fn save(
        &self,
        branch: &BranchId,
        cmd: SaveTransaction,
        actor_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<SavedTransaction> {
        self.run(branch, cmd, actor_id, now, None, None).await
    }

    /// The save as a signed-in person makes it: the form's `staff` becomes the
    /// saver's username whatever was sent, and every registered step runs
    /// before and after the save inside its transaction.
    pub async fn save_as(
        &self,
        branch: &BranchId,
        cmd: SaveTransaction,
        extras: &SaveExtras,
        actor: &SaveActor,
        now: DateTime<Utc>,
    ) -> AppResult<SavedTransaction> {
        self.save_as_booked(branch, cmd, extras, None, actor, now)
            .await
    }

    /// CA-7 (#623): the save of a Blueprint-session teller. The same save as `save_as`, except that the stock
    /// moves on the OPEN shift whose `teller_id` is `teller.user_id`, never on the branch's v2 shift; no such
    /// shift is `ShiftError::NoOpenShift`; and custody judges it by `BranchDay::check_save` with no custodian.
    pub async fn save_as_teller(
        &self,
        branch: &BranchId,
        mut cmd: SaveTransaction,
        extras: &SaveExtras,
        actor: &SaveActor,
        teller: &Actor,
        now: DateTime<Utc>,
    ) -> AppResult<SavedTransaction> {
        cmd.staff = actor.username.clone();
        self.run(
            branch,
            cmd,
            &actor.id,
            now,
            Some((extras.clone(), None, actor.clone())),
            Some(teller.clone()),
        )
        .await
    }

    /// The save as a signed-in person makes it, with an optional booking claim.
    pub async fn save_as_booked(
        &self,
        branch: &BranchId,
        mut cmd: SaveTransaction,
        extras: &SaveExtras,
        booking: Option<BookingClaim>,
        actor: &SaveActor,
        now: DateTime<Utc>,
    ) -> AppResult<SavedTransaction> {
        cmd.staff = actor.username.clone();
        self.run(
            branch,
            cmd,
            &actor.id,
            now,
            Some((extras.clone(), booking, actor.clone())),
            None,
        )
        .await
    }

    async fn run(
        &self,
        branch: &BranchId,
        cmd: SaveTransaction,
        actor_id: &str,
        now: DateTime<Utc>,
        stepped: Option<(SaveExtras, Option<BookingClaim>, SaveActor)>,
        teller: Option<Actor>,
    ) -> AppResult<SavedTransaction> {
        let b = branch.as_str();
        if !BranchId::is_well_formed(b) {
            return refuse(BAD_BRANCH);
        }
        let passport = match &cmd.passport {
            Some(key) => Some(self.attachment(key, actor_id, branch).await?),
            None => None,
        };
        let slip = match &cmd.slip {
            Some(key) => Some(self.attachment(key, actor_id, branch).await?),
            None => None,
        };
        let _ = self.branches.require(branch).await?;
        let form = TransactionForm::parse(&cmd)?;
        // a form that parsed names a passport file
        let Some(passport) = passport else {
            return refuse(FILE_MISSING);
        };
        let now = DateTime::<Utc>::from_timestamp_millis(now.timestamp_millis()).unwrap_or(now);

        let payload_hash = digest(
            &form,
            &passport.sha256,
            slip.as_ref().map(|s| s.sha256.as_str()),
        );

        let save_actor = stepped
            .as_ref()
            .map(|(_, _, a)| a.clone())
            .unwrap_or_else(|| SaveActor {
                id: actor_id.to_owned(),
                username: cmd.staff.clone(),
                employee_id: None,
                role: Role::System,
            });

        let (this, branch) = (self.clone(), branch.clone());
        let lock = branch.clone();
        self.branch_tx
            .run(&lock, move |tx| {
                Box::pin(async move {
                    let mode = form.customer.mode;
                    let prior = this
                        .transactions
                        .find_submission_in(tx, &branch, mode, &form.submission_id)
                        .await?;

                    if let Some((saved, hash)) = &prior {
                        if let Some((extras, booking, actor)) = &stepped {
                            let ctx = SaveContext {
                                branch: &branch,
                                form: &form,
                                extras,
                                booking: booking.as_ref(),
                                actor,
                                teller: teller.as_ref(),
                                now,
                                prior: Some(saved),
                            };
                            for step in &this.steps {
                                step.before_in(tx, &ctx).await?;
                            }
                        }
                        CustomerService::confirm_in(tx, &branch, &form.customer_id, &form.customer)
                            .await?;
                        if *hash != payload_hash {
                            return refuse(CHANGED);
                        }
                        if let Some((extras, booking, actor)) = &stepped {
                            let ctx = SaveContext {
                                branch: &branch,
                                form: &form,
                                extras,
                                booking: booking.as_ref(),
                                actor,
                                teller: teller.as_ref(),
                                now,
                                prior: Some(saved),
                            };
                            for step in &this.steps {
                                step.after_in(tx, &ctx, saved).await?;
                            }
                        }
                        return Ok(answer(saved));
                    }

                    // Step 1: Quote
                    let extras = stepped.as_ref().map(|(e, _, _)| e);
                    let kiosk_req_id = extras
                        .and_then(|e| e.kiosk_request_id.as_deref())
                        .filter(|s| !s.is_empty());

                    let (quote_id, quote_unit_size) = if let Some(req_id) = kiosk_req_id {
                        // A kiosk save first passes KioskSaveStep's currency/amount check.
                        if let Some((extras, booking, actor)) = &stepped {
                            let ctx = SaveContext {
                                branch: &branch,
                                form: &form,
                                extras,
                                booking: booking.as_ref(),
                                actor,
                                teller: teller.as_ref(),
                                now,
                                prior: None,
                            };
                            for step in &this.steps {
                                step.before_in(tx, &ctx).await?;
                            }
                        }

                        // Then compares form.client_rate with the request's own quote.
                        // A kiosk request with no quote is refused with RATE_MISMATCH. Never synthesize a quote (D-1).
                        let row: Option<(Option<Uuid>,)> = sqlx::query_as(
                            "SELECT quote_id FROM kiosk_requests WHERE branch_id = $1 AND id = $2"
                        )
                        .bind(branch.as_str())
                        .bind(req_id)
                        .fetch_optional(&mut **tx)
                        .await
                        .map_err(AppError::Database)?;

                        let Some((Some(qid),)) = row else {
                            return refuse(RATE_MISMATCH);
                        };

                        let quote_repo = PgQuoteRepository::new(this.branch_tx.pool().clone());
                        let Some(quote) = quote_repo.find_in(tx, qid).await? else {
                            return refuse(RATE_MISMATCH);
                        };
                        let line = quote.lines.first().ok_or_else(|| {
                            AppError::Internal(format!("quote {qid} has no line"))
                        })?;
                        let per_one = CompatRate::per_one(line.final_rate, line.unit_size)
                            .map_err(|_| AppError::Domain(RATE_MISMATCH.to_owned()))?;
                        if form.client_rate != per_one {
                            return refuse(RATE_MISMATCH);
                        }
                        (qid, line.unit_size)
                    } else {
                        // Non-kiosk save (desk / booking): quote first.
                        let Ok(code) = CurrencyCode::parse(&form.currency) else {
                            return refuse(RATE_MISMATCH);
                        };
                        if code.is_thb() {
                            return refuse(RATE_MISMATCH);
                        }
                        let quote_req = QuoteRequest {
                            branch_id: branch.clone(),
                            customer_id: None,
                            channel: QuoteChannel::Desk,
                            lines: vec![QuoteRequestLine {
                                currency: code,
                                shop_side: form.side,
                                quantity: ForeignAmount::from_decimal(Decimal::from_i128_with_scale(
                                    form.amount.minor_i128(),
                                    2,
                                )),
                                fee_thb: Some(form.fee),
                                denomination_group: GroupCode::all(),
                            }],
                        };
                        let quote_service = QuoteService::new(this.branch_tx.pool().clone());
                        let quote = match quote_service.quote_in(tx, quote_req, &save_actor, now).await {
                            Ok(q) => q,
                            Err(AppError::Domain(msg))
                                if mode == Mode::Live && msg.starts_with("rateUnavailable") =>
                            {
                                return refuse(RATE_MISMATCH);
                            }
                            Err(e) => return Err(e),
                        };
                        let line = quote.lines.first().ok_or_else(|| {
                            AppError::Internal(format!("quote {} has no line", quote.id))
                        })?;
                        let per_one = CompatRate::per_one(line.final_rate, line.unit_size)
                            .map_err(|_| AppError::Domain(RATE_MISMATCH.to_owned()))?;
                        if form.client_rate != per_one {
                            return refuse(RATE_MISMATCH);
                        }

                        // Then run steps.before_in
                        if let Some((extras, booking, actor)) = &stepped {
                            let ctx = SaveContext {
                                branch: &branch,
                                form: &form,
                                extras,
                                booking: booking.as_ref(),
                                actor,
                                teller: teller.as_ref(),
                                now,
                                prior: None,
                            };
                            for step in &this.steps {
                                step.before_in(tx, &ctx).await?;
                            }
                        }

                        (quote.id, line.unit_size)
                    };

                    CustomerService::confirm_in(tx, &branch, &form.customer_id, &form.customer)
                        .await?;

                    let resolved_teller = if mode == Mode::Live {
                        let Ok(code) = CurrencyCode::parse(&form.currency) else {
                            return refuse(RATE_MISMATCH);
                        };
                        let delta = StockDelta::for_tx(
                            form.side,
                            code,
                            form.amount,
                            &form.quote,
                            form.method.is_cash(),
                        )
                        .map_err(|_| AppError::Domain(RATE_MISMATCH.to_owned()))?;
                        match &teller {
                            Some(t) => {
                                this.shifts
                                    .apply_delta_for_teller_in(tx, &branch, &t.user_id, &delta)
                                    .await?;
                                None
                            }
                            None => {
                                let Some(teller) = this
                                    .shifts
                                    .teller_of_actor_in(tx, &branch, &save_actor.id)
                                    .await?
                                else {
                                    return Err(ShiftError::NoOpenShift.into());
                                };
                                this.shifts
                                    .apply_delta_for_teller_in(tx, &branch, &teller, &delta)
                                    .await?;
                                Some(teller)
                            }
                        }
                    } else {
                        None
                    };

                    // Step 2: Create Transaction
                    let create_req = CreateRequest {
                        branch_id: branch.clone(),
                        quote_id,
                        customer_id: form.customer_id.clone(),
                        // Stopgap for #1413: transactions.booking_id FK targets v2_bookings. Keep NULL until BK-11 repoints FK to bookings(id).
                        booking_id: None,
                        kiosk_request_id: extras.and_then(|e| e.kiosk_request_id.clone()),
                        staff_draft_id: extras.and_then(|e| e.staff_draft_id.clone()),
                        passport_key: passport.key.clone(),
                        payment: CreatePayment {
                            method: form.method,
                            reference: form.payment_ref.clone(),
                            account: form.account.clone(),
                            slip_key: slip.as_ref().map(|s| s.key.clone()),
                            bank_fee_thb: form.bank_fee,
                            paid_at: form.paid_at,
                        },
                        value_date: form.value_date,
                        notes: form.notes.clone(),
                        count_confirmed: true,
                    };

                    let stored_reference_rate = if let Some(client_ref) = form.client_reference_rate {
                        let per_unit_ref = client_ref * quote_unit_size.value();
                        let parsed = Rate::parse(&per_unit_ref.normalize().to_string())
                            .map_err(|_| AppError::Domain(TOO_MANY_DECIMALS.to_owned()))?;
                        Some(parsed)
                    } else {
                        form.reference_rate
                    };

                    let teller_user_id = teller
                        .as_ref()
                        .map(|t| t.user_id.as_str())
                        .or(resolved_teller.as_deref())
                        .and_then(|id| Uuid::parse_str(id).ok());
                    let create_tx = CreateTransaction::new(this.branch_tx.pool().clone(), this.files.clone());
                    let draft = create_tx
                        .run_for_save_in(
                            tx,
                            create_req,
                            &save_actor,
                            teller_user_id,
                            Some(form.submission_id.clone()),
                            Some(payload_hash.clone()),
                            mode,
                            form.customer.name.clone(),
                            form.customer.nationality.clone(),
                            stored_reference_rate,
                            now,
                        )
                        .await?;
                    // Step 3: Confirm Transaction
                    let approvals = crate::presentation::http::v3::commands::make_approval_service(
                        this.branch_tx.pool().clone(),
                    );
                    let confirm_tx = ConfirmTransaction::new(this.branch_tx.pool().clone(), approvals, vec![]);
                    let confirm_extras = extras.cloned().unwrap_or_default();
                    let outcome = confirm_tx
                        .run_in(tx, draft.id, &save_actor, &confirm_extras, now)
                        .await?;
                    match outcome {
                        ConfirmOutcome::Confirmed => {}
                        ConfirmOutcome::PendingApproval(req_id) => {
                            return Err(AppError::Domain(format!("approvalRequired: request {req_id}")));
                        }
                    }

                    // Step 4: Complete Exchange
                    let complete_steps = if stepped.is_some() {
                        this.steps.clone()
                    } else {
                        vec![]
                    };
                    let complete_tx = CompleteExchange::new(
                        this.branch_tx.pool().clone(),
                        this.files.clone(),
                        Arc::new(LedgerCashPostings::new()),
                        complete_steps,
                        Arc::new(ExchangeEffects::new(vec![])),
                    );
                    let completed = complete_tx
                        .run_in(tx, draft.id, &save_actor, now)
                        .await?;

                    sqlx::query(
                        "INSERT INTO transaction_files (transaction_id, kind, file_key) VALUES ($1, 'RECEIPT', $2) \
                         ON CONFLICT (transaction_id, kind) DO UPDATE SET file_key = EXCLUDED.file_key",
                    )
                    .bind(draft.id.0)
                    .bind(&completed.receipt.document_ref)
                    .execute(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

                    CustomerService::record_exchange_in(
                        tx,
                        &branch,
                        &form.customer_id,
                        &form.customer,
                        completed.transaction.document_ref.as_deref().unwrap_or(""),
                    )
                    .await?;

                    Ok(answer_from_completed(&completed))
                })
            })
            .await
    }
}
