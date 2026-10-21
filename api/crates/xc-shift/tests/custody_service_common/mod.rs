//! Shared by the custody service tests: the outside world the service asks
//! through its ports, the v2 fixture, and the written form in which a branch is
//! compared with what v2 held.
//! Provided with ticket T10 — do not edit.
//! CA-10 (#626, #2955): the fake answers the core's `CustodyFactsPort` (the v2 `CustodyPorts` is no longer asked),
//! and the written form no longer carries `CustodyState.requests` (deleted with the request model).
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, SecondsFormat, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;
use xc_shift::application::custody::service::{CustodyFactsPort, CustodyService};
use xc_shift::domain::custody::entities::stored_event::StoredEvent;
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use super::shift_service_common::TestDb;

pub type Custody = CustodyService<PgCustodyRepository, PgShiftRepository>;

// ── the outside world ───────────────────────────────────────────────────────

#[derive(Default)]
struct Outside {
    has_live_rate: bool,
    rates_updated: String,
    pending_transactions: Vec<String>,
    last_transaction: Option<String>,
    customers: BTreeSet<String>,
    /// evidence keys that do NOT belong to whoever asks
    foreign_evidence: BTreeSet<String>,
    /// employees of the branch that are not the usual cast, or `None` = not an employee
    employees: HashMap<String, Option<CustodyRole>>,
    evidence_questions: Vec<(String, String, Vec<String>)>,
}

/// What the other contexts would answer (rates, transactions, customers, files,
/// employees), set by the test. The service reaches them only through `CustodyFactsPort`.
#[derive(Default)]
pub struct FakeOutside {
    inner: Mutex<Outside>,
}

impl FakeOutside {
    pub fn new() -> Arc<FakeOutside> {
        Arc::new(FakeOutside::default())
    }

    /// A branch where a shift may be opened: a live rate exists.
    pub fn with_rates() -> Arc<FakeOutside> {
        let o = FakeOutside::new();
        o.set_rates(true, "2026-10-06T03:00:00.000Z");
        o
    }

    pub fn set_rates(&self, live: bool, updated: &str) {
        let mut o = self.inner.lock().unwrap();
        o.has_live_rate = live;
        o.rates_updated = updated.to_owned();
    }

    pub fn add_customer(&self, id: &str) {
        self.inner.lock().unwrap().customers.insert(id.to_owned());
    }

    pub fn disown_evidence(&self, key: &str) {
        self.inner
            .lock()
            .unwrap()
            .foreign_evidence
            .insert(key.to_owned());
    }

    pub fn set_employee(&self, id: &str, role: Option<CustodyRole>) {
        self.inner
            .lock()
            .unwrap()
            .employees
            .insert(id.to_owned(), role);
    }

    /// (branch, owner asked about, keys) of every evidence question so far.
    pub fn evidence_questions(&self) -> Vec<(String, String, Vec<String>)> {
        self.inner.lock().unwrap().evidence_questions.clone()
    }
}

impl CustodyFactsPort for FakeOutside {
    fn service_window_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _now: CustodyTime,
    ) -> BoxFuture<'a, AppResult<(ServiceWindow, i32)>> {
        // v2's hours, in Bangkok (UTC+7)
        Box::pin(async move { Ok((super::custody_common::v2_hours(), 7 * 60)) })
    }

    fn has_live_rate_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<(bool, String)>> {
        Box::pin(async move {
            let o = self.inner.lock().unwrap();
            Ok((o.has_live_rate, o.rates_updated.clone()))
        })
    }

    fn transactions_summary_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<(Vec<String>, Option<String>)>> {
        Box::pin(async move {
            let o = self.inner.lock().unwrap();
            Ok((o.pending_transactions.clone(), o.last_transaction.clone()))
        })
    }

    fn customer_exists_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move { Ok(self.inner.lock().unwrap().customers.contains(customer_id)) })
    }
}

pub fn custody(db: &TestDb, outside: &Arc<FakeOutside>) -> Custody {
    CustodyService::new(
        Arc::new(PgCustodyRepository::new(db.pool.clone())),
        db.service(),
        outside.clone(),
        db.branch_tx(),
    )
}

// ── time ────────────────────────────────────────────────────────────────────

pub fn at_millis(millis: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(millis).expect("a time in range")
}

pub fn iso(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn iso_ct(t: CustodyTime) -> Value {
    json!(iso(at_millis(t.epoch_millis())))
}

fn iso_opt(t: Option<CustodyTime>) -> Value {
    t.map(iso_ct).unwrap_or(Value::Null)
}

/// 2026-10-06, `hh:mm` in Bangkok.
pub fn bangkok(hhmm: &str) -> DateTime<Utc> {
    at_millis(super::custody_common::bangkok(hhmm))
}

// ── the written form of a branch ────────────────────────────────────────────

pub fn role_name(role: CustodyRole) -> &'static str {
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

fn balances_json(b: &BTreeMap<String, String>) -> Value {
    json!(b)
}

/// Ids are random on both sides: every id-looking text becomes `$n` by order of
/// first appearance, walking object keys in sorted order.
#[derive(Default)]
pub struct Ids {
    seen: Vec<String>,
}

fn looks_like_uuid(s: &[u8]) -> bool {
    s.len() == 36
        && s.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_digit() || (b'a'..=b'f').contains(c),
        })
}

