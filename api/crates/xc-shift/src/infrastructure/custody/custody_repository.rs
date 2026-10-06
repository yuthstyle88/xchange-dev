//! Custody repository postgres implementation.

use std::collections::HashMap;

use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
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

fn str_to_role(s: &str) -> CustodyRole {
    match s {
        "OWNER" => CustodyRole::Owner,
        "FRONT" => CustodyRole::Front,
        "FINANCE" => CustodyRole::Finance,
        "MONITOR" => CustodyRole::Monitor,
        "MANAGER" => CustodyRole::Manager,
        "BACKEND" => CustodyRole::Backend,
        _ => CustodyRole::Other,
    }
}

fn str_to_event_kind(s: &str) -> EventKind {
    match s {
        "SET_ENTITY" => EventKind::SetEntity,
        "ENABLE" => EventKind::Enable,
        "CLOCK_IN" => EventKind::ClockIn,
        "CLOCK_OUT" => EventKind::ClockOut,
        "CASH_TARGET" => EventKind::CashTarget,
        "SERVICE_OPEN" => EventKind::ServiceOpen,
        "CUT_OFF" => EventKind::CutOff,
        "PROPOSE" => EventKind::Propose,
        "HOLD_CUSTOMER" => EventKind::HoldCustomer,
        "RELEASE_CUSTOMER" => EventKind::ReleaseCustomer,
        "REJECT" => EventKind::Reject,
        "SIGN" => EventKind::Sign,
        "DISPATCH" => EventKind::Dispatch,
        "COMMIT" => EventKind::Commit,
        "RECONCILE" => EventKind::Reconcile,
        _ => EventKind::Propose,
    }
}

fn str_to_phase(s: &str) -> Phase {
    match s {
        "CLOSED" => Phase::Closed,
        "READY" => Phase::Ready,
        "OPEN" => Phase::Open,
        "CUT_OFF" => Phase::CutOff,
        "COUNTING" => Phase::Counting,
        _ => Phase::Closed,
    }
}

fn str_to_request_kind(s: &str) -> RequestKind {
    match s {
        "OPEN" => RequestKind::Open,
        "HANDOVER" => RequestKind::Handover,
        "CLOSE" => RequestKind::Close,
        "SAFE_OPEN" => RequestKind::SafeOpen,
        "SAFE_IN" => RequestKind::SafeIn,
        "SAFE_OUT" => RequestKind::SafeOut,
        "EXTERNAL_IN" => RequestKind::ExternalIn,
        "EXTERNAL_OUT" => RequestKind::ExternalOut,
        _ => RequestKind::Open,
    }
}

fn str_to_request_status(s: &str) -> RequestStatus {
    match s {
        "PENDING" => RequestStatus::Pending,
        "REJECTED" => RequestStatus::Rejected,
        "COMPLETED" => RequestStatus::Completed,
        _ => RequestStatus::Pending,
    }
}

fn balances_to_json(b: &Balances) -> Value {
    let mut map = serde_json::Map::new();
    for (code, money) in b.non_zero() {
        map.insert(code.as_str().to_string(), json!(money.minor_i128()));
    }
    Value::Object(map)
}

fn balances_from_json(v: &Value) -> Balances {
    let mut pairs = Vec::new();
    if let Value::Object(map) = v {
        for (k, val) in map {
            if let Ok(code) = CurrencyCode::parse(k) {
                if let Some(minor) = val.as_i64() {
                    if let Ok(m) = MoneyDelta::from_minor(minor as i128).to_money() {
                        if !m.is_zero() {
                            pairs.push((code, m));
                        }
                    }
                }
            }
        }
    }
    Balances::of(&pairs)
}

fn delta_to_json(d: &StockDelta) -> Value {
    let mut map = serde_json::Map::new();
    for (code, delta) in d.non_zero() {
        map.insert(code.as_str().to_string(), json!(delta.minor()));
    }
    Value::Object(map)
}

fn delta_from_json(v: &Value) -> StockDelta {
    let mut pairs = Vec::new();
    if let Value::Object(map) = v {
        for (k, val) in map {
            if let Ok(code) = CurrencyCode::parse(k) {
                if let Some(minor) = val.as_i64() {
                    let d = MoneyDelta::from_minor(minor as i128);
                    if !d.is_zero() {
                        pairs.push((code, d));
                    }
                }
            }
        }
    }
    StockDelta::of(&pairs)
}

fn count_sheet_to_json(cs: &CountSheet) -> Value {
    let denoms: Vec<Value> = cs
        .denominations
        .iter()
        .map(|r| {
            json!({
                "currency": r.currency.as_str(),
                "denomination": r.denomination.minor_i128(),
                "count": r.count
            })
        })
        .collect();
    json!({
        "totals": balances_to_json(&cs.totals),
        "denominations": denoms
    })
}

