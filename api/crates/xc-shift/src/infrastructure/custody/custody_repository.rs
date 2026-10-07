//! Custody repository postgres implementation.

use std::collections::HashMap;

use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Postgres, Row, Transaction};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta, Side};

use crate::domain::custody::entities::attendance::Attendance;
use crate::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use crate::domain::custody::entities::custody_request::{CustodyRequest, Decision};
use crate::domain::custody::entities::customer_hold::{CustomerHold, HoldStatus};
use crate::domain::custody::entities::stored_event::StoredEvent;
use crate::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
use crate::domain::custody::repositories::custody_repository::CustodyRepository;
use crate::domain::custody::state::CustodyState;
use crate::domain::custody::value_objects::cash_target::CashTarget;
use crate::domain::custody::value_objects::count_sheet::CountSheet;
use crate::domain::custody::value_objects::custody_event::EventKind;
use crate::domain::custody::value_objects::custody_role::CustodyRole;
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::cutoff::Cutoff;
use crate::domain::custody::value_objects::denomination_row::DenominationRow;
use crate::domain::custody::value_objects::entity::Entity;
use crate::domain::custody::value_objects::phase::Phase;
use crate::domain::custody::value_objects::request_kind::RequestKind;
use crate::domain::custody::value_objects::request_status::{
    Reconciliation, RequestStatus, TransferStage,
};
use crate::domain::custody::value_objects::signature::Signature;
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

fn undecodable(table: &str, field: &str, value: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!(
        "{table}.{field} holds an undecodable value: {value}"
    ))
}

fn str_to_role(table: &str, field: &str, s: &str) -> AppResult<CustodyRole> {
    match s {
        "OWNER" => Ok(CustodyRole::Owner),
        "FRONT" => Ok(CustodyRole::Front),
        "FINANCE" => Ok(CustodyRole::Finance),
        "MONITOR" => Ok(CustodyRole::Monitor),
        "MANAGER" => Ok(CustodyRole::Manager),
        "BACKEND" => Ok(CustodyRole::Backend),
        "OTHER" => Ok(CustodyRole::Other),
        other => Err(undecodable(table, field, other)),
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

fn str_to_phase(table: &str, field: &str, s: &str) -> AppResult<Phase> {
    match s {
        "CLOSED" => Ok(Phase::Closed),
        "READY" => Ok(Phase::Ready),
        "OPEN" => Ok(Phase::Open),
        "CUT_OFF" => Ok(Phase::CutOff),
        "COUNTING" => Ok(Phase::Counting),
        other => Err(undecodable(table, field, other)),
    }
}

fn str_to_request_kind(table: &str, field: &str, s: &str) -> AppResult<RequestKind> {
    match s {
        "OPEN" => Ok(RequestKind::Open),
        "HANDOVER" => Ok(RequestKind::Handover),
        "CLOSE" => Ok(RequestKind::Close),
        "SAFE_OPEN" => Ok(RequestKind::SafeOpen),
        "SAFE_IN" => Ok(RequestKind::SafeIn),
        "SAFE_OUT" => Ok(RequestKind::SafeOut),
        "EXTERNAL_IN" => Ok(RequestKind::ExternalIn),
        "EXTERNAL_OUT" => Ok(RequestKind::ExternalOut),
        other => Err(undecodable(table, field, other)),
    }
}

fn str_to_request_status(table: &str, field: &str, s: &str) -> AppResult<RequestStatus> {
    match s {
        "PENDING" => Ok(RequestStatus::Pending),
        "REJECTED" => Ok(RequestStatus::Rejected),
        "COMPLETED" => Ok(RequestStatus::Completed),
        other => Err(undecodable(table, field, other)),
    }
}

fn currency_from_json(table: &str, field: &str, v: &Value) -> AppResult<CurrencyCode> {
    match v {
        Value::String(s) => CurrencyCode::parse(s).map_err(|_| undecodable(table, field, s)),
        _ => Err(undecodable(table, field, v)),
    }
}

fn money_from_json(table: &str, field: &str, v: &Value) -> AppResult<Money> {
    let minor = minor_from_json(v).ok_or_else(|| undecodable(table, field, v))?;
    MoneyDelta::from_minor(minor)
        .to_money()
        .map_err(|_| undecodable(table, field, v))
}

fn time_from_json(table: &str, field: &str, v: &Value) -> AppResult<CustodyTime> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .map(CustodyTime::from_epoch_millis)
            .ok_or_else(|| undecodable(table, field, v)),
        _ => Err(undecodable(table, field, v)),
    }
}

