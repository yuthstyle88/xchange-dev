use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta};

use crate::domain::shift::entities::open_shift_row::OpenShiftRow;
use crate::domain::shift::entities::shift::{Shift, ShiftStatus};
use crate::domain::shift::entities::stock_movement::StockMovement;
use crate::domain::shift::repositories::shift_balances::{ShiftBalance, ShiftBalances};
use crate::domain::shift::repositories::shift_repository::ShiftRepository;
use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::shift_error::ShiftError;
use crate::domain::shift::value_objects::stock_delta::{Direction, StockDelta};

type ShiftRow = (
    String,                // id
    String,                // branch_id
    String,                // status
    DateTime<Utc>,         // opened_at
    String,                // opened_by
    Option<DateTime<Utc>>, // closed_at
    Option<String>,        // closed_by
    String,                // close_note
    String,                // open_request_id
    Option<String>,        // close_request_id
);

type MovementRow = (
    String,        // id
    String,        // shift_id
    DateTime<Utc>, // at
    String,        // by
    String,        // code
    String,        // direction
    String,        // amount
    String,        // reason
    String,        // request_id
);

type OpenShiftSqlRow = (
    String,         // id
    Option<String>, // teller_id
    Option<String>, // teller_name
    DateTime<Utc>,  // opened_at
);

fn undecodable(table: &str, column: &str, value: &str) -> AppError {
    AppError::Internal(format!("undecodable {table}.{column}: {value}"))
}

pub(crate) fn parse_money_unbounded(table: &str, column: &str, s: &str) -> Result<Money, AppError> {
    let trimmed = s.trim();
    if trimmed.starts_with('-') {
        return Err(undecodable(table, column, s));
    }
    let d = Decimal::from_str(trimmed).map_err(|_| undecodable(table, column, s))?;
    if d < Decimal::ZERO || d.normalize().scale() > 2 {
        return Err(undecodable(table, column, s));
    }
    let mut d = d.normalize();
    d.rescale(2);
    let minor = d.mantissa();
    MoneyDelta::from_minor(minor)
        .to_money()
        .map_err(|_| undecodable(table, column, s))
}

fn parse_direction(s: &str) -> Result<Direction, AppError> {
    match s.trim() {
        "IN" => Ok(Direction::In),
        "OUT" => Ok(Direction::Out),
        other => Err(undecodable("stock_movements", "direction", other)),
    }
}

fn parse_shift_status(s: &str) -> Result<ShiftStatus, AppError> {
    match s.trim() {
        "OPEN" => Ok(ShiftStatus::Open),
        "CLOSED" => Ok(ShiftStatus::Closed),
        other => Err(undecodable("shifts", "status", other)),
    }
}

macro_rules! select_shifts {
    ($tail:literal) => {
        concat!(
            "SELECT id, branch_id, status, opened_at, opened_by, closed_at, closed_by, close_note, \
             open_request_id, close_request_id FROM shifts ",
            $tail
        )
    };
}

fn build_shift(
    r: ShiftRow,
    status: ShiftStatus,
    (opening, expected, counted, difference): (
        Balances,
        Balances,
        Option<Balances>,
        Option<StockDelta>,
    ),
) -> Shift {
    Shift {
        id: r.0,
        branch_id: BranchId::new(r.1),
        status,
        opened_at: r.3,
        opened_by: r.4,
        opening,
        expected,
        closed_at: r.5,
        closed_by: r.6,
        counted,
        difference,
        note: r.7,
        open_request_id: r.8,
        close_request_id: r.9,
    }
}

fn map_movement_row(row: MovementRow) -> Result<StockMovement, AppError> {
    let (id, shift_id, at, by, code_str, dir_str, amount_str, reason, request_id) = row;
    let currency = CurrencyCode::parse(code_str.trim())
        .map_err(|_| undecodable("stock_movements", "code", &code_str))?;
    let direction = parse_direction(&dir_str)?;
    let amount = parse_money_unbounded("stock_movements", "amount", &amount_str)?;

    Ok(StockMovement {
        id,
        shift_id,
        at,
        by,
        currency,
        direction,
        amount,
        reason,
        request_id,
    })
}

