//! TX-5 #873: `CompleteExchange` (design `docs/design/blueprint/quote-transaction.md` §4.5, §7; ledger
//! `ledger-inventory.md` §4.3): a CONFIRMED exchange is completed in the caller's transaction, in §09's order.
//!
//! Scaffold written by the up-front test writer (claude/07d3765f): the `[required]` names of #873 §7 and of
//! `membership-points.md` §6.2, with `todo!()` bodies. The implementer fills them.
//!
//! The exchange-effect seam (`EffectPhase` … `ExchangeEffects`) is XS-0's (#1375,
//! `api/src/application/workflows/exchange/effects.rs`); it is re-exported here under the names TX-5 used.

use std::sync::Arc;

use chrono::{DateTime, Days, NaiveDate, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Transaction};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::entities::receipt::receipt_html;
use xc_exchange::domain::exchange::repositories::transaction_repository::TransactionRepository;
use xc_exchange::domain::exchange::value_objects::transaction_form::TransactionForm;
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::domain::transaction::receipt_record::Receipt;
use xc_exchange::domain::transaction::repository::{LineCosts, TransactionStore};
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::shift::value_objects::shift_error::ShiftError;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use super::create::check_valid_in;
use crate::application::workflows::save_step::{
    BookingClaim, SaveActor, SaveContext, SaveExtras, SaveStep,
};
use crate::infrastructure::devices::device_service::DeviceService;
use crate::infrastructure::files::file_service::FileService;
use serde_json::json;
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_approval::infrastructure::audit_log::PgAuditLog;

// ------------------------------------------------------------ the effect seam (XS-0's, `effects.rs`)

pub use super::effects::{
    BoxFuture, EffectPhase, ExchangeCompletedFacts, ExchangeEffect, ExchangeEffects, PostedReversal,
};

// ------------------------------------------------------------ the completion

/// `[required]` (#873 §7): the completed exchange and its receipt (revision 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedExchange {
    pub transaction: ExchangeTransaction,
    pub receipt: Receipt,
}

pub struct CompleteExchange {
    store: PgTransactionStore,
    quotes: PgQuoteRepository,
    shifts: ShiftService,
    saved: PgTransactionRepository,
    files: FileService,
    postings: Arc<dyn CashPostings>,
    steps: Vec<Arc<dyn SaveStep>>,
    effects: Arc<ExchangeEffects>,
    audit: PgAuditLog,
}

fn stale<T>(m: String) -> AppResult<T> {
    Err(AppError::Domain(format!("staleVersion: {m}")))
}

/// Ledger §4.3: one exchange line as its two legs. The foreign leg names the line; the THB leg carries the line's
/// `settle_thb` (ruling Q6) at the drawer for cash and the bank otherwise. BUY: the foreign goes customer → drawer
/// and the baht the other way; SELL: the reverse.
fn legs(t: &ExchangeTransaction, shift: &str) -> Vec<PostingRequest> {
    let source = SourceRef {
        source_type: SourceType::Exchange,
        source_id: t.id.0.to_string(),
        source_key: "complete".to_owned(),
    };
    let drawer = CashPlace::Drawer(shift.to_owned());
    let thb_place = if t.payment.method.is_cash() {
        drawer.clone()
    } else {
        CashPlace::Bank
    };
    let mut reqs = Vec::with_capacity(t.lines.len() * 2);
    for line in &t.lines {
        let (foreign_from, foreign_to, thb_from, thb_to) = match line.shop_side {
            Side::Buy => (
                CashPlace::Customer,
                drawer.clone(),
                thb_place.clone(),
                CashPlace::Customer,
            ),
            Side::Sell => (
                drawer.clone(),
                CashPlace::Customer,
                CashPlace::Customer,
                thb_place.clone(),
            ),
        };
        reqs.push(PostingRequest {
            from: foreign_from,
            to: foreign_to,
            currency: line.currency,
            quantity: line.quantity,
            source: source.clone(),
            line: Some(line.id),
            reverses: None,
        });
        reqs.push(PostingRequest {
            from: thb_from,
            to: thb_to,
            currency: CurrencyCode::THB,
            quantity: ForeignAmount::from_decimal(Decimal::from_i128_with_scale(
                line.settle_thb.minor_i128(),
                2,
            )),
            source: source.clone(),
            line: None,
            reverses: None,
        });
    }
    reqs
}

