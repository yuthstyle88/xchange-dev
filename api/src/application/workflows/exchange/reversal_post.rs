//! RV-3 #684: `PostReversal` (design `docs/design/blueprint/reversal.md` §4.3; ledger `ledger-inventory.md` §4.6): an
//! APPROVED reversal is posted at the counter in the caller's transaction, in §09's order — the opposite posting at
//! the original's frozen cost, the benefit and Loyalty effects, the Broker feed, the reversal document, the mark.
//!
//! The exchange-effect seam (`ExchangeEffects`, `PostedReversal`) is XS-0's, declared in `complete.rs` until XS-0
//! merges (lane controller's ruling on #684, issuecomment-6095137406): RV-3 owns the `run_reversed_in` call here.
//!
//! Every refusal before the posting writes nothing; a refusal from the posting, an effect, the feed or the mark is
//! the caller's to roll back (the command's `BranchTx`). Nothing is written on the original's `transactions` row, its
//! lines or its receipt revision 1 (V-2).

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::reversal::repository::ReversalRepository;
use xc_exchange::domain::reversal::reversal::{
    PostedReversalRow, Reversal, ReversalId, ReversalStatus,
};
use xc_exchange::domain::reversal::window::{ReversalWindow, Tz};
use xc_exchange::domain::transaction::repository::TransactionStore;
use xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionId};
use xc_exchange::infrastructure::reversal::repository::PgReversalRepository;
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};
use xc_referral::application::feed_writer::ExchangeFeedWriter;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use super::complete::{EffectPhase, ExchangeEffects, PostedReversal};
use crate::application::workflows::save_step::SaveActor;
use crate::infrastructure::files::file_service::FileService;

/// `[required]` (#684 §7): posts an APPROVED reversal.
pub struct PostReversal {
    pool: PgPool,
    files: FileService,
    postings: Arc<dyn CashPostings>,
    effects: Arc<ExchangeEffects>,
}

/// A refusal the client reads by its code (§8), as TX-1..TX-5 answer: `"<code>: <why>"`.
fn refuse<T>(code: &str, why: impl std::fmt::Display) -> AppResult<T> {
    Err(AppError::Domain(format!("{code}: {why}")))
}

/// THB carried as a posting quantity (minor unit 2), as `complete.rs`'s legs carry `settle_thb`.
fn thb(minor: i128) -> ForeignAmount {
    ForeignAmount::from_decimal(Decimal::from_i128_with_scale(minor, 2))
}

/// The original's requests, reversed (design §4.3, ledger §4.6): each foreign leg goes the other way between the
/// customer and the poster's drawer, valued at the original line's frozen cost (`reverses = Some(line)`, `line =
/// None`); each THB leg goes the other way through the original payment's place (R-4: the drawer for cash, the bank
/// otherwise) and carries neither field.
fn reversed_legs(
    t: &ExchangeTransaction,
    reversal: ReversalId,
    shift: &str,
) -> Vec<PostingRequest> {
    let source = SourceRef {
        source_type: SourceType::Reversal,
        source_id: reversal.0.to_string(),
        source_key: "post".to_owned(),
    };
    let drawer = CashPlace::Drawer(shift.to_owned());
    let thb_place = if t.payment.method.is_cash() {
        drawer.clone()
    } else {
        CashPlace::Bank
    };
    let mut reqs = Vec::with_capacity(t.lines.len() * 2);
    for line in &t.lines {
        // The original BUY moved the foreign customer → drawer and the THB place → customer; the reversal undoes
        // both. The original SELL is the mirror.
        let (foreign_from, foreign_to, thb_from, thb_to) = match line.shop_side {
            Side::Buy => (
                drawer.clone(),
                CashPlace::Customer,
                CashPlace::Customer,
                thb_place.clone(),
            ),
            Side::Sell => (
                CashPlace::Customer,
                drawer.clone(),
                thb_place.clone(),
                CashPlace::Customer,
            ),
        };
        reqs.push(PostingRequest {
            from: foreign_from,
            to: foreign_to,
            currency: line.currency,
            quantity: line.quantity,
            source: source.clone(),
            line: None,
            reverses: Some(line.id),
        });
        reqs.push(PostingRequest {
            from: thb_from,
            to: thb_to,
            currency: CurrencyCode::THB,
            quantity: thb(line.settle_thb.minor_i128()),
            source: source.clone(),
            line: None,
            reverses: None,
        });
    }
    reqs
}