pub(crate) async fn resolve_teller(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch_id: &BranchId,
    opened_by: &str,
) -> AppResult<String> {
    let org_id: Option<String> =
        sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = $1")
            .bind(branch_id.as_str())
            .fetch_optional(&mut **tx)
            .await?;

    let Some(org_id) = org_id else {
        return Err(ShiftError::BranchNotFound.into());
    };

    // 1. users.id (users.id::text = opened_by)
    let user_by_id: Option<String> = sqlx::query_scalar(
        "SELECT id::text FROM users WHERE organization_id = $1::uuid AND id::text = $2",
    )
    .bind(&org_id)
    .bind(opened_by)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(uid) = user_by_id {
        return Ok(uid);
    }

    // 2. employees.id -> users.legacy_employee_id
    let user_by_legacy: Option<String> = sqlx::query_scalar(
        "SELECT id::text FROM users WHERE organization_id = $1::uuid AND legacy_employee_id = $2",
    )
    .bind(&org_id)
    .bind(opened_by)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(uid) = user_by_legacy {
        return Ok(uid);
    }

    // 3. employees.username -> that employee's user
    let user_by_username: Option<String> = sqlx::query_scalar(
        "SELECT u.id::text FROM employees e \
         JOIN users u ON u.legacy_employee_id = e.id AND u.organization_id = $1::uuid \
         WHERE e.username = $2",
    )
    .bind(&org_id)
    .bind(opened_by)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(uid) = user_by_username {
        return Ok(uid);
    }

    // 4. "owner" or an owners.username -> the one organisation-wide EXECUTIVE + ADMIN user (legacy_employee_id IS NULL)
    let is_owner_str = opened_by == "owner";
    let is_owner_match = if is_owner_str {
        true
    } else {
        let in_owners: Option<bool> =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM owners WHERE username = $1)")
                .bind(opened_by)
                .fetch_optional(&mut **tx)
                .await?;
        in_owners.unwrap_or(false)
    };

    if is_owner_match {
        let owners: Vec<String> = sqlx::query_scalar(
            "SELECT u.id::text FROM users u \
             WHERE u.organization_id = $1::uuid \
               AND u.legacy_employee_id IS NULL \
               AND EXISTS ( \
                   SELECT 1 FROM user_roles r \
                   WHERE r.user_id = u.id \
                     AND r.organization_id = u.organization_id \
                     AND r.role_code = 'EXECUTIVE' \
                     AND r.branch_id IS NULL \
               ) \
               AND EXISTS ( \
                   SELECT 1 FROM user_roles r \
                   WHERE r.user_id = u.id \
                     AND r.organization_id = u.organization_id \
                     AND r.role_code = 'ADMIN' \
                     AND r.branch_id IS NULL \
               )",
        )
        .bind(&org_id)
        .fetch_all(&mut **tx)
        .await?;

        if owners.len() == 1 {
            return Ok(owners.into_iter().next().unwrap());
        } else {
            return Err(ShiftError::TellerUnknown.into());
        }
    }

    Err(ShiftError::TellerUnknown.into())
}

#[derive(Clone)]
pub struct PgShiftRepository {
    pool: PgPool,
    balances: Option<Arc<dyn ShiftBalances>>,
}

fn fold_ledger(
    shift_id: &str,
    rows: Vec<ShiftBalance>,
    status: ShiftStatus,
) -> AppResult<(Balances, Balances, Option<Balances>, Option<StockDelta>)> {
    let mut o = Vec::new();
    let mut e = Vec::new();
    let mut c = Vec::new();
    let mut d = Vec::new();
    for r in rows {
        if !r.opening.is_zero() {
            o.push((r.currency, r.opening));
        }
        if r.expected.is_negative() {
            return Err(AppError::Internal(format!(
                "shift {shift_id}: the ledger's expected {} is negative",
                r.currency.as_str()
            )));
        }
        let m = r
            .expected
            .to_money()
            .map_err(|_| AppError::Internal(format!("shift {shift_id}: expected out of range")))?;
        if !m.is_zero() {
            e.push((r.currency, m));
        }
        if let Some(x) = r.counted {
            if !x.is_zero() {
                c.push((r.currency, x));
            }
        }
        if let Some(x) = r.difference {
            if !x.is_zero() {
                d.push((r.currency, x));
            }
        }
    }
    let (counted, difference) = match status {
        ShiftStatus::Open => (None, None),
        ShiftStatus::Closed => (Some(Balances::of(&c)), Some(StockDelta::of(&d))),
    };
    Ok((Balances::of(&o), Balances::of(&e), counted, difference))
}

