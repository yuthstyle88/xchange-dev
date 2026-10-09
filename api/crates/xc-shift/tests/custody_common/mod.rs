//! Shared by the custody tests: the v2 fixture, the conversions it needs, and a small
//! desk that plays the part of the service around the machine.
//! Provided with ticket T9 — do not edit.
#![allow(dead_code, unused_imports)]

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value, json};
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta, Side};
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::domain::shift::value_objects::stock_delta::{Direction, StockDelta};

// Every custody path the tests name is in this one block.
pub use xc_shift::domain::custody::commands::custody_command::{ClockDirection, CustodyCommand};
pub use xc_shift::domain::custody::commands::proposal::Proposal;
pub use xc_shift::domain::custody::entities::attendance::Attendance;
pub use xc_shift::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
pub use xc_shift::domain::custody::entities::custody_machine::CustodyMachine;
pub use xc_shift::domain::custody::entities::custody_request::{CustodyRequest, Decision};
pub use xc_shift::domain::custody::entities::customer_hold::{CustomerHold, HoldStatus};
pub use xc_shift::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
pub use xc_shift::domain::custody::state::CustodyState;
pub use xc_shift::domain::custody::value_objects::cash_target::CashTarget;
pub use xc_shift::domain::custody::value_objects::count_sheet::CountSheet;
pub use xc_shift::domain::custody::value_objects::custody_actor::CustodyActor;
pub use xc_shift::domain::custody::value_objects::custody_error::CustodyError;
pub use xc_shift::domain::custody::value_objects::custody_event::{CustodyEvent, EventKind};
pub use xc_shift::domain::custody::value_objects::custody_facts::{CustodyFacts, OpenShift};
pub use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
pub use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
pub use xc_shift::domain::custody::value_objects::cutoff::Cutoff;
pub use xc_shift::domain::custody::value_objects::denomination_row::DenominationRow;
pub use xc_shift::domain::custody::value_objects::entity::Entity;
pub use xc_shift::domain::custody::value_objects::phase::Phase;
pub use xc_shift::domain::custody::value_objects::request_kind::RequestKind;
pub use xc_shift::domain::custody::value_objects::request_status::{
    Reconciliation, RequestStatus, TransferStage,
};
pub use xc_shift::domain::custody::value_objects::save_attempt::{PriorSave, SaveAttempt};
pub use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;
pub use xc_shift::domain::custody::value_objects::shift_effect::ShiftEffect;
pub use xc_shift::domain::custody::value_objects::signature::Signature;
pub use xc_shift::domain::custody::value_objects::transition::Transition;

// ── the fixture ──────────────────────────────────────────────────────────────

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ShiftFact {
    pub id: String,
    pub expected: BTreeMap<String, String>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Facts {
    pub open_shift: Option<ShiftFact>,
    pub has_live_rate: bool,
    pub rates_updated: String,
    pub pending_transactions: Vec<String>,
    pub last_transaction: Option<String>,
    pub customer_known: bool,
    /// Absent on the facts a matrix state shares; each probe carries its own.
    #[serde(default)]
    pub new_id: String,
}

/// What v2 answered: a refusal, or the events, shift operations, returned id and the
/// parts of the state that changed.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub error: Option<String>,
    #[serde(default)]
    pub v2_differs_on_purpose: bool,
    #[serde(default)]
    pub events: Vec<Value>,
    #[serde(default)]
    pub effects: Vec<Value>,
    pub id: Option<String>,
    #[serde(default)]
    pub changed: Map<String, Value>,
}

#[derive(Deserialize)]
pub struct Step {
    pub actor: String,
    pub command: Value,
    pub now: i64,
    pub facts: Facts,
    #[serde(flatten)]
    pub answer: Answer,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub actor: String,
    pub command: usize,
    pub new_id: String,
    #[serde(flatten)]
    pub answer: Answer,
}

#[derive(Deserialize)]
pub struct MatrixState {
    pub state: String,
    pub phase: String,
    pub now: i64,
    pub facts: Facts,
    pub commands: Vec<Value>,
    pub steps: Vec<Step>,
    pub probes: Vec<Probe>,
}