/// The branch's name (printed on the receipt) and the end of `business_date` in its timezone (reversal R-1).
async fn branch_day_in(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    business_date: NaiveDate,
) -> AppResult<(String, DateTime<Utc>)> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT name, timezone FROM branches WHERE id = $1")
            .bind(branch.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;
    let Some((name, timezone)) = row else {
        return Err(AppError::Internal(format!(
            "branch {} does not exist",
            branch.as_str()
        )));
    };
    let tz: chrono_tz::Tz = timezone
        .parse()
        .map_err(|_| AppError::Internal(format!("branch timezone {timezone}")))?;
    let end = business_date
        .checked_add_days(Days::new(1))
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|d| tz.from_local_datetime(&d).earliest())
        .ok_or_else(|| AppError::Internal(format!("the end of {business_date}")))?;
    Ok((name, end.with_timezone(&Utc)))
}

impl CompleteExchange {
    /// `[required]` (#873 Q1, Q4, Q8): the posting port and the effects are injected, so a test passes its own;
    /// `files` writes the receipt (`FileService::store_receipt_in`, as today's save).
    /// The route passes `LedgerCashPostings` (LG-4's cutover, design §6.1), today's save steps, and
    /// `ExchangeEffects::new(vec![])`.
    pub fn new(
        pool: PgPool,
        files: FileService,
        postings: Arc<dyn CashPostings>,
        steps: Vec<Arc<dyn SaveStep>>,
        effects: Arc<ExchangeEffects>,
    ) -> Self {
        Self {
            store: PgTransactionStore::new(pool.clone()),
            quotes: PgQuoteRepository::new(pool.clone()),
            shifts: ShiftService::new(
                Arc::new(PgShiftRepository::new(pool.clone())),
                BranchTx::new(Transactor::new(pool.clone())),
            ),
            saved: PgTransactionRepository::new(pool.clone()),
            files,
            postings,
            steps,
            effects,
            audit: PgAuditLog::new(pool),
        }
    }

