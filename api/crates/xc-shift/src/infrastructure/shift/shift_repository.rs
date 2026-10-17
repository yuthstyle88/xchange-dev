use std::collections::HashMap;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta};

use crate::domain::shift::entities::open_shift_row::OpenShiftRow;
use crate::domain::shift::entities::shift::{Shift, ShiftStatus};
use crate::domain::shift::entities::stock_movement::StockMovement;
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

type BalanceRow = (
    String,         // code
    String,         // opening
    String,         // expected
    Option<String>, // counted
    Option<String>, // difference
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

fn format_delta(d: MoneyDelta) -> String {
    // Safe: MoneyDelta::minor() returns an i128, cannot wrap (ratchet floor, #2063 / #2857).
    let minor = d.minor();
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.unsigned_abs();
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
}

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

fn parse_delta_str(table: &str, column: &str, s: &str) -> Result<MoneyDelta, AppError> {
    let trimmed = s.trim();
    let (negative, num_str) = if let Some(rest) = trimmed.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = trimmed.strip_prefix('+') {
        (false, rest)
    } else {
        (false, trimmed)
    };
    if let Ok(d) = Decimal::from_str(num_str) {
        if d >= Decimal::ZERO && d.scale() <= 2 {
            let mut d = d;
            d.rescale(2);
            let minor = d.mantissa();
            return Ok(if negative {
                MoneyDelta::from_minor(-minor)
            } else {
                MoneyDelta::from_minor(minor)
            });
        }
    }
    Err(undecodable(table, column, s))
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

fn fold_balances(
    rows: Vec<BalanceRow>,
    status: ShiftStatus,
) -> AppResult<(Balances, Balances, Option<Balances>, Option<StockDelta>)> {
    let mut opening_pairs = Vec::new();
    let mut expected_pairs = Vec::new();
    let mut counted_pairs = Vec::new();
    let mut diff_pairs = Vec::new();

    for (code_str, opening_str, expected_str, counted_opt, diff_opt) in rows {
        let code = CurrencyCode::parse(code_str.trim())
            .map_err(|_| undecodable("shift_balances", "code", &code_str))?;

        let opening = parse_money_unbounded("shift_balances", "opening", &opening_str)?;
        if !opening.is_zero() {
            opening_pairs.push((code, opening));
        }

        let expected = parse_money_unbounded("shift_balances", "expected", &expected_str)?;
        if !expected.is_zero() {
            expected_pairs.push((code, expected));
        }

        if status == ShiftStatus::Closed {
            if let Some(c_str) = counted_opt {
                let counted = parse_money_unbounded("shift_balances", "counted", &c_str)?;
                if !counted.is_zero() {
                    counted_pairs.push((code, counted));
                }
            }
            if let Some(d_str) = diff_opt {
                let diff = parse_delta_str("shift_balances", "difference", &d_str)?;
                if !diff.is_zero() {
                    diff_pairs.push((code, diff));
                }
            }
        }
    }

    let opening = Balances::of(&opening_pairs);
    let expected = Balances::of(&expected_pairs);
    let (counted, difference) = match status {
        ShiftStatus::Open => (None, None),
        ShiftStatus::Closed => (
            Some(Balances::of(&counted_pairs)),
            Some(StockDelta::of(&diff_pairs)),
        ),
    };

    Ok((opening, expected, counted, difference))
}

async fn load_shift_balances<'e, E>(
    executor: E,
    shift_id: &str,
    status: ShiftStatus,
) -> AppResult<(Balances, Balances, Option<Balances>, Option<StockDelta>)>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let rows: Vec<BalanceRow> = sqlx::query_as(
        "SELECT code, opening::text, expected::text, counted::text, difference::text \
         FROM shift_balances WHERE shift_id = $1",
    )
    .bind(shift_id)
    .fetch_all(executor)
    .await?;

    fold_balances(rows, status)
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

async fn shift_from_row<'e, E>(executor: E, r: ShiftRow) -> AppResult<Shift>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    let status = parse_shift_status(&r.2)?;
    let balances = load_shift_balances(executor, &r.0, status).await?;
    Ok(build_shift(r, status, balances))
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

async fn resolve_teller(
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
}

impl PgShiftRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
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
            Some(r) => Ok(Some(shift_from_row(&self.pool, r).await?)),
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
            Some(r) => Ok(Some(shift_from_row(&mut **tx, r).await?)),
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
        type ShiftBalanceRow = (
            String,         // shift_id
            String,         // code
            String,         // opening
            String,         // expected
            Option<String>, // counted
            Option<String>, // difference
        );
        let balance_rows: Vec<ShiftBalanceRow> = sqlx::query_as(
            "SELECT shift_id, code, opening::text, expected::text, counted::text, difference::text \
             FROM shift_balances WHERE shift_id = ANY($1)",
        )
        .bind(&shift_ids)
        .fetch_all(&self.pool)
        .await?;

        let mut by_shift: HashMap<String, Vec<BalanceRow>> = HashMap::with_capacity(rows.len());
        for (shift_id, code, opening, expected, counted, difference) in balance_rows {
            by_shift
                .entry(shift_id)
                .or_default()
                .push((code, opening, expected, counted, difference));
        }

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let status = parse_shift_status(&r.2)?;
            let shift_balances = by_shift.remove(&r.0).unwrap_or_default();
            let balances = fold_balances(shift_balances, status)?;
            out.push(build_shift(r, status, balances));
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
            Some(r) => Ok(Some(shift_from_row(&self.pool, r).await?)),
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
            Some(r) => Ok(Some(shift_from_row(&mut **tx, r).await?)),
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
            Some(r) => Ok(Some(shift_from_row(&mut **tx, r).await?)),
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

        for (code, amount) in shift.opening.non_zero() {
            let amt_str = amount.to_string();
            sqlx::query(
                "INSERT INTO shift_balances (shift_id, code, opening, expected) \
                 VALUES ($1, $2, $3::numeric, $3::numeric)",
            )
            .bind(&shift.id)
            .bind(code.as_str())
            .bind(&amt_str)
            .execute(&mut **tx)
            .await?;
        }

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

    async fn update_expected(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        shift_id: &str,
        delta: &StockDelta,
        new_expected: &Balances,
    ) -> AppResult<()> {
        for (code, _d) in delta.non_zero() {
            let exp_str = new_expected.get(code).to_string();
            sqlx::query(
                "INSERT INTO shift_balances (shift_id, code, opening, expected) \
                 VALUES ($1, $2, 0, $3::numeric) \
                 ON CONFLICT (shift_id, code) DO UPDATE SET expected = $3::numeric",
            )
            .bind(shift_id)
            .bind(code.as_str())
            .bind(&exp_str)
            .execute(&mut **tx)
            .await?;
        }
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

        for &code in CurrencyCode::all() {
            let cnt = counted.get(code);
            let diff = difference.get(code);
            if !cnt.is_zero() || !diff.is_zero() {
                let cnt_str = cnt.to_string();
                let diff_str = format_delta(diff);
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
        }

        sqlx::query(
            "UPDATE shift_balances \
             SET counted = COALESCE(counted, 0), difference = COALESCE(difference, 0) \
             WHERE shift_id = $1",
        )
        .bind(shift_id)
        .execute(&mut **tx)
        .await?;

        Ok(())
    }
}