#[derive(Deserialize)]
pub struct Story {
    pub name: String,
    pub steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub matrix_states: usize,
    pub probes: usize,
    pub probes_accepted: usize,
    pub stories: usize,
    pub story_steps: usize,
    pub story_steps_refused: usize,
    pub distinct_errors: usize,
    pub v2_differs_on_purpose: usize,
}

#[derive(Deserialize)]
pub struct Fixture {
    pub counts: Counts,
    pub matrix: Vec<MatrixState>,
    pub stories: Vec<Story>,
}

pub fn fixture() -> Fixture {
    serde_json::from_str(include_str!("../fixtures/v2_custody_cases.json")).expect("fixture parses")
}

// ── figures ──────────────────────────────────────────────────────────────────

pub fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).unwrap_or_else(|_| panic!("{text} is a registered code"))
}

/// `"12.5"` → 1250 hundredths. At most two decimals, no sign.
pub fn money(text: &str) -> Money {
    let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
    let minor: i128 = format!("{whole}{frac:0<2}").parse().expect("digits");
    MoneyDelta::from_minor(minor)
        .to_money()
        .expect("not negative")
}

pub fn bal(pairs: &[(&str, &str)]) -> Balances {
    let pairs: Vec<(CurrencyCode, Money)> =
        pairs.iter().map(|(c, v)| (code(c), money(v))).collect();
    Balances::of(&pairs)
}

fn balances(map: &BTreeMap<String, String>) -> Balances {
    let pairs: Vec<(CurrencyCode, Money)> = map.iter().map(|(c, v)| (code(c), money(v))).collect();
    Balances::of(&pairs)
}

fn text_map(v: &Value) -> BTreeMap<String, String> {
    serde_json::from_value(v.clone()).expect("a map of figures")
}

/// A count whose rows add up: hundreds, then what is left as one piece.
pub fn sheet(pairs: &[(&str, &str)]) -> CountSheet {
    let mut denominations = Vec::new();
    for (c, v) in pairs {
        let minor = money(v).minor();
        if minor / 10_000 > 0 {
            denominations.push(row(c, "100", (minor / 10_000) as u32));
        }
        if minor % 10_000 > 0 {
            let rest = Money::from_minor(minor % 10_000).unwrap();
            denominations.push(row(c, &rest.to_string(), 1));
        }
    }
    CountSheet {
        totals: bal(pairs),
        denominations,
    }
}

pub fn row(currency: &str, denomination: &str, count: u32) -> DenominationRow {
    DenominationRow {
        currency: code(currency),
        denomination: money(denomination),
        count,
    }
}

// ── people ───────────────────────────────────────────────────────────────────

/// `owner` is the owner session; the others are employees whose id is their name here:
/// `F1`‥ Front, `FIN1`‥ Finance, `MON1` Monitor, `MGR1` Manager, `BE1`‥ Backend, `RT1` Rates.
pub fn actor(name: &str) -> CustodyActor {
    if name == "owner" {
        return CustodyActor {
            id: "owner-1".into(),
            employee_id: None,
            role: CustodyRole::Owner,
        };
    }
    let role = match name.trim_end_matches(|c: char| c.is_ascii_digit()) {
        "F" => CustodyRole::Front,
        "FIN" => CustodyRole::Finance,
        "MON" => CustodyRole::Monitor,
        "MGR" => CustodyRole::Manager,
        "BE" => CustodyRole::Backend,
        "RT" => CustodyRole::Other,
        other => panic!("unknown actor {other}"),
    };
    CustodyActor {
        id: name.into(),
        employee_id: Some(name.into()),
        role,
    }
}

fn role_name(role: CustodyRole) -> &'static str {
    match role {
        CustodyRole::Owner => "OWNER",
        CustodyRole::Front => "FRONT",
        CustodyRole::Finance => "FINANCE",
        CustodyRole::Monitor => "MONITOR",
        CustodyRole::Manager => "MANAGER",
        CustodyRole::Backend => "BACKEND",
        CustodyRole::Other => "RATES",
    }
}

// ── fixture → machine input ──────────────────────────────────────────────────

