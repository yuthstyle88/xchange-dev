//! Custody repository postgres implementation.

use std::collections::HashMap;

use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;

use super::decode::{
    attendance_from_row, currency_from_json, cutoff_from_json, entity_from_json,
    exception_from_row, hold_from_row, money_from_json, str_to_phase, str_to_role,
    target_from_json, time_from_json, undecodable,
};
use crate::domain::custody::branch_day::BranchDay;
use crate::domain::custody::entities::attendance::Attendance;
use crate::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use crate::domain::custody::entities::customer_hold::{CustomerHold, HoldStatus};
use crate::domain::custody::entities::stored_event::StoredEvent;
use crate::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
use crate::domain::custody::repositories::custody_repository::CustodyRepository;
use crate::domain::custody::state::CustodyState;
use crate::domain::custody::value_objects::cash_target::CashTarget;
use crate::domain::custody::value_objects::custody_event::EventKind;
use crate::domain::custody::value_objects::custody_role::CustodyRole;
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::cutoff::Cutoff;
use crate::domain::custody::value_objects::entity::Entity;
use crate::domain::custody::value_objects::phase::Phase;
use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::stock_delta::StockDelta;

fn truncate_millis(t: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(t.timestamp_millis())
        .single()
        .unwrap_or(t)
}

fn role_to_str(role: CustodyRole) -> &'static str {
    match role {
        CustodyRole::Owner => "OWNER",
        CustodyRole::Front => "FRONT",
        CustodyRole::Finance => "FINANCE",
        CustodyRole::Monitor => "MONITOR",
        CustodyRole::Manager => "MANAGER",
        CustodyRole::Backend => "BACKEND",
        CustodyRole::Other => "OTHER",
    }
}

fn str_to_event_kind(table: &str, field: &str, s: &str) -> AppResult<EventKind> {
    match s {
        "SET_ENTITY" => Ok(EventKind::SetEntity),
        "ENABLE" => Ok(EventKind::Enable),
        "CLOCK_IN" => Ok(EventKind::ClockIn),
        "CLOCK_OUT" => Ok(EventKind::ClockOut),
        "CASH_TARGET" => Ok(EventKind::CashTarget),
        "SERVICE_OPEN" => Ok(EventKind::ServiceOpen),
        "CUT_OFF" => Ok(EventKind::CutOff),
        "PROPOSE" => Ok(EventKind::Propose),
        "HOLD_CUSTOMER" => Ok(EventKind::HoldCustomer),
        "RELEASE_CUSTOMER" => Ok(EventKind::ReleaseCustomer),
        "REJECT" => Ok(EventKind::Reject),
        "SIGN" => Ok(EventKind::Sign),
        "DISPATCH" => Ok(EventKind::Dispatch),
        "COMMIT" => Ok(EventKind::Commit),
        "RECONCILE" => Ok(EventKind::Reconcile),
        other => Err(undecodable(table, field, other)),
    }
}

/// An amount in the smallest unit, as it is stored: a JSON integer while it
/// fits 64 bits, its decimal text beyond that (JSON numbers of that size do
/// not survive every reader).
fn minor_to_json(minor: i128) -> Value {
    match i64::try_from(minor) {
        Ok(n) => json!(n),
        Err(_) => Value::String(minor.to_string()),
    }
}

fn decimal_hundredths(mut d: rust_decimal::Decimal) -> i128 {
    d.rescale(2);
    d.mantissa()
}

fn balances_to_json(b: &Balances) -> Value {
    let mut map = serde_json::Map::new();
    for (code, money) in b.non_zero() {
        map.insert(
            code.as_str().to_string(),
            minor_to_json(decimal_hundredths(money.value())),
        );
    }
    Value::Object(map)
}

fn delta_to_json(d: &StockDelta) -> Value {
    let mut map = serde_json::Map::new();
    for (code, delta) in d.non_zero() {
        map.insert(
            code.as_str().to_string(),
            minor_to_json(decimal_hundredths(delta.value())),
        );
    }
    Value::Object(map)
}

fn target_to_json(target: &CashTarget) -> Value {
    json!({
        "amounts": balances_to_json(&target.amounts),
        "by": target.by,
        "at": target.at.epoch_millis()
    })
}

