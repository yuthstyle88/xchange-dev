//! Written before the implementation (ticket T12c). Do not edit, delete or weaken.
//!
//! The harness of `kiosk_http_test.rs`: the replay of `tests/fixtures/kiosk/*.json`
//! — what v2 answered on `POST /api/kiosk` — through the server's own route
//! table, on a PostgreSQL schema of the test's own.
//!
//! It stands on the harness of ticket T11a (`rpc_common`): the desk, the app,
//! real sessions, and the replay of one step with the ids v3 made put where v2's
//! were. A recorded file mixes two kinds of step and both go through HTTP: the
//! kiosk's own, and the `/api/rpc` steps that set the counter up (a rate, a
//! shift, a second branch) or show what a kiosk step left behind.
//!
//! Four things a test controls, each registered after the route table so that
//! it replaces what the route table registered:
//!
//! - the clock (`web::Data<dyn RpcClock>`) and the Bank of Thailand
//!   (`web::Data<Arc<dyn RateSource>>`), as in `rpc_common`;
//! - the ids the kiosk route makes (`web::Data<dyn KioskIds>`): a quote gets the
//!   id v2 gave it, so that every later step can name it;
//! - where the kiosk's audit lines go (`web::Data<dyn KioskAuditLog>`): into the
//!   table `test_kiosk_audit`, **on the transaction they are handed**, so a line
//!   of a step that was rolled back is not there. The log also refuses a line
//!   that comes on a transaction other than the step's own.
//!
//! Edited by the author of ticket #756 (RE-4b, `docs/design/blueprint/rates.md` D-3, §4.8, §4.9):
//! the kiosk prices from the branch's current publication, not from the newest `rates` row. v2
//! had no publication: a saved rate was the counter rate at once. So when a replayed
//! `adminSaveRate` or kiosk `configure` changes a branch's saved rates, the replay publishes that
//! branch's saved rates as they now stand, per each currency's `unit_size`, through the rate
//! engine (`publish_saved_rates`) — what the owner of a v3 shop does after a direct save. A step
//! the replay plays is never changed by this.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.
//!
//! Amended by the ticket author of #603 (RE-6, claude/e5ab6c): v3's `configure` no longer saves
//! the counter rate (item 1) and switches a currency on only when the branch's current rate
//! publication has it on both sides (item 2). What v2's `configure` did to the rate is therefore
//! applied behind the routes before an accepted `configure` step is sent (`configure_behind`): the
//! rate is saved through the rate rules as v2 saved it, which later `rates`, `manage` and
//! `adminSnapshot` steps read, and the currency is published when the step switches it on. A step
//! v3 accepts where v2 refused (`v3_accepts`) is held to the audit line v3 writes for it.
//!
//! Amended again by the ticket author of #603 (ruling of 2026-10-09): configure sets no rate, so
//! its `KIOSK_CURRENCY_CONFIGURED` line records `buy` and `sell` as `""`. A recorded configure
//! line of `behind.json` is compared with those two as `""` (`v3_detail`); the file is unchanged.
//!
//! Amended again by the ticket author of #603 after the rebase onto RE-4b (#756): one publisher.
//! RE-4b's publish after a save (the branch's saved rates per each currency's `unit_size`, through
//! the rate engine) publishes every replayed save, including the rate `configure_behind` saves.
//! `configure_behind` publishes only when its step switches on a currency the branch's current
//! publication does not hold on both sides, because RE-6's `configure` refuses it otherwise; it
//! publishes that same content (`publish_saved_rates`), earlier, and the step then publishes
//! nothing again. Its own publication (only the switched-on currencies, at v2's per-1 rate under
//! the currency's `unit_size`) replaced RE-4b's lines and priced KRW per 1000 as per 1
//! (`currencies.json` #33: 0.0000231 for 0.0231).
#![allow(dead_code)]

#[path = "../rates_re4_common/mod.rs"]
pub mod rates_re4_common;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use serde_json::{Map, Value, json};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_kiosk::application::kiosk::service::KioskService;
use xc_kiosk::domain::kiosk::value_objects::audit_line::{AuditLine, KioskAudit};
use xc_kiosk::domain::kiosk::value_objects::kiosk_actor::KioskActor;
use xc_kiosk::domain::kiosk::value_objects::kiosk_role::KioskRole;
use xc_rate::application::rate::service::RateService;
use xc_rate::domain::rate::commands::save_rate::SaveRate;
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xchange::infrastructure::kiosk::kiosk_ports::{KioskAuditLog, PgKioskPorts};
use xchange::presentation::http::kiosk::KioskIds;