pub fn facts(f: &Facts, new_id: &str) -> CustodyFacts {
    CustodyFacts {
        service_window: v2_hours(),
        open_shift: f.open_shift.as_ref().map(|s| OpenShift {
            id: s.id.clone(),
            expected: balances(&s.expected),
        }),
        has_live_rate: f.has_live_rate,
        rates_updated: f.rates_updated.clone(),
        pending_transactions: f.pending_transactions.clone(),
        last_transaction: f.last_transaction.clone(),
        customer_known: f.customer_known,
        new_id: new_id.to_string(),
    }
}

fn s(v: &Value, key: &str) -> String {
    v[key]
        .as_str()
        .unwrap_or_else(|| panic!("{key} in {v}"))
        .to_string()
}

fn count_sheet(v: &Value) -> CountSheet {
    CountSheet {
        totals: balances(&text_map(&v["totals"])),
        denominations: v["denominations"]
            .as_array()
            .expect("rows")
            .iter()
            .map(|r| {
                row(
                    r["currency"].as_str().unwrap(),
                    r["denomination"].as_str().unwrap(),
                    r["count"].as_u64().unwrap() as u32,
                )
            })
            .collect(),
    }
}

fn kind(text: &str) -> RequestKind {
    [
        RequestKind::Open,
        RequestKind::Handover,
        RequestKind::Close,
        RequestKind::SafeOpen,
        RequestKind::SafeIn,
        RequestKind::SafeOut,
        RequestKind::ExternalIn,
        RequestKind::ExternalOut,
    ]
    .into_iter()
    .find(|k| k.as_str() == text)
    .unwrap_or_else(|| panic!("kind {text}"))
}

fn side(text: &str) -> Side {
    if text == "BUY" { Side::Buy } else { Side::Sell }
}

/// A command as the fixture writes it (v2's action name and payload, figures as text).
pub fn command(v: &Value) -> CustodyCommand {
    match v["action"].as_str().expect("action") {
        "opsSetEntity" => CustodyCommand::SetEntity {
            company: s(v, "company"),
            site: s(v, "site"),
            confirm_separated: v["confirmSeparated"].as_bool().unwrap(),
        },
        "opsEnable" => CustodyCommand::Enable,
        "opsClock" => CustodyCommand::Clock(if v["direction"] == "IN" {
            ClockDirection::In
        } else {
            ClockDirection::Out
        }),
        "opsBeginTransaction" => CustodyCommand::BeginTransaction {
            side: side(&s(v, "type")),
            currency: code(&s(v, "currency")),
            amount: money(&s(v, "amount")),
        },
        "opsTarget" => CustodyCommand::SetTarget {
            amounts: balances(&text_map(&v["amounts"])),
        },
        "opsStartService" => CustodyCommand::StartService {
            equipment_ready: v["equipmentReady"].as_bool().unwrap(),
            rates_verified: v["ratesVerified"].as_bool().unwrap(),
            rate_updated: s(v, "rateUpdated"),
        },
        "opsCutoff" => CustodyCommand::Cutoff,
        "opsPropose" => CustodyCommand::Propose(Box::new(Proposal {
            request_id: s(v, "requestId"),
            payload_hash: s(v, "payloadHash"),
            kind: kind(&s(v, "kind")),
            receiver: v["receiver"].as_str().map(str::to_string),
            note: s(v, "note"),
            evidence_keys: serde_json::from_value(v["evidenceKeys"].clone()).unwrap(),
            counted: count_sheet(&v["counted"]),
            safe_count: count_sheet(&v["safeCount"]),
            currency: v["currency"].as_str().map(code),
            amount: v["amount"].as_str().map(money),
        })),
        "opsHold" => CustodyCommand::Hold {
            customer_id: s(v, "customerId"),
            reason: s(v, "reason"),
        },
        "opsRelease" => CustodyCommand::Release {
            hold_id: s(v, "id"),
            note: s(v, "note"),
        },
        "opsReject" => CustodyCommand::Reject {
            id: s(v, "id"),
            reason: s(v, "reason"),
        },
        "opsSign" => CustodyCommand::Sign { id: s(v, "id") },
        "opsDispatch" => CustodyCommand::Dispatch { id: s(v, "id") },
        "opsCommit" => CustodyCommand::Commit { id: s(v, "id") },
        "opsReconcile" => CustodyCommand::Reconcile {
            id: s(v, "id"),
            note: s(v, "note"),
        },
        other => panic!("unknown action {other}"),
    }
}

