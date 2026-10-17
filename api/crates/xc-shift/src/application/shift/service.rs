//! Shift service application service.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_kernel::shift_hook::{NoShiftLifecycleHook, ShiftLifecycleHook};

use crate::domain::shift::entities::open_shift_row::OpenShiftRow;
use crate::domain::shift::entities::shift::{Shift, ShiftStatus};
use crate::domain::shift::entities::shift_ledger::ShiftLedger;
use crate::domain::shift::entities::stock_movement::StockMovement;
use crate::domain::shift::repositories::shift_repository::ShiftRepository;
use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::shift_error::ShiftError;
use crate::domain::shift::value_objects::stock_delta::{StockDelta, StockError};
use crate::infrastructure::shift::shift_repository::PgShiftRepository;

pub struct ShiftService<R: ShiftRepository = PgShiftRepository> {
    repo: Arc<R>,
    branch_tx: BranchTx,
    lifecycle: Arc<dyn ShiftLifecycleHook>,
}

impl<R: ShiftRepository> Clone for ShiftService<R> {
    fn clone(&self) -> Self {
        Self {
            repo: Arc::clone(&self.repo),
            branch_tx: self.branch_tx.clone(),
            lifecycle: Arc::clone(&self.lifecycle),
        }
    }
}

impl<R: ShiftRepository> ShiftService<R> {
    pub fn new(repo: Arc<R>, branch_tx: BranchTx) -> Self {
        Self {
            repo,
            branch_tx,
            lifecycle: Arc::new(NoShiftLifecycleHook),
        }
    }

    /// The same service, telling `hook` when a teller's shift opens or closes (#1128).
    pub fn with_lifecycle_hook(self, hook: Arc<dyn ShiftLifecycleHook>) -> Self {
        Self {
            lifecycle: hook,
            ..self
        }
    }

    pub fn repo(&self) -> &Arc<R> {
        &self.repo
    }

    pub fn branch_tx(&self) -> &BranchTx {
        &self.branch_tx
    }

    pub async fn open_of_teller(&self, teller: &str) -> AppResult<Option<Shift>> {
        self.repo.open_of_teller(teller).await
    }