use crate::compat_common::{Fixture, IdMap, Person, Step, fixtures_dir};
use crate::rpc_common::{
    Answer, App, BANK_DOWN, Call, Cookies, Desk, MAIN, Replay, Tally, sign_in,
};

pub const KIOSK: &str = "/api/kiosk";

// v2's sentences, as the kiosk screens and `kiosk-i18n.js` know them
pub const REQUEST_REJECTED: &str = "Request rejected.";
pub const REQUEST_TOO_LARGE: &str = "Request too large.";
pub const INVALID_REQUEST: &str = "Invalid request.";
pub const INVALID_BRANCH: &str = "Invalid branch.";
pub const UNKNOWN_ACTION: &str = "Unknown kiosk action.";
pub const STAFF_REQUIRED: &str = "Staff access required.";
pub const OWNER_OR_RATES: &str = "Owner or Rates access required.";
pub const ALREADY_ASSIGNED: &str = "This request is already assigned or unavailable.";
pub const ACCEPT_FIRST: &str = "Accept this request using your own account first.";
/// v3's own: v2 has no limit on quotes (design §6).
pub const TOO_MANY_QUOTES: &str = "Too many requests. Please wait a moment and try again.";
pub const SERVER_ERROR: &str = "เซิร์ฟเวอร์ขัดข้อง";

// ── the ids the route makes ──────────────────────────────────────────────────

/// Hands out the ids a test queued, in order; once they are used up, ids of its
/// own that no recorded file holds.
#[derive(Default)]
pub struct QueuedIds {
    next: Mutex<VecDeque<String>>,
    made: Mutex<u64>,
}

impl QueuedIds {
    pub fn push(&self, id: &str) {
        self.next
            .lock()
            .expect("the ids' lock")
            .push_back(id.to_string());
    }

    pub fn waiting(&self) -> usize {
        self.next.lock().expect("the ids' lock").len()
    }
}

impl KioskIds for QueuedIds {
    fn new_id(&self) -> String {
        if let Some(id) = self.next.lock().expect("the ids' lock").pop_front() {
            return id;
        }
        let mut made = self.made.lock().expect("the ids' lock");
        *made += 1;
        format!("7e570000-0000-4000-9000-{:012x}", *made)
    }
}

// ── the audit log ────────────────────────────────────────────────────────────

/// The kiosk's audit log in the test's hands: every line into `test_kiosk_audit`
/// through the transaction it came with, or a failure when told to fail.
#[derive(Default)]
pub struct TestAudit {
    fails: AtomicBool,
}

impl TestAudit {
    pub fn fail(&self, fails: bool) {
        self.fails.store(fails, Ordering::SeqCst);
    }
}

fn detail_of(what: &KioskAudit) -> Value {
    match what {
        KioskAudit::Request { quote } => json!({
            "type": match quote.side { Side::Buy => "BUY", Side::Sell => "SELL" },
            "currency": quote.currency.as_str(),
            "rate": quote.rate.to_string(),
            "foreign": quote.foreign.to_string(),
            "thb": quote.thb.to_string(),
            "inputAmount": quote.input_amount.to_string(),
            "paymentMethod": quote.payment_method.as_str(),
        }),
        KioskAudit::Accepted | KioskAudit::IdChecked => json!({}),
        KioskAudit::Completed { document_ref } => json!({ "documentRef": document_ref }),
        KioskAudit::CurrencyConfigured { enabled, buy, sell } => {
            json!({ "enabled": enabled, "buy": buy, "sell": sell })
        }
    }
}

