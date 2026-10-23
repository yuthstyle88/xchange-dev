//! Row decoders for custody entities and value objects.

use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::Row;
use sqlx::postgres::PgRow;
use xc_kernel::money::{CurrencyCode, ForeignAmount, QuantityDelta};

use crate::domain::custody::entities::attendance::Attendance;
use crate::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use crate::domain::custody::entities::customer_hold::{CustomerHold, HoldStatus};
use crate::domain::custody::value_objects::cash_target::CashTarget;
use crate::domain::custody::value_objects::custody_role::CustodyRole;
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::cutoff::Cutoff;
use crate::domain::custody::value_objects::entity::Entity;
use crate::domain::custody::value_objects::phase::Phase;
use crate::domain::shift::value_objects::balances::Balances;
use crate::domain::shift::value_objects::stock_delta::StockDelta;

pub(crate) fn undecodable(table: &str, field: &str, value: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!(
        "{table}.{field} holds an undecodable value: {value}"
    ))
}

pub(crate) fn str_to_role(table: &str, field: &str, s: &str) -> AppResult<CustodyRole> {
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

pub(crate) fn str_to_phase(table: &str, field: &str, s: &str) -> AppResult<Phase> {
    match s {
        "CLOSED" => Ok(Phase::Closed),
        "READY" => Ok(Phase::Ready),
        "OPEN" => Ok(Phase::Open),
        "CUT_OFF" => Ok(Phase::CutOff),
        other => Err(undecodable(table, field, other)),
    }
}

pub(crate) fn currency_from_json(table: &str, field: &str, v: &Value) -> AppResult<CurrencyCode> {
    match v {
        Value::String(s) => CurrencyCode::parse(s).map_err(|_| undecodable(table, field, s)),
        _ => Err(undecodable(table, field, v)),
    }
}

fn decimal_from_json(table: &str, field: &str, v: &Value) -> AppResult<Decimal> {
    let text = v.as_str().ok_or_else(|| undecodable(table, field, v))?;
    Decimal::from_str_exact(text).map_err(|_| undecodable(table, field, v))
}

pub(crate) fn money_from_json(table: &str, field: &str, v: &Value) -> AppResult<ForeignAmount> {
    let dec = decimal_from_json(table, field, v)?;
    ForeignAmount::checked_from_decimal(dec).map_err(|_| undecodable(table, field, v))
}

pub(crate) fn time_from_json(table: &str, field: &str, v: &Value) -> AppResult<CustodyTime> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .map(CustodyTime::from_epoch_millis)
            .ok_or_else(|| undecodable(table, field, v)),
        _ => Err(undecodable(table, field, v)),
    }
}

pub(crate) fn balances_from_json(table: &str, field: &str, v: &Value) -> AppResult<Balances> {
    let map = match v {
        Value::Object(map) => map,
        _ => return Err(undecodable(table, field, v)),
    };
    let mut pairs = Vec::with_capacity(map.len());
    for (k, val) in map {
        let code = CurrencyCode::parse(k).map_err(|_| undecodable(table, field, k))?;
        let m = money_from_json(table, field, val)?;
        if !m.is_zero() {
            pairs.push((code, m));
        }
    }
    Ok(Balances::of(&pairs))
}

pub(crate) fn delta_from_json(table: &str, field: &str, v: &Value) -> AppResult<StockDelta> {
    let map = match v {
        Value::Object(map) => map,
        _ => return Err(undecodable(table, field, v)),
    };
    let mut pairs = Vec::with_capacity(map.len());
    for (k, val) in map {
        let code = CurrencyCode::parse(k).map_err(|_| undecodable(table, field, k))?;
        let dec = decimal_from_json(table, field, val)?;
        let d = QuantityDelta::from_decimal(dec).map_err(|_| undecodable(table, field, val))?;
        if !d.is_zero() {
            pairs.push((code, d));
        }
    }
    Ok(StockDelta::of(&pairs))
}

pub(crate) fn target_from_json(table: &str, field: &str, v: &Value) -> AppResult<CashTarget> {
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

pub(crate) fn cutoff_from_json(table: &str, field: &str, v: &Value) -> AppResult<Cutoff> {
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

pub(crate) fn entity_from_json(table: &str, field: &str, v: &Value) -> AppResult<Entity> {
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

pub fn attendance_from_row(r: &PgRow) -> AppResult<Attendance> {
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

    Ok(Attendance {
        id,
        employee_id,
        role,
        clock_in,
        clock_out,
    })
}

pub fn hold_from_row(r: &PgRow) -> AppResult<CustomerHold> {
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

    Ok(CustomerHold {
        id,
        customer_id,
        reason,
        by,
        at,
        status,
        reviewed_by,
        reviewed_at,
        note,
    })
}

pub fn exception_from_row(r: &PgRow) -> AppResult<CashException> {
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

    Ok(CashException {
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
    })
}