fn cutoff_to_json(cutoff: &Cutoff) -> Value {
    json!({
        "at": cutoff.at.epoch_millis(),
        "by": cutoff.by,
        "shiftId": cutoff.shift_id,
        "lastRef": cutoff.last_ref,
        "pending": cutoff.pending
    })
}

fn entity_to_json(e: &Entity) -> Value {
    json!({
        "company": e.company,
        "site": e.site,
        "confirmedBy": e.confirmed_by,
        "at": e.at.epoch_millis()
    })
}

// ── one row of each table, as it is stored ──────────────────────────────────

fn attendance_data(att: &Attendance) -> Value {
    json!({
        "role": role_to_str(att.role),
        "clockIn": att.clock_in.epoch_millis(),
        "clockOut": att.clock_out.map(|t| t.epoch_millis())
    })
}

fn hold_data(hold: &CustomerHold) -> Value {
    json!({
        "customerId": hold.customer_id,
        "reason": hold.reason,
        "by": hold.by,
        "at": hold.at.epoch_millis(),
        "status": match hold.status {
            HoldStatus::Open => "OPEN",
            HoldStatus::Released => "RELEASED",
        },
        "reviewedBy": hold.reviewed_by,
        "reviewedAt": hold.reviewed_at.map(|t| t.epoch_millis()),
        "note": hold.note
    })
}

fn draft_data(draft: &TxDraft) -> Value {
    json!({
        "shiftId": draft.shift_id,
        "employeeId": draft.employee_id,
        "side": match draft.side {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        },
        "currency": draft.currency.as_str(),
        "amount": minor_to_json(decimal_hundredths(draft.amount.value())),
        "at": draft.at.epoch_millis(),
        "status": match draft.status {
            DraftStatus::Open => "OPEN",
            DraftStatus::Completed => "COMPLETED",
        },
        "documentRef": draft.document_ref
    })
}

fn exception_data(exc: &CashException) -> Value {
    json!({
        "requestId": exc.request_id,
        "status": match exc.status {
            ExceptionStatus::Open => "OPEN",
            ExceptionStatus::Reviewed => "REVIEWED",
        },
        "targetDifference": exc.target_difference.as_ref().map(delta_to_json),
        "cashVariance": exc.cash_variance.as_ref().map(delta_to_json),
        "safeVariance": exc.safe_variance.as_ref().map(delta_to_json),
        "note": exc.note,
        "at": exc.at.epoch_millis(),
        "reviewedBy": exc.reviewed_by,
        "reviewedAt": exc.reviewed_at.map(|t| t.epoch_millis())
    })
}

