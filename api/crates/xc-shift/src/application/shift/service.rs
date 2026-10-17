//! Shift service application service.

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, plain_decimal};
use xc_kernel::shift_hook::{NoShiftLifecycleHook, ShiftLifecycleHook};

use crate::domain::shift::commands::close_shift::CloseShift;
use crate::domain::shift::commands::move_stock::MoveStock;
use crate::domain::shift::commands::open_shift::OpenShift;
use crate::domain::shift::entities::open_shift_row::OpenShiftRow;
use crate::domain::shift::entities::shift::{Shift, ShiftStatus};
use crate::domain::shift::entities::shift_ledger::ShiftLedger;
use crate::domain::shift::entities::stock_movement::StockMovement;
use crate::domain::shift::repositories::shift_repository::ShiftRepository;
use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::shift_error::ShiftError;
use crate::domain::shift::value_objects::stock_delta::{StockDelta, StockError};

fn truncate_millis(t: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(t.timestamp_millis())
        .single()
        .unwrap_or(t)
}

fn verify_claimed_expected(
    claimed: &serde_json::Value,
    server: &Balances,
) -> Result<(), ShiftError> {
    let map = match claimed {
        serde_json::Value::Object(m) => m,
        serde_json::Value::Null => {
            for &code in CurrencyCode::all() {
                if !server.get(code).is_zero() {
                    return Err(ShiftError::ExpectedChanged);
                }
            }
            return Ok(());
        }
        _ => return Err(ShiftError::ExpectedChanged),
    };

    for &code in CurrencyCode::all() {
        let claimed_minor: i128 = match map.get(code.as_str()) {
            None | Some(serde_json::Value::Null) => 0,
            Some(serde_json::Value::Number(n)) => {
                let d = rust_decimal::Decimal::from_str(&n.to_string())
                    .map_err(|_| ShiftError::ExpectedChanged)?
                    .normalize();
                if d < rust_decimal::Decimal::ZERO || d.scale() > 2 {
                    return Err(ShiftError::ExpectedChanged);
                }
                let mut d = d;
                d.rescale(2);
                d.mantissa()
            }
            Some(serde_json::Value::String(s)) => {
                let trimmed = s.trim();
                // "250.000" claims 250 (#48 § 8.2 step 4): the zeros past the second
                // decimal are the written form, not the value. Dropping them keeps
                // 250.004 at scale 3 — a figure no balance can hold, never rounded.
                let d = plain_decimal(trimmed)
                    .ok_or(ShiftError::ExpectedChanged)?
                    .normalize();
                if d < rust_decimal::Decimal::ZERO || d.scale() > 2 {
                    return Err(ShiftError::ExpectedChanged);
                }
                let mut d = d;
                d.rescale(2);
                d.mantissa()
            }
            _ => return Err(ShiftError::ExpectedChanged),
        };

        if claimed_minor != server.get(code).minor_i128() {
            return Err(ShiftError::ExpectedChanged);
        }
    }
    Ok(())
}

use crate::infrastructure::shift::shift_repository::PgShiftRepository;

/// The `users.id` of the shift's own teller (`shifts.teller_id`, queue-tickets.md Q7), read in `tx`
/// after the shift row is written: the counter is the teller's, whoever acted.
async fn shift_teller_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    shift_id: &str,
) -> AppResult<String> {
    let teller: String = sqlx::query_scalar("SELECT teller_id::text FROM shifts WHERE id = $1")
        .bind(shift_id)
        .fetch_one(&mut **tx)
        .await?;
    Ok(teller)
}

pub struct ShiftService<R: ShiftRepository = PgShiftRepository> {
    repo: Arc<R>,
    branch_tx: BranchTx,
    /// What else happens when a teller's shift opens or closes (#1128): `open_in_as` /
    /// `close_in_as` tell it, in the shift's transaction.
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