/// `text` safe inside HTML.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// THB minor units as `1,234.56`.
fn baht(minor: i128) -> String {
    let negative = minor < 0;
    let minor = minor.unsigned_abs();
    let whole = (minor / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!(
        "{}{grouped}.{:02}",
        if negative { "-" } else { "" },
        minor % 100
    )
}

/// The reversal document (V-4): a document of its own, never an edited receipt (R-2). It names the original receipt
/// it reverses, the lines given back and the THB returned through the original payment's way (R-4).
fn reversal_html(
    t: &ExchangeTransaction,
    reversal: &Reversal,
    number: &str,
    original_number: &str,
    branch_name: &str,
    posted_at: DateTime<Utc>,
) -> AppResult<String> {
    let mut lines = String::new();
    for line in &t.lines {
        let side = match line.shop_side {
            Side::Buy => "ร้านรับซื้อ (คืน)",
            Side::Sell => "ร้านขาย (คืน)",
        };
        lines.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            escape(side),
            escape(line.currency.as_str()),
            escape(&line.quantity.to_string()),
            baht(line.settle_thb.minor_i128()),
        ));
    }
    let way = match t.payment.method {
        PaymentMethod::Cash => "เงินสด",
        _ => "ช่องทางเดิมที่ชำระ",
    };
    let amount_minor = reversal.amount_thb.try_minor().map_err(|e| {
        AppError::Internal(format!("reversal document amount_thb overflows i64: {e}"))
    })?;
    Ok(format!(
        "<!doctype html><html lang=\"th\"><head><meta charset=\"utf-8\"><title>ใบกลับรายการ {number}</title>\
         <style>body{{font-family:sans-serif}}table{{border-collapse:collapse;width:100%}}\
         td,th{{border:1px solid #999;padding:4px}}.num{{text-align:right}}</style></head><body>\
         <h1>ใบกลับรายการ (Reversal)</h1>\
         <p>เลขที่ {number}</p>\
         <p>อ้างอิงใบเสร็จเดิม {original}</p>\
         <p>สาขา {branch}</p>\
         <p>วันที่ {at}</p>\
         <p>เหตุผล {reason}</p>\
         <table><thead><tr><th>รายการ</th><th>สกุลเงิน</th><th>จำนวน</th><th>บาท</th></tr></thead>\
         <tbody>{lines}</tbody></table>\
         <p>คืนเงิน {amount} บาท ทาง{way}</p>\
         </body></html>",
        number = escape(number),
        original = escape(original_number),
        branch = escape(branch_name),
        at = escape(&posted_at.to_rfc3339()),
        reason = escape(&reversal.reason),
        amount = baht(i128::from(amount_minor)),
        way = escape(way),
    ))
}

impl PostReversal {
    /// `[proposed]` (as `CompleteExchange::new`): the posting port and the effects are injected, so a test passes
    /// its own; `files` stores the reversal document's HTML (`FileService`, as the receipt).
    pub fn new(
        pool: PgPool,
        files: FileService,
        postings: Arc<dyn CashPostings>,
        effects: Arc<ExchangeEffects>,
    ) -> Self {
        Self {
            pool,
            files,
            postings,
            effects,
        }
    }