impl KioskAuditLog for TestAudit {
    fn record_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        line: &'a AuditLine,
        at: DateTime<Utc>,
    ) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>> {
        Box::pin(async move {
            if self.fails.load(Ordering::SeqCst) {
                return Err(AppError::Internal("the audit log is down".to_string()));
            }
            // The line must come on the transaction of the step it records: on
            // that transaction — and on no other — the request already stands
            // where the step moved it (`KioskService` stores the move, then
            // hands the line over, `xc-kiosk/src/application/kiosk/service.rs`).
            let moved_to = match &line.what {
                KioskAudit::Request { .. } => Some("WAITING"),
                KioskAudit::Accepted => Some("ACCEPTED"),
                KioskAudit::IdChecked => Some("ID_CHECKED"),
                KioskAudit::Completed { .. } | KioskAudit::CurrencyConfigured { .. } => None,
            };
            if let Some(expected) = moved_to {
                let seen: Option<(String,)> = sqlx::query_as(
                    "SELECT status FROM kiosk_requests WHERE branch_id = $1 AND id = $2",
                )
                .bind(branch.as_str())
                .bind(&line.subject)
                .fetch_optional(&mut **tx)
                .await?;
                if seen.as_ref().map(|row| row.0.as_str()) != Some(expected) {
                    return Err(AppError::Internal(format!(
                        "the audit line of {} came on a transaction that does not see the \
                         request as {expected} (it sees {seen:?}): not the step's own",
                        line.subject
                    )));
                }
            }
            sqlx::query(
                "INSERT INTO test_kiosk_audit (branch_id, actor, kind, subject, detail, at) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(branch.as_str())
            .bind(&line.actor)
            .bind(line.what.kind())
            .bind(&line.subject)
            .bind(detail_of(&line.what).to_string())
            .bind(at)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    }
}

/// One audit line as a test compares it: amounts and rates as plain decimal
/// text without trailing zeros, whether v2 wrote a number or a text.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub branch: String,
    pub actor: String,
    pub kind: String,
    pub subject: String,
    pub detail: Value,
    pub at: Option<DateTime<Utc>>,
}

fn plain(value: &Value) -> Value {
    let text = match value {
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        other => return other.clone(),
    };
    match text.parse::<Decimal>() {
        Ok(decimal) => json!(decimal.normalize().to_string()),
        Err(_) => json!(text),
    }
}

/// The part of a recorded or written detail the two sides share. v2's
/// `KIOSK_REQUEST` line also carries the branch's name and
/// `financialTransaction: false`, which v3's line does not (the line is typed:
/// `KioskAudit::Request { quote }`).
fn comparable(kind: &str, detail: &Value) -> Value {
    let keys: &[&str] = match kind {
        "KIOSK_REQUEST" => &[
            "type",
            "currency",
            "rate",
            "foreign",
            "thb",
            "inputAmount",
            "paymentMethod",
        ],
        "KIOSK_CURRENCY_CONFIGURED" => &["enabled", "buy", "sell"],
        "KIOSK_COMPLETED" => &["documentRef"],
        _ => &[],
    };
    let mut out = Map::new();
    for key in keys {
        let value = detail
            .get(*key)
            .unwrap_or_else(|| panic!("a {kind} line without `{key}`: {detail}"));
        let numeric = matches!(
            *key,
            "rate" | "foreign" | "thb" | "inputAmount" | "buy" | "sell"
        );
        out.insert(
            (*key).to_string(),
            if numeric { plain(value) } else { value.clone() },
        );
    }
    Value::Object(out)
}

/// #603 (ruling of 2026-10-09): a recorded detail as v3 writes it. A configure line's `buy` and
/// `sell` are `""`; every other detail is as recorded.
fn v3_detail(kind: &str, detail: &Value) -> Value {
    let mut detail = detail.clone();
    if kind == "KIOSK_CURRENCY_CONFIGURED" {
        detail["buy"] = json!("");
        detail["sell"] = json!("");
    }
    detail
}

// ── the app ──────────────────────────────────────────────────────────────────

/// What a test holds beside the desk: the ids and the audit log it registered.
#[derive(Clone, Default)]
pub struct Hands {
    pub ids: Arc<QueuedIds>,
    pub audit: Arc<TestAudit>,
}

/// The server's whole route table with the test's clock, bank, ids and audit
/// log registered after it.
#[macro_export]
macro_rules! kiosk_app {
    ($desk:expr, $hands:expr) => {{
        let state = $desk.state();
        let clock: std::sync::Arc<dyn xchange::presentation::http::rpc::RpcClock> =
            $desk.clock.clone();
        let bank: std::sync::Arc<dyn xc_rate::domain::rate::entities::rate_source::RateSource> =
            $desk.bank.clone();
        let ids: std::sync::Arc<dyn xchange::presentation::http::kiosk::KioskIds> =
            $hands.ids.clone();
        let audit: std::sync::Arc<dyn xchange::infrastructure::kiosk::kiosk_ports::KioskAuditLog> =
            $hands.audit.clone();
        $crate::rpc_common::App::new(
            actix_web::test::init_service(
                actix_web::App::new()
                    .configure(move |cfg| xchange::bootstrap::configure(cfg, state))
                    .app_data(actix_web::web::Data::from(clock))
                    .app_data(actix_web::web::Data::new(bank))
                    .app_data(actix_web::web::Data::from(ids))
                    .app_data(actix_web::web::Data::from(audit)),
            )
            .await,
            |t| t.to_request(),
        )
    }};
}

/// The table the test's audit log writes to.
pub async fn make_the_audit_table(desk: &Desk) {
    sqlx::query(
        "CREATE TABLE test_kiosk_audit (seq bigserial PRIMARY KEY, branch_id text NOT NULL, \
         actor text NOT NULL, kind text NOT NULL, subject text NOT NULL, detail text NOT NULL, \
         at timestamptz NOT NULL)",
    )
    .execute(&desk.db.pool)
    .await
    .expect("create the test's audit table");
}

/// A row of `test_kiosk_audit`: seq, branch, actor, kind, subject, detail, at.
type AuditRow = (i64, String, String, String, String, String, DateTime<Utc>);

/// The audit lines written after line `after`, and the number of the last one.
pub async fn audit_since(desk: &Desk, after: i64) -> (Vec<Line>, i64) {
    let rows: Vec<AuditRow> = sqlx::query_as(
        "SELECT seq, branch_id, actor, kind, subject, detail, at FROM test_kiosk_audit \
         WHERE seq > $1 ORDER BY seq",
    )
    .bind(after)
    .fetch_all(&desk.db.pool)
    .await
    .expect("read the test's audit table");
    let last = rows.last().map_or(after, |row| row.0);
    let lines = rows
        .into_iter()
        .map(|(_, branch, actor, kind, subject, detail, at)| {
            let detail: Value = serde_json::from_str(&detail).expect("a detail written as JSON");
            Line {
                detail: comparable(&kind, &detail),
                branch,
                actor,
                kind,
                subject,
                at: Some(at),
            }
        })
        .collect();
    (lines, last)
}

// ── a kiosk request by hand ──────────────────────────────────────────────────

/// `{action, payload}` to `/api/kiosk` as the screens send it.
pub fn kiosk(origin: &str, action: &str, payload: Value) -> Call {
    Call::action(origin, KIOSK, action, payload)
}

pub fn as_who(call: Call, cookies: &Cookies, who: &str) -> Call {
    call.cookie(
        cookies
            .get(who)
            .unwrap_or_else(|| panic!("{who} is not signed in")),
    )
}

// ── the replay of a recorded file ────────────────────────────────────────────

/// What happened behind the answers of one file: `tests/fixtures/kiosk/behind.json`.
fn behind_of(file: &str) -> Map<String, Value> {
    let path = fixtures_dir().join("kiosk/behind.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let all: Value = serde_json::from_str(&raw).expect("behind.json is JSON");
    all.get(file)
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("behind.json has nothing for {file}"))
        .clone()
}

pub struct KioskReplay<'f> {
    pub replay: Replay<'f>,
    pub hands: Hands,
    behind: Map<String, Value>,
    /// People of a branch that does not exist until a step creates it.
    late: Vec<Person>,
    audit_seen: i64,
    /// Step number → the audit lines that step wrote.
    pub lines: BTreeMap<usize, Vec<Line>>,
    /// How many recorded audit lines were compared.
    pub lines_compared: usize,
    /// #603: `configure` steps v3 accepts where v2 refused; each writes its audit line.
    v3_accepts: BTreeSet<usize>,
}