// ── machine output → what the fixture writes ─────────────────────────────────

fn shown(b: &Balances) -> Value {
    let map: Map<String, Value> = b
        .non_zero()
        .into_iter()
        .map(|(c, v)| (c.as_str().to_string(), json!(v.to_string())))
        .collect();
    Value::Object(map)
}

fn shown_delta(d: &StockDelta) -> Value {
    let map: Map<String, Value> = d
        .non_zero()
        .into_iter()
        .map(|(c, v)| (c.as_str().to_string(), json!(v.to_string())))
        .collect();
    Value::Object(map)
}

fn opt<T>(v: &Option<T>, f: impl Fn(&T) -> Value) -> Value {
    v.as_ref().map(f).unwrap_or(Value::Null)
}

pub fn shown_events(events: &[CustodyEvent]) -> Vec<Value> {
    events
        .iter()
        .map(|e| json!({ "kind": e.kind.as_str(), "subject": e.subject }))
        .collect()
}

pub fn shown_effects(effects: &[ShiftEffect]) -> Vec<Value> {
    effects
        .iter()
        .map(|e| match e {
            ShiftEffect::OpenShift {
                request_id,
                opening,
            } => {
                json!({ "effect": "OpenShift", "requestId": request_id, "opening": shown(opening) })
            }
            ShiftEffect::CloseShift {
                request_id,
                shift_id,
                expected,
                counted,
                note,
            } => json!({ "effect": "CloseShift", "requestId": request_id, "shiftId": shift_id,
                "expected": shown(expected), "counted": shown(counted), "note": note }),
            ShiftEffect::MoveStock {
                request_id,
                shift_id,
                currency,
                direction,
                amount,
                reason,
            } => json!({ "effect": "MoveStock", "requestId": request_id, "shiftId": shift_id,
                "currency": currency.as_str(),
                "direction": if *direction == Direction::In { "IN" } else { "OUT" },
                "amount": amount.to_string(), "reason": reason }),
        })
        .collect()
}

fn shown_request(r: &CustodyRequest) -> Value {
    json!({
        "id": r.id, "requestId": r.request_id, "payloadHash": r.payload_hash, "kind": r.kind.as_str(),
        "status": match r.status {
            RequestStatus::Pending => "PENDING",
            RequestStatus::Rejected => "REJECTED",
            RequestStatus::Completed => "COMPLETED",
        },
        "stage": opt(&r.stage, |s| json!(match s {
            TransferStage::InTransit => "IN_TRANSIT",
            TransferStage::Received => "RECEIVED",
        })),
        "sender": r.sender, "receiver": r.receiver, "note": r.note, "shiftId": r.shift_id,
        "evidenceKeys": r.evidence_keys,
        "signatures": r.signatures.iter().map(|s| json!([s.employee_id, role_name(s.role)])).collect::<Vec<_>>(),
        "counted": opt(&r.counted, |c| shown(&c.totals)),
        "targetDifference": opt(&r.target_difference, shown_delta),
        "expected": opt(&r.expected, shown),
        "variance": opt(&r.variance, shown_delta),
        "safeCount": opt(&r.safe_count, |c| shown(&c.totals)),
        "safeExpected": opt(&r.safe_expected, shown),
        "safeVariance": opt(&r.safe_variance, shown_delta),
        "cutoffShift": opt(&r.cutoff, |c| json!(c.shift_id)),
        "currency": opt(&r.currency, |c| json!(c.as_str())),
        "amount": opt(&r.amount, |a| json!(a.to_string())),
        "dispatchedBy": r.dispatched_by,
        "rejected": opt(&r.rejected, |d| json!([d.by, d.text])),
        "completed": r.completed_at.is_some(),
        "reconciliation": opt(&r.reconciliation, |x| json!(match x {
            Reconciliation::Pending => "PENDING",
            Reconciliation::Checked => "CHECKED",
        })),
        "reconciled": opt(&r.reconciled, |d| json!([d.by, d.text])),
    })
}