    pub async fn open(
        &self,
        branch: &BranchId,
        cmd: OpenShift,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Shift> {
        if !self.repo.branch_exists(branch).await? {
            return Err(ShiftError::BranchNotFound.into());
        }

        let this = self.clone();
        let b = branch.clone();
        let actor = actor.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { this.open_in(tx, &b, cmd, &actor, now).await })
            })
            .await
    }

    pub async fn move_stock(
        &self,
        branch: &BranchId,
        cmd: MoveStock,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<StockMovement> {
        if !self.repo.branch_exists(branch).await? {
            return Err(ShiftError::BranchNotFound.into());
        }

        let this = self.clone();
        let b = branch.clone();
        let actor = actor.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { this.move_stock_in(tx, &b, cmd, &actor, now).await })
            })
            .await
    }

    pub async fn close(
        &self,
        branch: &BranchId,
        cmd: CloseShift,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Shift> {
        if !self.repo.branch_exists(branch).await? {
            return Err(ShiftError::BranchNotFound.into());
        }

        let this = self.clone();
        let b = branch.clone();
        let actor = actor.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { this.close_in(tx, &b, cmd, &actor, now).await })
            })
            .await
    }

    pub async fn open_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        cmd: OpenShift,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Shift> {
        let now = truncate_millis(now);

        if self
            .repo
            .is_request_abandoned(tx, branch, &cmd.request_id)
            .await?
        {
            return Err(ShiftError::RequestAbandoned.into());
        }

        if let Some(existing) = self
            .repo
            .find_shift_by_open_request_id(tx, branch, &cmd.request_id)
            .await?
        {
            if existing.opening == cmd.opening {
                return Ok(existing);
            } else {
                return Err(ShiftError::RequestChanged.into());
            }
        }

        if self.repo.active_in(tx, branch).await?.is_some() {
            return Err(ShiftError::ShiftAlreadyOpen.into());
        }

        let id = format!("SH-{}", uuid::Uuid::new_v4());
        let shift = Shift {
            id,
            branch_id: branch.clone(),
            status: ShiftStatus::Open,
            opened_at: now,
            opened_by: actor.to_owned(),
            opening: cmd.opening.clone(),
            expected: cmd.opening.clone(),
            closed_at: None,
            closed_by: None,
            counted: None,
            difference: None,
            note: String::new(),
            open_request_id: cmd.request_id,
            close_request_id: None,
        };

        self.repo.insert_shift(tx, &shift).await?;
        Ok(shift)
    }

    /// `open_in` for a caller signed in with a Blueprint session (#1128): `session_user` is the
    /// caller's `users.id`. With `Some`, the lifecycle hook is told, in `tx`, the `users.id` of the
    /// shift's own teller (`shifts.teller_id`, queue-tickets.md Q7), which can differ from the
    /// caller. `None` (a legacy caller with no session user) skips the hook.
    pub async fn open_in_as(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        cmd: OpenShift,
        actor: &str,
        session_user: Option<uuid::Uuid>,
        now: DateTime<Utc>,
    ) -> AppResult<Shift> {
        let shift = self.open_in(tx, branch, cmd, actor, now).await?;
        if session_user.is_some() {
            let teller = shift_teller_in(tx, &shift.id).await?;
            self.lifecycle.on_opened(tx, branch, &teller, now).await?;
        }
        Ok(shift)
    }

    pub async fn move_stock_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        cmd: MoveStock,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<StockMovement> {
        let now = truncate_millis(now);

        if self
            .repo
            .is_request_abandoned(tx, branch, &cmd.request_id)
            .await?
        {
            return Err(ShiftError::RequestAbandoned.into());
        }

        if let Some(existing) = self
            .repo
            .find_movement_by_request_id(tx, branch, &cmd.request_id)
            .await?
        {
            if existing.shift_id == cmd.shift_id
                && existing.currency == cmd.currency
                && existing.direction == cmd.direction
                && existing.amount == cmd.amount
                && existing.reason == cmd.reason
            {
                return Ok(existing);
            } else {
                return Err(ShiftError::RequestChanged.into());
            }
        }

        let active = self.repo.active_in(tx, branch).await?;
        let active_shift = match active {
            Some(s) if s.id == cmd.shift_id => s,
            _ => return Err(ShiftError::ShiftChanged.into()),
        };

        let delta = StockDelta::for_movement(cmd.currency, cmd.direction, cmd.amount);
        if let Err(e) = ShiftLedger::ensure_non_negative(&active_shift.expected, &delta) {
            return Err(match e {
                StockError::Insufficient(c) => ShiftError::Insufficient(c).into(),
                StockError::NotAStockCurrency => ShiftError::MovementInvalid.into(),
                StockError::OutOfRange => ShiftError::AmountInvalid.into(),
            });
        }

        let new_expected =
            ShiftLedger::expected(&active_shift.expected, std::slice::from_ref(&delta)).map_err(
                |e| match e {
                    StockError::Insufficient(c) => ShiftError::Insufficient(c),
                    StockError::NotAStockCurrency => ShiftError::MovementInvalid,
                    StockError::OutOfRange => ShiftError::AmountInvalid,
                },
            )?;

        let id = format!("MV-{}", uuid::Uuid::new_v4());
        let movement = StockMovement {
            id,
            shift_id: cmd.shift_id,
            at: now,
            by: actor.to_owned(),
            currency: cmd.currency,
            direction: cmd.direction,
            amount: cmd.amount,
            reason: cmd.reason,
            request_id: cmd.request_id,
        };

        self.repo.insert_movement(tx, branch, &movement).await?;
        self.repo
            .update_expected(tx, &active_shift.id, &delta, &new_expected)
            .await?;

        Ok(movement)
    }

    pub async fn close_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        cmd: CloseShift,
        actor: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Shift> {
        let now = truncate_millis(now);

        if self
            .repo
            .is_request_abandoned(tx, branch, &cmd.request_id)
            .await?
        {
            return Err(ShiftError::RequestAbandoned.into());
        }

        if let Some(existing) = self
            .repo
            .find_shift_by_close_request_id(tx, branch, &cmd.request_id)
            .await?
        {
            if existing.id == cmd.shift_id
                && existing.counted == Some(cmd.counted.clone())
                && existing.note == cmd.note
            {
                return Ok(existing);
            } else {
                return Err(ShiftError::RequestChanged.into());
            }
        }

        let active = self.repo.active_in(tx, branch).await?;
        let active_shift = match active {
            Some(s) if s.id == cmd.shift_id => s,
            _ => return Err(ShiftError::ShiftChanged.into()),
        };

        verify_claimed_expected(&cmd.claimed_expected, &active_shift.expected)?;

        let diff = ShiftLedger::difference(&active_shift.expected, &cmd.counted);
        if !diff.non_zero().is_empty() && cmd.note.is_empty() {
            return Err(ShiftError::DifferenceNeedsNote.into());
        }

        self.repo
            .close_shift(
                tx,
                &active_shift.id,
                now,
                actor,
                &cmd.counted,
                &diff,
                &cmd.note,
                &cmd.request_id,
            )
            .await?;

        let mut closed_shift = active_shift;
        closed_shift.status = ShiftStatus::Closed;
        closed_shift.closed_at = Some(now);
        closed_shift.closed_by = Some(actor.to_owned());
        closed_shift.counted = Some(cmd.counted);
        closed_shift.difference = Some(diff);
        closed_shift.note = cmd.note;
        closed_shift.close_request_id = Some(cmd.request_id);

        Ok(closed_shift)
    }

    /// `close_in` for a caller signed in with a Blueprint session (#1128): with `Some`, the
    /// lifecycle hook is told, in `tx`, the `users.id` of the closed shift's own teller
    /// (`shifts.teller_id`), not the caller's, so an admin closing a teller's shift releases the
    /// teller's counter. `None` skips the hook.
    pub async fn close_in_as(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        cmd: CloseShift,
        actor: &str,
        session_user: Option<uuid::Uuid>,
        now: DateTime<Utc>,
    ) -> AppResult<Shift> {
        let shift = self.close_in(tx, branch, cmd, actor, now).await?;
        if session_user.is_some() {
            let teller = shift_teller_in(tx, &shift.id).await?;
            self.lifecycle.on_closed(tx, branch, &teller, now).await?;
        }
        Ok(shift)
    }

    pub async fn apply_delta_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        delta: &StockDelta,
    ) -> AppResult<String> {
        let active = self.repo.active_in(tx, branch).await?;
        let active_shift = active.ok_or(ShiftError::NoOpenShift)?;

        if let Err(e) = ShiftLedger::ensure_non_negative(&active_shift.expected, delta) {
            return Err(match e {
                StockError::Insufficient(c) => ShiftError::Insufficient(c).into(),
                StockError::NotAStockCurrency => ShiftError::MovementInvalid.into(),
                StockError::OutOfRange => ShiftError::AmountInvalid.into(),
            });
        }

        let new_expected =
            ShiftLedger::expected(&active_shift.expected, std::slice::from_ref(delta)).map_err(
                |e| match e {
                    StockError::Insufficient(c) => ShiftError::Insufficient(c),
                    StockError::NotAStockCurrency => ShiftError::MovementInvalid,
                    StockError::OutOfRange => ShiftError::AmountInvalid,
                },
            )?;

        self.repo
            .update_expected(tx, &active_shift.id, delta, &new_expected)
            .await?;

        Ok(active_shift.id)
    }

    pub async fn active_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
    ) -> AppResult<Option<Shift>> {
        self.repo.active_in(tx, branch).await
    }

    pub async fn active(&self, branch: &BranchId) -> AppResult<Option<Shift>> {
        self.repo.active(branch).await
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
            "SELECT count(*) FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN' AND opened_by_count IS NOT NULL",
        )
        .bind(teller_id)
        .fetch_one(&mut **tx)
        .await?;

        if existing_open > 0 {
            return Err(ShiftError::ShiftAlreadyOpen.into());
        }

        let id = format!("SH-{}", uuid::Uuid::new_v4());
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id, opened_by_count) \
             VALUES ($1, $2, 'OPEN', $3, $4, '', $5, $4::uuid, $5::uuid)",
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