    pub async fn open_of_teller_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        teller: &str,
    ) -> AppResult<Option<Shift>> {
        self.repo.open_of_teller_in(tx, teller).await
    }

    pub async fn open_of_branch(&self, branch: &BranchId) -> AppResult<Vec<OpenShiftRow>> {
        self.repo.open_of_branch(branch).await
    }

    pub async fn shift(&self, branch: &BranchId, id: &str) -> AppResult<Option<Shift>> {
        self.repo.shift(branch, id).await
    }

    pub async fn shifts(&self, branch: &BranchId) -> AppResult<Vec<Shift>> {
        self.repo.shifts(branch).await
    }

    pub async fn movements(&self, branch: &BranchId) -> AppResult<Vec<StockMovement>> {
        self.repo.movements(branch).await
    }

    pub async fn apply_delta_for_teller_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        teller: &str,
        delta: &StockDelta,
    ) -> AppResult<String> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM shifts WHERE branch_id = $1 AND teller_id = $2::uuid AND status = 'OPEN'",
        )
        .bind(branch.as_str())
        .bind(teller)
        .fetch_optional(&mut **tx)
        .await?;

        let (shift_id,) = match row {
            Some(r) => r,
            None => return Err(ShiftError::NoOpenShift.into()),
        };

        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT code, expected::text FROM shift_balances WHERE shift_id = $1")
                .bind(&shift_id)
                .fetch_all(&mut **tx)
                .await?;

        let mut exp_pairs = Vec::new();
        for (code_str, exp_str) in rows {
            let code = CurrencyCode::parse(code_str.trim()).map_err(|_| {
                AppError::Internal(format!("undecodable shift_balances.code: {code_str}"))
            })?;
            let amt = crate::infrastructure::shift::shift_repository::parse_money_unbounded(
                "shift_balances",
                "expected",
                &exp_str,
            )?;
            if !amt.is_zero() {
                exp_pairs.push((code, amt));
            }
        }
        let current_expected = Balances::of(&exp_pairs);

        if let Err(e) = ShiftLedger::ensure_non_negative(&current_expected, delta) {
            return Err(match e {
                StockError::Insufficient(c) => ShiftError::Insufficient(c).into(),
                StockError::NotAStockCurrency => ShiftError::MovementInvalid.into(),
                StockError::OutOfRange => ShiftError::AmountInvalid.into(),
            });
        }

        let new_expected = ShiftLedger::expected(&current_expected, std::slice::from_ref(delta))
            .map_err(|e| match e {
                StockError::Insufficient(c) => ShiftError::Insufficient(c),
                StockError::NotAStockCurrency => ShiftError::MovementInvalid,
                StockError::OutOfRange => ShiftError::AmountInvalid,
            })?;

        self.repo
            .update_expected(tx, &shift_id, delta, &new_expected)
            .await?;

        Ok(shift_id)
    }

    pub async fn open_on_verified(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        teller_id: &str,
        count_id: &str,
        opening: &[(String, Decimal)],
        now: DateTime<Utc>,
    ) -> AppResult<Shift> {
        let existing_open: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'",
        )
        .bind(teller_id)
        .fetch_one(&mut **tx)
        .await?;

        if existing_open > 0 {
            return Err(ShiftError::ShiftAlreadyOpen.into());
        }

        let id = format!("SH-{}", uuid::Uuid::new_v4());
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', $3, $4, '', $5, $4::uuid)",
        )
        .bind(&id)
        .bind(branch.as_str())
        .bind(now)
        .bind(teller_id)
        .bind(count_id)
        .execute(&mut **tx)
        .await?;

        let mut pairs = Vec::new();
        for (code_str, amount) in opening {
            if !amount.is_zero() {
                let mut amt_norm = amount.normalize();
                if amt_norm.scale() < 2 {
                    amt_norm.rescale(2);
                }
                let amt_str = amt_norm.to_string();
                sqlx::query(
                    "INSERT INTO shift_balances (shift_id, code, opening, expected) \
                     VALUES ($1, $2, $3::numeric, $3::numeric)",
                )
                .bind(&id)
                .bind(code_str.as_str())
                .bind(&amt_str)
                .execute(&mut **tx)
                .await?;

                let code = CurrencyCode::parse(code_str.trim()).map_err(|_| {
                    AppError::Internal(format!("undecodable shift_balances.code: {code_str}"))
                })?;
                let money = crate::infrastructure::shift::shift_repository::parse_money_unbounded(
                    "shift_balances",
                    "opening",
                    &amt_str,
                )?;
                pairs.push((code, money));
            }
        }

        let balances = Balances::of(&pairs);

        Ok(Shift {
            id,
            branch_id: branch.clone(),
            status: ShiftStatus::Open,
            opened_at: now,
            opened_by: teller_id.to_owned(),
            opening: balances.clone(),
            expected: balances,
            closed_at: None,
            closed_by: None,
            counted: None,
            difference: None,
            note: String::new(),
            open_request_id: count_id.to_owned(),
            close_request_id: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn close_on_verified(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        shift_id: &str,
        closed_by: &str,
        close_balances: &[(String, Decimal, Decimal)],
        note: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE shifts \
             SET status = 'CLOSED', closed_at = $1, closed_by = $2, close_note = $3 \
             WHERE id = $4",
        )
        .bind(now)
        .bind(closed_by)
        .bind(note)
        .bind(shift_id)
        .execute(&mut **tx)
        .await?;

        for (code, cnt, diff) in close_balances {
            let cnt_str = cnt.to_string();
            let diff_str = diff.to_string();
            sqlx::query(
                "INSERT INTO shift_balances (shift_id, code, opening, expected, counted, difference) \
                 VALUES ($1, $2, 0, 0, $3::numeric, $4::numeric) \
                 ON CONFLICT (shift_id, code) DO UPDATE SET counted = $3::numeric, difference = $4::numeric",
            )
            .bind(shift_id)
            .bind(code.as_str())
            .bind(&cnt_str)
            .bind(&diff_str)
            .execute(&mut **tx)
            .await?;
        }

        sqlx::query(
            "UPDATE shift_balances \
             SET counted = COALESCE(counted, 0), difference = COALESCE(difference, 0 - expected) \
             WHERE shift_id = $1 AND counted IS NULL",
        )
        .bind(shift_id)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }
}