/// An amount in the smallest unit, as it is stored: a JSON integer while it
/// fits 64 bits, its decimal text beyond that (JSON numbers of that size do
/// not survive every reader). [`minor_from_json`] reads these two forms and
/// nothing else.
fn minor_to_json(minor: i128) -> Value {
    match i64::try_from(minor) {
        Ok(n) => json!(n),
        Err(_) => Value::String(minor.to_string()),
    }
}

fn minor_from_json(v: &Value) -> Option<i128> {
    match v {
        Value::Number(n) => n.as_i64().map(i128::from),
        // text is written only for what 64 bits cannot hold
        Value::String(text) => text
            .parse::<i128>()
            .ok()
            .filter(|minor| i64::try_from(*minor).is_err()),
        _ => None,
    }
}

fn balances_to_json(b: &Balances) -> Value {
    let mut map = serde_json::Map::new();
    for (code, money) in b.non_zero() {
        map.insert(code.as_str().to_string(), minor_to_json(money.minor_i128()));
    }
    Value::Object(map)
}

fn balances_from_json(table: &str, field: &str, v: &Value) -> AppResult<Balances> {
    let map = match v {
        Value::Object(map) => map,
        _ => return Err(undecodable(table, field, v)),
    };
    let mut pairs = Vec::with_capacity(map.len());
    for (k, val) in map {
        let code = CurrencyCode::parse(k).map_err(|_| undecodable(table, field, k))?;
        let minor = minor_from_json(val).ok_or_else(|| undecodable(table, field, val))?;
        let m = MoneyDelta::from_minor(minor)
            .to_money()
            .map_err(|_| undecodable(table, field, val))?;
        if !m.is_zero() {
            pairs.push((code, m));
        }
    }
    Ok(Balances::of(&pairs))
}

fn delta_to_json(d: &StockDelta) -> Value {
    let mut map = serde_json::Map::new();
    for (code, delta) in d.non_zero() {
        map.insert(code.as_str().to_string(), minor_to_json(delta.minor()));
    }
    Value::Object(map)
}

fn delta_from_json(table: &str, field: &str, v: &Value) -> AppResult<StockDelta> {
    let map = match v {
        Value::Object(map) => map,
        _ => return Err(undecodable(table, field, v)),
    };
    let mut pairs = Vec::with_capacity(map.len());
    for (k, val) in map {
        let code = CurrencyCode::parse(k).map_err(|_| undecodable(table, field, k))?;
        let minor = minor_from_json(val).ok_or_else(|| undecodable(table, field, val))?;
        let d = MoneyDelta::from_minor(minor);
        if !d.is_zero() {
            pairs.push((code, d));
        }
    }
    Ok(StockDelta::of(&pairs))
}

fn count_sheet_to_json(cs: &CountSheet) -> Value {
    let denoms: Vec<Value> = cs
        .denominations
        .iter()
        .map(|r| {
            json!({
                "currency": r.currency.as_str(),
                "denomination": minor_to_json(r.denomination.minor_i128()),
                "count": r.count
            })
        })
        .collect();
    json!({
        "totals": balances_to_json(&cs.totals),
        "denominations": denoms
    })
}