/// Appends a cash exception to the branch's dense, 0-based exceptions list (CA-7d #1425).
pub async fn append_cash_exception_in(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    exception: &CashException,
) -> AppResult<()> {
    let position: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(position) + 1, 0) FROM cash_exceptions WHERE branch_id = $1",
    )
    .bind(branch.as_str())
    .fetch_one(&mut **tx)
    .await?;

    sqlx::query(
        "INSERT INTO cash_exceptions (id, branch_id, position, data) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(&exception.id)
    .bind(branch.as_str())
    .bind(position)
    .bind(exception_data(exception))
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Moves the branch day to READY inside the OPEN-count verify transaction when the day reads CLOSED before the new
/// shift is inserted (CA-7e #1911, design §4.2). Writes only `ops_state.phase`; leaves every other column untouched.
pub async fn day_ready_on_open_count_in(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
) -> AppResult<()> {
    let stored_raw: Option<String> =
        sqlx::query_scalar("SELECT phase FROM ops_state WHERE branch_id = $1")
            .bind(branch.as_str())
            .fetch_optional(&mut **tx)
            .await?;
    let stored = match stored_raw.as_deref() {
        Some(s) => str_to_phase("ops_state", "phase", s)?,
        None => Phase::Closed,
    };

    let open_shifts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM shifts WHERE branch_id = $1 AND status = 'OPEN'")
            .bind(branch.as_str())
            .fetch_one(&mut **tx)
            .await?;
    let open_shifts_before = usize::try_from(open_shifts).unwrap_or(0);

    if let Some(Phase::Ready) = BranchDay::ready_on_open_count(stored, open_shifts_before) {
        sqlx::query(
            "INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'READY') \
             ON CONFLICT (branch_id) DO UPDATE SET phase = EXCLUDED.phase",
        )
        .bind(branch.as_str())
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

/// The items of `after` that have to be written, with their position: those
/// `before` does not hold under the same id, holds at another position, or
/// holds with other values. Each comes with the item it replaces, if any.
fn to_write<'a, T: PartialEq>(
    before: &'a [T],
    after: &'a [T],
    id: impl Fn(&T) -> &str,
) -> Vec<(usize, &'a T, Option<&'a T>)> {
    let was: HashMap<&str, (usize, &T)> = before
        .iter()
        .enumerate()
        .map(|(pos, item)| (id(item), (pos, item)))
        .collect();
    after
        .iter()
        .enumerate()
        .filter_map(|(pos, item)| match was.get(id(item)) {
            Some((was_pos, was_item)) if *was_pos == pos && *was_item == item => None,
            Some((_, was_item)) => Some((pos, item, Some(*was_item))),
            None => Some((pos, item, None)),
        })
        .collect()
}

#[derive(Clone)]
pub struct PgCustodyRepository {
    pool: PgPool,
}

impl PgCustodyRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

async fn fetch_state(conn: &mut PgConnection, branch: &BranchId) -> AppResult<CustodyState> {
    let ops_row = sqlx::query(
        "SELECT phase, entity, target, cutoff, service_opened_at \
         FROM ops_state WHERE branch_id = $1",
    )
    .bind(branch.as_str())
    .fetch_optional(&mut *conn)
    .await?;

    let ops_row = match ops_row {
        Some(r) => r,
        None => return Ok(CustodyState::initial()),
    };

    let phase_str: String = ops_row.get(0);
    let phase = str_to_phase("ops_state", "phase", &phase_str)?;
    let entity: Option<Entity> = match ops_row.get::<Option<Value>, _>(1) {
        Some(v) if !v.is_null() => Some(entity_from_json("ops_state", "entity", &v)?),
        _ => None,
    };
    let target: Option<CashTarget> = match ops_row.get::<Option<Value>, _>(2) {
        Some(v) if !v.is_null() => Some(target_from_json("ops_state", "target", &v)?),
        _ => None,
    };
    let cutoff: Option<Cutoff> = match ops_row.get::<Option<Value>, _>(3) {
        Some(v) if !v.is_null() => Some(cutoff_from_json("ops_state", "cutoff", &v)?),
        _ => None,
    };
    let service_opened_at: Option<CustodyTime> = ops_row
        .get::<Option<DateTime<Utc>>, _>(4)
        .map(|t| CustodyTime::from_epoch_millis(t.timestamp_millis()));

    // attendance
    let att_rows = sqlx::query(
        "SELECT id, employee_id, data \
         FROM attendance WHERE branch_id = $1 ORDER BY position ASC",
    )
    .bind(branch.as_str())
    .fetch_all(&mut *conn)
    .await?;

    let mut attendance = Vec::with_capacity(att_rows.len());
    for r in &att_rows {
        attendance.push(attendance_from_row(r)?);
    }

    // customer_holds
    let hold_rows = sqlx::query(
        "SELECT id, data \
         FROM customer_holds WHERE branch_id = $1 ORDER BY position ASC",
    )
    .bind(branch.as_str())
    .fetch_all(&mut *conn)
    .await?;

    let mut holds = Vec::with_capacity(hold_rows.len());
    for r in &hold_rows {
        holds.push(hold_from_row(r)?);
    }

    // tx_drafts
    let draft_rows = sqlx::query(
        "SELECT id, data \
         FROM tx_drafts WHERE branch_id = $1 ORDER BY position ASC",
    )
    .bind(branch.as_str())
    .fetch_all(&mut *conn)
    .await?;

    let mut drafts = Vec::with_capacity(draft_rows.len());
    for r in draft_rows {
        let id: String = r.get(0);
        let data: Value = r.get(1);

        let shift_id = match data.get("shiftId") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => return Err(undecodable("tx_drafts", "shiftId", other)),
            None => return Err(undecodable("tx_drafts", "shiftId", "absent")),
        };
        let employee_id = match data.get("employeeId") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => return Err(undecodable("tx_drafts", "employeeId", other)),
            None => return Err(undecodable("tx_drafts", "employeeId", "absent")),
        };
        let side = match data.get("side") {
            Some(Value::String(s)) => match s.as_str() {
                "BUY" => Side::Buy,
                "SELL" => Side::Sell,
                other => return Err(undecodable("tx_drafts", "side", other)),
            },
            Some(other) => return Err(undecodable("tx_drafts", "side", other)),
            None => return Err(undecodable("tx_drafts", "side", "absent")),
        };
        let currency = match data.get("currency") {
            Some(v) => currency_from_json("tx_drafts", "currency", v)?,
            None => return Err(undecodable("tx_drafts", "currency", "absent")),
        };
        let amount = match data.get("amount") {
            Some(v) => money_from_json("tx_drafts", "amount", v)?,
            None => return Err(undecodable("tx_drafts", "amount", "absent")),
        };
        let at = match data.get("at") {
            Some(v) => time_from_json("tx_drafts", "at", v)?,
            None => return Err(undecodable("tx_drafts", "at", "absent")),
        };
        let status = match data.get("status") {
            Some(Value::String(s)) => match s.as_str() {
                "OPEN" => DraftStatus::Open,
                "COMPLETED" => DraftStatus::Completed,
                other => return Err(undecodable("tx_drafts", "status", other)),
            },
            Some(other) => return Err(undecodable("tx_drafts", "status", other)),
            None => return Err(undecodable("tx_drafts", "status", "absent")),
        };
        let document_ref = match data.get("documentRef") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(other) => return Err(undecodable("tx_drafts", "documentRef", other)),
        };

        drafts.push(TxDraft {
            id,
            shift_id,
            employee_id,
            side,
            currency,
            amount,
            at,
            status,
            document_ref,
        });
    }

    // cash_exceptions
    let exc_rows = sqlx::query(
        "SELECT id, data \
         FROM cash_exceptions WHERE branch_id = $1 ORDER BY position ASC",
    )
    .bind(branch.as_str())
    .fetch_all(&mut *conn)
    .await?;

    let mut exceptions = Vec::with_capacity(exc_rows.len());
    for r in &exc_rows {
        exceptions.push(exception_from_row(r)?);
    }

    Ok(CustodyState {
        phase,
        target,
        entity,
        attendance,
        cutoff,
        exceptions,
        holds,
        drafts,
        service_opened_at,
    })
}

