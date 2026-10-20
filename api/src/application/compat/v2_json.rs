//! v2 JSON printing: numbers, balances, times and the snapshot lists.
//!
//! v2 held every amount as a JavaScript number and every answer was
//! `JSON.stringify` of an object built key by key. The unchanged v2 screens do
//! arithmetic on those numbers (`r.buy.toFixed(4)`) and show some objects in the
//! order of their keys (`Object.entries` over the 153-code objects), so v3 has to
//! print the same numbers in the same order. This file turns v3's domain types
//! into that JSON. It reads no database and decides nothing.
//!
//! Everything here answers a [`V2Json`]: a JSON tree that **keeps the order it
//! was built in**. `serde_json::Value` cannot be the answer — the workspace does
//! not switch on `preserve_order`, so a `Value` sorts its keys by name and would
//! show `AED` before `THB`. Send [`V2Json::to_text`] to a screen; use
//! [`V2Json::to_value`] only to compare.
//!
//! Known differences from v2, each decided before this file was written:
//!
//! - a shift carries no `row`, `startRow`, `endRow` (spreadsheet row numbers
//!   nothing reads);
//! - a request's `hash` is v3's payload hash;
//! - an event's `details` is what the custody service stored, printed with its
//!   keys sorted by name (nothing reads `events`);
//! - `member` is `false` and `memberVerifiedAt` is `null` for every customer
//!   until the members decision (D5) lands;
//! - a name inside a custody row is today's name of the account, joined by the
//!   caller through [`People`]; v2 froze the name when the row was written.

use std::collections::BTreeMap;

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::Value;
use xc_customer::domain::customer::entities::customer::Customer;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta, Rate, Side};
use xc_rate::domain::branch::entities::branch::Branch;
use xc_rate::domain::rate::entities::counter_rate::CounterRate;
use xc_rate::domain::rate::entities::rate_change::RateChange;
use xc_shift::domain::custody::entities::attendance::Attendance;
use xc_shift::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use xc_shift::domain::custody::entities::customer_hold::{CustomerHold, HoldStatus};
use xc_shift::domain::custody::entities::stored_event::StoredEvent;
use xc_shift::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::cash_target::CashTarget;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::cutoff::Cutoff;
use xc_shift::domain::custody::value_objects::entity::Entity;
use xc_shift::domain::shift::entities::shift::{Shift, ShiftStatus};
use xc_shift::domain::shift::entities::stock_movement::StockMovement;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::domain::shift::value_objects::stock_delta::{Direction, StockDelta};

// ── the printed tree ─────────────────────────────────────────────────────────

/// A JSON value that keeps the order its objects were built in.
#[derive(Debug, Clone, PartialEq)]
pub enum V2Json {
    Null,
    Bool(bool),
    /// A number, already as the text v2 printed: digits, an optional leading
    /// `-`, an optional fraction with no trailing zero. Build one with
    /// [`V2Json::int`], [`money_json`], [`rate_json`] or [`delta_json`].
    Number(String),
    Text(String),
    Array(Vec<V2Json>),
    /// Key and value, in the order they are printed.
    Object(Vec<(String, V2Json)>),
}

impl V2Json {
    pub fn text(text: impl Into<String>) -> V2Json {
        V2Json::Text(text.into())
    }

    pub fn int(n: i64) -> V2Json {
        V2Json::Number(n.to_string())
    }