impl PgShiftRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            balances: None,
        }
    }

    pub fn with_balances(pool: PgPool, balances: Arc<dyn ShiftBalances>) -> Self {
        Self {
            pool,
            balances: Some(balances),
        }
    }

    fn port(&self) -> AppResult<&Arc<dyn ShiftBalances>> {
        self.balances
            .as_ref()
            .ok_or_else(|| AppError::Internal("PgShiftRepository has no ShiftBalances port".into()))
    }

    async fn shift_from_row(&self, r: ShiftRow) -> AppResult<Shift> {
        let status = parse_shift_status(&r.2)?;
        let rows = self.port()?.shift_balances(&r.0).await?;
        let balances = fold_ledger(&r.0, rows, status)?;
        Ok(build_shift(r, status, balances))
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

impl ShiftRepository for PgShiftRepository {
    async fn branch_exists(&self, branch: &BranchId) -> AppResult<bool> {
        let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM branches WHERE id = $1")
            .bind(branch.as_str())
            .fetch_optional(&self.pool)
            .await?;
        Ok(exists.is_some())
    }

    async fn open_of_teller(&self, teller: &str) -> AppResult<Option<Shift>> {
        let mut rows: Vec<ShiftRow> = sqlx::query_as(select_shifts!(
            "WHERE teller_id = $1::uuid AND status = 'OPEN'"
        ))
        .bind(teller)
        .fetch_all(&self.pool)
        .await?;

        if rows.len() > 1 {
            return Err(AppError::Internal(format!(
                "teller {teller} has {} OPEN shifts",
                rows.len()
            )));
        }

        match rows.pop() {
            Some(r) => Ok(Some(self.shift_from_row(r).await?)),
            None => Ok(None),
        }
    }

    async fn open_of_teller_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        teller: &str,
    ) -> AppResult<Option<Shift>> {
        let mut rows: Vec<ShiftRow> = sqlx::query_as(select_shifts!(
            "WHERE teller_id = $1::uuid AND status = 'OPEN'"
        ))
        .bind(teller)
        .fetch_all(&mut **tx)
        .await?;

        if rows.len() > 1 {
            return Err(AppError::Internal(format!(
                "teller {teller} has {} OPEN shifts",
                rows.len()
            )));
        }

        match rows.pop() {
            Some(r) => Ok(Some(self.shift_from_row(r).await?)),
            None => Ok(None),
        }
    }

    async fn open_of_branch(&self, branch: &BranchId) -> AppResult<Vec<OpenShiftRow>> {
        let rows: Vec<OpenShiftSqlRow> = sqlx::query_as(
            "SELECT s.id, s.teller_id::text, u.name, s.opened_at \
             FROM shifts s \
             LEFT JOIN users u ON u.id = s.teller_id \
             WHERE s.branch_id = $1 AND s.status = 'OPEN' \
             ORDER BY s.opened_at ASC, s.id ASC",
        )
        .bind(branch.as_str())
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for (shift_id, teller_id, teller_name, opened_at) in rows {
            let (Some(teller_id), Some(teller_name)) = (teller_id, teller_name) else {
                return Err(AppError::Internal(format!(
                    "OPEN shift {shift_id} has no teller user"
                )));
            };
            out.push(OpenShiftRow {
                shift_id,
                teller_id,
                teller_name,
                opened_at,
            });
        }
        Ok(out)
    }

    async fn shifts(&self, branch: &BranchId) -> AppResult<Vec<Shift>> {
        let rows: Vec<ShiftRow> =
            sqlx::query_as(select_shifts!("WHERE branch_id = $1 ORDER BY seq ASC"))
                .bind(branch.as_str())
                .fetch_all(&self.pool)
                .await?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let shift_ids: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
        let mut many = self.port()?.shift_balances_many(&shift_ids).await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let status = parse_shift_status(&r.2)?;
            let b = fold_ledger(&r.0, many.remove(&r.0).unwrap_or_default(), status)?;
            out.push(build_shift(r, status, b));
        }
        Ok(out)
    }

    async fn movements(&self, branch: &BranchId) -> AppResult<Vec<StockMovement>> {
        let rows: Vec<MovementRow> = sqlx::query_as(
            "SELECT id, shift_id, at, by, code, direction, amount::text, reason, request_id \
             FROM stock_movements WHERE branch_id = $1 ORDER BY seq ASC",
        )
        .bind(branch.as_str())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_movement_row).collect()
    }

    async fn shift(&self, branch: &BranchId, id: &str) -> AppResult<Option<Shift>> {
        let row: Option<ShiftRow> =
            sqlx::query_as(select_shifts!("WHERE branch_id = $1 AND id = $2"))
                .bind(branch.as_str())
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;

        match row {
            Some(r) => Ok(Some(self.shift_from_row(r).await?)),
            None => Ok(None),
        }
    }

    async fn shift_movements(
        &self,
        branch: &BranchId,
        shift_id: &str,
    ) -> AppResult<Vec<StockMovement>> {
        let rows: Vec<MovementRow> = sqlx::query_as(
            "SELECT id, shift_id, at, by, code, direction, amount::text, reason, request_id \
             FROM stock_movements WHERE branch_id = $1 AND shift_id = $2 ORDER BY seq ASC",
        )
        .bind(branch.as_str())
        .bind(shift_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(map_movement_row).collect()
    }

    async fn is_request_abandoned(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        request_id: &str,
    ) -> AppResult<bool> {
        let exists: Option<i32> = sqlx::query_scalar(
            "SELECT 1 FROM abandoned_requests WHERE branch_id = $1 AND request_id = $2",
        )
        .bind(branch.as_str())
        .bind(request_id)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(exists.is_some())
    }

    async fn find_shift_by_open_request_id(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        request_id: &str,
    ) -> AppResult<Option<Shift>> {
        let row: Option<ShiftRow> = sqlx::query_as(select_shifts!(
            "WHERE branch_id = $1 AND open_request_id = $2"
        ))
        .bind(branch.as_str())
        .bind(request_id)
        .fetch_optional(&mut **tx)
        .await?;

        match row {
            Some(r) => Ok(Some(self.shift_from_row(r).await?)),
            None => Ok(None),
        }
    }

    async fn find_shift_by_close_request_id(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        request_id: &str,
    ) -> AppResult<Option<Shift>> {
        let row: Option<ShiftRow> = sqlx::query_as(select_shifts!(
            "WHERE branch_id = $1 AND close_request_id = $2"
        ))
        .bind(branch.as_str())
        .bind(request_id)
        .fetch_optional(&mut **tx)
        .await?;

        match row {
            Some(r) => Ok(Some(self.shift_from_row(r).await?)),
            None => Ok(None),
        }
    }

    async fn insert_shift(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        shift: &Shift,
    ) -> AppResult<()> {
        let teller_id = resolve_teller(tx, &shift.branch_id, &shift.opened_by).await?;

        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', $3, $4, '', $5, $6::uuid)",
        )
        .bind(&shift.id)
        .bind(shift.branch_id.as_str())
        .bind(shift.opened_at)
        .bind(&shift.opened_by)
        .bind(&shift.open_request_id)
        .bind(&teller_id)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    async fn find_movement_by_request_id(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        request_id: &str,
    ) -> AppResult<Option<StockMovement>> {
        let row: Option<MovementRow> = sqlx::query_as(
            "SELECT id, shift_id, at, by, code, direction, amount::text, reason, request_id \
             FROM stock_movements WHERE branch_id = $1 AND request_id = $2",
        )
        .bind(branch.as_str())
        .bind(request_id)
        .fetch_optional(&mut **tx)
        .await?;

        row.map(map_movement_row).transpose()
    }

    async fn insert_movement(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        movement: &StockMovement,
    ) -> AppResult<()> {
        let dir_str = match movement.direction {
            Direction::In => "IN",
            Direction::Out => "OUT",
        };
        let amt_str = movement.amount.to_string();

        sqlx::query(
            "INSERT INTO stock_movements (id, branch_id, shift_id, code, direction, amount, reason, request_id, at, by) \
             VALUES ($1, $2, $3, $4, $5, $6::numeric, $7, $8, $9, $10)",
        )
        .bind(&movement.id)
        .bind(branch.as_str())
        .bind(&movement.shift_id)
        .bind(movement.currency.as_str())
        .bind(dir_str)
        .bind(&amt_str)
        .bind(&movement.reason)
        .bind(&movement.request_id)
        .bind(movement.at)
        .bind(&movement.by)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn close_shift(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        shift_id: &str,
        closed_at: DateTime<Utc>,
        closed_by: &str,
        counted: &Balances,
        difference: &StockDelta,
        note: &str,
        close_request_id: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE shifts \
             SET status = 'CLOSED', closed_at = $1, closed_by = $2, close_note = $3, close_request_id = $4 \
             WHERE id = $5",
        )
        .bind(closed_at)
        .bind(closed_by)
        .bind(note)
        .bind(close_request_id)
        .bind(shift_id)
        .execute(&mut **tx)
        .await?;

        let _ = (counted, difference);
        Ok(())
    }
}