impl<'f> KioskReplay<'f> {
    /// A desk with the people of `MAIN` employed and signed in; the people of
    /// another branch join when the step that creates their branch was replayed.
    pub async fn start(fixture: &'f Fixture) -> KioskReplay<'f> {
        let desk = Desk::open().await;
        make_the_audit_table(&desk).await;
        // the recorded shop's owner is called "Owner" (the generator wrote the row);
        // `create-owner` names a new owner after the username
        let owner = fixture
            .people
            .iter()
            .find(|p| p.is_owner())
            .expect("a recorded file has an owner");
        sqlx::query("UPDATE owners SET name = $1 WHERE username = $2")
            .bind(&owner.name)
            .bind(&owner.username)
            .execute(&desk.db.pool)
            .await
            .expect("name the owner as recorded");
        let hands = Hands::default();
        let app: App = kiosk_app!(desk, hands);

        let (now, late): (Vec<Person>, Vec<Person>) = fixture
            .people
            .iter()
            .cloned()
            .partition(|p| p.is_owner() || p.branch_id.as_deref() == Some(MAIN));
        desk.employ(&now).await;
        let cookies = sign_in(&app, &fixture.origin, &now).await;
        let file = fixture.name.rsplit('/').next().unwrap_or(&fixture.name);
        KioskReplay {
            replay: Replay {
                desk,
                app,
                fixture,
                cookies,
                ids: IdMap::new(),
                tally: Tally::default(),
                answers: BTreeMap::new(),
                rules: BTreeMap::new(),
            },
            hands,
            behind: behind_of(file),
            late,
            audit_seen: 0,
            lines: BTreeMap::new(),
            lines_compared: 0,
            v3_accepts: BTreeSet::new(),
        }
    }

    /// An accepted kiosk step of the owner's queue work, through the kiosk service as the route
    /// runs it, as the owner; its audit lines go to the test's log as the route's would.
    async fn kiosk_behind(&mut self, step: &Step) {
        let label = self.replay.label(step);
        let payload = self
            .replay
            .ids
            .to_actual(step.payload.as_ref().unwrap_or(&json!({})));
        let branch = BranchId::new(
            payload
                .get("branchId")
                .and_then(Value::as_str)
                .filter(|b| !b.is_empty())
                .unwrap_or(MAIN),
        );
        let pool = self.replay.desk.db.pool.clone();
        let audit: Arc<dyn KioskAuditLog> = self.hands.audit.clone();
        let source: Arc<dyn RateSource> = self.replay.desk.bank.clone();
        let service = KioskService::new(
            self.replay.desk.db.branch_tx(),
            Arc::new(PgKioskPorts::new(pool, audit, source)),
        );
        let owner = KioskActor::Staff {
            username: "owner".to_string(),
            role: KioskRole::Owner,
        };
        let id = payload.get("id").and_then(Value::as_str).unwrap_or("");
        let now = crate::snapshot_common::time(&step.at);
        match step.action.as_deref() {
            Some("accept") => {
                service
                    .accept(&branch, &owner, id, now)
                    .await
                    .unwrap_or_else(|e| panic!("{label}: applied behind the routes: {e:?}"));
            }
            Some("identityChecked") => {
                let confirmed = payload
                    .get("confirmed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                service
                    .identity_checked(&branch, &owner, id, confirmed, now)
                    .await
                    .unwrap_or_else(|e| panic!("{label}: applied behind the routes: {e:?}"));
            }
            Some("publishReceipt") => {
                let image = payload.get("image").and_then(Value::as_str).unwrap_or("");
                service
                    .publish_receipt(&branch, &owner, id, image, now)
                    .await
                    .unwrap_or_else(|e| panic!("{label}: applied behind the routes: {e:?}"));
            }
            // reading changes nothing
            Some("get") => {}
            other => panic!("{label}: v2 accepted {other:?}; nothing applies it behind the routes"),
        }
        self.replay.tally.applied_behind.push(step.n);
    }

    /// #603: step `n` is a `configure` v2 refused and v3 accepts; the audit line it writes is the
    /// step's own (its code and switch, `buy` and `sell` `""`).
    pub fn v3_accepts(mut self, n: usize) -> Self {
        self.v3_accepts.insert(n);
        self
    }

    pub fn rule(mut self, n: usize, rule: crate::rpc_common::Rule) -> Self {
        self.replay = self.replay.rule(n, rule);
        self
    }

    fn is_kiosk(step: &Step) -> bool {
        step.path() == Some(KIOSK)
    }

    /// Every step in the order recorded.
    pub async fn run(&mut self) {
        let fixture = self.replay.fixture;
        for step in fixture.steps() {
            self.step(step).await;
        }
        assert_eq!(
            self.replay.tally.total(),
            fixture.steps().len(),
            "{}: every step is accounted for",
            fixture.name
        );
        assert_eq!(
            self.hands.ids.waiting(),
            0,
            "{}: an id v2 made was never asked for",
            fixture.name
        );
    }

    pub async fn step(&mut self, step: &Step) {
        let label = self.replay.label(step);
        let behind = self.behind.get(&step.n.to_string()).cloned();
        let action = step.action.as_deref();
        let accepted = step.status == Some(200);

        let saves = matches!(action, Some("adminSaveRate") | Some("configure"));
        let mut saved_before = if saves {
            saved_rates(&self.replay.desk.db.pool).await
        } else {
            BTreeMap::new()
        };

        if Self::is_kiosk(step) {
            // the id v2 gave the quote is the id every later step names
            if action == Some("quote") && accepted {
                let id = step.json()["id"].as_str().expect("a quote has an id");
                self.hands.ids.push(id);
            }
            if action == Some("configure") {
                let bank = &self.replay.desk.bank;
                match behind.as_ref().and_then(|b| b.get("bank")) {
                    Some(reference) => bank.answer_next(ReferenceRates {
                        source: text(reference, "source"),
                        fetched_at: text(reference, "fetchedAt"),
                        kind: text(reference, "kind"),
                        rows: reference["rows"].as_array().cloned().unwrap_or_default(),
                    }),
                    None if step.error() == Some(BANK_DOWN) => bank.go_down(),
                    None => bank.answer_nothing_special(),
                }
                // #603: published already when it switches on an unpublished currency, so
                // the same saved rates are not published twice
                if accepted && self.configure_behind(step).await {
                    saved_before = saved_rates(&self.replay.desk.db.pool).await;
                }
            }
        }

        self.replay.step(step).await;
        // OR-4b (#767, §4.2 C8): the owner's queue work v2 accepted is refused by role; what it
        // changed is applied behind the routes, by the owner as v2 recorded
        if Self::is_kiosk(step)
            && accepted
            && self.replay.tally.blueprint.contains(&step.n)
            && step.action.as_deref() != Some("list")
        {
            self.kiosk_behind(step).await;
        }

        // #756: a direct save is published, as a v3 owner publishes it (D-3)
        if saves {
            let saved_after = saved_rates(&self.replay.desk.db.pool).await;
            for (branch, lines) in &saved_after {
                if saved_before.get(branch) != Some(lines) {
                    let lines: Vec<(&str, &str, &str)> = lines
                        .iter()
                        .map(|(code, buy, sell)| (code.as_str(), buy.as_str(), sell.as_str()))
                        .collect();
                    rates_re4_common::publish(&self.replay.desk.db.pool, branch, &lines).await;
                }
            }
        }

        // a quote that v3 refused where v2 accepted must not leave its id behind
        if Self::is_kiosk(step) && action == Some("quote") && accepted {
            assert_eq!(
                self.hands.ids.waiting(),
                0,
                "{label}: the route did not ask for a new id"
            );
        }

        // a branch was created: its people can be employed now
        if action == Some("adminCreateBranch") && accepted {
            let recorded = step.json()["branchId"].as_str().expect("a branch id");
            let actual = self.replay.answers[&step.n].json()["branchId"]
                .as_str()
                .expect("a branch id")
                .to_string();
            let (mut joining, later): (Vec<Person>, Vec<Person>) = std::mem::take(&mut self.late)
                .into_iter()
                .partition(|p| p.branch_id.as_deref() == Some(recorded));
            self.late = later;
            for person in &mut joining {
                person.branch_id = Some(actual.clone());
            }
            self.replay.desk.employ(&joining).await;
            let cookies = sign_in(&self.replay.app, &self.replay.fixture.origin, &joining).await;
            self.replay.cookies.extend(cookies);
        }

        // the audit lines of this step against v2's
        let (written, last) = audit_since(&self.replay.desk, self.audit_seen).await;
        self.audit_seen = last;
        let recorded: Vec<Line> = behind
            .as_ref()
            .and_then(|b| b.get("audit"))
            .and_then(Value::as_array)
            .map(|lines| {
                lines
                    .iter()
                    .map(|line| {
                        let kind = text(line, "kind");
                        Line {
                            branch: self.actual_text(&text(line, "branch")),
                            actor: text(line, "actor"),
                            subject: self.actual_text(&text(line, "ref")),
                            detail: comparable(&kind, &v3_detail(&kind, &line["detail"])),
                            kind,
                            at: None,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let recorded = if self.v3_accepts.contains(&step.n) {
            assert!(recorded.is_empty(), "{label}: v2 wrote no line here");
            vec![self.own_line(step)]
        } else {
            recorded
        };
        let written_without_time: Vec<Line> = written
            .iter()
            .cloned()
            .map(|line| Line { at: None, ..line })
            .collect();
        assert_eq!(
            written_without_time, recorded,
            "{label}: the audit lines of this step"
        );
        for line in &written {
            assert_eq!(
                line.at,
                Some(crate::snapshot_common::time(&step.at)),
                "{label}: an audit line carries the instant of its request"
            );
        }
        self.lines_compared += recorded.len();
        if !written.is_empty() {
            self.lines.insert(step.n, written);
        }
    }

    /// The branch a kiosk step works on: an employee's own, the owner's named one, else `MAIN`.
    fn branch_of_step(&self, step: &Step, payload: &Value) -> String {
        let sender = self.replay.fixture.sender(step).expect("somebody sent it");
        match &sender.branch_id {
            Some(own) => self.actual_text(own),
            None => payload
                .get("branchId")
                .and_then(Value::as_str)
                .filter(|b| !b.is_empty())
                .unwrap_or(MAIN)
                .to_string(),
        }
    }

    /// #603: what v2's accepted `configure` did to the rate, applied behind the routes before the
    /// step is sent: the rate saved through the rate rules (on the bank this step set). When the
    /// step switches on a currency the branch's current publication does not hold on both sides,
    /// the branch's saved rates are published now (`publish_saved_rates`'s content), so that v3's
    /// `configure` accepts it. Answers whether it published.
    async fn configure_behind(&mut self, step: &Step) -> bool {
        let payload = self
            .replay
            .ids
            .to_actual(step.payload.as_ref().expect("a configure has a payload"));
        let wire = |key: &str| match payload.get(key) {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        };
        let branch = self.branch_of_step(step, &payload);
        let code = wire("code").expect("a code");
        let (buy, sell) = (
            wire("buy").unwrap_or_default(),
            wire("sell").unwrap_or_default(),
        );
        let sender = self.replay.fixture.sender(step).expect("somebody sent it");
        let cmd = SaveRate {
            request_id: format!("7e57ca1e-0000-4000-9000-{:012x}", step.n),
            code: code.clone(),
            buy: buy.clone(),
            sell: sell.clone(),
            partner_name: payload
                .get("partnerName")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            partner_cost: wire("partnerCost"),
            previous_updated: payload
                .get("previousUpdated")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        };
        let pool = self.replay.desk.db.pool.clone();
        let source: Arc<dyn RateSource> = self.replay.desk.bank.clone();
        RateService::new(BranchTx::new(Transactor::new(pool.clone())), source)
            .save_rate(
                &BranchId::new(branch.clone()),
                cmd,
                &sender.username,
                crate::snapshot_common::time(&step.at),
            )
            .await
            .unwrap_or_else(|e| {
                panic!("{}: the rate saved behind: {e:?}", self.replay.label(step))
            });

        if payload.get("enabled") != Some(&Value::Bool(true)) {
            return false;
        }
        let published: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM branches b JOIN published_rate_lines l \
               ON l.publication_id = b.current_publication_id \
             WHERE b.id = $1 AND l.currency_code = $2 \
               AND l.buy_rate IS NOT NULL AND l.sell_rate IS NOT NULL)",
        )
        .bind(&branch)
        .bind(&code)
        .fetch_one(&pool)
        .await
        .expect("read the current publication");
        if published {
            return false;
        }
        publish_saved_rates(&pool, &branch).await;
        true
    }

    /// #603: the audit line v3's `configure` writes for a step v2 refused.
    fn own_line(&self, step: &Step) -> Line {
        let payload = self
            .replay
            .ids
            .to_actual(step.payload.as_ref().expect("a configure has a payload"));
        let sender = self.replay.fixture.sender(step).expect("somebody sent it");
        let kind = "KIOSK_CURRENCY_CONFIGURED".to_string();
        Line {
            branch: self.branch_of_step(step, &payload),
            actor: sender.username.clone(),
            subject: payload["code"].as_str().expect("a code").to_string(),
            detail: comparable(
                &kind,
                &json!({ "enabled": payload["enabled"], "buy": "", "sell": "" }),
            ),
            kind,
            at: None,
        }
    }

    fn actual_text(&self, recorded: &str) -> String {
        match self.replay.ids.to_actual(&json!(recorded)) {
            Value::String(actual) => actual,
            other => panic!("{recorded} became {other}"),
        }
    }

    pub fn answer(&self, n: usize) -> &Answer {
        self.replay
            .answers
            .get(&n)
            .unwrap_or_else(|| panic!("step {n} was not replayed"))
    }

    pub async fn finish(self) {
        self.replay.finish().await;
    }
}

/// #603: publish `branch`'s saved rates as they now stand, the content and the publisher of the
/// replay's own publish after a save (RE-4b, `step`).
pub async fn publish_saved_rates(pool: &sqlx::PgPool, branch: &str) {
    let saved = saved_rates(pool).await;
    let lines: Vec<(&str, &str, &str)> = saved
        .get(branch)
        .unwrap_or_else(|| panic!("{branch} has saved rates to publish"))
        .iter()
        .map(|(code, buy, sell)| (code.as_str(), buy.as_str(), sell.as_str()))
        .collect();
    rates_re4_common::publish(pool, branch, &lines).await;
}

/// Every branch's newest saved rate per currency, per that currency's `unit_size` (#756):
/// branch → `(code, buy, sell)` in code order.
pub async fn saved_rates(pool: &sqlx::PgPool) -> BTreeMap<String, Vec<(String, String, String)>> {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT r.branch_id, trim(r.code::text), (r.buy * c.unit_size)::text, \
                (r.sell * c.unit_size)::text \
         FROM (SELECT DISTINCT ON (branch_id, code) branch_id, code, buy, sell \
                 FROM rates ORDER BY branch_id, code, id DESC) r \
         JOIN currencies c ON c.code = r.code \
         ORDER BY 1, 2",
    )
    .fetch_all(pool)
    .await
    .expect("read the saved rates");
    let mut saved: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
    for (branch, code, buy, sell) in rows {
        saved.entry(branch).or_default().push((code, buy, sell));
    }
    saved
}

fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("no text `{key}` in {value}"))
        .to_string()
}

// ── a counter set up by hand, for the tests that are not a replay ────────────

/// A desk whose `MAIN` has the dollar at 32.1 / 32.6 and an open shift, the
/// owner and the people given signed in — all through the routes, and the dollar
/// published (#756 RE-4b).
pub struct Counter {
    pub desk: Desk,
    pub app: App,
    pub hands: Hands,
    pub cookies: Cookies,
    pub origin: String,
}

pub const ORIGIN: &str = "http://desk.test";

pub fn person(key: &str, role: &str, n: u8) -> Person {
    Person {
        key: key.to_string(),
        kind: "staff".to_string(),
        id: format!("00000000-0000-4000-8000-0000000000{n:02x}"),
        username: key.to_lowercase(),
        name: format!("{key} of the test"),
        role: role.to_string(),
        branch_id: Some(MAIN.to_string()),
        must_change: false,
    }
}

pub fn the_owner() -> Person {
    Person {
        key: "owner".to_string(),
        kind: "owner".to_string(),
        id: "owner".to_string(),
        username: "owner".to_string(),
        name: "Owner".to_string(),
        role: "OWNER".to_string(),
        branch_id: None,
        must_change: false,
    }
}

impl Counter {
    pub async fn open(staff: &[Person]) -> Counter {
        let desk = Desk::open().await;
        make_the_audit_table(&desk).await;
        let hands = Hands::default();
        let app: App = kiosk_app!(desk, hands);
        let mut people = vec![the_owner()];
        people.extend(staff.iter().cloned());
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let counter = Counter {
            desk,
            app,
            hands,
            cookies,
            origin: ORIGIN.to_string(),
        };
        counter
            .owner_rpc(
                "adminSaveRate",
                json!({ "requestId": "00000000-0000-4000-8000-0000000b0001", "code": "USD",
                        "buy": 32.1, "sell": 32.6, "previousUpdated": "" }),
            )
            .await;
        // Amended for #756 (RE-4b, `docs/design/blueprint/rates.md` D-3, §4.8, §4.9 R1): the
        // kiosk's counter rate is the line of the current publication, no longer the newest
        // `rates` row; the same dollar is published (the rate engine is RE-1's, merged).
        rates_re4_common::publish(&counter.desk.db.pool, MAIN, &[("USD", "32.1", "32.6")]).await;
        counter
            .owner_rpc(
                "adminOpenShift",
                json!({ "requestId": "00000000-0000-4000-8000-0000000b0002",
                        "opening": { "THB": 100000, "USD": 2000 } }),
            )
            .await;
        counter
    }

    pub async fn owner_rpc(&self, action: &str, payload: Value) -> Value {
        let call = Call::action(&self.origin, "/api/rpc", action, payload);
        let answer = self.app.send(as_who(call, &self.cookies, "owner")).await;
        assert_eq!(answer.status, 200, "{action}: {}", answer.text);
        answer.json()
    }

    /// One kiosk action; `who` is a person's key, or `None` for the customer.
    pub async fn ask(&self, who: Option<&str>, action: &str, payload: Value) -> Answer {
        self.ask_from(who, action, payload, None).await
    }

    /// The same from a given client address (`X-Real-IP`).
    pub async fn ask_from(
        &self,
        who: Option<&str>,
        action: &str,
        payload: Value,
        address: Option<&str>,
    ) -> Answer {
        let mut call = kiosk(&self.origin, action, payload);
        if let Some(who) = who {
            call = as_who(call, &self.cookies, who);
        }
        if let Some(address) = address {
            call = call.header("x-real-ip", address);
        }
        self.app.send(call).await
    }

    /// A quote confirmed into the queue: its id.
    pub async fn waiting_request(&self, id: &str) -> String {
        self.hands.ids.push(id);
        let quote = self
            .ask(
                None,
                "quote",
                json!({ "type": "BUY", "currency": "USD", "amount": 100 }),
            )
            .await;
        assert_eq!(quote.status, 200, "quote: {}", quote.text);
        assert_eq!(quote.json()["id"], id, "the route used the id it was given");
        self.desk
            .clock
            .set(self.now() + chrono::Duration::seconds(4));
        let confirmed = self.ask(None, "confirm", json!({ "id": id })).await;
        assert_eq!(confirmed.status, 200, "confirm: {}", confirmed.text);
        id.to_string()
    }

    pub fn now(&self) -> DateTime<Utc> {
        use xchange::presentation::http::rpc::RpcClock;
        self.desk.clock.now()
    }

    pub async fn finish(self) {
        self.desk.finish().await;
    }
}
