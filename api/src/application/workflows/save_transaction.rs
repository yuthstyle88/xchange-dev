//! Save transaction workflow.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use xc_auth::domain::access::Actor;
use xc_exchange::application::customer::service::CustomerService;
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_exchange::domain::exchange::entities::receipt::receipt_html;
use xc_exchange::domain::exchange::entities::transaction::Transaction;
use xc_exchange::domain::exchange::value_objects::document_ref::DocumentRef;
use xc_exchange::domain::exchange::value_objects::transaction_form::{
    FILE_MISSING, TOO_MANY_DECIMALS, TransactionForm,
};
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{
    CompatRate, CurrencyCode, ExchangeArithmetic, ForeignAmount, Rate, Side, UnitSize,
};
use xc_rate::application::branch::service::BranchService;
use xc_rate::domain::publication::published_rates::PublishedRates;
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;

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
    let parts = [
        form.customer.name.clone(),
        form.customer.passport_no.clone(),
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

/// The OPEN shift of the teller whose `users.id` is `user_id`, in `branch`.
async fn teller_shift_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
    user_id: &str,
) -> AppResult<Option<String>> {
    sqlx::query_scalar(
        "SELECT id FROM shifts WHERE branch_id = $1 AND teller_id = $2::uuid AND status = 'OPEN'",
    )
    .bind(branch.as_str())
    .bind(user_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(AppError::Database)
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
        let branch_name = self.branches.require(branch).await?.name;
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

        let (this, branch, actor_id) = (self.clone(), branch.clone(), actor_id.to_owned());
        let lock = branch.clone();
        self.branch_tx
            .run(&lock, move |tx| {
                Box::pin(async move {
                    let mode = form.customer.mode;
                    let prior = this
                        .transactions
                        .find_submission_in(tx, &branch, mode, &form.submission_id)
                        .await?;

                    let mut stored_line_id: Option<uuid::Uuid> = None;
                    let mut stored_rate: Rate = form.rate;
                    let mut stored_unit_size: UnitSize = UnitSize::ONE;

                    if prior.is_none()
                        && let Some((extras, booking, _)) = &stepped
                    {
                            if let Some(req_id) =
                                extras.kiosk_request_id.as_deref().filter(|s| !s.is_empty())
                            {
                                let row: Option<(Decimal, Decimal, Option<uuid::Uuid>)> =
                                    sqlx::query_as(
                                        "SELECT rate, unit_size, published_rate_line_id FROM kiosk_requests WHERE branch_id = $1 AND id = $2",
                                    )
                                    .bind(branch.as_str())
                                    .bind(req_id)
                                    .fetch_optional(&mut **tx)
                                    .await
                                    .map_err(AppError::Database)?;

                                if let Some((req_rate, req_unit, req_line_id)) = row {
                                    let unit_size = UnitSize::from_decimal(req_unit)
                                        .map_err(|_| AppError::Domain(RATE_MISMATCH.to_owned()))?;
                                    let req_rate_vo = Rate::parse(&req_rate.normalize().to_string())
                                        .map_err(|_| AppError::Domain(RATE_MISMATCH.to_owned()))?;
                                    let req_per_one = CompatRate::per_one(req_rate_vo, unit_size)
                                        .map_err(|_| AppError::Domain(RATE_MISMATCH.to_owned()))?;
                                    if form.client_rate != req_per_one {
                                        return refuse(RATE_MISMATCH);
                                    }
                                    stored_line_id = req_line_id;
                                    stored_rate = req_rate_vo;
                                    stored_unit_size = unit_size;
                                }
                            }

                            if let Some(claim) = booking {
                                let row: Option<(Decimal, Decimal, Option<uuid::Uuid>)> =
                                    sqlx::query_as(
                                        "SELECT rate, unit_size, published_rate_line_id FROM bookings WHERE branch_id = $1 AND id = $2",
                                    )
                                    .bind(branch.as_str())
                                    .bind(&claim.booking_id)
                                    .fetch_optional(&mut **tx)
                                    .await
                                    .map_err(AppError::Database)?;

                                if let Some((bk_rate, bk_unit, bk_line_id)) = row {
                                    let unit_size = UnitSize::from_decimal(bk_unit)
                                        .map_err(|_| AppError::Domain(RATE_MISMATCH.to_owned()))?;
                                    let bk_rate_vo = Rate::parse(&bk_rate.normalize().to_string())
                                        .map_err(|_| AppError::Domain(RATE_MISMATCH.to_owned()))?;
                                    let bk_per_one = CompatRate::per_one(bk_rate_vo, unit_size)
                                        .map_err(|_| AppError::Domain(RATE_MISMATCH.to_owned()))?;
                                    if bk_line_id.is_some() && form.client_rate != bk_per_one {
                                        return refuse(RATE_MISMATCH);
                                    }
                                    stored_line_id = bk_line_id;
                                    stored_rate = bk_rate_vo;
                                    stored_unit_size = unit_size;
                                }
                            }
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
                            prior: prior.as_ref().map(|(saved, _)| saved),
                        };
                        for step in &this.steps {
                            step.before_in(tx, &ctx).await?;
                        }
                    }
                    CustomerService::confirm_in(tx, &branch, &form.customer_id, &form.customer)
                        .await?;
                    if let Some((saved, hash)) = &prior {
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

                    let mut shift_id = None;
                    if mode == Mode::Live {
                        // CA-7 (#623): a Blueprint-session teller posts to their own OPEN shift, never the
                        // branch's v2 one.
                        let open = match &teller {
                            Some(t) => teller_shift_in(tx, &branch, &t.user_id).await?.is_some(),
                            None => this.shifts.active_in(tx, &branch).await?.is_some(),
                        };
                        if !open {
                            return Err(ShiftError::NoOpenShift.into());
                        }
                        let code = CurrencyCode::parse(&form.currency).ok();
                        let vouched = stored_line_id.is_some()
                            || stepped.as_ref().is_some_and(|(extras, booking, actor)| {
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
                                this.steps.iter().any(|s| s.vouches_for_rate(&ctx))
                            });
                        if !vouched {
                            let Some(code) = code else {
                                return refuse(RATE_MISMATCH);
                            };
                            if code.is_thb() {
                                return refuse(RATE_MISMATCH);
                            }
                            let published_rates =
                                PgPublishedRates::new(this.shifts.branch_tx().pool().clone());
                            let published_side = published_rates
                                .line_in(tx, &branch, code, form.side)
                                .await?;
                            let Some(side) = published_side else {
                                return refuse(RATE_MISMATCH);
                            };

                            let per_one = CompatRate::per_one(side.rate, side.unit_size)
                                .map_err(|_| AppError::Domain(RATE_MISMATCH.to_owned()))?;
                            if form.client_rate != per_one {
                                return refuse(RATE_MISMATCH);
                            }

                            if side.stale {
                                let stale_policy =
                                    published_rates.stale_policy_in(tx, &side).await?;
                                let foreign = ForeignAmount::from_decimal(
                                    Decimal::from_i128_with_scale(
                                        form.amount.minor_i128(),
                                        2,
                                    ),
                                );
                                let thb_amount =
                                    ExchangeArithmetic::thb(foreign, side.rate, side.unit_size);
                                if stale_policy.admits(&side, thb_amount).is_err() {
                                    return refuse(RATE_MISMATCH);
                                }
                            }

                            stored_line_id = Some(side.line_id.as_uuid());
                            stored_rate = side.rate;
                            stored_unit_size = side.unit_size;
                        }
                        let Some(code) = code else {
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
                        shift_id = Some(match &teller {
                            Some(t) => {
                                this.shifts
                                    .apply_delta_for_teller_in(tx, &branch, &t.user_id, &delta)
                                    .await?
                            }
                            None => this.shifts.apply_delta_in(tx, &branch, &delta).await?,
                        });
                    }

                    let stored_reference_rate = if let Some(client_ref) = form.client_reference_rate {
                        let per_unit_ref = client_ref * stored_unit_size.value();
                        let parsed = Rate::parse(&per_unit_ref.normalize().to_string())
                            .map_err(|_| AppError::Domain(TOO_MANY_DECIMALS.to_owned()))?;
                        Some(parsed)
                    } else {
                        form.reference_rate
                    };

                    let org = PgTransactionRepository::organization_of_in(tx, &branch).await?;
                    let number = this
                        .transactions
                        .next_number_in(tx, &DocumentRef::counter_key(org, mode, now))
                        .await?;
                    let document_ref = DocumentRef::format(mode, now, number);
                    let html = receipt_html(&form, &document_ref, &branch_name);
                    let receipt_key = this
                        .files
                        .store_receipt_in(tx, &branch, &actor_id, &html, now)
                        .await?;
                    let saved = Transaction {
                        document_ref: document_ref.clone(),
                        branch_id: branch.clone(),
                        mode,
                        shift_id,
                        submission_id: form.submission_id.clone(),
                        customer_id: form.customer_id.clone(),
                        customer_name: form.customer.name.clone(),
                        nationality: form.customer.nationality.clone(),
                        side: form.side,
                        currency: form.currency.clone(),
                        amount: form.amount,
                        rate: stored_rate,
                        reference_rate: stored_reference_rate,
                        principal: form.quote.principal,
                        fee: form.fee,
                        bank_fee: form.bank_fee,
                        received: form.quote.received,
                        paid: form.quote.paid,
                        method: form.method,
                        payment_ref: form.payment_ref.clone(),
                        account: form.account.clone(),
                        staff: form.staff.clone(),
                        booking: form.booking.clone(),
                        notes: form.notes.clone(),
                        value_date: form.value_date,
                        paid_at: form.paid_at,
                        passport_key: passport.key,
                        slip_key: slip.map(|s| s.key),
                        receipt_key,
                        recorded_at: now,
                        recorded_by: actor_id.clone(),
                    };
                    this.transactions
                        .record_in(tx, &saved, &payload_hash)
                        .await?;
                    if let Some(line_id) = stored_line_id {
                        sqlx::query(
                            "UPDATE transactions SET published_rate_line_id = $1, unit_size = $2::numeric WHERE document_ref = $3 AND branch_id = $4",
                        )
                        .bind(line_id)
                        .bind(stored_unit_size.value())
                        .bind(&document_ref)
                        .bind(branch.as_str())
                        .execute(&mut **tx)
                        .await
                        .map_err(AppError::Database)?;
                        sqlx::query(
                            "UPDATE transaction_lines SET unit_size = $1::numeric \
                             WHERE transaction_id = (SELECT id FROM transactions WHERE document_ref = $2 AND branch_id = $3)",
                        )
                        .bind(stored_unit_size.value())
                        .bind(&document_ref)
                        .bind(branch.as_str())
                        .execute(&mut **tx)
                        .await
                        .map_err(AppError::Database)?;
                    } else if stored_unit_size != UnitSize::ONE {
                        sqlx::query(
                            "UPDATE transactions SET unit_size = $1::numeric WHERE document_ref = $2 AND branch_id = $3",
                        )
                        .bind(stored_unit_size.value())
                        .bind(&document_ref)
                        .bind(branch.as_str())
                        .execute(&mut **tx)
                        .await
                        .map_err(AppError::Database)?;
                        sqlx::query(
                            "UPDATE transaction_lines SET unit_size = $1::numeric \
                             WHERE transaction_id = (SELECT id FROM transactions WHERE document_ref = $2 AND branch_id = $3)",
                        )
                        .bind(stored_unit_size.value())
                        .bind(&document_ref)
                        .bind(branch.as_str())
                        .execute(&mut **tx)
                        .await
                        .map_err(AppError::Database)?;
                    }
                    CustomerService::record_exchange_in(
                        tx,
                        &branch,
                        &form.customer_id,
                        &form.customer,
                        &document_ref,
                    )
                    .await?;
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
                            step.after_in(tx, &ctx, &saved).await?;
                        }
                    }
                    Ok(answer(&saved))
                })
            })
            .await
    }
}