    /// An object whose keys are printed in the order given.
    pub fn object(pairs: Vec<(&str, V2Json)>) -> V2Json {
        V2Json::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    /// A value stored as JSON (a rate's `reference`, an event's `details`),
    /// passed on. Its keys come in the order `serde_json` holds them — sorted by
    /// name — and a float without a fraction is printed as the integer v2
    /// printed (`37.0` → `37`).
    pub fn from_value(value: &Value) -> V2Json {
        match value {
            Value::Null => V2Json::Null,
            Value::Bool(b) => V2Json::Bool(*b),
            Value::Number(n) => V2Json::Number(stored_number_text(n)),
            Value::String(s) => V2Json::Text(s.clone()),
            Value::Array(items) => V2Json::Array(items.iter().map(V2Json::from_value).collect()),
            Value::Object(map) => V2Json::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), V2Json::from_value(v)))
                    .collect(),
            ),
        }
    }

    /// The JSON text, without spaces, keys in the order built: what
    /// `JSON.stringify` gave v2.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    /// The same as a `serde_json::Value`. **The order of keys is lost**; use it
    /// to compare, not to answer a screen.
    pub fn to_value(&self) -> Value {
        match self {
            V2Json::Null => Value::Null,
            V2Json::Bool(b) => Value::Bool(*b),
            V2Json::Number(text) => serde_json::from_str::<Value>(text)
                .ok()
                .filter(Value::is_number)
                .unwrap_or(Value::Null),
            V2Json::Text(text) => Value::String(text.clone()),
            V2Json::Array(items) => Value::Array(items.iter().map(V2Json::to_value).collect()),
            V2Json::Object(pairs) => Value::Object(
                pairs
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_value()))
                    .collect(),
            ),
        }
    }

    fn write(&self, out: &mut String) {
        match self {
            V2Json::Null => out.push_str("null"),
            V2Json::Bool(true) => out.push_str("true"),
            V2Json::Bool(false) => out.push_str("false"),
            V2Json::Number(text) => out.push_str(text),
            V2Json::Text(text) => write_text(text, out),
            V2Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            V2Json::Object(pairs) => {
                out.push('{');
                for (i, (key, value)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_text(key, out);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// A JSON string as `JSON.stringify` writes it: `"` and `\` escaped, the five
/// short escapes, every other control character as `\u00xx`, the rest as is.
fn write_text(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A stored JSON number as v2 printed it: a float that holds a whole number
/// loses its `.0`.
fn stored_number_text(n: &serde_json::Number) -> String {
    match n.as_f64() {
        Some(f) if n.is_f64() && f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 => {
            format!("{f:.0}")
        }
        _ => n.to_string(),
    }
}

/// Plain decimal text (`"1000.50"`, `"-0.50"`, `"32.1000"`) as JavaScript prints
/// the number: no trailing zero, no trailing point.
pub fn shortest(decimal: &str) -> String {
    if !decimal.contains('.') {
        return decimal.to_string();
    }
    decimal
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn optional<T>(value: Option<T>, print: impl FnOnce(T) -> V2Json) -> V2Json {
    value.map_or(V2Json::Null, print)
}

fn text_or_null(value: Option<&String>) -> V2Json {
    optional(value, |text| V2Json::text(text.as_str()))
}

fn list<T>(items: &[T], print: impl Fn(&T) -> V2Json) -> V2Json {
    V2Json::Array(items.iter().map(print).collect())
}

// ── numbers ──────────────────────────────────────────────────────────────────

/// An amount as the JSON number v2 printed: `100.00` → `100`, `1000.50` →
/// `1000.5`, `0.01` → `0.01`. Never `100.0`, never a string. Printed from the
/// decimal itself, never through a float.
pub fn money_json(amount: Money) -> V2Json {
    V2Json::Number(shortest(&amount.to_string()))
}

/// A rate as the JSON number v2 printed: `32.1000` → `32.1`, `0.2105` → `0.2105`.
pub fn rate_json(rate: Rate) -> V2Json {
    V2Json::Number(shortest(&rate.to_string()))
}

/// A signed amount as the JSON number v2 printed: `-0.50` → `-0.5`, zero → `0`.
pub fn delta_json(delta: MoneyDelta) -> V2Json {
    V2Json::Number(shortest(&delta.to_string()))
}

// ── the 153-code objects ─────────────────────────────────────────────────────

/// Every register code in register order (`THB` first) with its amount, zeros
/// included: v2's `opening`, `expected`, `counted`, `safe`, `target.amounts`.
pub fn balances_json(balances: &Balances) -> V2Json {
    V2Json::Object(
        CurrencyCode::all()
            .iter()
            .map(|code| (code.as_str().to_string(), money_json(balances.get(*code))))
            .collect(),
    )
}

/// The same for signed amounts: v2's `difference`, `variance`,
/// `targetDifference`, `safeVariance`, `cashVariance`.
pub fn stock_delta_json(delta: &StockDelta) -> V2Json {
    V2Json::Object(
        CurrencyCode::all()
            .iter()
            .map(|code| (code.as_str().to_string(), delta_json(delta.get(*code))))
            .collect(),
    )
}

// ── time ─────────────────────────────────────────────────────────────────────

/// ISO-8601 in UTC with milliseconds: `"2026-10-06T05:00:00.000Z"`. What is
/// finer than a millisecond is cut off.
pub fn time_json(at: DateTime<Utc>) -> V2Json {
    V2Json::Text(at.to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// [`time_json`], or `""` — not `null` — for a time that has not come.
pub fn time_or_empty_json(at: Option<DateTime<Utc>>) -> V2Json {
    at.map_or_else(|| V2Json::text(""), time_json)
}

/// A custody time (epoch milliseconds) as [`time_json`] prints it.
pub fn custody_time_json(at: CustodyTime) -> V2Json {
    time_or_empty_json(DateTime::<Utc>::from_timestamp_millis(at.epoch_millis()))
}

// ── rates ────────────────────────────────────────────────────────────────────

/// v2's `rates`: one row per currency in the order given (`RateService::current`
/// gives the 152 in register order), `THB` left out.
///
/// `{code, name, symbol?, buy, sell, updated, requestId, partnerName,
/// partnerCost, botReference}` — `symbol` is absent where there is none; an
/// unset rate is `null`, `null`, `""`, `""`.
pub fn rates_json(rates: &[CounterRate]) -> V2Json {
    V2Json::Array(
        rates
            .iter()
            .filter(|rate| !rate.code.is_thb())
            .map(counter_rate_json)
            .collect(),
    )
}

fn counter_rate_json(rate: &CounterRate) -> V2Json {
    let mut pairs = vec![
        ("code", V2Json::text(rate.code.as_str())),
        ("name", V2Json::text(rate.name.as_str())),
    ];
    if let Some(symbol) = rate.symbol.as_deref().filter(|s| !s.is_empty()) {
        pairs.push(("symbol", V2Json::text(symbol)));
    }
    pairs.extend([
        ("buy", optional(rate.buy, rate_json)),
        ("sell", optional(rate.sell, rate_json)),
        ("updated", time_or_empty_json(rate.updated_at)),
        (
            "requestId",
            V2Json::text(rate.request_id.as_deref().unwrap_or_default()),
        ),
        ("partnerName", V2Json::text(rate.partner_name.as_str())),
        ("partnerCost", optional(rate.partner_cost, rate_json)),
        (
            "botReference",
            optional(rate.reference.as_ref(), V2Json::from_value),
        ),
    ]);
    V2Json::object(pairs)
}

/// v2's `rateHistory`, in the order given (`RateService::history` gives the
/// newest first): `{code, buy, sell, updated, actor, partnerName, partnerCost,
/// botReference}`. The request id is not shown.
pub fn rate_history_json(history: &[RateChange]) -> V2Json {
    list(history, |change| {
        V2Json::object(vec![
            ("code", V2Json::text(change.code.as_str())),
            ("buy", rate_json(change.buy)),
            ("sell", rate_json(change.sell)),
            ("updated", time_json(change.updated_at)),
            ("actor", V2Json::text(change.updated_by.as_str())),
            ("partnerName", V2Json::text(change.partner_name.as_str())),
            ("partnerCost", optional(change.partner_cost, rate_json)),
            (
                "botReference",
                optional(change.reference.as_ref(), V2Json::from_value),
            ),
        ])
    })
}

// ── branches and customers ───────────────────────────────────────────────────

/// v2's `branches`, in the order given: `{id, name, requestId?}`. A branch no
/// request created (`MAIN`) has no `requestId` key.
pub fn branches_json(branches: &[Branch]) -> V2Json {
    list(branches, |branch| {
        let mut pairs = vec![
            ("id", V2Json::text(branch.id.as_str())),
            ("name", V2Json::text(branch.name.as_str())),
        ];
        if let Some(request_id) = branch.request_id.as_deref().filter(|id| !id.is_empty()) {
            pairs.push(("requestId", V2Json::text(request_id)));
        }
        V2Json::object(pairs)
    })
}

/// v2's `customers`, in the order given, LIVE only: `{id, name, nationality,
/// created, member, memberVerifiedAt}`. No passport number, no date of birth.
/// `member` is `false` and `memberVerifiedAt` `null` until D5.
pub fn customers_json(customers: &[Customer]) -> V2Json {
    V2Json::Array(
        customers
            .iter()
            .filter(|customer| customer.mode == Mode::Live)
            .map(|customer| {
                V2Json::object(vec![
                    ("id", V2Json::text(customer.id.as_str())),
                    ("name", V2Json::text(customer.name.as_str())),
                    ("nationality", V2Json::text(customer.nationality.as_str())),
                    ("created", time_json(customer.created_at)),
                    ("member", V2Json::Bool(false)),
                    ("memberVerifiedAt", V2Json::Null),
                ])
            })
            .collect(),
    )
}

// ── shifts and movements ─────────────────────────────────────────────────────

/// v2's `stock.shifts`, in the order given: `{id, status, openedAt, staff,
/// opening, closedAt, closedBy, expected, counted, difference, note,
/// openRequest, closeRequest}`.
///
/// While a shift is OPEN `expected`, `counted` and `difference` are `null` —
/// v3 always holds an expected figure, but the screens need the three falsy on
/// an open shift — and `closedAt`, `closedBy`, `closeRequest` are `""`.
pub fn shifts_json(shifts: &[Shift]) -> V2Json {
    list(shifts, |shift| {
        let closed = shift.status == ShiftStatus::Closed;
        let (expected, counted, difference) = if closed {
            (
                balances_json(&shift.expected),
                optional(shift.counted.as_ref(), balances_json),
                optional(shift.difference.as_ref(), stock_delta_json),
            )
        } else {
            (V2Json::Null, V2Json::Null, V2Json::Null)
        };
        V2Json::object(vec![
            ("id", V2Json::text(shift.id.as_str())),
            (
                "status",
                V2Json::text(if closed { "CLOSED" } else { "OPEN" }),
            ),
            ("openedAt", time_json(shift.opened_at)),
            ("staff", V2Json::text(shift.opened_by.as_str())),
            ("opening", balances_json(&shift.opening)),
            ("closedAt", time_or_empty_json(shift.closed_at)),
            (
                "closedBy",
                V2Json::text(shift.closed_by.as_deref().unwrap_or_default()),
            ),
            ("expected", expected),
            ("counted", counted),
            ("difference", difference),
            ("note", V2Json::text(shift.note.as_str())),
            ("openRequest", V2Json::text(shift.open_request_id.as_str())),
            (
                "closeRequest",
                V2Json::text(shift.close_request_id.as_deref().unwrap_or_default()),
            ),
        ])
    })
}

/// v2's `stock.movements`, in the order given: `{id, shiftId, created, staff,
/// currency, direction, amount, reason, requestId}`.
pub fn movements_json(movements: &[StockMovement]) -> V2Json {
    list(movements, |movement| {
        let direction = match movement.direction {
            Direction::In => "IN",
            Direction::Out => "OUT",
        };
        V2Json::object(vec![
            ("id", V2Json::text(movement.id.as_str())),
            ("shiftId", V2Json::text(movement.shift_id.as_str())),
            ("created", time_json(movement.at)),
            ("staff", V2Json::text(movement.by.as_str())),
            ("currency", V2Json::text(movement.currency.as_str())),
            ("direction", V2Json::text(direction)),
            ("amount", money_json(movement.amount)),
            ("reason", V2Json::text(movement.reason.as_str())),
            ("requestId", V2Json::text(movement.request_id.as_str())),
        ])
    })
}

// ── custody ──────────────────────────────────────────────────────────────────

/// What the accounts say about one person, for the names v2 kept inside
/// custody rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonLabel {
    /// The display name (`FR1 Front`).
    pub name: String,
    /// The account's role as v2 spells it (`FRONT`, `RATES`, `SYSTEM`, …). Read
    /// only where custody holds [`CustodyRole::Other`].
    pub role: String,
}

/// The people of a branch by employee id — disabled ones included, or a sender
/// who has left is printed without a name.
pub type People = BTreeMap<String, PersonLabel>;

/// v2's `operations`: `{enabled, phase, safe, target, entity, custodian,
/// attendance, requests, events, cutoff, exceptions, holds, drafts}` and, once
/// service has started, `serviceOpenedAt` — absent before, not `null`.
///
/// `events` are what `CustodyService::events` gives, printed in the order given
/// (oldest first). A request's optional keys are absent until they happen.
/// `people` supplies `attendance[].name`, `requests[].senderName`, the name of
/// every signature but the proposer's, and the role of anyone custody holds as
/// [`CustodyRole::Other`]; someone it does not know is printed with the name
/// `""` and the role `OTHER`.
pub fn operations_json(state: &CustodyState, events: &[StoredEvent], people: &People) -> V2Json {
    operations_with_safe_json(state, None, events, people)
}

pub fn operations_with_safe_json(
    state: &CustodyState,
    safe: Option<&Balances>,
    events: &[StoredEvent],
    people: &People,
) -> V2Json {
    let mut pairs = vec![
        ("enabled", V2Json::Bool(false)),
        ("phase", V2Json::text(state.phase.as_str())),
        ("safe", optional(safe, balances_json)),
        ("target", optional(state.target.as_ref(), target_json)),
        ("entity", optional(state.entity.as_ref(), entity_json)),
        ("custodian", V2Json::Null),
        (
            "attendance",
            list(&state.attendance, |a| attendance_json(a, people)),
        ),
        ("requests", V2Json::Array(Vec::new())),
        ("events", list(events, |e| event_json(e, people))),
        ("cutoff", optional(state.cutoff.as_ref(), cutoff_json)),
        ("exceptions", list(&state.exceptions, exception_json)),
        ("holds", list(&state.holds, hold_json)),
        ("drafts", list(&state.drafts, draft_json)),
    ];
    if let Some(opened) = state.service_opened_at {
        pairs.push(("serviceOpenedAt", custody_time_json(opened)));
    }
    V2Json::object(pairs)
}

fn name_of(employee_id: &str, people: &People) -> V2Json {
    V2Json::text(people.get(employee_id).map_or("", |p| p.name.as_str()))
}

/// The role as v2 spells it. Custody knows six roles; any other employee
/// (`RATES`, `SYSTEM`, …) is [`CustodyRole::Other`] there and named from the
/// account.
fn role_json(role: CustodyRole, employee_id: &str, people: &People) -> V2Json {
    let word = match role {
        CustodyRole::Owner => "OWNER",
        CustodyRole::Front => "FRONT",
        CustodyRole::Finance => "FINANCE",
        CustodyRole::Monitor => "MONITOR",
        CustodyRole::Manager => "MANAGER",
        CustodyRole::Backend => "BACKEND",
        CustodyRole::Other => people
            .get(employee_id)
            .map(|p| p.role.as_str())
            .filter(|role| !role.is_empty())
            .unwrap_or("OTHER"),
    };
    V2Json::text(word)
}

fn target_json(target: &CashTarget) -> V2Json {
    V2Json::object(vec![
        ("amounts", balances_json(&target.amounts)),
        ("by", V2Json::text(target.by.as_str())),
        ("at", custody_time_json(target.at)),
    ])
}

fn entity_json(entity: &Entity) -> V2Json {
    V2Json::object(vec![
        ("company", V2Json::text(entity.company.as_str())),
        ("site", V2Json::text(entity.site.as_str())),
        ("confirmedBy", V2Json::text(entity.confirmed_by.as_str())),
        ("at", custody_time_json(entity.at)),
    ])
}

fn attendance_json(attendance: &Attendance, people: &People) -> V2Json {
    V2Json::object(vec![
        ("id", V2Json::text(attendance.id.as_str())),
        ("employeeId", V2Json::text(attendance.employee_id.as_str())),
        ("name", name_of(&attendance.employee_id, people)),
        (
            "role",
            role_json(attendance.role, &attendance.employee_id, people),
        ),
        ("in", custody_time_json(attendance.clock_in)),
        ("out", optional(attendance.clock_out, custody_time_json)),
    ])
}

fn cutoff_json(cutoff: &Cutoff) -> V2Json {
    V2Json::object(vec![
        ("at", custody_time_json(cutoff.at)),
        ("by", V2Json::text(cutoff.by.as_str())),
        ("shiftId", V2Json::text(cutoff.shift_id.as_str())),
        ("lastRef", text_or_null(cutoff.last_ref.as_ref())),
        (
            "pending",
            list(&cutoff.pending, |r| V2Json::text(r.as_str())),
        ),
    ])
}

fn event_json(event: &StoredEvent, people: &People) -> V2Json {
    V2Json::object(vec![
        ("id", V2Json::text(event.id.as_str())),
        ("type", V2Json::text(event.kind.as_str())),
        ("at", time_json(event.at)),
        ("actor", V2Json::text(event.actor.as_str())),
        ("role", role_json(event.role, &event.actor, people)),
        ("details", V2Json::from_value(&event.details)),
    ])
}

fn exception_json(exception: &CashException) -> V2Json {
    let status = match exception.status {
        ExceptionStatus::Open => "OPEN",
        ExceptionStatus::Reviewed => "REVIEWED",
    };
    let mut pairs = vec![
        ("id", V2Json::text(exception.id.as_str())),
        ("requestId", V2Json::text(exception.request_id.as_str())),
        ("status", V2Json::text(status)),
        (
            "targetDifference",
            optional(exception.target_difference.as_ref(), stock_delta_json),
        ),
        (
            "cashVariance",
            optional(exception.cash_variance.as_ref(), stock_delta_json),
        ),
        (
            "safeVariance",
            optional(exception.safe_variance.as_ref(), stock_delta_json),
        ),
        ("note", V2Json::text(exception.note.as_str())),
        ("at", custody_time_json(exception.at)),
    ];
    if let Some(by) = &exception.reviewed_by {
        pairs.push(("reviewedBy", V2Json::text(by.as_str())));
    }
    if let Some(at) = exception.reviewed_at {
        pairs.push(("reviewedAt", custody_time_json(at)));
    }
    V2Json::object(pairs)
}

fn hold_json(hold: &CustomerHold) -> V2Json {
    let status = match hold.status {
        HoldStatus::Open => "OPEN",
        HoldStatus::Released => "RELEASED",
    };
    let mut pairs = vec![
        ("id", V2Json::text(hold.id.as_str())),
        ("customerId", V2Json::text(hold.customer_id.as_str())),
        ("reason", V2Json::text(hold.reason.as_str())),
        ("by", V2Json::text(hold.by.as_str())),
        ("at", custody_time_json(hold.at)),
        ("status", V2Json::text(status)),
    ];
    if let Some(by) = &hold.reviewed_by {
        pairs.push(("reviewedBy", V2Json::text(by.as_str())));
    }
    if let Some(at) = hold.reviewed_at {
        pairs.push(("reviewedAt", custody_time_json(at)));
    }
    if let Some(note) = &hold.note {
        pairs.push(("note", V2Json::text(note.as_str())));
    }
    V2Json::object(pairs)
}

fn draft_json(draft: &TxDraft) -> V2Json {
    let side = match draft.side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    };
    let status = match draft.status {
        DraftStatus::Open => "OPEN",
        DraftStatus::Completed => "COMPLETED",
    };
    let mut pairs = vec![
        ("id", V2Json::text(draft.id.as_str())),
        ("shiftId", V2Json::text(draft.shift_id.as_str())),
        ("employeeId", V2Json::text(draft.employee_id.as_str())),
        ("type", V2Json::text(side)),
        ("currency", V2Json::text(draft.currency.as_str())),
        ("amount", money_json(draft.amount)),
        ("at", custody_time_json(draft.at)),
        ("status", V2Json::text(status)),
    ];
    if let Some(document_ref) = &draft.document_ref {
        pairs.push(("documentRef", V2Json::text(document_ref.as_str())));
    }
    V2Json::object(pairs)
}