impl Ids {
    fn text(&mut self, s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = String::new();
        let mut i = 0;
        while i < bytes.len() {
            if i + 36 <= bytes.len() && looks_like_uuid(&bytes[i..i + 36]) {
                let id = &s[i..i + 36];
                let n = match self.seen.iter().position(|x| x == id) {
                    Some(n) => n,
                    None => {
                        self.seen.push(id.to_owned());
                        self.seen.len() - 1
                    }
                };
                out.push_str(&format!("${}", n + 1));
                i += 36;
            } else {
                let ch = s[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
        out
    }

    /// A payload of the fixture names a record the server made as `"id": "$n"`:
    /// put this server's id for it back.
    pub fn resolve(&self, payload: &Value) -> Value {
        let mut p = payload.clone();
        if let Some(n) = p
            .get("id")
            .and_then(Value::as_str)
            .and_then(|t| t.strip_prefix('$'))
        {
            let n: usize = n.parse().expect("an id number");
            p["id"] = json!(self.seen[n - 1]);
        }
        p
    }

    pub fn normalise(&mut self, v: &Value) -> Value {
        match v {
            Value::String(s) => Value::String(self.text(s)),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.normalise(x)).collect()),
            // serde_json keeps object keys sorted, which is the order the fixture walked
            Value::Object(m) => Value::Object(
                m.iter()
                    .map(|(k, x)| (k.clone(), self.normalise(x)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

fn event_json(e: &StoredEvent) -> Value {
    json!({
        "type": e.kind.as_str(), "at": iso(e.at), "actor": e.actor,
        "role": role_name(e.role), "details": e.details,
    })
}

fn times_json(st: &CustodyState) -> Value {
    json!({
        "entity": st.entity.as_ref().map(|e| iso_ct(e.at)).unwrap_or(Value::Null),
        "target": st.target.as_ref().map(|t| iso_ct(t.at)).unwrap_or(Value::Null),
        "cutoff": st.cutoff.as_ref().map(|c| iso_ct(c.at)).unwrap_or(Value::Null),
        "serviceOpened": iso_opt(st.service_opened_at),
        "attendance": st.attendance.iter().map(|a| json!([iso_ct(a.clock_in), iso_opt(a.clock_out)])).collect::<Vec<_>>(),
        "holds": st.holds.iter().map(|h| json!([iso_ct(h.at), iso_opt(h.reviewed_at)])).collect::<Vec<_>>(),
        "drafts": st.drafts.iter().map(|d| iso_ct(d.at)).collect::<Vec<_>>(),
        "exceptions": st.exceptions.iter().map(|x| json!([iso_ct(x.at), iso_opt(x.reviewed_at)])).collect::<Vec<_>>(),
    })
}

fn flatten(doc: &Value) -> BTreeMap<String, Value> {
    fn put(out: &mut BTreeMap<String, Value>, key: String, v: &Value) {
        match v {
            Value::Array(a) => {
                out.insert(format!("{key}#len"), json!(a.len()));
                for (i, x) in a.iter().enumerate() {
                    out.insert(format!("{key}#{i}"), x.clone());
                }
            }
            other => {
                out.insert(key, other.clone());
            }
        }
    }
    let mut out = BTreeMap::new();
    for (k, v) in doc.as_object().expect("an object") {
        if k == "ops" || k == "times" {
            for (sub, x) in v.as_object().expect("an object") {
                put(&mut out, format!("{k}.{sub}"), x);
            }
        } else {
            put(&mut out, k.clone(), v);
        }
    }
    out
}

// ── a desk for the hand-written tests ───────────────────────────────────────

pub fn evidence(n: u32) -> String {
    format!("00000000-0000-4000-8000-0000000e{n:04x}")
}

pub fn request_id(n: u32) -> String {
    format!("00000000-0000-4000-8000-0000000a{n:04x}")
}

/// Count rows that add up to the totals: hundreds, then the rest as one piece.
pub fn rows(totals: &[(&str, i64)]) -> Value {
    let mut out = Vec::new();
    for (currency, units) in totals {
        if *units >= 100 {
            out.push(json!({ "currency": currency, "denomination": 100, "count": units / 100 }));
        }
        if units % 100 != 0 {
            out.push(json!({ "currency": currency, "denomination": units % 100, "count": 1 }));
        }
    }
    json!(out)
}

pub fn totals(pairs: &[(&str, i64)]) -> Value {
    Value::Object(
        pairs
            .iter()
            .map(|(c, v)| (c.to_string(), json!(v)))
            .collect(),
    )
}

/// One branch and the people around it, driven through `CustodyService::command`.
pub struct Desk {
    pub custody: Custody,
    pub outside: Arc<FakeOutside>,
    pub branch: BranchId,
    minute: i64,
    request: u32,
}

impl Desk {
    pub async fn new(db: &TestDb) -> Desk {
        let outside = FakeOutside::with_rates();
        Desk {
            custody: custody(db, &outside),
            outside,
            branch: db.branch("desk").await,
            minute: 0,
            request: 0,
        }
    }

    /// The next minute after 12:00 in Bangkok.
    pub fn tick(&mut self) -> DateTime<Utc> {
        self.minute += 1;
        bangkok("12:00") + chrono::Duration::minutes(self.minute)
    }

    /// The time of the latest command sent through this desk.
    pub fn now(&self) -> DateTime<Utc> {
        bangkok("12:00") + chrono::Duration::minutes(self.minute)
    }
}