    /// `[required]` (#873 §7): design §4.5's order, in the caller's transaction. Every refusal before the posting
    /// writes nothing; a refusal from the posting, an effect or a step is the caller's to roll back.
    pub async fn run_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: TransactionId,
        who: &SaveActor,
        now: DateTime<Utc>,
    ) -> AppResult<CompletedExchange> {
        // 1. CONFIRMED only (the route's Idempotency-Key replays a repeat, CM-1)
        let Some(t) = self.store.find_in(tx, id).await? else {
            return stale(format!("transaction {} does not exist", id.0));
        };
        if t.status != TransactionStatus::Confirmed {
            return stale(format!("a {} exchange is not completed", t.status.as_str()));
        }

        // 2. LIVE: the exchange's shift is still the branch's open one; DEMO has none (§4.4 step 2)
        let shift = match t.mode {
            Mode::Live => {
                let open = self.shifts.active_in(tx, &t.branch_id).await?;
                match (open, t.shift_id.as_deref()) {
                    (Some(open), Some(shift)) if open.id == shift => Some(open.id),
                    _ => return Err(ShiftError::NoOpenShift.into()),
                }
            }
            Mode::Demo => None,
        };

        // 3. The ACCEPTED quote is still valid (D-5, D-6)
        let Some(quote_id) = t.quote_id else {
            return stale(format!("transaction {} has no quote", id.0));
        };
        let Some(q) = self.quotes.find_in(tx, quote_id).await? else {
            return stale(format!("quote {quote_id} does not exist"));
        };
        check_valid_in(tx, &q, now).await?;

        // 4. LIVE cash is counted first (the device step's COUNT_FIRST; DEMO is not checked, as today)
        if t.mode == Mode::Live {
            DeviceService::require_count_confirmed(t.payment.method.is_cash(), t.count_confirmed)?;
        }

        // 5. Both legs in one posting, never allowing negative stock (Q7); the costs it answers go on the lines
        if let Some(shift) = &shift {
            let posted = self.postings.post_in(tx, &legs(&t, shift), None).await?;
            let costs: Vec<LineCosts> = posted
                .costs
                .iter()
                .map(|c| LineCosts {
                    line_id: c.line,
                    cost_basis_thb: c.cost_basis_thb,
                    realized_margin_thb: c.realized_margin_thb,
                })
                .collect();
            self.store.record_line_costs_in(tx, id, &costs).await?;
        }

        // 6. Redeem the benefit, before COMPLETED (membership-points §6.2)
        let (branch_name, reversible_until) = branch_day_in(tx, &t.branch_id, t.value_date).await?;
        let facts = ExchangeCompletedFacts {
            organization_id: t.organization_id,
            branch_id: t.branch_id.clone(),
            transaction_id: id.0,
            quote_id,
            customer_id: (!t.customer_id.is_empty()).then(|| t.customer_id.clone()),
            mode: t.mode,
            completed_at: now,
            business_date: t.value_date,
            reversible_until,
        };
        self.effects
            .run_completed_in(tx, EffectPhase::Redeem, &facts)
            .await?;

        // 7. Mark COMPLETED (Q8): number → receipt file → COMPLETED with receipt revision 1
        let form = TransactionForm::from_transaction(&t);
        let document_ref = self.store.next_document_ref_in(tx, id, now).await?;
        let html = receipt_html(&form, &document_ref, &branch_name);
        let receipt_key = self
            .files
            .store_receipt_in(tx, &t.branch_id, &who.id, &html, now)
            .await?;
        let receipt = self
            .store
            .complete_numbered_in(tx, id, &document_ref, &receipt_key, now)
            .await?;

        // 8. Every step's after_in, unchanged (D-7), told the exchange as the save steps know it
        let Some((saved, _)) = self
            .saved
            .find_submission_in(tx, &t.branch_id, t.mode, &t.submission_id)
            .await?
        else {
            return Err(AppError::Internal(format!(
                "completed transaction {} does not read back",
                id.0
            )));
        };
        let extras = SaveExtras {
            staff_draft_id: t.staff_draft_id.clone(),
            count_confirmed: t.count_confirmed,
            kiosk_request_id: t.kiosk_request_id.clone(),
        };
        let booking = t.booking_id.clone().map(|booking_id| BookingClaim {
            booking_id,
            customer_confirmed: true,
        });
        let ctx = SaveContext {
            branch: &t.branch_id,
            form: &form,
            extras: &extras,
            booking: booking.as_ref(),
            actor: who,
            teller: None,
            now,
            prior: None,
        };
        for step in &self.steps {
            step.after_in(tx, &ctx, &saved).await?;
        }

        // 9. Write the outbox
        self.effects
            .run_completed_in(tx, EffectPhase::Outbox, &facts)
            .await?;

        let Some(transaction) = self.store.find_in(tx, id).await? else {
            return Err(AppError::Internal(format!(
                "completed transaction {} does not read back",
                id.0
            )));
        };
        let completed_at = transaction
            .completed_at
            .ok_or_else(|| AppError::Internal("completed transaction missing completed_at".into()))?
            .to_rfc3339();
        let doc_ref = transaction
            .document_ref
            .as_deref()
            .unwrap_or(document_ref.as_str());
        self.audit
            .append(
                tx,
                &AuditEvent {
                    actor_id: who.id.clone(),
                    actor_role: who.role.as_str().to_string(),
                    scope: "branch".to_string(),
                    action: "transaction.completed".to_string(),
                    object_ref: id.0.to_string(),
                    branch_id: Some(t.branch_id.as_str().to_string()),
                    before: Some(json!({ "status": "CONFIRMED" })),
                    after: Some(json!({
                        "status": "COMPLETED",
                        "documentRef": doc_ref,
                        "completedAt": completed_at,
                    })),
                },
            )
            .await?;
        Ok(CompletedExchange {
            transaction,
            receipt,
        })
    }
}
