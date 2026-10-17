//! Shared by the custody service tests: the outside world the service asks
//! through its ports, the v2 fixture, and the written form in which a branch is
//! compared with what v2 held.
//! Provided with ticket T10 — do not edit.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, SecondsFormat, Utc};
use futures::future::BoxFuture;
use pos_error::AppResult;
use serde_json::{Map, Value, json};
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::domain::custody::entities::stored_event::StoredEvent;
use xc_shift::domain::custody::repositories::custody_ports::{CustodyPorts, OutsideFacts};
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;
use xc_shift::domain::shift::entities::shift::ShiftStatus;
use xc_shift::domain::shift::value_objects::stock_delta::Direction;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use crate::custody_common::{actor, shown_state};
use crate::shift_service_common::{TestDb, shown_balances, shown_delta};

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
/// employees), set by the test. The service reaches them only through `CustodyPorts`.
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

impl CustodyPorts for FakeOutside {
    fn outside_facts<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _open_shift_id: Option<&'a str>,
    ) -> BoxFuture<'a, AppResult<OutsideFacts>> {
        Box::pin(async move {
            let o = self.inner.lock().unwrap();
            Ok(OutsideFacts {
                has_live_rate: o.has_live_rate,
                rates_updated: o.rates_updated.clone(),
                pending_transactions: o.pending_transactions.clone(),
                last_transaction: o.last_transaction.clone(),
            })
        })
    }

    fn customer_known<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move { Ok(self.inner.lock().unwrap().customers.contains(customer_id)) })
    }

    fn evidence_owned<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        owner_id: &'a str,
        keys: &'a [String],
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            let mut o = self.inner.lock().unwrap();
            o.evidence_questions.push((
                branch.as_str().to_owned(),
                owner_id.to_owned(),
                keys.to_vec(),
            ));
            Ok(keys.iter().all(|k| !o.foreign_evidence.contains(k)))
        })
    }

    fn employee_role<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        employee_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<CustodyRole>>> {
        Box::pin(async move {
            let o = self.inner.lock().unwrap();
            if let Some(answer) = o.employees.get(employee_id) {
                return Ok(*answer);
            }
            // the usual cast: F1, FIN1, MON1, MGR1, BE1 … are employees of every test branch
            let known = ["F", "FIN", "MON", "MGR", "BE", "RT"]
                .contains(&employee_id.trim_end_matches(|c: char| c.is_ascii_digit()));
            Ok(known.then(|| actor(employee_id).role))
        })
    }

    fn service_window<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _now: CustodyTime,
    ) -> BoxFuture<'a, AppResult<ServiceWindow>> {
        Box::pin(async move { Ok(crate::custody_common::v2_hours()) })
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
    at_millis(crate::custody_common::bangkok(hhmm))
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
    let rows = |sheet: &Option<
        xc_shift::domain::custody::value_objects::count_sheet::CountSheet,
    >| {
        sheet
            .as_ref()
            .map(|s| {
                json!(
                    s.denominations
                        .iter()
                        .map(|d| json!([d.currency.as_str(), d.denomination.to_string(), d.count]))
                        .collect::<Vec<_>>()
                )
            })
            .unwrap_or(Value::Null)
    };
    let shown = |b: &xc_shift::domain::shift::value_objects::balances::Balances| {
        balances_json(&shown_balances(b))
    };
    json!({
        "entity": st.entity.as_ref().map(|e| iso_ct(e.at)).unwrap_or(Value::Null),
        "target": st.target.as_ref().map(|t| iso_ct(t.at)).unwrap_or(Value::Null),
        "cutoff": st.cutoff.as_ref().map(|c| iso_ct(c.at)).unwrap_or(Value::Null),
        "serviceOpened": iso_opt(st.service_opened_at),
        "attendance": st.attendance.iter().map(|a| json!([iso_ct(a.clock_in), iso_opt(a.clock_out)])).collect::<Vec<_>>(),
        "requests": st.requests.iter().map(|r| json!({
            "at": iso_ct(r.at),
            "signatures": r.signatures.iter().map(|s| iso_ct(s.at)).collect::<Vec<_>>(),
            "dispatched": iso_opt(r.dispatched_at),
            "rejected": r.rejected.as_ref().map(|d| iso_ct(d.at)).unwrap_or(Value::Null),
            "completed": iso_opt(r.completed_at),
            "reconciled": r.reconciled.as_ref().map(|d| iso_ct(d.at)).unwrap_or(Value::Null),
            "target": r.target.as_ref().map(|t| json!({ "amounts": shown(&t.amounts), "by": t.by, "at": iso_ct(t.at) })).unwrap_or(Value::Null),
            "cutoff": r.cutoff.as_ref().map(|c| json!({ "at": iso_ct(c.at), "by": c.by, "shiftId": c.shift_id, "lastRef": c.last_ref, "pending": c.pending })).unwrap_or(Value::Null),
            // rows are kept for the four counted kinds only; a transfer has none
            "rows": if r.kind.is_transfer() { Value::Null } else { rows(&r.counted) },
            "safeRows": rows(&r.safe_count),
        })).collect::<Vec<_>>(),
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

/// Everything a branch holds — custody state, its events, the shifts and the
/// stock movements — in the fixture's written form: ids replaced, one entry per
/// scalar part and per list element.
pub async fn written(
    db: &TestDb,
    custody: &Custody,
    branch: &BranchId,
    result: &Value,
    ids: &mut Ids,
) -> BTreeMap<String, Value> {
    let state = custody.state(branch).await.expect("read the custody state");
    let events = custody.events(branch).await.expect("read the events");
    let shifts = db.service().shifts(branch).await.expect("list shifts");
    let movements = db
        .service()
        .movements(branch)
        .await
        .expect("list movements");
    let shift_ids: Vec<&str> = shifts.iter().map(|s| s.id.as_str()).collect();

    let mut ops: Map<String, Value> = shown_state(&state);
    if let Some(Value::Array(requests)) = ops.get_mut("requests") {
        for r in requests {
            // the hash is this server's own; v2's is not compared
            r.as_object_mut().unwrap().remove("payloadHash");
        }
    }
    let doc = json!({
        "events": events.iter().map(event_json).collect::<Vec<_>>(),
        "movements": movements.iter().map(|m| json!({
            "shift": shift_ids.iter().position(|id| *id == m.shift_id).map(|i| i as i64).unwrap_or(-1),
            "currency": m.currency.as_str(),
            "direction": if m.direction == Direction::In { "IN" } else { "OUT" },
            "amount": m.amount.to_string(), "reason": m.reason, "requestId": m.request_id,
        })).collect::<Vec<_>>(),
        "ops": Value::Object(ops),
        "result": result,
        "shifts": shifts.iter().map(|s| json!({
            "id": s.id,
            "status": if s.status == ShiftStatus::Open { "OPEN" } else { "CLOSED" },
            "opening": shown_balances(&s.opening), "expected": shown_balances(&s.expected),
            "counted": s.counted.as_ref().map(shown_balances),
            "difference": s.difference.as_ref().map(shown_delta),
            "note": s.note, "openRequest": s.open_request_id, "closeRequest": s.close_request_id,
        })).collect::<Vec<_>>(),
        "times": times_json(&state),
    });
    flatten(&ids.normalise(&doc))
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

    pub async fn act(&mut self, who: &str, action: &str, payload: Value) -> AppResult<Value> {
        let now = self.tick();
        self.custody
            .command(&self.branch, action, &payload, &actor(who), now)
            .await
    }

    pub async fn ok(&mut self, who: &str, action: &str, payload: Value) -> Value {
        match self.act(who, action, payload.clone()).await {
            Ok(v) => v,
            Err(e) => panic!("{who} {action} {payload} must be accepted: {e:?}"),
        }
    }

    /// A proposal payload with a fresh request id and one evidence file.
    pub fn proposal(&mut self, kind: &str, more: Value) -> Value {
        self.request += 1;
        let mut p = json!({
            "requestId": request_id(self.request), "kind": kind,
            "evidenceKeys": [evidence(self.request)], "note": "",
        });
        for (k, v) in more.as_object().expect("an object") {
            p[k] = v.clone();
        }
        p
    }

    pub async fn propose(&mut self, who: &str, kind: &str, more: Value) -> String {
        let p = self.proposal(kind, more);
        let r = self.ok(who, "opsPropose", p).await;
        r["id"].as_str().expect("an id").to_owned()
    }

    /// Custody enabled, everyone clocked in, a target set, the safe counted in
    /// (THB 500,000 · USD 10,000).
    pub async fn with_safe(db: &TestDb) -> Desk {
        let mut d = Desk::new(db).await;
        d.ok(
            "owner",
            "opsSetEntity",
            json!({ "company": "XC", "site": "B", "confirmSeparated": true }),
        )
        .await;
        d.ok("owner", "opsEnable", json!({})).await;
        for who in ["F1", "F2", "FIN1", "FIN2", "MON1", "MGR1", "BE1", "BE2"] {
            d.ok(who, "opsClock", json!({ "direction": "IN" })).await;
        }
        d.ok(
            "FIN1",
            "opsTarget",
            json!({ "amounts": { "THB": 100000, "USD": 2000 } }),
        )
        .await;
        let safe = [("THB", 500_000), ("USD", 10_000)];
        let id = d
            .propose(
                "FIN1",
                "SAFE_OPEN",
                json!({ "counted": totals(&safe), "denominations": rows(&safe) }),
            )
            .await;
        d.ok("MON1", "opsSign", json!({ "id": id })).await;
        d.ok("BE1", "opsCommit", json!({ "id": id })).await;
        d
    }

    /// `with_safe`, then F1 has proposed opening the drawer with THB 100,000 · USD 2,000.
    /// Returns the request's id; Backend has not committed it yet.
    pub async fn opening_proposed(db: &TestDb) -> (Desk, String) {
        let mut d = Desk::with_safe(db).await;
        let drawer = [("THB", 100_000), ("USD", 2_000)];
        let id = d
            .propose(
                "F1",
                "OPEN",
                json!({ "counted": totals(&drawer), "denominations": rows(&drawer) }),
            )
            .await;
        (d, id)
    }

    /// The drawer is open and serving: F1 holds THB 100,000 · USD 2,000.
    pub async fn serving(db: &TestDb) -> Desk {
        let (mut d, id) = Desk::opening_proposed(db).await;
        d.ok("BE1", "opsCommit", json!({ "id": id })).await;
        d.ok(
            "BE1",
            "opsStartService",
            json!({ "equipmentReady": true, "ratesVerified": true, "rateUpdated": "2026-10-06T03:00:00.000Z" }),
        )
        .await;
        d
    }
}