/// The whole custody state the way the fixture writes it (times left out).
pub fn shown_state(st: &CustodyState) -> Map<String, Value> {
    let v = json!({
        "enabled": st.enabled,
        "phase": st.phase.as_str(),
        "custodian": st.custodian,
        "safe": opt(&st.safe, shown),
        "target": opt(&st.target, |t| json!({ "amounts": shown(&t.amounts), "by": t.by })),
        "entity": opt(&st.entity, |e| json!({ "company": e.company, "site": e.site, "confirmedBy": e.confirmed_by })),
        "clockedIn": st.attendance.iter().filter(|a| a.clock_out.is_none()).map(|a| a.employee_id.clone()).collect::<Vec<_>>(),
        "attendance": st.attendance.iter().map(|a| json!({ "id": a.id, "employeeId": a.employee_id,
            "role": role_name(a.role), "out": a.clock_out.is_some() })).collect::<Vec<_>>(),
        "cutoff": opt(&st.cutoff, |c| json!({ "by": c.by, "shiftId": c.shift_id, "lastRef": c.last_ref, "pending": c.pending })),
        "serviceOpened": st.service_opened_at.is_some(),
        "requests": st.requests.iter().map(shown_request).collect::<Vec<_>>(),
        "holds": st.holds.iter().map(|h| json!({ "id": h.id, "customerId": h.customer_id, "reason": h.reason,
            "by": h.by, "status": if h.status == HoldStatus::Open { "OPEN" } else { "RELEASED" },
            "reviewedBy": h.reviewed_by, "note": h.note })).collect::<Vec<_>>(),
        "drafts": st.drafts.iter().map(|d| json!({ "id": d.id, "shiftId": d.shift_id, "employeeId": d.employee_id,
            "type": if d.side == Side::Buy { "BUY" } else { "SELL" }, "currency": d.currency.as_str(),
            "amount": d.amount.to_string(),
            "status": if d.status == DraftStatus::Open { "OPEN" } else { "COMPLETED" } })).collect::<Vec<_>>(),
        "exceptions": st.exceptions.iter().map(|x| json!({ "id": x.id, "requestId": x.request_id,
            "status": if x.status == ExceptionStatus::Open { "OPEN" } else { "REVIEWED" },
            "targetDifference": opt(&x.target_difference, shown_delta),
            "cashVariance": opt(&x.cash_variance, shown_delta),
            "safeVariance": opt(&x.safe_variance, shown_delta),
            "note": x.note, "reviewedBy": x.reviewed_by })).collect::<Vec<_>>(),
    });
    match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

/// Applies one fixture command and holds the machine to v2's answer.
/// `Some(state)` when v2 accepted, `None` when it refused.
pub fn check(
    label: &str,
    state: &CustodyState,
    who: &str,
    cmd: &Value,
    facts: &CustodyFacts,
    now: i64,
    answer: &Answer,
) -> Option<CustodyState> {
    let got = CustodyMachine::apply(
        state,
        &command(cmd),
        &actor(who),
        facts,
        CustodyTime::from_epoch_millis(now),
    );
    match (&answer.error, got) {
        (Some(sentence), Err(e)) => {
            assert_eq!(&e.to_string(), sentence, "{label}: {who} {cmd}");
            None
        }
        (Some(sentence), Ok(_)) => panic!("{label}: {who} {cmd} must be refused: {sentence}"),
        (None, Err(e)) => panic!("{label}: {who} {cmd} must be accepted, got {e:?} ({e})"),
        (None, Ok(t)) => {
            assert_eq!(
                shown_events(&t.events),
                answer.events,
                "{label}: events of {who} {cmd}"
            );
            assert_eq!(
                shown_effects(&t.effects),
                answer.effects,
                "{label}: shift operations of {who} {cmd}"
            );
            assert_eq!(t.id, answer.id, "{label}: id returned to {who} {cmd}");
            let mut expected = shown_state(state);
            for (k, v) in &answer.changed {
                expected.insert(k.clone(), v.clone());
            }
            let after = shown_state(&t.state);
            for (k, v) in &expected {
                assert_eq!(&after[k], v, "{label}: `{k}` after {who} {cmd}");
            }
            Some(t.state)
        }
    }
}

// ── a desk for the hand-written cases ────────────────────────────────────────

pub const NOON: i64 = 1_791_262_800_000; // 2026-10-06 12:00 in Bangkok

/// `"11:00"` in Bangkok on 2026-10-06, as epoch milliseconds.
pub fn bangkok(hhmm: &str) -> i64 {
    let (h, m) = hhmm.split_once(':').expect("hh:mm");
    let minutes: i64 = h.parse::<i64>().unwrap() * 60 + m.parse::<i64>().unwrap();
    NOON + (minutes - 12 * 60) * 60_000
}

pub fn v2_hours() -> ServiceWindow {
    ServiceWindow::Open {
        open: CustodyTime::from_epoch_millis(bangkok("11:00")),
        close: CustodyTime::from_epoch_millis(bangkok("23:00")),
    }
}

pub fn request_id(n: u32) -> String {
    format!("00000000-0000-4000-8000-0000000a{n:04x}")
}

pub fn evidence(n: u32) -> String {
    format!("00000000-0000-4000-8000-0000000e{n:04x}")
}

pub fn proposal(kind: RequestKind, n: u32) -> Proposal {
    Proposal {
        request_id: request_id(n),
        payload_hash: format!("hash-{n}"),
        kind,
        receiver: None,
        note: String::new(),
        evidence_keys: vec![evidence(1)],
        counted: CountSheet::empty(),
        safe_count: CountSheet::empty(),
        currency: None,
        amount: None,
    }
}

/// The custody state of one branch together with the outside world the machine is told
/// about: the open shift (kept in step with the shift operations each transition asks for),
/// the rates, the clock. Each accepted command moves the clock one minute.
pub struct Desk {
    pub state: CustodyState,
    pub shift: Option<OpenShift>,
    pub now: i64,
    pub rates_updated: String,
    pub has_live_rate: bool,
    pub pending_transactions: Vec<String>,
    pub customer_known: bool,
    pub last: Option<Transition>,
    shifts: u32,
    ids: u32,
}

pub const DRAWER: &[(&str, &str)] = &[("THB", "50000"), ("USD", "1000")];
pub const SAFE: &[(&str, &str)] = &[("THB", "200000"), ("USD", "5000"), ("EUR", "300")];
pub const STAFF: &[&str] = &[
    "F1", "F2", "FIN1", "FIN2", "MON1", "MGR1", "BE1", "BE2", "RT1",
];

impl Desk {
    /// Custody never used.
    pub fn new() -> Desk {
        Desk {
            state: CustodyState::initial(),
            shift: None,
            now: NOON,
            rates_updated: "2026-10-06T03:00:00.000Z".into(),
            has_live_rate: true,
            pending_transactions: Vec::new(),
            customer_known: true,
            last: None,
            shifts: 0,
            ids: 0,
        }
    }

    pub fn facts(&self) -> CustodyFacts {
        CustodyFacts {
            service_window: v2_hours(),
            open_shift: self.shift.clone(),
            has_live_rate: self.has_live_rate,
            rates_updated: self.rates_updated.clone(),
            pending_transactions: self.pending_transactions.clone(),
            last_transaction: None,
            customer_known: self.customer_known,
            new_id: format!("id-{}", self.ids + 1),
        }
    }

    /// Applies a command at the desk's clock; an accepted one becomes the new state.
    pub fn act(&mut self, who: &str, cmd: CustodyCommand) -> Result<Transition, CustodyError> {
        self.act_at(who, cmd, self.now + 60_000)
    }

    pub fn act_at(
        &mut self,
        who: &str,
        cmd: CustodyCommand,
        now: i64,
    ) -> Result<Transition, CustodyError> {
        let before = self.state.clone();
        let facts = self.facts();
        let got = CustodyMachine::apply(
            &self.state,
            &cmd,
            &actor(who),
            &facts,
            CustodyTime::from_epoch_millis(now),
        );
        assert_eq!(
            self.state, before,
            "apply must not change the state it was given"
        );
        let t = got?;
        self.ids += 1;
        self.now = self.now.max(now);
        for e in &t.effects {
            match e {
                ShiftEffect::OpenShift { opening, .. } => {
                    assert!(self.shift.is_none(), "a shift is already open");
                    self.shifts += 1;
                    self.shift = Some(OpenShift {
                        id: format!("S{}", self.shifts),
                        expected: opening.clone(),
                    });
                }
                ShiftEffect::CloseShift { shift_id, .. } => {
                    assert_eq!(Some(shift_id), self.shift.as_ref().map(|s| &s.id));
                    self.shift = None;
                }
                ShiftEffect::MoveStock {
                    currency,
                    direction,
                    amount,
                    ..
                } => {
                    let shift = self.shift.as_mut().expect("a shift to move stock in");
                    let have = shift.expected.get(*currency).minor();
                    let after = match direction {
                        Direction::In => have + amount.minor(),
                        Direction::Out => have - amount.minor(),
                    };
                    let mut pairs = shift.expected.non_zero();
                    pairs.push((*currency, Money::from_minor(after).expect("enough stock")));
                    shift.expected = Balances::of(&pairs);
                }
            }
        }
        self.state = t.state.clone();
        self.last = Some(t.clone());
        Ok(t)
    }

    /// Accepts or panics.
    pub fn ok(&mut self, who: &str, cmd: CustodyCommand) -> Transition {
        let shown = format!("{cmd:?}");
        self.act(who, cmd)
            .unwrap_or_else(|e| panic!("{who} {shown}: {e:?} ({e})"))
    }

    /// The id of the request proposed last.
    pub fn request(&self) -> String {
        self.state.requests.last().expect("a request").id.clone()
    }

    pub fn propose(&mut self, who: &str, p: Proposal) -> String {
        self.ok(who, CustodyCommand::Propose(Box::new(p)))
            .id
            .expect("the request id")
    }

    pub fn sign(&mut self, who: &str, id: &str) {
        self.ok(who, CustodyCommand::Sign { id: id.into() });
    }

    pub fn commit(&mut self, who: &str, id: &str) -> Transition {
        self.ok(who, CustodyCommand::Commit { id: id.into() })
    }

    /// The owner has named the business and enabled custody; all of `STAFF` are clocked in.
    pub fn enabled() -> Desk {
        let mut d = Desk::new();
        d.ok(
            "owner",
            CustodyCommand::SetEntity {
                company: "XCHANGE Co., Ltd.".into(),
                site: "Main".into(),
                confirm_separated: true,
            },
        );
        d.ok("owner", CustodyCommand::Enable);
        for who in STAFF {
            d.ok(who, CustodyCommand::Clock(ClockDirection::In));
        }
        d
    }

    /// … and the safe holds `SAFE`, Finance has set `DRAWER` as the target. Phase `Closed`.
    pub fn with_safe() -> Desk {
        let mut d = Desk::enabled();
        let id = d.propose(
            "FIN1",
            Proposal {
                counted: sheet(SAFE),
                ..proposal(RequestKind::SafeOpen, 1)
            },
        );
        d.sign("MON1", &id);
        d.commit("BE1", &id);
        d.ok(
            "FIN1",
            CustodyCommand::SetTarget {
                amounts: bal(DRAWER),
            },
        );
        d
    }

    /// … and F1 has counted `DRAWER` into shift `S1`. Phase `Ready`, F1 holds the drawer.
    pub fn ready() -> Desk {
        let mut d = Desk::with_safe();
        let id = d.propose(
            "F1",
            Proposal {
                counted: sheet(DRAWER),
                ..proposal(RequestKind::Open, 2)
            },
        );
        d.commit("BE1", &id);
        d
    }

    /// … and Backend has opened the service. Phase `Open`.
    pub fn open() -> Desk {
        let mut d = Desk::ready();
        let rate_updated = d.rates_updated.clone();
        d.ok(
            "BE1",
            CustodyCommand::StartService {
                equipment_ready: true,
                rates_verified: true,
                rate_updated,
            },
        );
        d
    }

    /// … and Backend has cut off. Phase `CutOff`.
    pub fn cut_off() -> Desk {
        let mut d = Desk::open();
        d.ok("BE1", CustodyCommand::Cutoff);
        d
    }

    /// A transfer of 200 USD proposed by F1 to FIN1 from the open service.
    pub fn transfer(kind: RequestKind) -> (Desk, String) {
        let mut d = Desk::open();
        let id = d.propose(
            "F1",
            Proposal {
                receiver: Some("FIN1".into()),
                currency: Some(code("USD")),
                amount: Some(money("200")),
                ..proposal(kind, 3)
            },
        );
        (d, id)
    }
}