fn count_sheet_from_json(v: &Value) -> CountSheet {
    let totals = v
        .get("totals")
        .map(balances_from_json)
        .unwrap_or_else(Balances::empty);

    let mut denominations = Vec::new();
    if let Some(Value::Array(arr)) = v.get("denominations") {
        for item in arr {
            let curr = item
                .get("currency")
                .and_then(|s| s.as_str())
                .and_then(|s| CurrencyCode::parse(s).ok())
                .unwrap_or(CurrencyCode::THB);
            let denom = item
                .get("denomination")
                .and_then(|d| d.as_i64())
                .and_then(|min| MoneyDelta::from_minor(min as i128).to_money().ok())
                .unwrap_or(Money::ZERO);
            let count = item.get("count").and_then(|c| c.as_u64()).unwrap_or(0) as u32;
            denominations.push(DenominationRow {
                currency: curr,
                denomination: denom,
                count,
            });
        }
    }

    CountSheet {
        totals,
        denominations,
    }
}

fn target_to_json(target: &CashTarget) -> Value {
    json!({
        "amounts": balances_to_json(&target.amounts),
        "by": target.by,
        "at": target.at.epoch_millis()
    })
}

fn target_from_json(v: &Value) -> Option<CashTarget> {
    let amounts = v.get("amounts").map(balances_from_json)?;
    let by = v.get("by").and_then(|s| s.as_str())?.to_string();
    let at = v.get("at").and_then(|n| n.as_i64())?;
    Some(CashTarget {
        amounts,
        by,
        at: CustodyTime::from_epoch_millis(at),
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

fn cutoff_from_json(v: &Value) -> Option<Cutoff> {
    let at = v.get("at").and_then(|n| n.as_i64())?;
    let by = v.get("by").and_then(|s| s.as_str())?.to_string();
    let shift_id = v.get("shiftId").and_then(|s| s.as_str())?.to_string();
    let last_ref = v.get("lastRef").and_then(|s| s.as_str()).map(String::from);
    let pending: Vec<String> = v
        .get("pending")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Some(Cutoff {
        at: CustodyTime::from_epoch_millis(at),
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

fn entity_from_json(v: &Value) -> Option<Entity> {
    let company = v.get("company").and_then(|s| s.as_str())?.to_string();
    let site = v.get("site").and_then(|s| s.as_str())?.to_string();
    let confirmed_by = v.get("confirmedBy").and_then(|s| s.as_str())?.to_string();
    let at = v.get("at").and_then(|n| n.as_i64())?;
    Some(Entity {
        company,
        site,
        confirmed_by,
        at: CustodyTime::from_epoch_millis(at),
    })
}

fn decision_to_json(d: &Decision) -> Value {
    json!({
        "by": d.by,
        "at": d.at.epoch_millis(),
        "text": d.text
    })
}

fn decision_from_json(v: &Value) -> Option<Decision> {
    let by = v.get("by").and_then(|s| s.as_str())?.to_string();
    let at = v.get("at").and_then(|n| n.as_i64())?;
    let text = v.get("text").and_then(|s| s.as_str())?.to_string();
    Some(Decision {
        by,
        at: CustodyTime::from_epoch_millis(at),
        text,
    })
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
    let phase = str_to_phase(&phase_str);
    let custodian: Option<String> = ops_row.get(2);
    let entity: Option<Entity> = ops_row
        .get::<Option<Value>, _>(3)
        .as_ref()
        .and_then(entity_from_json);
    let target: Option<CashTarget> = ops_row
        .get::<Option<Value>, _>(4)
        .as_ref()
        .and_then(target_from_json);
    let safe: Option<Balances> = ops_row
        .get::<Option<Value>, _>(5)
        .as_ref()
        .filter(|v| !v.is_null())
        .map(balances_from_json);
    let cutoff: Option<Cutoff> = ops_row
        .get::<Option<Value>, _>(6)
        .as_ref()
        .and_then(cutoff_from_json);
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

        let role = data
            .get("role")
            .and_then(|s| s.as_str())
            .map(str_to_role)
            .unwrap_or(CustodyRole::Front);
        let clock_in = data
            .get("clockIn")
            .and_then(|n| n.as_i64())
            .map(CustodyTime::from_epoch_millis)
            .unwrap_or_else(|| CustodyTime::from_epoch_millis(0));
        let clock_out = data
            .get("clockOut")
            .and_then(|n| n.as_i64())
            .map(CustodyTime::from_epoch_millis);

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
        let role = str_to_role(&role_str);
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

        let kind = str_to_request_kind(&kind_str);
        let status = str_to_request_status(&status_str);

        let payload_hash = data
            .get("payloadHash")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let at = data
            .get("at")
            .and_then(|n| n.as_i64())
            .map(CustodyTime::from_epoch_millis)
            .unwrap_or_else(|| CustodyTime::from_epoch_millis(0));
        let sender = data
            .get("sender")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let receiver = data
            .get("receiver")
            .and_then(|s| s.as_str())
            .map(String::from);
        let note = data
            .get("note")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let evidence_keys = data
            .get("evidenceKeys")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let shift_id = data
            .get("shiftId")
            .and_then(|s| s.as_str())
            .map(String::from);
        let counted = data
            .get("counted")
            .filter(|v| !v.is_null())
            .map(count_sheet_from_json);
        let target = data.get("target").and_then(target_from_json);
        let target_difference = data
            .get("targetDifference")
            .filter(|v| !v.is_null())
            .map(delta_from_json);
        let expected = data
            .get("expected")
            .filter(|v| !v.is_null())
            .map(balances_from_json);
        let variance = data
            .get("variance")
            .filter(|v| !v.is_null())
            .map(delta_from_json);
        let cutoff = data.get("cutoff").and_then(cutoff_from_json);
        let safe_count = data
            .get("safeCount")
            .filter(|v| !v.is_null())
            .map(count_sheet_from_json);
        let safe_expected = data
            .get("safeExpected")
            .filter(|v| !v.is_null())
            .map(balances_from_json);
        let safe_variance = data
            .get("safeVariance")
            .filter(|v| !v.is_null())
            .map(delta_from_json);
        let currency = data
            .get("currency")
            .and_then(|s| s.as_str())
            .and_then(|s| CurrencyCode::parse(s).ok());
        let amount = data
            .get("amount")
            .and_then(|n| n.as_i64())
            .and_then(|min| MoneyDelta::from_minor(min as i128).to_money().ok());

        let stage = data
            .get("stage")
            .and_then(|s| s.as_str())
            .map(|st| match st {
                "IN_TRANSIT" => TransferStage::InTransit,
                _ => TransferStage::Received,
            });
        let dispatched_at = data
            .get("dispatchedAt")
            .and_then(|n| n.as_i64())
            .map(CustodyTime::from_epoch_millis);
        let dispatched_by = data
            .get("dispatchedBy")
            .and_then(|s| s.as_str())
            .map(String::from);
        let rejected = data.get("rejected").and_then(decision_from_json);
        let completed_at = data
            .get("completedAt")
            .and_then(|n| n.as_i64())
            .map(CustodyTime::from_epoch_millis);
        let reconciliation =
            data.get("reconciliation")
                .and_then(|s| s.as_str())
                .map(|st| match st {
                    "CHECKED" => Reconciliation::Checked,
                    _ => Reconciliation::Pending,
                });
        let reconciled = data.get("reconciled").and_then(decision_from_json);

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

        let customer_id = data
            .get("customerId")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let reason = data
            .get("reason")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let by = data
            .get("by")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let at = data
            .get("at")
            .and_then(|n| n.as_i64())
            .map(CustodyTime::from_epoch_millis)
            .unwrap_or_else(|| CustodyTime::from_epoch_millis(0));
        let status = match data.get("status").and_then(|s| s.as_str()) {
            Some("RELEASED") => HoldStatus::Released,
            _ => HoldStatus::Open,
        };
        let reviewed_by = data
            .get("reviewedBy")
            .and_then(|s| s.as_str())
            .map(String::from);
        let reviewed_at = data
            .get("reviewedAt")
            .and_then(|n| n.as_i64())
            .map(CustodyTime::from_epoch_millis);
        let note = data.get("note").and_then(|s| s.as_str()).map(String::from);

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

        let shift_id = data
            .get("shiftId")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let employee_id = data
            .get("employeeId")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let side = match data.get("side").and_then(|s| s.as_str()) {
            Some("SELL") => Side::Sell,
            _ => Side::Buy,
        };
        let currency = data
            .get("currency")
            .and_then(|s| s.as_str())
            .and_then(|s| CurrencyCode::parse(s).ok())
            .unwrap_or(CurrencyCode::THB);
        let amount = data
            .get("amount")
            .and_then(|n| n.as_i64())
            .and_then(|min| MoneyDelta::from_minor(min as i128).to_money().ok())
            .unwrap_or(Money::ZERO);
        let at = data
            .get("at")
            .and_then(|n| n.as_i64())
            .map(CustodyTime::from_epoch_millis)
            .unwrap_or_else(|| CustodyTime::from_epoch_millis(0));
        let status = match data.get("status").and_then(|s| s.as_str()) {
            Some("COMPLETED") => DraftStatus::Completed,
            _ => DraftStatus::Open,
        };
        let document_ref = data
            .get("documentRef")
            .and_then(|s| s.as_str())
            .map(String::from);

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

        let request_id = data
            .get("requestId")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let status = match data.get("status").and_then(|s| s.as_str()) {
            Some("REVIEWED") => ExceptionStatus::Reviewed,
            _ => ExceptionStatus::Open,
        };
        let target_difference = data
            .get("targetDifference")
            .filter(|v| !v.is_null())
            .map(delta_from_json);
        let cash_variance = data
            .get("cashVariance")
            .filter(|v| !v.is_null())
            .map(delta_from_json);
        let safe_variance = data
            .get("safeVariance")
            .filter(|v| !v.is_null())
            .map(delta_from_json);
        let note = data
            .get("note")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let at = data
            .get("at")
            .and_then(|n| n.as_i64())
            .map(CustodyTime::from_epoch_millis)
            .unwrap_or_else(|| CustodyTime::from_epoch_millis(0));
        let reviewed_by = data
            .get("reviewedBy")
            .and_then(|s| s.as_str())
            .map(String::from);
        let reviewed_at = data
            .get("reviewedAt")
            .and_then(|n| n.as_i64())
            .map(CustodyTime::from_epoch_millis);

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
        state: &'a CustodyState,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let entity_json = state.entity.as_ref().map(entity_to_json);
            let target_json = state.target.as_ref().map(target_to_json);
            let safe_json = state.safe.as_ref().map(balances_to_json);
            let cutoff_json = state.cutoff.as_ref().map(cutoff_to_json);
            let service_opened_at_dt = state.service_opened_at.map(|t| {
                truncate_millis(
                    DateTime::from_timestamp_millis(t.epoch_millis())
                        .unwrap_or_else(|| Utc.timestamp_millis_opt(0).unwrap()),
                )
            });

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
            .bind(state.enabled)
            .bind(state.phase.as_str())
            .bind(&state.custodian)
            .bind(entity_json)
            .bind(target_json)
            .bind(safe_json)
            .bind(cutoff_json)
            .bind(service_opened_at_dt)
            .execute(&mut **tx)
            .await?;

            for (pos, att) in state.attendance.iter().enumerate() {
                let data = json!({
                    "role": role_to_str(att.role),
                    "clockIn": att.clock_in.epoch_millis(),
                    "clockOut": att.clock_out.map(|t| t.epoch_millis())
                });
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
                .bind(data)
                .execute(&mut **tx)
                .await?;
            }

            for (pos, req) in state.requests.iter().enumerate() {
                let status_str = match req.status {
                    RequestStatus::Pending => "PENDING",
                    RequestStatus::Rejected => "REJECTED",
                    RequestStatus::Completed => "COMPLETED",
                };

                let data = json!({
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
                    "amount": req.amount.map(|m| m.minor_i128()),
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
                });

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
                .bind(status_str)
                .bind(data)
                .execute(&mut **tx)
                .await?;

                for (sig_pos, sig) in req.signatures.iter().enumerate() {
                    let sig_at = truncate_millis(
                        DateTime::from_timestamp_millis(sig.at.epoch_millis())
                            .unwrap_or_else(|| Utc.timestamp_millis_opt(0).unwrap()),
                    );
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
                    .bind(sig_at)
                    .execute(&mut **tx)
                    .await?;
                }
            }

            for (pos, hold) in state.holds.iter().enumerate() {
                let data = json!({
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
                });
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
                .bind(data)
                .execute(&mut **tx)
                .await?;
            }

            for (pos, draft) in state.drafts.iter().enumerate() {
                let data = json!({
                    "shiftId": draft.shift_id,
                    "employeeId": draft.employee_id,
                    "side": match draft.side {
                        Side::Buy => "BUY",
                        Side::Sell => "SELL",
                    },
                    "currency": draft.currency.as_str(),
                    "amount": draft.amount.minor_i128(),
                    "at": draft.at.epoch_millis(),
                    "status": match draft.status {
                        DraftStatus::Open => "OPEN",
                        DraftStatus::Completed => "COMPLETED",
                    },
                    "documentRef": draft.document_ref
                });
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
                .bind(data)
                .execute(&mut **tx)
                .await?;
            }

            for (pos, exc) in state.exceptions.iter().enumerate() {
                let data = json!({
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
                });
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
                .bind(data)
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
                    kind: str_to_event_kind(&kind_str),
                    at,
                    actor,
                    role: str_to_role(&role_str),
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