fn count_sheet_from_json(table: &str, field: &str, v: &Value) -> AppResult<CountSheet> {
    let obj = match v {
        Value::Object(o) => o,
        _ => return Err(undecodable(table, field, v)),
    };
    let totals_val = obj
        .get("totals")
        .ok_or_else(|| undecodable(table, field, "missing totals"))?;
    let totals = balances_from_json(table, "totals", totals_val)?;

    let arr = match obj.get("denominations") {
        Some(Value::Array(a)) => a,
        Some(other) => return Err(undecodable(table, field, other)),
        None => return Err(undecodable(table, field, "missing denominations")),
    };

    let mut denominations = Vec::with_capacity(arr.len());
    for item in arr {
        let item_obj = match item {
            Value::Object(o) => o,
            _ => return Err(undecodable(table, "denominations", item)),
        };
        let curr_val = item_obj
            .get("currency")
            .ok_or_else(|| undecodable(table, "denominations.currency", "missing"))?;
        let curr = currency_from_json(table, "denominations.currency", curr_val)?;

        let denom_val = item_obj
            .get("denomination")
            .ok_or_else(|| undecodable(table, "denominations.denomination", "missing"))?;
        let denom = money_from_json(table, "denominations.denomination", denom_val)?;

        let count_val = item_obj
            .get("count")
            .and_then(|c| c.as_u64())
            .ok_or_else(|| undecodable(table, "denominations.count", item))?;
        let count = u32::try_from(count_val)
            .map_err(|_| undecodable(table, "denominations.count", count_val))?;

        denominations.push(DenominationRow {
            currency: curr,
            denomination: denom,
            count,
        });
    }

    Ok(CountSheet {
        totals,
        denominations,
    })
}

fn target_to_json(target: &CashTarget) -> Value {
    json!({
        "amounts": balances_to_json(&target.amounts),
        "by": target.by,
        "at": target.at.epoch_millis()
    })
}