impl CustodyRepository for PgCustodyRepository {
    fn branch_exists<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM branches WHERE id = $1)")
                    .bind(branch.as_str())
                    .fetch_one(&self.pool)
                    .await?;
            Ok(exists)
        })
    }

    fn load_state<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<CustodyState>> {
        Box::pin(async move {
            let mut conn = self.pool.acquire().await?;
            fetch_state(&mut conn, branch).await
        })
    }

    fn load_state_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<CustodyState>> {
        Box::pin(async move { fetch_state(tx, branch).await })
    }

    fn store_state_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        before: &'a CustodyState,
        after: &'a CustodyState,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if before == after {
                return Ok(());
            }

            // The branch's one row: written when one of its own values changed,
            // and with the first thing ever stored for the branch (a state that
            // was never stored loads as `initial`).
            let own_values_changed = before.phase != after.phase
                || before.entity != after.entity
                || before.target != after.target
                || before.cutoff != after.cutoff
                || before.service_opened_at != after.service_opened_at;
            if own_values_changed || *before == CustodyState::initial() {
                let service_opened_at = match after.service_opened_at {
                    Some(t) => {
                        let dt =
                            DateTime::from_timestamp_millis(t.epoch_millis()).ok_or_else(|| {
                                AppError::Internal(format!(
                                    "service_opened_at time out of range: {}",
                                    t.epoch_millis()
                                ))
                            })?;
                        Some(truncate_millis(dt))
                    }
                    None => None,
                };
                sqlx::query(
                    "INSERT INTO ops_state (branch_id, phase, entity, target, cutoff, service_opened_at) \
                     VALUES ($1, $2, $3, $4, $5, $6) \
                     ON CONFLICT (branch_id) DO UPDATE SET \
                       phase = EXCLUDED.phase, \
                       entity = EXCLUDED.entity, \
                       target = EXCLUDED.target, \
                       cutoff = EXCLUDED.cutoff, \
                       service_opened_at = EXCLUDED.service_opened_at",
                )
                .bind(branch.as_str())
                .bind(after.phase.as_str())
                .bind(after.entity.as_ref().map(entity_to_json))
                .bind(after.target.as_ref().map(target_to_json))
                .bind(after.cutoff.as_ref().map(cutoff_to_json))
                .bind(service_opened_at)
                .execute(&mut **tx)
                .await?;
            }

            for (pos, att, _) in to_write(&before.attendance, &after.attendance, |a| &a.id) {
                sqlx::query(
                    "INSERT INTO attendance (id, branch_id, position, employee_id, data) \
                     VALUES ($1, $2, $3, $4, $5) \
                     ON CONFLICT (id) DO UPDATE SET \
                       position = EXCLUDED.position, \
                       employee_id = EXCLUDED.employee_id, \
                       data = EXCLUDED.data",
                )
                .bind(&att.id)
                .bind(branch.as_str())
                .bind(pos as i32)
                .bind(&att.employee_id)
                .bind(attendance_data(att))
                .execute(&mut **tx)
                .await?;
            }

            for (pos, hold, _) in to_write(&before.holds, &after.holds, |h| &h.id) {
                sqlx::query(
                    "INSERT INTO customer_holds (id, branch_id, position, data) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (id) DO UPDATE SET \
                       position = EXCLUDED.position, \
                       data = EXCLUDED.data",
                )
                .bind(&hold.id)
                .bind(branch.as_str())
                .bind(pos as i32)
                .bind(hold_data(hold))
                .execute(&mut **tx)
                .await?;
            }

            for (pos, draft, _) in to_write(&before.drafts, &after.drafts, |d| &d.id) {
                sqlx::query(
                    "INSERT INTO tx_drafts (id, branch_id, position, data) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (id) DO UPDATE SET \
                       position = EXCLUDED.position, \
                       data = EXCLUDED.data",
                )
                .bind(&draft.id)
                .bind(branch.as_str())
                .bind(pos as i32)
                .bind(draft_data(draft))
                .execute(&mut **tx)
                .await?;
            }

            for (pos, exc, _) in to_write(&before.exceptions, &after.exceptions, |e| &e.id) {
                sqlx::query(
                    "INSERT INTO cash_exceptions (id, branch_id, position, data) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (id) DO UPDATE SET \
                       position = EXCLUDED.position, \
                       data = EXCLUDED.data",
                )
                .bind(&exc.id)
                .bind(branch.as_str())
                .bind(pos as i32)
                .bind(exception_data(exc))
                .execute(&mut **tx)
                .await?;
            }

            Ok(())
        })
    }

    fn append_events_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        events: &'a [StoredEvent],
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            for ev in events {
                let ev_at = truncate_millis(ev.at);
                sqlx::query(
                    "INSERT INTO ops_events (id, branch_id, kind, at, actor, role, details) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                )
                .bind(&ev.id)
                .bind(branch.as_str())
                .bind(ev.kind.as_str())
                .bind(ev_at)
                .bind(&ev.actor)
                .bind(role_to_str(ev.role))
                .bind(&ev.details)
                .execute(&mut **tx)
                .await?;
            }
            Ok(())
        })
    }

    fn list_events<'a>(
        &'a self,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Vec<StoredEvent>>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, kind, at, actor, role, details \
                 FROM ops_events WHERE branch_id = $1 ORDER BY seq ASC",
            )
            .bind(branch.as_str())
            .fetch_all(&self.pool)
            .await?;

            let mut events = Vec::with_capacity(rows.len());
            for r in rows {
                let id: String = r.get(0);
                let kind_str: String = r.get(1);
                let at: DateTime<Utc> = r.get(2);
                let actor: String = r.get(3);
                let role_str: String = r.get(4);
                let details: Value = r.get(5);

                events.push(StoredEvent {
                    id,
                    kind: str_to_event_kind("ops_events", "kind", &kind_str)?,
                    at,
                    actor,
                    role: str_to_role("ops_events", "role", &role_str)?,
                    details,
                });
            }
            Ok(events)
        })
    }
}
