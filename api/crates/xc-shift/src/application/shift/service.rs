//! Shift service application service.

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppResult;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::shift::commands::close_shift::CloseShift;
use crate::domain::shift::commands::move_stock::MoveStock;
use crate::domain::shift::commands::open_shift::OpenShift;
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
                    .map_err(|_| ShiftError::ExpectedChanged)?;
                if d < rust_decimal::Decimal::ZERO || d.scale() > 2 {
                    return Err(ShiftError::ExpectedChanged);
                }
                let mut d = d;
                d.rescale(2);
                d.mantissa()
            }
            Some(serde_json::Value::String(s)) => {
                let trimmed = s.trim();
                let d = rust_decimal::Decimal::from_str(trimmed)
                    .map_err(|_| ShiftError::ExpectedChanged)?;
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

pub struct ShiftService<R: ShiftRepository = PgShiftRepository> {
    repo: Arc<R>,
    branch_tx: BranchTx,
}

impl<R: ShiftRepository> Clone for ShiftService<R> {
    fn clone(&self) -> Self {
        Self {
            repo: Arc::clone(&self.repo),
            branch_tx: self.branch_tx.clone(),
        }
    }
}

impl<R: ShiftRepository> ShiftService<R> {
    pub fn new(repo: Arc<R>, branch_tx: BranchTx) -> Self {
        Self { repo, branch_tx }
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
            });
        }

        let new_expected =
            ShiftLedger::expected(&active_shift.expected, std::slice::from_ref(&delta)).map_err(
                |e| match e {
                    StockError::Insufficient(c) => ShiftError::Insufficient(c),
                    StockError::NotAStockCurrency => ShiftError::MovementInvalid,
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
            });
        }

        let new_expected =
            ShiftLedger::expected(&active_shift.expected, std::slice::from_ref(delta)).map_err(
                |e| match e {
                    StockError::Insufficient(c) => ShiftError::Insufficient(c),
                    StockError::NotAStockCurrency => ShiftError::MovementInvalid,
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

    pub async fn shifts(&self, branch: &BranchId) -> AppResult<Vec<Shift>> {
        self.repo.shifts(branch).await
    }

    pub async fn movements(&self, branch: &BranchId) -> AppResult<Vec<StockMovement>> {
        self.repo.movements(branch).await
    }
}