    /// `[required]` (#684 §7): validate, post, benefit, Broker feed, Loyalty event, document, mark — in that order,
    /// on `tx`. Answers what the reversal's effects were told.
    pub async fn run_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        id: ReversalId,
        who: &SaveActor,
        now: DateTime<Utc>,
    ) -> AppResult<PostedReversal> {
        let reversals = PgReversalRepository::new(self.pool.clone());
        let store = PgTransactionStore::new(self.pool.clone());
        let shifts = ShiftService::new(
            Arc::new(PgShiftRepository::new(self.pool.clone())),
            BranchTx::new(Transactor::new(self.pool.clone())),
        );

        // 1. Validate. The row is locked first, so two posts of one reversal queue behind each other and the
        //    second finds it POSTED (a repeat posts once).
        let locked: Option<String> =
            sqlx::query_scalar("SELECT status FROM reversals WHERE id = $1 FOR UPDATE")
                .bind(id.0)
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)?;
        if locked.is_none() {
            return refuse("staleVersion", format!("reversal {} does not exist", id.0));
        }
        let Some(reversal) = reversals.find_in(tx, id).await? else {
            return refuse("staleVersion", format!("reversal {} does not exist", id.0));
        };
        if reversal.status != ReversalStatus::Approved {
            return refuse(
                "staleVersion",
                format!("a {} reversal is not posted", reversal.status.as_str()),
            );
        }
        let original = TransactionId(reversal.original_transaction_id.0);
        let Some(t) = store.find_in(tx, original).await? else {
            return Err(AppError::Internal(format!(
                "the original {} of reversal {} does not exist",
                original.0, id.0
            )));
        };

        //    R-1: inside the original's business day, in the branch timezone.
        let (branch_name, zone) = branch_in(tx, &reversal.branch_id).await?;
        let tz = Tz::from_str(&zone)
            .map_err(|_| AppError::Internal(format!("branch timezone {zone}")))?;
        if !ReversalWindow::is_open(reversal.business_date, now, tz) {
            return refuse(
                "reversalWindowClosed",
                format!(
                    "the business day {} of the original has ended",
                    reversal.business_date
                ),
            );
        }

        //    The poster's own open shift at this branch (§4.3, #582 C1).
        let Some(shift_id) = posting_shift(tx, &reversal.branch_id, who, &shifts).await? else {
            return refuse(
                "noOpenShift",
                format!("branch {} has no open shift", reversal.branch_id.as_str()),
            );
        };

        // 2. The opposite posting at the original's frozen cost (V-6). A shortfall is the port's
        //    `stockInsufficient`; no approved NEGATIVE_STOCK is passed, as the completion passes none.
        let posted = self
            .postings
            .post_in(tx, &reversed_legs(&t, id, &shift_id), None)
            .await?;
        // Safe: MoneyDelta::minor() returns an i128, cannot wrap (ratchet floor, #2063 / #2862).
        let cost_difference_minor: i128 = posted
            .reversed
            .iter()
            .map(|r| r.difference_thb.minor())
            .sum();
        // Safe: MoneyDelta::minor() returns an i128, cannot wrap (ratchet floor, #2063 / #2862).
        let margin_minor: i128 = t
            .lines
            .iter()
            .filter_map(|l| l.realized_margin_thb)
            .map(|m| m.minor())
            .sum();

        let amount_thb_minor = reversal.amount_thb.try_minor().map_err(|e| {
            AppError::Internal(format!("reversal facts amount_thb overflows i64: {e}"))
        })?;

        let facts = PostedReversal {
            organization_id: t.organization_id,
            reversal_id: id.0,
            transaction_id: original.0,
            customer_id: (!t.customer_id.is_empty()).then(|| t.customer_id.clone()),
            mode: t.mode,
            posted_at: now,
            amount_thb: Decimal::new(amount_thb_minor, 2),
        };

        // 3. Benefit adjustment: the Redeem-phase effects (#560's, none until it registers one).
        self.effects
            .run_reversed_in(tx, EffectPhase::Redeem, &facts)
            .await?;

        // 4. Commission adjustment: the Broker feed. A reversal is always full (V-1), so the cumulative reversed
        //    amount is the feed row's own `value_satang` (broker-seam §6.3). An exchange with no feed row has nothing
        //    to adjust.
        let fed: Option<i64> = sqlx::query_scalar(
            "SELECT value_satang FROM broker_exchange_feed WHERE transaction_id = $1",
        )
        .bind(original.0)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        if let Some(value_satang) = fed {
            ExchangeFeedWriter::record_reversed(tx, original.0, value_satang, now).await?;
        }

        // 5. Member recalculation event: the Outbox-phase effects (#559's Loyalty handler registers here).
        self.effects
            .run_reversed_in(tx, EffectPhase::Outbox, &facts)
            .await?;

        // 6. The reversal document (V-4): receipts revision 2, kind REVERSAL, its own stored HTML.
        let Some(original_number) = t.document_ref.clone() else {
            return Err(AppError::Internal(format!(
                "the original {} has no document number",
                original.0
            )));
        };
        let number = format!("{original_number}-R2");
        let html = reversal_html(&t, &reversal, &number, &original_number, &branch_name, now)?;
        let document_ref = self
            .files
            .store_receipt_in(tx, &reversal.branch_id, &who.id, &html, now)
            .await?;
        let receipt_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, kind, issued_at, \
                document_ref, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, 2, 'REVERSAL', $5, $6, $5, $5)",
        )
        .bind(receipt_id)
        .bind(t.organization_id)
        .bind(original.0)
        .bind(&number)
        .bind(now)
        .bind(&document_ref)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        // 7. The mark.
        reversals
            .post_in(
                tx,
                id,
                &PostedReversalRow {
                    posting_id: posted.posting_id.0,
                    receipt_id,
                    shift_id: Some(shift_id),
                    posted_by: who.id.clone(),
                    posted_at: now,
                    realized_margin_reversed_thb: Some(Decimal::from_i128_with_scale(
                        -margin_minor,
                        2,
                    )),
                    cost_difference_thb: Some(Decimal::from_i128_with_scale(
                        cost_difference_minor,
                        2,
                    )),
                },
            )
            .await?;

        Ok(facts)
    }
}

async fn posting_shift(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    who: &SaveActor,
    shifts: &ShiftService,
) -> AppResult<Option<String>> {
    if let Ok(teller_id) = Uuid::parse_str(&who.id) {
        let own: Option<String> = sqlx::query_scalar(
            "SELECT id FROM shifts WHERE branch_id = $1 AND teller_id = $2 AND status = 'OPEN'",
        )
        .bind(branch.as_str())
        .bind(teller_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        if own.is_some() {
            return Ok(own);
        }
    }
    Ok(shifts.active_in(tx, branch).await?.map(|s| s.id))
}

/// The branch's name (printed on the document) and its timezone (R-1).
async fn branch_in(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
) -> AppResult<(String, String)> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT name, timezone FROM branches WHERE id = $1")
            .bind(branch.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;
    row.ok_or_else(|| AppError::Internal(format!("branch {} does not exist", branch.as_str())))
}
