//! Row decoders for custody entities and value objects.

use pos_error::{AppError, AppResult};
use serde_json::Value;
use sqlx::Row;
use sqlx::postgres::PgRow;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta};

use crate::domain::custody::entities::attendance::Attendance;
use crate::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use crate::domain::custody::entities::custody_request::{CustodyRequest, Decision};
use crate::domain::custody::entities::customer_hold::{CustomerHold, HoldStatus};
use crate::domain::custody::value_objects::cash_target::CashTarget;
use crate::domain::custody::value_objects::count_sheet::CountSheet;
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
        "COUNTING" => Ok(Phase::Counting),
        other => Err(undecodable(table, field, other)),
    }
}

pub(crate) fn str_to_request_kind(table: &str, field: &str, s: &str) -> AppResult<RequestKind> {
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

pub(crate) fn str_to_request_status(table: &str, field: &str, s: &str) -> AppResult<RequestStatus> {
    match s {
        "PENDING" => Ok(RequestStatus::Pending),
        "REJECTED" => Ok(RequestStatus::Rejected),
        "COMPLETED" => Ok(RequestStatus::Completed),
        other => Err(undecodable(table, field, other)),
    }
}

pub(crate) fn currency_from_json(table: &str, field: &str, v: &Value) -> AppResult<CurrencyCode> {
    match v {
        Value::String(s) => CurrencyCode::parse(s).map_err(|_| undecodable(table, field, s)),
        _ => Err(undecodable(table, field, v)),
    }
}

pub(crate) fn minor_from_json(v: &Value) -> Option<i128> {
    match v {
        Value::Number(n) => n.as_i64().map(i128::from),
        Value::String(text) => text
            .parse::<i128>()
            .ok()
            .filter(|minor| i64::try_from(*minor).is_err()),
        _ => None,
    }
}

pub(crate) fn money_from_json(table: &str, field: &str, v: &Value) -> AppResult<Money> {
    let minor = minor_from_json(v).ok_or_else(|| undecodable(table, field, v))?;
    MoneyDelta::from_minor(minor)
        .to_money()
        .map_err(|_| undecodable(table, field, v))
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

pub(crate) fn delta_from_json(table: &str, field: &str, v: &Value) -> AppResult<StockDelta> {
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

pub(crate) fn count_sheet_from_json(table: &str, field: &str, v: &Value) -> AppResult<CountSheet> {
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

pub(crate) fn decision_from_json(table: &str, field: &str, v: &Value) -> AppResult<Decision> {
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

pub fn request_from_row(r: &PgRow, signatures: Vec<Signature>) -> AppResult<CustodyRequest> {
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

    Ok(CustodyRequest {
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
