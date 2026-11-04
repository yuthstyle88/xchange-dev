//! Backdated keying (`docs/design/backdated-keying.md` §2, §4–§7; the owner's rules R1–R4 and guard G of
//! 2026-11-04): one COMPLETED, backdated exchange keyed at its real past time, with no shift, at the typed rate, its
//! cash in the branch SAFE. Runs in the caller's transaction (the command seam's, under the branch lock).

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::AppResult;
use sqlx::PgPool;
use xc_exchange::domain::transaction::transaction::ExchangeTransaction;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side};

use super::create::CreatePayment;
use crate::application::workflows::save_step::SaveActor;
use crate::infrastructure::files::file_service::FileService;

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
        let _ = (&self.pool, &self.files, tx, req, who, now);
        todo!("BK-8: backdated-keying.md §2, §4–§7")
    }
}