fn target_from_json(table: &str, field: &str, v: &Value) -> AppResult<CashTarget> {
    let obj = match v {
        Value::Object(o) => o,
        _ => return Err(undecodable(table, field, v)),
    };
    let amounts_val = obj
        .get("amounts")
        .ok_or_else(|| undecodable(table, field, "missing amounts"))?;
    let amounts = balances_from_json(table, "amounts", amounts_val)?;
    let by = obj
        .get("by")
        .and_then(|s| s.as_str())
        .ok_or_else(|| undecodable(table, field, "missing or invalid by"))?
        .to_string();
    let at_val = obj
        .get("at")
        .ok_or_else(|| undecodable(table, field, "missing at"))?;
    let at = time_from_json(table, field, at_val)?;
    Ok(CashTarget { amounts, by, at })
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

fn cutoff_from_json(table: &str, field: &str, v: &Value) -> AppResult<Cutoff> {
    let obj = match v {
        Value::Object(o) => o,
        _ => return Err(undecodable(table, field, v)),
    };
    let at_val = obj
        .get("at")
        .ok_or_else(|| undecodable(table, field, "missing at"))?;
    let at = time_from_json(table, field, at_val)?;
    let by = obj
        .get("by")
        .and_then(|s| s.as_str())
        .ok_or_else(|| undecodable(table, field, "missing or invalid by"))?
        .to_string();
    let shift_id = obj
        .get("shiftId")
        .and_then(|s| s.as_str())
        .ok_or_else(|| undecodable(table, field, "missing or invalid shiftId"))?
        .to_string();
    let last_ref = match obj.get("lastRef") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => return Err(undecodable(table, field, other)),
    };
    let pending = match obj.get("pending") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(arr)) => {
            let mut list = Vec::with_capacity(arr.len());
            for item in arr {
                match item {
                    Value::String(s) => list.push(s.clone()),
                    other => return Err(undecodable(table, field, other)),
                }
            }
            list
        }
        Some(other) => return Err(undecodable(table, field, other)),
    };

    Ok(Cutoff {
        at,
        by,
        shift_id,
        last_ref,
        pending,
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

fn entity_from_json(table: &str, field: &str, v: &Value) -> AppResult<Entity> {
    let obj = match v {
        Value::Object(o) => o,
        _ => return Err(undecodable(table, field, v)),
    };
    let company = obj
        .get("company")
        .and_then(|s| s.as_str())
        .ok_or_else(|| undecodable(table, field, "missing or invalid company"))?
        .to_string();
    let site = obj
        .get("site")
        .and_then(|s| s.as_str())
        .ok_or_else(|| undecodable(table, field, "missing or invalid site"))?
        .to_string();
    let confirmed_by = obj
        .get("confirmedBy")
        .and_then(|s| s.as_str())
        .ok_or_else(|| undecodable(table, field, "missing or invalid confirmedBy"))?
        .to_string();
    let at_val = obj
        .get("at")
        .ok_or_else(|| undecodable(table, field, "missing at"))?;
    let at = time_from_json(table, field, at_val)?;
    Ok(Entity {
        company,
        site,
        confirmed_by,
        at,
    })
}

fn decision_to_json(d: &Decision) -> Value {
    json!({
        "by": d.by,
        "at": d.at.epoch_millis(),
        "text": d.text
    })
}

fn decision_from_json(table: &str, field: &str, v: &Value) -> AppResult<Decision> {
    let obj = match v {
        Value::Object(o) => o,
        _ => return Err(undecodable(table, field, v)),
    };
    let by = obj
        .get("by")
        .and_then(|s| s.as_str())
        .ok_or_else(|| undecodable(table, field, "missing or invalid by"))?
        .to_string();
    let at_val = obj
        .get("at")
        .ok_or_else(|| undecodable(table, field, "missing at"))?;
    let at = time_from_json(table, field, at_val)?;
    let text = obj
        .get("text")
        .and_then(|s| s.as_str())
        .ok_or_else(|| undecodable(table, field, "missing or invalid text"))?
        .to_string();
    Ok(Decision { by, at, text })
}

// ── one row of each table, as it is stored ──────────────────────────────────

fn attendance_data(att: &Attendance) -> Value {
    json!({
        "role": role_to_str(att.role),
        "clockIn": att.clock_in.epoch_millis(),
        "clockOut": att.clock_out.map(|t| t.epoch_millis())
    })
}

fn request_status_str(status: RequestStatus) -> &'static str {
    match status {
        RequestStatus::Pending => "PENDING",
        RequestStatus::Rejected => "REJECTED",
        RequestStatus::Completed => "COMPLETED",
    }
}

fn request_data(req: &CustodyRequest) -> Value {
    json!({
        "payloadHash": req.payload_hash,
        "at": req.at.epoch_millis(),
        "sender": req.sender,
        "receiver": req.receiver,
        "note": req.note,
        "evidenceKeys": req.evidence_keys,
        "shiftId": req.shift_id,
        "counted": req.counted.as_ref().map(count_sheet_to_json),
        "target": req.target.as_ref().map(target_to_json),
        "targetDifference": req.target_difference.as_ref().map(delta_to_json),
        "expected": req.expected.as_ref().map(balances_to_json),
        "variance": req.variance.as_ref().map(delta_to_json),
        "cutoff": req.cutoff.as_ref().map(cutoff_to_json),
        "safeCount": req.safe_count.as_ref().map(count_sheet_to_json),
        "safeExpected": req.safe_expected.as_ref().map(balances_to_json),
        "safeVariance": req.safe_variance.as_ref().map(delta_to_json),
        "currency": req.currency.map(|c| c.as_str()),
        "amount": req.amount.map(|m| minor_to_json(m.minor_i128())),
        "stage": req.stage.map(|st| match st {
            TransferStage::InTransit => "IN_TRANSIT",
            TransferStage::Received => "RECEIVED",
        }),
        "dispatchedAt": req.dispatched_at.map(|t| t.epoch_millis()),
        "dispatchedBy": req.dispatched_by,
        "rejected": req.rejected.as_ref().map(decision_to_json),
        "completedAt": req.completed_at.map(|t| t.epoch_millis()),
        "reconciliation": req.reconciliation.map(|r| match r {
            Reconciliation::Checked => "CHECKED",
            Reconciliation::Pending => "PENDING",
        }),
        "reconciled": req.reconciled.as_ref().map(decision_to_json)
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
        "amount": minor_to_json(draft.amount.minor_i128()),
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

fn signature_at(sig: &Signature) -> AppResult<DateTime<Utc>> {
    let dt = DateTime::from_timestamp_millis(sig.at.epoch_millis()).ok_or_else(|| {
        AppError::Internal(format!(
            "signature time out of range: {}",
            sig.at.epoch_millis()
        ))
    })?;
    Ok(truncate_millis(dt))
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
        "SELECT enabled, phase, custodian_id, entity, target, safe, cutoff, service_opened_at \
         FROM ops_state WHERE branch_id = $1",
    )
    .bind(branch.as_str())
    .fetch_optional(&mut *conn)
    .await?;

    let ops_row = match ops_row {
        Some(r) => r,
        None => return Ok(CustodyState::initial()),
    };

    let enabled: bool = ops_row.get(0);
    let phase_str: String = ops_row.get(1);
    let phase = str_to_phase("ops_state", "phase", &phase_str)?;
    let custodian: Option<String> = ops_row.get(2);
    let entity: Option<Entity> = match ops_row.get::<Option<Value>, _>(3) {
        Some(v) if !v.is_null() => Some(entity_from_json("ops_state", "entity", &v)?),
        _ => None,
    };
    let target: Option<CashTarget> = match ops_row.get::<Option<Value>, _>(4) {
        Some(v) if !v.is_null() => Some(target_from_json("ops_state", "target", &v)?),
        _ => None,
    };
    let safe: Option<Balances> = match ops_row.get::<Option<Value>, _>(5) {
        Some(v) if !v.is_null() => Some(balances_from_json("ops_state", "safe", &v)?),
        _ => None,
    };
    let cutoff: Option<Cutoff> = match ops_row.get::<Option<Value>, _>(6) {
        Some(v) if !v.is_null() => Some(cutoff_from_json("ops_state", "cutoff", &v)?),
        _ => None,
    };
    let service_opened_at: Option<CustodyTime> = ops_row
        .get::<Option<DateTime<Utc>>, _>(7)
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
    for r in att_rows {
        let id: String = r.get(0);
        let employee_id: String = r.get(1);
        let data: Value = r.get(2);

        let role = match data.get("role") {
            Some(Value::String(s)) => str_to_role("attendance", "role", s)?,
            Some(other) => return Err(undecodable("attendance", "role", other)),
            None => return Err(undecodable("attendance", "role", "absent")),
        };
        let clock_in = match data.get("clockIn") {
            Some(v) => time_from_json("attendance", "clockIn", v)?,
            None => return Err(undecodable("attendance", "clockIn", "absent")),
        };
        let clock_out = match data.get("clockOut") {
            None | Some(Value::Null) => None,
            Some(v) => Some(time_from_json("attendance", "clockOut", v)?),
        };

        attendance.push(Attendance {
            id,
            employee_id,
            role,
            clock_in,
            clock_out,
        });
    }

    // signatures grouped by request_id
    let sig_rows = sqlx::query(
        "SELECT s.request_id, s.employee_id, s.role, s.at \
         FROM custody_signatures s JOIN custody_requests r ON r.id = s.request_id \
         WHERE r.branch_id = $1 ORDER BY s.position ASC",
    )
    .bind(branch.as_str())
    .fetch_all(&mut *conn)
    .await?;

    let mut signatures_by_req: HashMap<String, Vec<Signature>> = HashMap::new();
    for r in sig_rows {
        let req_id: String = r.get(0);
        let employee_id: String = r.get(1);
        let role_str: String = r.get(2);
        let at_dt: DateTime<Utc> = r.get(3);
        let role = str_to_role("custody_signatures", "role", &role_str)?;
        let at = CustodyTime::from_epoch_millis(at_dt.timestamp_millis());
        signatures_by_req
            .entry(req_id)
            .or_default()
            .push(Signature {
                employee_id,
                role,
                at,
            });
    }

    // requests
    let req_rows = sqlx::query(
        "SELECT id, request_id, kind, status, data \
         FROM custody_requests WHERE branch_id = $1 ORDER BY position ASC",
    )
    .bind(branch.as_str())
    .fetch_all(&mut *conn)
    .await?;

    let mut requests = Vec::with_capacity(req_rows.len());
    for r in req_rows {
        let id: String = r.get(0);
        let request_id: String = r.get(1);
        let kind_str: String = r.get(2);
        let status_str: String = r.get(3);
        let data: Value = r.get(4);

        let kind = str_to_request_kind("custody_requests", "kind", &kind_str)?;
        let status = str_to_request_status("custody_requests", "status", &status_str)?;

        let payload_hash = match data.get("payloadHash") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => return Err(undecodable("custody_requests", "payloadHash", other)),
            None => return Err(undecodable("custody_requests", "payloadHash", "absent")),
        };
        let at = match data.get("at") {
            Some(v) => time_from_json("custody_requests", "at", v)?,
            None => return Err(undecodable("custody_requests", "at", "absent")),
        };
        let sender = match data.get("sender") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => return Err(undecodable("custody_requests", "sender", other)),
            None => return Err(undecodable("custody_requests", "sender", "absent")),
        };
        let receiver = match data.get("receiver") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(other) => return Err(undecodable("custody_requests", "receiver", other)),
        };
        let note = match data.get("note") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => return Err(undecodable("custody_requests", "note", other)),
            None => return Err(undecodable("custody_requests", "note", "absent")),
        };
        let evidence_keys = match data.get("evidenceKeys") {
            Some(Value::Array(arr)) => {
                let mut keys = Vec::with_capacity(arr.len());
                for item in arr {
                    match item {
                        Value::String(s) => keys.push(s.clone()),
                        other => {
                            return Err(undecodable("custody_requests", "evidenceKeys", other));
                        }
                    }
                }
                keys
            }
            Some(other) => return Err(undecodable("custody_requests", "evidenceKeys", other)),
            None => return Err(undecodable("custody_requests", "evidenceKeys", "absent")),
        };

        let shift_id = match data.get("shiftId") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(other) => return Err(undecodable("custody_requests", "shiftId", other)),
        };
        let counted = match data.get("counted") {
            None | Some(Value::Null) => None,
            Some(v) => Some(count_sheet_from_json("custody_requests", "counted", v)?),
        };
        let target = match data.get("target") {
            None | Some(Value::Null) => None,
            Some(v) => Some(target_from_json("custody_requests", "target", v)?),
        };
        let target_difference = match data.get("targetDifference") {
            None | Some(Value::Null) => None,
            Some(v) => Some(delta_from_json("custody_requests", "targetDifference", v)?),
        };
        let expected = match data.get("expected") {
            None | Some(Value::Null) => None,
            Some(v) => Some(balances_from_json("custody_requests", "expected", v)?),
        };
        let variance = match data.get("variance") {
            None | Some(Value::Null) => None,
            Some(v) => Some(delta_from_json("custody_requests", "variance", v)?),
        };
        let cutoff = match data.get("cutoff") {
            None | Some(Value::Null) => None,
            Some(v) => Some(cutoff_from_json("custody_requests", "cutoff", v)?),
        };
        let safe_count = match data.get("safeCount") {
            None | Some(Value::Null) => None,
            Some(v) => Some(count_sheet_from_json("custody_requests", "safeCount", v)?),
        };
        let safe_expected = match data.get("safeExpected") {
            None | Some(Value::Null) => None,
            Some(v) => Some(balances_from_json("custody_requests", "safeExpected", v)?),
        };
        let safe_variance = match data.get("safeVariance") {
            None | Some(Value::Null) => None,
            Some(v) => Some(delta_from_json("custody_requests", "safeVariance", v)?),
        };
        let currency = match data.get("currency") {
            None | Some(Value::Null) => None,
            Some(v) => Some(currency_from_json("custody_requests", "currency", v)?),
        };
        let amount = match data.get("amount") {
            None | Some(Value::Null) => None,
            Some(v) => Some(money_from_json("custody_requests", "amount", v)?),
        };

        let stage = match data.get("stage") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => match s.as_str() {
                "IN_TRANSIT" => Some(TransferStage::InTransit),
                "RECEIVED" => Some(TransferStage::Received),
                other => return Err(undecodable("custody_requests", "stage", other)),
            },
            Some(other) => return Err(undecodable("custody_requests", "stage", other)),
        };
        let dispatched_at = match data.get("dispatchedAt") {
            None | Some(Value::Null) => None,
            Some(v) => Some(time_from_json("custody_requests", "dispatchedAt", v)?),
        };
        let dispatched_by = match data.get("dispatchedBy") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(other) => return Err(undecodable("custody_requests", "dispatchedBy", other)),
        };
        let rejected = match data.get("rejected") {
            None | Some(Value::Null) => None,
            Some(v) => Some(decision_from_json("custody_requests", "rejected", v)?),
        };
        let completed_at = match data.get("completedAt") {
            None | Some(Value::Null) => None,
            Some(v) => Some(time_from_json("custody_requests", "completedAt", v)?),
        };
        let reconciliation = match data.get("reconciliation") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => match s.as_str() {
                "CHECKED" => Some(Reconciliation::Checked),
                "PENDING" => Some(Reconciliation::Pending),
                other => return Err(undecodable("custody_requests", "reconciliation", other)),
            },
            Some(other) => return Err(undecodable("custody_requests", "reconciliation", other)),
        };
        let reconciled = match data.get("reconciled") {
            None | Some(Value::Null) => None,
            Some(v) => Some(decision_from_json("custody_requests", "reconciled", v)?),
        };

        let signatures = signatures_by_req.remove(&id).unwrap_or_default();

        requests.push(CustodyRequest {
            id,
            request_id,
            payload_hash,
            kind,
            status,
            at,
            sender,
            receiver,
            note,
            evidence_keys,
            signatures,
            shift_id,
            counted,
            target,
            target_difference,
            expected,
            variance,
            cutoff,
            safe_count,
            safe_expected,
            safe_variance,
            currency,
            amount,
            stage,
            dispatched_at,
            dispatched_by,
            rejected,
            completed_at,
            reconciliation,
            reconciled,
        });
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
    for r in hold_rows {
        let id: String = r.get(0);
        let data: Value = r.get(1);

        let customer_id = match data.get("customerId") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => return Err(undecodable("customer_holds", "customerId", other)),
            None => return Err(undecodable("customer_holds", "customerId", "absent")),
        };
        let reason = match data.get("reason") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => return Err(undecodable("customer_holds", "reason", other)),
            None => return Err(undecodable("customer_holds", "reason", "absent")),
        };
        let by = match data.get("by") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => return Err(undecodable("customer_holds", "by", other)),
            None => return Err(undecodable("customer_holds", "by", "absent")),
        };
        let at = match data.get("at") {
            Some(v) => time_from_json("customer_holds", "at", v)?,
            None => return Err(undecodable("customer_holds", "at", "absent")),
        };
        let status = match data.get("status") {
            Some(Value::String(s)) => match s.as_str() {
                "OPEN" => HoldStatus::Open,
                "RELEASED" => HoldStatus::Released,
                other => return Err(undecodable("customer_holds", "status", other)),
            },
            Some(other) => return Err(undecodable("customer_holds", "status", other)),
            None => return Err(undecodable("customer_holds", "status", "absent")),
        };
        let reviewed_by = match data.get("reviewedBy") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(other) => return Err(undecodable("customer_holds", "reviewedBy", other)),
        };
        let reviewed_at = match data.get("reviewedAt") {
            None | Some(Value::Null) => None,
            Some(v) => Some(time_from_json("customer_holds", "reviewedAt", v)?),
        };
        let note = match data.get("note") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(other) => return Err(undecodable("customer_holds", "note", other)),
        };

        holds.push(CustomerHold {
            id,
            customer_id,
            reason,
            by,
            at,
            status,
            reviewed_by,
            reviewed_at,
            note,
        });
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
    for r in exc_rows {
        let id: String = r.get(0);
        let data: Value = r.get(1);

        let request_id = match data.get("requestId") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => return Err(undecodable("cash_exceptions", "requestId", other)),
            None => return Err(undecodable("cash_exceptions", "requestId", "absent")),
        };
        let status = match data.get("status") {
            Some(Value::String(s)) => match s.as_str() {
                "OPEN" => ExceptionStatus::Open,
                "REVIEWED" => ExceptionStatus::Reviewed,
                other => return Err(undecodable("cash_exceptions", "status", other)),
            },
            Some(other) => return Err(undecodable("cash_exceptions", "status", other)),
            None => return Err(undecodable("cash_exceptions", "status", "absent")),
        };
        let target_difference = match data.get("targetDifference") {
            None | Some(Value::Null) => None,
            Some(v) => Some(delta_from_json("cash_exceptions", "targetDifference", v)?),
        };
        let cash_variance = match data.get("cashVariance") {
            None | Some(Value::Null) => None,
            Some(v) => Some(delta_from_json("cash_exceptions", "cashVariance", v)?),
        };
        let safe_variance = match data.get("safeVariance") {
            None | Some(Value::Null) => None,
            Some(v) => Some(delta_from_json("cash_exceptions", "safeVariance", v)?),
        };
        let note = match data.get("note") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => return Err(undecodable("cash_exceptions", "note", other)),
            None => return Err(undecodable("cash_exceptions", "note", "absent")),
        };
        let at = match data.get("at") {
            Some(v) => time_from_json("cash_exceptions", "at", v)?,
            None => return Err(undecodable("cash_exceptions", "at", "absent")),
        };
        let reviewed_by = match data.get("reviewedBy") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(other) => return Err(undecodable("cash_exceptions", "reviewedBy", other)),
        };
        let reviewed_at = match data.get("reviewedAt") {
            None | Some(Value::Null) => None,
            Some(v) => Some(time_from_json("cash_exceptions", "reviewedAt", v)?),
        };

        exceptions.push(CashException {
            id,
            request_id,
            status,
            target_difference,
            cash_variance,
            safe_variance,
            note,
            at,
            reviewed_by,
            reviewed_at,
        });
    }

    Ok(CustodyState {
        enabled,
        phase,
        safe,
        target,
        entity,
        custodian,
        attendance,
        requests,
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
            let own_values_changed = before.enabled != after.enabled
                || before.phase != after.phase
                || before.custodian != after.custodian
                || before.entity != after.entity
                || before.target != after.target
                || before.safe != after.safe
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
                    "INSERT INTO ops_state (branch_id, enabled, phase, custodian_id, entity, target, safe, cutoff, service_opened_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
                     ON CONFLICT (branch_id) DO UPDATE SET \
                       enabled = EXCLUDED.enabled, \
                       phase = EXCLUDED.phase, \
                       custodian_id = EXCLUDED.custodian_id, \
                       entity = EXCLUDED.entity, \
                       target = EXCLUDED.target, \
                       safe = EXCLUDED.safe, \
                       cutoff = EXCLUDED.cutoff, \
                       service_opened_at = EXCLUDED.service_opened_at",
                )
                .bind(branch.as_str())
                .bind(after.enabled)
                .bind(after.phase.as_str())
                .bind(&after.custodian)
                .bind(after.entity.as_ref().map(entity_to_json))
                .bind(after.target.as_ref().map(target_to_json))
                .bind(after.safe.as_ref().map(balances_to_json))
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

            for (pos, req, was) in to_write(&before.requests, &after.requests, |r| &r.id) {
                // A request is a row of its own and one row per signature: a new
                // signature alone leaves the request's row as it is.
                let row_as_stored = was.is_some_and(|was| {
                    before
                        .requests
                        .get(pos)
                        .is_some_and(|at_pos| at_pos.id == req.id)
                        && was.request_id == req.request_id
                        && was.kind == req.kind
                        && was.status == req.status
                        && request_data(was) == request_data(req)
                });
                if !row_as_stored {
                    sqlx::query(
                        "INSERT INTO custody_requests (id, branch_id, position, request_id, kind, status, data) \
                         VALUES ($1, $2, $3, $4, $5, $6, $7) \
                         ON CONFLICT (id) DO UPDATE SET \
                           position = EXCLUDED.position, \
                           status = EXCLUDED.status, \
                           data = EXCLUDED.data",
                    )
                    .bind(&req.id)
                    .bind(branch.as_str())
                    .bind(pos as i32)
                    .bind(&req.request_id)
                    .bind(req.kind.as_str())
                    .bind(request_status_str(req.status))
                    .bind(request_data(req))
                    .execute(&mut **tx)
                    .await?;
                }

                for (sig_pos, sig) in req.signatures.iter().enumerate() {
                    if was.and_then(|was| was.signatures.get(sig_pos)) == Some(sig) {
                        continue;
                    }
                    sqlx::query(
                        "INSERT INTO custody_signatures (request_id, position, employee_id, role, at) \
                         VALUES ($1, $2, $3, $4, $5) \
                         ON CONFLICT (request_id, position) DO UPDATE SET \
                           employee_id = EXCLUDED.employee_id, \
                           role = EXCLUDED.role, \
                           at = EXCLUDED.at",
                    )
                    .bind(&req.id)
                    .bind(sig_pos as i32)
                    .bind(&sig.employee_id)
                    .bind(role_to_str(sig.role))
                    .bind(signature_at(sig)?)
                    .execute(&mut **tx)
                    .await?;
                }
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

    fn is_enabled_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            let enabled: Option<bool> =
                sqlx::query_scalar("SELECT enabled FROM ops_state WHERE branch_id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(&mut **tx)
                    .await?;
            Ok(enabled.unwrap_or(false))
        })
    }
}
