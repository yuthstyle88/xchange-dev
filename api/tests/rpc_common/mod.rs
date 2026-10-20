//! Written before the implementation (ticket T11a part one). Do not edit, delete or weaken.
//!
//! The harness of `rpc_test.rs`: the server's own route table on a PostgreSQL
//! schema of the test's own, real sessions for the people of a recorded file,
//! and the replay of a recorded v2 scenario **through HTTP**.
//!
//! Three things a test controls, each registered after the route table so it
//! replaces what the route table registered:
//!
//! - the clock a request is stamped with (`web::Data<dyn RpcClock>`): a
//!   recorded step is sent at its recorded `at`;
//! - the reference-rate source (`web::Data<Arc<dyn RateSource>>`): the Bank of
//!   Thailand "answers" what v2 recorded for the rate being saved;
//! - nothing else. Sessions are made by the real routes of ticket T4c.
//!
//! What this part of T11a does not dispatch (`saveTransaction`, `documents`,
//! `ops*`) is not compared. So that the steps after an accepted save still see
//! the state v2 saw, the harness applies an accepted `saveTransaction` and the
//! uploads before it through the real `SaveTransactionWorkflow` and
//! `FileService`, behind the routes. Every step is accounted for in a
//! [`Tally`]: replayed, known difference, excluded, or applied behind.
//!
//! Since T11a-0 "not dispatched" is decided per family ([`Family`]): a family
//! whose `gap_*.rs` says `WIRED` is sent and held to v2 like any other step.
//! (That edit — `Family`, `is_part_two` — is the lane controller's, made on
//! ticket T11a-0; with no family wired nothing here changes.)
//!
//! Re-expected for RE-7 #604 by the ticket author (design `docs/design/blueprint/rates.md` §8
//! migration 6, §9): `adminSaveRate` is gone, and v3 answers it exactly as it answers a name v2
//! does not know. A recorded `adminSaveRate` step is a known difference (design §9 removes the
//! action, [`Rule::Retired`]): it is held to the answer v3 gives the same request naming
//! [`NO_SUCH_NAME`] instead, and when v2 accepted it the rate it saved is published behind the
//! routes (`snapshot_common::publish_rate_per_one`), so the steps after it still see that rate.
//! A recorded snapshot is held to `rateHistory: []` and no partner, with `updated` the current
//! publication's `published_at` (`snapshot_common::without_direct_saves`, rates.md R10, §4.1;
//! ruling Q3 on #604).
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::http::{Method, header};
use actix_web::{Error, test};
use chrono::{DateTime, Duration, Utc};
use pos_error::{AppError, AppResult};
use serde_json::{Value, json};
use xc_auth::application::account::service::AccountService;
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::role_grants::RoleGrants;
use xc_auth::domain::access::v3_roles::actor_from_v3;
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_cash::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_kernel::ids::BranchId;
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::shift::commands::move_stock::MoveStock;
use xchange::application::workflows::save_transaction::SaveTransactionWorkflow;
use xchange::bootstrap::AppState;
use xchange::infrastructure::files::file_service::FileService;
use xchange::presentation::http::rpc::RpcClock;

use crate::common::settings;
use crate::compat_common::{Fixture, IdMap, Person, Step};
use crate::snapshot_common::{
    TestDb, assert_same, publish_rate_per_one, rates_re4c_common, time, without_direct_saves,
    without_dropped_keys,
};

pub const MAIN: &str = "MAIN";
pub const OWNER_PASSWORD: &str = "owner-password-01";
pub const STAFF_PASSWORD: &str = "staff-password-02";

pub const RPC: &str = "/api/rpc";
pub const STAFF_RPC: &str = "/api/staff/rpc";

// The sentences of the two routes themselves.
pub const INVALID_REQUEST: &str = "คำขอไม่ถูกต้อง";
pub const ROLE_NOT_ALLOWED_RPC: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการนี้";
pub const ROLE_NOT_ALLOWED_STAFF: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการ";
pub const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
pub const EMPLOYEE_AUTH_REQUIRED: &str = "EMPLOYEE_AUTH_REQUIRED";
pub const FRONT_ONLY: &str = "ใช้ได้เฉพาะ Front";
pub const BAD_BRANCH: &str = "สาขาไม่ถูกต้อง";
pub const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
pub const SERVER_ERROR: &str = "เซิร์ฟเวอร์ขัดข้อง";
pub const BANK_DOWN: &str = "ไม่สามารถดึงเรทล่าสุดจาก ธปท. ได้ กรุณาลองใหม่ก่อนบันทึกเรท";
pub const USE_CUSTODY: &str = "ใช้ขั้นตอนรับ–ส่งเงินและการลงชื่อในเมนูปฏิบัติงาน";
/// v2's own fault: `adminCreateBranch` sent again from another branch.
pub const V2_HEADERS_FAULT: &str = "หัวตารางเปลี่ยน กรุณาคืนลำดับคอลัมน์ก่อนบันทึก: Transactions";
/// Re-expected for RE-7 #604 by the ticket author: a name v2 does not know, as long as
/// `adminSaveRate` (so a body that names it keeps its length when the name is swapped).
pub const NO_SUCH_NAME: &str = "re7NoSuchName";
/// The action RE-7 retired (design §9).
pub const RETIRED: &str = "adminSaveRate";
/// Re-expected for CA-10 #626 by the ticket author (design `docs/design/blueprint/cash-shift-transfer.md`
/// §6): the admin direct shift actions, replaced by counts and moves.
pub const CA10_RETIRED: [&str; 3] = ["adminOpenShift", "adminCloseShift", "adminMoveStock"];

/// LG-6.r5 (#1044, BE controller's rulings A, issuecomment-6245895868, and A″, issuecomment-6245971463): the
/// recordings whose `$.operations.safe` is a known difference from the first step after an opening that SAFE
/// funded (the step number here) to the end. v3 funds the opening from SAFE; v2 never posted SAFE for it,
/// LG-6.r5.
pub const LG6_R5_SAFE_FUNDED_AN_OPENING: [(&str, usize); 3] = [
    ("snapshot/branches.json", 11),
    ("snapshot/shift.json", 3),
    ("snapshot/transactions.json", 3),
];

/// Re-expected for CA-10 #626 by the ticket author (design §6): an action CA-10 retires, i.e. a
/// direct shift action or one of the `ops*` compat names (`opsNope` included: no `ops*` name is
/// custody's to answer any more). v3 answers it as a name v2 does not know.
pub fn retired_by_ca10(action: &str) -> bool {
    CA10_RETIRED.contains(&action)
        || action
            .strip_prefix("ops")
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_uppercase()))
}

/// The name sent in place of a retired `action`: [`NO_SUCH_NAME`] for RE-7's, otherwise a name v2
/// does not know of the same length (so a body that names it keeps its length).
pub fn unknown_name_for(action: &str) -> String {
    if action == RETIRED {
        NO_SUCH_NAME.to_owned()
    } else {
        format!("ca10{}", "Z".repeat(action.len().saturating_sub(4)))
    }
}

#[allow(clippy::duplicate_mod)]
#[path = "../shift_setup_common/mod.rs"]
mod shift_setup_common;

#[allow(clippy::duplicate_mod)]
#[rustfmt::skip]
#[path = "../shift_holder_common/mod.rs"]
mod shift_holder_common;

/// The ten actions part one of T11a dispatches.
pub const PART_ONE: [&str; 10] = [
    "adminSnapshot",
    "adminDeskConfig",
    "getConnectionStatus",
    "adminSaveRate",
    "adminCreateBranch",
    "adminOpenShift",
    "adminMoveStock",
    "adminCloseShift",
    "adminAbandonRequest",
    "reserveCustomer",
];

pub mod gap_documents;
pub mod gap_save;

/// A family of part two: the names one file of
/// `api/src/application/compat/actions/` answers once it is wired. Whether it
/// is wired is the constant `WIRED` of the family's own `gap_*.rs` here, so the
/// ticket that wires a family edits that one file and no shared one (T11a-0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// `documents`.
    Documents,
    /// `saveTransaction`.
    Save,
}

impl Family {
    // CA-10 #626 (design §6): the custody family (`ops*`) is retired, not part two any more
    pub const ALL: [Family; 2] = [Family::Documents, Family::Save];

    /// The family `action` belongs to; `None` for a name of part one and for a
    /// name nobody knows.
    pub fn of(action: &str) -> Option<Family> {
        if action == "saveTransaction" {
            Some(Family::Save)
        } else if action == "documents" {
            Some(Family::Documents)
        } else {
            None
        }
    }

    /// Whether the family's ticket says its file answers its names.
    pub fn wired(self) -> bool {
        match self {
            Family::Documents => gap_documents::WIRED,
            Family::Save => gap_save::WIRED,
        }
    }
}

/// A name of part two that is still answered through the gap: `saveTransaction`,
/// `documents`, anything starting `ops` — each for as long as its family is not
/// wired. With no family wired this is every name of part two.
pub fn is_part_two(action: &str) -> bool {
    Family::of(action).is_some_and(|family| !family.wired())
}

const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, b'I', b'H', b'D', b'R',
];
const PDF: &[u8] = b"%PDF-1.7 a slip\n";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

// ── what a test controls ─────────────────────────────────────────────────────

/// The clock the routes read.
pub struct TestClock(Mutex<DateTime<Utc>>);

impl TestClock {
    pub fn set(&self, at: DateTime<Utc>) {
        *self.0.lock().expect("the clock's lock") = at;
    }

    /// Moves the clock on by `by`, so what the routes make next is made at a later time.
    pub fn advance(&self, by: chrono::Duration) {
        *self.0.lock().expect("the clock's lock") += by;
    }
}

impl RpcClock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().expect("the clock's lock")
    }
}

/// The Bank of Thailand. It answers — a day with no row, unless the test gave
/// it the reference v2 recorded — or, once told to, does not answer at all.
pub struct Bank {
    next: Mutex<Option<ReferenceRates>>,
    asked: AtomicU64,
}

impl Default for Bank {
    fn default() -> Self {
        let bank = Bank {
            next: Mutex::new(None),
            asked: AtomicU64::new(0),
        };
        bank.answer_nothing_special();
        bank
    }
}

impl Bank {
    pub fn answer_next(&self, rates: ReferenceRates) {
        *self.next.lock().expect("the bank's lock") = Some(rates);
    }

    /// Up, with no row for any currency.
    pub fn answer_nothing_special(&self) {
        self.answer_next(ReferenceRates {
            source: "https://www.bot.or.th/th/statistics/exchange-rate.html".to_string(),
            fetched_at: "2026-10-06T05:00:00.000Z".to_string(),
            kind: "DAILY_COMMERCIAL_BANK_AVERAGE".to_string(),
            rows: Vec::new(),
        });
    }

    pub fn go_down(&self) {
        *self.next.lock().expect("the bank's lock") = None;
    }

    pub fn times_asked(&self) -> u64 {
        self.asked.load(Ordering::Relaxed)
    }
}

impl RateSource for Bank {
    fn fetch(&self) -> Pin<Box<dyn Future<Output = AppResult<ReferenceRates>> + Send + '_>> {
        Box::pin(async move {
            self.asked.fetch_add(1, Ordering::Relaxed);
            self.next
                .lock()
                .expect("the bank's lock")
                .clone()
                .ok_or_else(|| AppError::Domain(BANK_DOWN.to_string()))
        })
    }
}

// ── the desk: a database, the owner, and what sits behind the routes ─────────

pub struct Desk {
    pub db: TestDb,
    pub clock: Arc<TestClock>,
    pub bank: Arc<Bank>,
    pub files: FileService,
    pub workflow: SaveTransactionWorkflow,
    pub shift_balances:
        Option<Arc<dyn xc_shift::domain::shift::repositories::shift_balances::ShiftBalances>>,
    scratch: PathBuf,
}

impl Desk {
    /// A fresh schema with `api/migrations/` applied (so `MAIN` exists) and the
    /// owner `owner`, as after `xchange create-owner --username owner`.
    pub async fn open() -> Desk {
        let desk = Desk::open_without_cash().await;
        // LG-4 (#929, lane controller's 1:1 ruling): a shift's opening now leaves the ledger's SAFE (design §4.4),
        // which v2 never had to hold. MAIN's safe holds enough for every opening a test makes.
        crate::snapshot_common::lg4_fund_safe(&desk.db.pool, "MAIN").await;
        desk
    }

    /// [`Desk::open`] without MAIN's ledger SAFE funded (LG-4 #929): for a test that opens no shift and removes a
    /// currency from the register, which a funded SAFE's accounts would hold on to.
    pub async fn open_without_cash() -> Desk {
        let db = TestDb::start().await;
        AccountService::new(
            Arc::new(PgAccountRepository::new(db.pool.clone())),
            PasswordHasher::with_cost(8, 1, 1),
        )
        .create_owner("owner", OWNER_PASSWORD, SystemTime::now().into())
        .await
        .expect("create the owner");

        let scratch = std::env::temp_dir().join(unique("xc-t11a"));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("create a scratch directory");
        let files = FileService::new(db.pool.clone(), scratch.join("uploads"));
        let shifts = ShiftService::new(
            Arc::new(ledger_shift_repository(db.pool.clone())),
            db.branch_tx(),
        );
        let workflow = SaveTransactionWorkflow::new(db.branch_tx(), shifts, files.clone());
        Desk {
            db,
            clock: Arc::new(TestClock(Mutex::new(time("2026-10-06T05:00:00.000Z")))),
            bank: Arc::new(Bank::default()),
            files,
            workflow,
            shift_balances: None,
            scratch,
        }
    }

    pub fn with_shift_balances(
        mut self,
        sb: Arc<dyn xc_shift::domain::shift::repositories::shift_balances::ShiftBalances>,
    ) -> Self {
        self.shift_balances = Some(sb);
        self
    }

    /// The server's state, its upload directory the one [`Desk::files`] keeps
    /// files in: a save through the routes finds the files uploaded behind
    /// them (P2-S).
    pub fn state(&self) -> AppState {
        let uploads = self.scratch.join("uploads");
        let uploads = uploads.to_str().expect("a scratch path in UTF-8");
        let mut state = AppState::new(
            Arc::new(settings(&[("APP_FILES__DIR", uploads)])),
            self.db.pool.clone(),
        );
        if let Some(ref sb) = self.shift_balances {
            state = state.with_shift_balances(sb.clone());
        }
        state
    }

    /// The employees of a recorded file, with the ids v2 gave them (a staff
    /// answer prints them), in the order recorded (the order they were made
    /// in), each with the password [`STAFF_PASSWORD`]. Somebody recorded with
    /// `mustChange` has not changed the first password.
    pub async fn employ(&self, people: &[Person]) {
        let hash = PasswordHasher::with_cost(8, 1, 1)
            .hash(STAFF_PASSWORD)
            .expect("hash the staff password");
        let first = time("2026-10-01T00:00:00.000Z");
        for (i, person) in people.iter().filter(|p| !p.is_owner()).enumerate() {
            sqlx::query(
                "INSERT INTO employees (id, username, name, role, branch_id, password_hash, \
                 active, must_change, version, created_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, true, $7, 1, $8)",
            )
            .bind(&person.id)
            .bind(&person.username)
            .bind(&person.name)
            .bind(&person.role)
            .bind(person.branch_id.as_deref().expect("an employee's branch"))
            .bind(&hash)
            .bind(person.must_change)
            .bind(first + Duration::seconds(i as i64))
            .execute(&self.db.pool)
            .await
            .unwrap_or_else(|e| panic!("employ {}: {e}", person.key));
            crate::snapshot_common::seed_opener(&self.db.pool, &person.id).await;
        }
    }

    /// The branch's day exists, CLOSED, behind the routes. Since CA-10 (#626 Q2) custody always applies: there is
    /// no `enabled` switch left to turn on.
    pub async fn custody_on(&self, branch: &str) {
        sqlx::query(
            "INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'CLOSED') \
             ON CONFLICT (branch_id) DO NOTHING",
        )
        .bind(branch)
        .execute(&self.db.pool)
        .await
        .expect("switch custody on");
    }

    pub async fn finish(self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
        self.db.finish().await;
    }
}

/// The server's whole route table (`bootstrap::configure`) with the test's
/// clock and bank registered after it, so that they replace the real ones.
#[macro_export]
macro_rules! rpc_app {
    ($desk:expr) => {{
        let state = $desk.state();
        let clock: std::sync::Arc<dyn xchange::presentation::http::rpc::RpcClock> =
            $desk.clock.clone();
        let bank: std::sync::Arc<dyn xc_rate::domain::rate::entities::rate_source::RateSource> =
            $desk.bank.clone();
        $crate::rpc_common::App::new(
            actix_web::test::init_service(
                actix_web::App::new()
                    .configure(move |cfg| xchange::bootstrap::configure(cfg, state))
                    .app_data(actix_web::web::Data::from(clock))
                    .app_data(actix_web::web::Data::new(bank)),
            )
            .await,
            |t| t.to_request(),
        )
    }};
}

// ── one request ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Call {
    pub method: Method,
    pub host: String,
    pub path: String,
    /// Header name in lower case → value. Absent = not sent.
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Call {
    /// A POST as the kept screens send it from `origin`: same origin, the
    /// marker header, JSON.
    pub fn post(origin: &str, path: &str, body: impl Into<Vec<u8>>) -> Call {
        let host = origin
            .split_once("://")
            .map_or(origin, |(_, host)| host)
            .to_string();
        let mut headers = BTreeMap::new();
        headers.insert("origin".to_string(), origin.to_string());
        headers.insert("x-xchange-request".to_string(), "1".to_string());
        headers.insert("content-type".to_string(), "application/json".to_string());
        Call {
            method: Method::POST,
            host,
            path: path.to_string(),
            headers,
            body: body.into(),
        }
    }

    /// `{"action": action, "payload": payload}` sent to `path`.
    pub fn action(origin: &str, path: &str, action: &str, payload: Value) -> Call {
        Call::post(
            origin,
            path,
            json!({ "action": action, "payload": payload }).to_string(),
        )
    }

    pub fn header(mut self, name: &str, value: &str) -> Call {
        self.headers.insert(name.to_string(), value.to_string());
        self
    }

    pub fn without(mut self, name: &str) -> Call {
        self.headers.remove(name);
        self
    }

    /// With the session cookie(s) of somebody signed in.
    pub fn cookie(self, cookie: &str) -> Call {
        self.header("cookie", cookie)
    }
}

#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    /// Header name in lower case → value (`set-cookie`: the last one).
    pub headers: BTreeMap<String, String>,
    pub cookies: Vec<String>,
    pub text: String,
}

impl Answer {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text)
            .unwrap_or_else(|e| panic!("the body is JSON ({e}): {:.300}", self.text))
    }

    pub fn error(&self) -> String {
        self.json()["error"]
            .as_str()
            .unwrap_or_else(|| panic!("an error answer: {:.300}", self.text))
            .to_string()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    /// Fails unless this is the refusal `status` `{"error": sentence}`, byte
    /// for byte, as JSON, never cached.
    pub fn assert_refused(&self, status: u16, sentence: &str, label: &str) {
        assert_eq!(self.status, status, "{label}: {:.300}", self.text);
        assert_eq!(
            self.text,
            format!("{{\"error\":{}}}", Value::String(sentence.to_string())),
            "{label}"
        );
        assert_eq!(
            self.header("content-type"),
            Some("application/json"),
            "{label}"
        );
    }
}

type CallFn = dyn Fn(Call) -> Pin<Box<dyn Future<Output = Answer>>>;

/// The app under test with its request type hidden.
pub struct App {
    call: Box<CallFn>,
}

impl App {
    /// `to_request` is always `|t| t.to_request()`; it is passed in so that the
    /// request type is inferred where the app is built.
    pub fn new<S, R, B>(service: S, to_request: fn(test::TestRequest) -> R) -> Self
    where
        S: Service<R, Response = ServiceResponse<B>, Error = Error> + 'static,
        R: 'static,
        B: MessageBody + 'static,
    {
        let service = std::rc::Rc::new(service);
        Self {
            call: Box::new(move |call| {
                let service = service.clone();
                Box::pin(async move { exchange(&*service, to_request, call).await })
            }),
        }
    }

    pub async fn send(&self, call: Call) -> Answer {
        (self.call)(call).await
    }
}

async fn exchange<S, R, B>(app: &S, to_request: fn(test::TestRequest) -> R, call: Call) -> Answer
where
    S: Service<R, Response = ServiceResponse<B>, Error = Error>,
    B: MessageBody,
{
    let mut request = test::TestRequest::default()
        .method(call.method.clone())
        .uri(&call.path)
        .insert_header((header::HOST, call.host.as_str()));
    for (name, value) in &call.headers {
        request = request.insert_header((name.as_str(), value.as_str()));
    }
    if call.method != Method::GET {
        request = request.set_payload(call.body.clone());
    }
    let response = test::call_service(app, to_request(request)).await;
    let status = response.status().as_u16();
    let mut headers = BTreeMap::new();
    let mut cookies = Vec::new();
    for (name, value) in response.headers() {
        let value = value.to_str().expect("an ASCII header").to_string();
        if name == header::SET_COOKIE {
            cookies.push(value.clone());
        }
        headers.insert(name.as_str().to_ascii_lowercase(), value);
    }
    let bytes = test::read_body(response).await;
    Answer {
        status,
        headers,
        cookies,
        text: String::from_utf8(bytes.to_vec()).expect("a UTF-8 body"),
    }
}

// ── sessions, through the real doors of T4c ──────────────────────────────────

/// Person key → the `Cookie` header that person's browser sends.
pub type Cookies = BTreeMap<String, String>;

fn session_cookie(answer: &Answer, name: &str, who: &str) -> String {
    assert_eq!(answer.status, 200, "{who} signs in: {}", answer.text);
    answer
        .cookies
        .iter()
        .filter_map(|line| line.split(';').next())
        .find(|pair| pair.starts_with(name) && pair.len() > name.len() + 1)
        .unwrap_or_else(|| panic!("{who}: no {name} cookie in {:?}", answer.cookies))
        .to_string()
}

/// Signs everybody of `people` in — the owner at `/api/auth`, an employee at
/// `/api/staff/login` — and gives back each one's cookie.
pub async fn sign_in(app: &App, origin: &str, people: &[Person]) -> Cookies {
    let mut cookies = Cookies::new();
    for person in people {
        let cookie = if person.is_owner() {
            let answer = app
                .send(
                    Call::post(
                        origin,
                        "/api/auth",
                        json!({ "action": "login", "payload": {
                            "username": person.username, "password": OWNER_PASSWORD } })
                        .to_string(),
                    )
                    .header("x-real-ip", &unique("owner-address")),
                )
                .await;
            session_cookie(&answer, "__Host-xc_owner", &person.key)
        } else {
            let answer = app
                .send(
                    Call::post(
                        origin,
                        "/api/staff/login",
                        json!({ "payload": {
                            "username": person.username, "password": STAFF_PASSWORD } })
                        .to_string(),
                    )
                    .header("x-real-ip", &unique("staff-address")),
                )
                .await;
            session_cookie(&answer, "__Host-xc_staff", &person.key)
        };
        cookies.insert(person.key.clone(), cookie);
    }
    cookies
}

/// One employee of every staff role in `MAIN`, and the owner: the callers the
/// role table is checked with. Keys are the role names (`OWNER`, `FRONT`, …).
pub fn one_of_each_role() -> Vec<Person> {
    let mut people = vec![Person {
        key: "OWNER".to_string(),
        kind: "owner".to_string(),
        id: "owner".to_string(),
        username: "owner".to_string(),
        name: "Owner".to_string(),
        role: "OWNER".to_string(),
        branch_id: None,
        must_change: false,
    }];
    for (i, role) in Role::STAFF.iter().enumerate() {
        people.push(Person {
            key: role.as_str().to_string(),
            kind: "staff".to_string(),
            id: format!("00000000-0000-4000-8000-0000000000{:02x}", i + 1),
            username: format!("role{i}"),
            name: format!("Somebody {i}"),
            role: role.as_str().to_string(),
            branch_id: Some(MAIN.to_string()),
            must_change: false,
        });
    }
    people
}

// ── reading a recorded step ──────────────────────────────────────────────────

/// The body a step sent. A long body is written in the file as its length and
/// its beginning: the padding letter is repeated and the two braces closed.
pub fn body_of(step: &Step, payload: Option<&Value>) -> String {
    if let Some(raw) = &step.raw {
        return raw.clone();
    }
    if let Some(length) = step.raw_length {
        let start = step
            .raw_start
            .clone()
            .unwrap_or_else(|| panic!("step {} has no rawStart", step.n));
        let pad = start.chars().last().expect("a rawStart");
        assert_eq!(pad.len_utf16(), 1, "step {}", step.n);
        let tail = "\"}}";
        let have = start.encode_utf16().count() + tail.len();
        let mut body = start;
        body.extend(std::iter::repeat_n(pad, length - have));
        body.push_str(tail);
        assert_eq!(body.encode_utf16().count(), length, "step {}", step.n);
        return body;
    }
    match (&step.action, payload) {
        (Some(action), Some(payload)) => {
            json!({ "action": action, "payload": payload }).to_string()
        }
        _ => String::new(),
    }
}

/// The name a step asked for: its `action`, or the `action` of its raw body
/// when that is an object naming one as text.
pub fn action_of(step: &Step) -> Option<String> {
    if let Some(action) = &step.action {
        return Some(action.clone());
    }
    let raw = step.raw.as_deref()?;
    let body: Value = serde_json::from_str(raw).ok()?;
    body.get("action")?.as_str().map(str::to_string)
}

/// The request a step was, with the cookie of whoever sent it. `payload` is
/// the step's payload with v2's ids replaced by v3's.
pub fn call_of(fixture: &Fixture, step: &Step, cookies: &Cookies, payload: Option<&Value>) -> Call {
    let route = step
        .route
        .as_deref()
        .unwrap_or_else(|| panic!("{}#{}: not a request", fixture.name, step.n));
    // a whole URL: the request went to another host
    let (origin_of_request, path) = match route.find("://") {
        Some(scheme_end) => {
            let after = &route[scheme_end + 3..];
            let slash = after.find('/').unwrap_or(after.len());
            (&route[..scheme_end + 3 + slash], &after[slash..])
        }
        None => (fixture.origin.as_str(), route),
    };
    let mut call = Call::post(origin_of_request, path, body_of(step, payload));
    // the browser was on the desk's own origin whatever host it called
    call.headers
        .insert("origin".to_string(), fixture.origin.clone());
    call.method = Method::from_bytes(step.method.as_deref().unwrap_or("POST").as_bytes())
        .expect("a recorded method");
    if let Some(cookie) = cookies.get(&step.sent_as) {
        call.headers.insert("cookie".to_string(), cookie.clone());
    }
    for (name, value) in &step.request_headers {
        match value {
            Some(value) => call
                .headers
                .insert(name.to_ascii_lowercase(), value.clone()),
            None => call.headers.remove(&name.to_ascii_lowercase()),
        };
    }
    call
}

/// The caller of a step as `Policy` sees them; `None` for `nobody`, and for an
/// employee whose first password is unchanged (no route lets them in).
pub fn actor_of(fixture: &Fixture, step: &Step) -> Option<Actor> {
    let person = fixture.sender(step)?;
    if person.is_owner() {
        return Some(Actor::owner("owner"));
    }
    if person.must_change {
        return None;
    }
    Actor::staff(
        person.id.clone(),
        Role::parse(&person.role)?,
        BranchId::new(person.branch_id.clone()?),
    )
}

/// The organisation a password session's user is in (B1, design §3.3): any one name, because
/// `Policy::authorize` checks the caller against their own organisation.
pub const ORGANIZATION: &str = "XC";

/// The rule a route asks once every route is on `Policy::authorize` (OR-4b, #767; design §4.2):
/// the caller as B1 maps them (OWNER → EXECUTIVE + ADMIN organisation-wide, an employee's role on
/// their branch), naming no branch. The custody rows (`Need::Custody`, #582) stay on
/// `Policy::allow`.
pub fn route_rule(
    actor: Option<&Actor>,
    entry: EntryPath,
    action: &str,
) -> Result<(), PolicyError> {
    let action = Action::new(action);
    if Policy::need(entry, &action) == Some(Need::Custody) {
        return Policy::allow(actor, entry, &action);
    }
    let user = actor.map(|a| actor_from_v3(a, ORGANIZATION, DateTime::<Utc>::UNIX_EPOCH));
    Policy::authorize(user.as_ref(), entry, &action, None, Utc::now()).map(|_| ())
}

fn text_of(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        None | Some(Value::Null) => String::new(),
        Some(other) => panic!("`{key}` is {other}"),
    }
}

// ── the replay ───────────────────────────────────────────────────────────────

/// What is done with one recorded step.
#[derive(Debug, Clone)]
pub enum Rule {
    /// Sent through HTTP and held to what v2 answered.
    Replay,
    /// Sent through HTTP and held to what was decided instead of v2's answer:
    /// this status and sentence.
    Differs { status: u16, sentence: &'static str },
    /// A caller v2 let through and the Blueprint's role table refuses (OR-4b, #767; design §4.2:
    /// C2 `ExchangeTransact` is TELLER's, C3 `CashManage` is BRANCH_MANAGER's and FINANCE's):
    /// sent, and held to the route's refusal. What v2 accepted is applied behind the routes, as
    /// recorded, so the steps after it see the state v2 recorded.
    Blueprint(PolicyError),
    /// A claim v2 let through and the queue refuses (QU-3: a teller holds one CALLED or SERVING ticket;
    /// QU-6 #1354: the kiosk's `accept` starts the request's ticket; design `queue-tickets.md` §7, "a
    /// teller 'accepts' any waiting request" becomes call/start): sent, and held to the kiosk route's
    /// refusal, `400 {"error": <code>}`. What v2 accepted is applied behind the routes, as recorded and
    /// by its own sender (`KioskReplay`), so the steps after it see the state v2 recorded.
    QueueRefuses(&'static str),
    /// Sent through HTTP and held to v2's answer as `edit` changes it: a caller the Blueprint
    /// lets through and answers otherwise (OR-4b, #767; e.g. C8, the owner's user is not offered
    /// the kiosk queue's work).
    RecordedWith(fn(&mut Value)),
    /// Sent through HTTP and held to the recorded answer of another step (v3
    /// accepts what v2 refused): everything but `syncedAt`, which is this
    /// step's own clock. (An answer without `syncedAt` — a kiosk answer — is
    /// held to the other step's whole; added for ticket POL.)
    AnswersAs(usize),
    /// A name of part two that `Policy` lets through: sent, and held to the
    /// unknown-name refusal part one answers. When v2 accepted it and it
    /// changed something later steps see, it is then applied behind the routes.
    PartTwo,
    /// Not a request of these two routes; applied behind them when later steps
    /// need it (an upload), otherwise left out.
    Excluded(&'static str),
    /// Re-expected for RE-7 #604 by the ticket author: a known difference (design §9 removes
    /// the action). `adminSaveRate`, sent through HTTP and held to the answer v3 gives the same
    /// request naming [`NO_SUCH_NAME`]; when v2 accepted it, the rate it saved is then
    /// published behind the routes.
    Retired,
    /// Re-expected for RE-7 #604 by the ticket author: a known difference (design §9 removes
    /// the action). An `adminAbandonRequest` v2 accepted for an id only a direct save had
    /// applied: v3 saved nothing for it, so it is held to v2's answer with this
    /// `requestResolution` (`CANCELLED` where v2 said `APPLIED`).
    ResolvedAs(&'static str),
    /// A caller v2's `Policy::allow` refused and RBAC §11.2 (`RoleGrants`, `rbac-groups.md` §11.2,
    /// #3659) lets through: sent through HTTP, held to not be the role refusal v2 recorded, and
    /// counted in [`Tally::widened`].
    Widened(PolicyError),
    /// An owner `saveTransaction` step that RBAC §11.2 (#3659) lets past `Policy::authorize` (via
    /// `EXECUTIVE` Customers Full) and that the CA-10 teller gate (`check_teller_save_in`) refuses
    /// with `ShiftError::NoOpenShift` (`กรุณาเปิดกะจากหน้าแอดมินก่อน`) because the owner has no open
    /// shift on the recorded day; those v2 accepted are then applied behind the routes (`save_behind`).
    OwnerNoOpenShift(&'static str),
}

/// Recorded steps where v2's `Policy::allow` refused the caller and RBAC §11.2 (`RoleGrants`,
/// `rbac-groups.md` §11.2, #3659) widens the caller's role so `Policy::authorize` lets them
/// through: `(fixture, step, permission)`.
pub const WIDENED_BY_11_2: &[(&str, usize, Permission)] = &[
    ("rpc/engine_actions.json", 81, Permission::ExchangeTransact),
    ("rpc/engine_actions.json", 126, Permission::ExchangeTransact),
    (
        "rpc/engine_actions.json",
        129,
        Permission::IdentityDocumentRead,
    ),
    ("snapshot/empty.json", 12, Permission::ExchangeTransact),
    ("snapshot/empty.json", 16, Permission::ExchangeTransact),
    ("snapshot/empty.json", 20, Permission::ExchangeTransact),
    ("snapshot/rates.json", 17, Permission::ExchangeTransact),
    ("snapshot/branches.json", 28, Permission::ExchangeTransact),
    ("snapshot/shift.json", 20, Permission::ExchangeTransact),
    ("snapshot/shift.json", 24, Permission::ExchangeTransact),
    ("snapshot/shift.json", 32, Permission::ExchangeTransact),
    ("snapshot/shift.json", 55, Permission::ExchangeTransact),
    (
        "snapshot/transactions.json",
        34,
        Permission::ExchangeTransact,
    ),
    (
        "snapshot/transactions.json",
        38,
        Permission::ExchangeTransact,
    ),
    (
        "snapshot/transactions.json",
        53,
        Permission::IdentityDocumentRead,
    ),
    ("kiosk/queue.json", 77, Permission::QueueOperate),
    ("kiosk/queue.json", 78, Permission::QueueOperate),
    ("kiosk/queue.json", 79, Permission::QueueOperate),
    ("kiosk/queue.json", 80, Permission::QueueOperate),
    ("kiosk/queue_work.json", 64, Permission::QueueOperate),
];

/// CA-10 #626 + RBAC §11.2 (#3659): the owner holds `ExchangeTransact` again via `EXECUTIVE`
/// (Customers Full), so `Policy::authorize` lets the owner's `saveTransaction` steps through to the
/// workflow, where the CA-10 teller gate (`check_teller_save_in`) refuses with `ShiftError::NoOpenShift`
/// (`กรุณาเปิดกะจากหน้าแอดมินก่อน`) because the owner has no open shift on the recorded day:
/// `(fixture, step, reason)`.
pub const OWNER_SAVE_NO_OPEN_SHIFT: &[(&str, usize, &str)] = &[
    (
        "rpc/engine_actions.json",
        111,
        "CA-10 teller gate before customer confirm: owner has no open shift",
    ),
    (
        "rpc/engine_actions.json",
        112,
        "CA-10 teller gate before customer confirm: owner has no open shift",
    ),
    (
        "rpc/engine_actions.json",
        114,
        "CA-10 teller gate before shift stock check: owner has no open shift",
    ),
    (
        "rpc/engine_actions.json",
        115,
        "CA-10 teller gate before shift stock check: owner has no open shift",
    ),
    (
        "rpc/engine_actions.json",
        120,
        "CA-10 teller gate on live BUY save: owner has no open shift",
    ),
    (
        "rpc/engine_actions.json",
        121,
        "CA-10 teller gate on repeat of #120: owner has no open shift",
    ),
    (
        "rpc/engine_actions.json",
        122,
        "CA-10 teller gate before prior-submission hash check: owner has no open shift",
    ),
    (
        "rpc/engine_actions.json",
        123,
        "CA-10 teller gate on live SELL save: owner has no open shift",
    ),
    (
        "snapshot/transactions.json",
        10,
        "CA-10 teller gate on live save: owner has no open shift",
    ),
    (
        "snapshot/transactions.json",
        11,
        "CA-10 teller gate on live save: owner has no open shift",
    ),
    (
        "snapshot/transactions.json",
        21,
        "CA-10 teller gate on live save: owner has no open shift",
    ),
    (
        "kiosk/queue.json",
        138,
        "QU-6 queue desk gate on kiosk accept: owner has no open shift (#3810)",
    ),
    (
        "kiosk/queue_work.json",
        72,
        "QU-6 queue desk gate on kiosk accept: owner has no open shift (#3810)",
    ),
];

/// Every step of a replay, by what was done with it.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Tally {
    /// Held to v2's recorded answer.
    pub replayed: Vec<usize>,
    /// Held to a decided difference.
    pub differences: Vec<usize>,
    /// Names of part two, held to the unknown-name refusal.
    pub part_two: Vec<usize>,
    /// Left out, with the reason.
    pub excluded: Vec<(usize, &'static str)>,
    /// Of `part_two` and `excluded`: those applied behind the routes.
    pub applied_behind: Vec<usize>,
    /// Held to the Blueprint's refusal (OR-4b); those v2 accepted are also in `applied_behind`.
    pub blueprint: Vec<usize>,
    /// Held to not be v2's recorded role refusal (RBAC §11.2, #3659).
    pub widened: Vec<usize>,
    /// Owner `saveTransaction` and kiosk `accept` steps refused for having no open shift (RBAC §11.2, #3659, #3810).
    pub owner_no_open_shift: Vec<usize>,
}

impl Tally {
    pub fn total(&self) -> usize {
        self.replayed.len()
            + self.differences.len()
            + self.part_two.len()
            + self.excluded.len()
            + self.blueprint.len()
            + self.widened.len()
            + self
                .owner_no_open_shift
                .iter()
                .filter(|n| !self.blueprint.contains(n))
                .count()
    }
}

pub struct Replay<'f> {
    pub desk: Desk,
    pub app: App,
    pub fixture: &'f Fixture,
    pub cookies: Cookies,
    pub ids: IdMap,
    pub tally: Tally,
    /// Step number → the answer the routes gave, for a test that looks closer.
    pub answers: BTreeMap<usize, Answer>,
    /// Decided differences and exclusions of this file, by step number.
    pub rules: BTreeMap<usize, Rule>,
}

/// (who, a file's key) → the key of the same file uploaded again by `who` (OR-4b):
/// [`Replay::uploads_as`]. Keys are made at random, so one map serves every test.
static UPLOADS_BY: Mutex<BTreeMap<(String, String), String>> = Mutex::new(BTreeMap::new());

/// (test schema, branch) → this replay opened that branch's day for a save (CA-10 #626, ruling A: try 2):
/// [`Replay::open_day_for_save`]. Read by the `operations.phase` restatement in the snapshot comparison, so only a
/// day the replay opened is compared as OPEN; a day the code under test opens by itself is compared as recorded.
static DAYS_OPENED_FOR_A_SAVE: Mutex<BTreeSet<(String, String)>> = Mutex::new(BTreeSet::new());

impl<'f> Replay<'f> {
    /// A desk with the people of `fixture` employed and signed in.
    pub async fn start(desk: Desk, app: App, fixture: &'f Fixture) -> Replay<'f> {
        desk.employ(&fixture.people).await;
        // LG-6.r5 (#1044, lane controller's ruling): a v2 recording with custody off never held a safe and
        // printed `operations.safe` null. `Desk::open` funded MAIN's ledger SAFE; empty it, and fund each
        // opening or IN move just in time instead (`Replay::step`).
        sweep_safe(&desk.db.pool, &BranchId::new(MAIN.to_string())).await;
        let cookies = sign_in(&app, &fixture.origin, &fixture.people).await;
        Replay {
            desk,
            app,
            fixture,
            cookies,
            ids: IdMap::new(),
            tally: Tally::default(),
            answers: BTreeMap::new(),
            rules: BTreeMap::new(),
        }
    }

    /// A TELLER (v2's FRONT) of `branch`, employed and signed in now: the key of their cookie
    /// in [`Replay::cookies`]. OR-4b (#767, design §4.2 C2): `ExchangeTransact` is TELLER's, so
    /// whoever reserves and saves in a branch is one of its tellers, not the owner's user.
    pub async fn employ_teller(&mut self, branch: &str) -> String {
        let key = format!("TELLER@{branch}");
        let n = self.cookies.len() + 1;
        let teller = Person {
            key: key.clone(),
            kind: "staff".to_string(),
            id: format!("00000000-0000-4000-8000-0000000f{n:04x}"),
            username: format!("teller{n}"),
            name: format!("Teller of {branch}"),
            role: "FRONT".to_string(),
            branch_id: Some(branch.to_string()),
            must_change: false,
        };
        let people = [teller];
        self.desk.employ(&people).await;
        let cookies = sign_in(&self.app, &self.fixture.origin, &people).await;
        self.cookies.extend(cookies);
        key
    }

    /// `payload` with its `passport` and `slip` uploaded again by `who`, in `who`'s branch. A save
    /// attaches only files its saver uploaded; OR-4b (#767, C2) moves the owner's saves to a
    /// TELLER, so the files the recorded owner uploaded are uploaded again by the teller. The same
    /// file asked twice is the same new file.
    pub async fn uploads_as(&self, who: &str, payload: &Value) -> Value {
        let person = self
            .fixture
            .person(who)
            .unwrap_or_else(|| panic!("{who} is a person of {}", self.fixture.name));
        let mut out = payload.clone();
        for field in ["passport", "slip"] {
            let Some(key) = payload
                .get(field)
                .and_then(|f| f.get("uploadKey"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let known = UPLOADS_BY
                .lock()
                .expect("the uploads")
                .get(&(who.to_string(), key.to_string()))
                .cloned();
            let new_key = match known {
                Some(k) => k,
                None => {
                    let mime: Option<String> =
                        sqlx::query_scalar("SELECT mime FROM files WHERE key = $1")
                            .bind(key)
                            .fetch_optional(&self.desk.db.pool)
                            .await
                            .expect("read a file's kind");
                    let Some(mime) = mime else {
                        // not a file anybody uploaded: sent as it is
                        continue;
                    };
                    let bytes = if mime == "application/pdf" { PDF } else { PNG };
                    let branch = person.branch_id.clone().unwrap_or_else(|| MAIN.to_string());
                    let k = self
                        .desk
                        .files
                        .upload(&BranchId::new(branch), &person.id, &mime, bytes, Utc::now())
                        .await
                        .unwrap_or_else(|e| panic!("upload {field} again as {who}: {e:?}"));
                    UPLOADS_BY
                        .lock()
                        .expect("the uploads")
                        .insert((who.to_string(), key.to_string()), k.clone());
                    k
                }
            };
            out[field]["uploadKey"] = json!(new_key);
        }
        out
    }

    pub fn rule(mut self, n: usize, rule: Rule) -> Self {
        self.rules.insert(n, rule);
        self
    }

    pub fn label(&self, step: &Step) -> String {
        format!(
            "{}#{} ({} {} as {})",
            self.fixture.name,
            step.n,
            step.route.as_deref().unwrap_or("-"),
            action_of(step).unwrap_or_else(|| "-".to_string()),
            step.sent_as
        )
    }

    /// The rule of a step: the test's own where it gave one, else by what the
    /// step is.
    fn rule_of(&self, step: &Step) -> Rule {
        if let Some(rule) = self.rules.get(&step.n) {
            return rule.clone();
        }
        let path = step.path().unwrap_or_default();
        let entry = match path {
            RPC => EntryPath::Rpc,
            STAFF_RPC => EntryPath::StaffRpc,
            // OR-4b: the kiosk route asks the route table too (C8)
            "/api/kiosk" => EntryPath::Kiosk,
            "/api/upload" | "/api/staff/upload" => {
                return Rule::Excluded("an upload: the file routes are another ticket's");
            }
            // any other path is sent as it was and held to what v2 answered
            _ => return Rule::Replay,
        };
        let Some(action) = action_of(step) else {
            return Rule::Replay;
        };
        // CA-10 #626: the direct shift actions and the `ops*` names are retired as RE-7's was
        if action == RETIRED || retired_by_ca10(&action) {
            return Rule::Retired;
        }
        // the route judges the name before the payload, unless it refuses the request first
        let judged_by_name = step.method.as_deref() == Some("POST")
            && step.request_headers.is_empty()
            && step.payload.as_ref().is_none_or(Value::is_object);
        let actor = actor_of(self.fixture, step);
        // OR-4b (#767, §4.2 C2, C3, C8): the Blueprint refuses this caller where v2 let them
        // through, or refuses them in other words
        if judged_by_name {
            let v2 = Policy::allow(actor.as_ref(), entry, &Action::new(&action));
            // the staff route reads the staff session only: the owner's is nobody there
            let caller = actor.as_ref().filter(|_| {
                !(entry == EntryPath::StaffRpc
                    && self.fixture.sender(step).is_some_and(Person::is_owner))
            });
            match route_rule(caller, entry, &action) {
                Err(refusal) if v2.as_ref().err() != Some(&refusal) => {
                    return Rule::Blueprint(refusal);
                }
                Ok(()) if v2.is_err() => {
                    let v2_refusal = v2.unwrap_err();
                    if let Some((_, _, permission)) = WIDENED_BY_11_2
                        .iter()
                        .find(|(f, n, _)| *f == self.fixture.name && *n == step.n)
                    {
                        let caller = caller.expect("a widened caller is signed in");
                        let user = actor_from_v3(caller, ORGANIZATION, DateTime::<Utc>::UNIX_EPOCH);
                        assert!(
                            user.roles
                                .iter()
                                .any(|r| RoleGrants::of(r.role.as_str()).contains(permission)),
                            "{}: listed in WIDENED_BY_11_2 for {permission:?}, which {:?} lacks",
                            self.label(step),
                            caller.role()
                        );
                        return Rule::Widened(v2_refusal);
                    }
                    panic!(
                        "{}: the Blueprint lets through a caller v2 refused; no recorded step does",
                        self.label(step)
                    );
                }
                _ => {}
            }
        }
        if !is_part_two(&action) {
            if let Some((_, _, reason)) = OWNER_SAVE_NO_OPEN_SHIFT
                .iter()
                .find(|(f, n, _)| *f == self.fixture.name && *n == step.n)
            {
                return Rule::OwnerNoOpenShift(reason);
            }
            return Rule::Replay;
        }
        // a part-two name is this part's to answer exactly when Policy refuses
        // it, or when the route refuses the request before any name is judged
        let allowed = route_rule(actor.as_ref(), entry, &action).is_ok();
        if judged_by_name && allowed {
            Rule::PartTwo
        } else {
            Rule::Replay
        }
    }

    /// Every step in the order recorded.
    pub async fn run(&mut self) {
        let fixture = self.fixture;
        for step in fixture.steps() {
            self.step(step).await;
        }
        assert_eq!(
            self.tally.total(),
            fixture.steps().len(),
            "{}: every step is accounted for",
            fixture.name
        );
    }

    pub async fn step(&mut self, step: &Step) {
        assert!(
            step.is_request(),
            "{}: a row written by the generator has no rule here",
            self.label(step)
        );
        let rule = self.rule_of(step);
        if let Rule::Excluded(reason) = rule {
            self.tally.excluded.push((step.n, reason));
            if step.upload.is_some() && step.status == Some(200) {
                self.upload_behind(step).await;
            }
            return;
        }

        self.desk.clock.set(time(&step.at));
        let payload = step.payload.as_ref().map(|p| self.ids.to_actual(p));
        let call = call_of(self.fixture, step, &self.cookies, payload.as_ref());
        // CA-10 #626 (ruling A, try 2): a compat save meets the teller gate (`check_teller_save_in`), which needs an
        // OPEN branch day; v2 had no day phase. Before a recorded save on a branch whose shift a verified OPEN count
        // opened (READY), the replay opens that day, as `dayStartService` would.
        if action_of(step).as_deref() == Some("saveTransaction") {
            let branch = payload
                .as_ref()
                .and_then(|p| p.get("branchId"))
                .and_then(Value::as_str)
                .unwrap_or("MAIN")
                .to_owned();
            if self.opened_by_count(&branch).await {
                self.open_day_for_save(&branch).await;
            }
        }
        // LG-6.r5 (#1044, lane controller's ruling): v2 with custody off took an opening, and an IN move, from no
        // safe; v3 takes them from the ledger's SAFE (design §4.4). Before a step v2 accepted, SAFE gets exactly what
        // the step takes; after it, whatever is left goes back to BANK, so SAFE holds nothing between steps and
        // `operations.safe` prints v2's null.
        let jit = if step.status == Some(200) {
            let actual = step.payload.as_ref().map(|p| self.ids.to_actual(p));
            match (action_of(step).as_deref(), actual) {
                (Some("adminOpenShift"), Some(p)) => {
                    xc_shift::domain::shift::commands::open_shift::OpenShift::from_payload(&p)
                        .ok()
                        .map(|c| c.opening.non_zero())
                }
                (Some("adminMoveStock"), Some(p)) => {
                    MoveStock::from_payload(&p).ok().and_then(|c| {
                        matches!(
                            c.direction,
                            xc_shift::domain::shift::value_objects::stock_delta::Direction::In
                        )
                        .then(|| vec![(c.currency, c.amount)])
                    })
                }
                _ => None,
            }
        } else {
            None
        };
        let jit_branch = jit.as_ref().map(|_| self.branch_of(step));
        if let (Some(amounts), Some(b)) = (&jit, &jit_branch) {
            fund_exact(&self.desk.db.pool, b, amounts).await;
        }
        let answer = self.app.send(call).await;
        if let Some(b) = &jit_branch {
            sweep_safe(&self.desk.db.pool, b).await;
        }
        let label = self.label(step);
        // LG-6.r5 (#1044, lane controller's ruling): a branch a recording creates gets no standing funds in its
        // ledger SAFE (LG-4 #929 gave it 10,000,000 of every code); the opening steps above fund it just in time.

        // CI-4 (#651): a decided customer difference (compat_common::SAME_CUSTOMER_AS), checked strictly.
        #[allow(clippy::collapsible_if)]
        if matches!(rule, Rule::Replay) {
            if let Some(same) = crate::compat_common::same_customer_as(&self.fixture.name, step.n) {
                let earlier_answer = self
                    .answers
                    .get(&same.same_as)
                    .unwrap_or_else(|| panic!("{label}: step {} was not replayed", same.same_as));
                let earlier_customer = if earlier_answer.status == 200 {
                    text_of(&earlier_answer.json(), "customerId")
                } else {
                    text_of(
                        &self.ids.to_actual(self.fixture.step(same.same_as).json()),
                        "customerId",
                    )
                };
                let expected = crate::compat_common::same_customer_answer(
                    step.json(),
                    &earlier_customer,
                    &label,
                );
                assert_eq!(
                    answer.status, 200,
                    "{label} ({}): {:.300}",
                    same.reason, answer.text
                );
                assert_same(
                    &expected,
                    &answer.json(),
                    &format!("{label} ({})", same.reason),
                );
                self.ids
                    .alias_customer(&text_of(step.json(), "customerId"), &earlier_customer);
                self.tally.differences.push(step.n);
                self.answers.insert(step.n, answer);
                return;
            }
        }

        // CI-4 (#651): a save for a customer a SameCustomerAs step aliased: its receipt names that customer.
        let saved_customer = step
            .payload
            .as_ref()
            .and_then(|p| p.get("customerId"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if matches!(rule, Rule::Replay)
            && step.status == Some(200)
            && action_of(step).as_deref() == Some("saveTransaction")
            && saved_customer.is_some_and(|c| self.ids.is_aliased_customer(&c))
        {
            let actual = answer.json();
            let url = text_of(&actual, "receiptUrl");
            let key = url.trim_start_matches("/files/").to_owned();
            let (branch_id,): (String,) =
                sqlx::query_as("SELECT branch_id FROM files WHERE key = $1")
                    .bind(&key)
                    .fetch_one(&self.desk.db.pool)
                    .await
                    .unwrap_or_else(|e| panic!("{label}: the receipt {key}: {e}"));
            let stored = self
                .desk
                .files
                .open_in_branch(&key, &BranchId::new(branch_id))
                .await
                .expect("open the receipt")
                .unwrap_or_else(|| panic!("{label}: the receipt {key} is not stored"));
            let expected = crate::compat_common::same_customer_save_answer(
                step.json(),
                &url,
                &stored.bytes,
                &label,
            );
            assert_eq!(answer.status, 200, "{label}: {:.300}", answer.text);
            assert_same(&self.ids.to_actual(&expected), &actual, &label);
            self.tally.differences.push(step.n);
            self.answers.insert(step.n, answer);
            return;
        }

        match rule {
            Rule::Replay => {
                self.hold_to_recorded(step, &answer).await;
                self.tally.replayed.push(step.n);
            }
            Rule::Differs { status, sentence } => {
                assert_ne!(
                    step.error(),
                    Some(sentence),
                    "{label}: listed as a difference, but v2 says the same"
                );
                answer.assert_refused(status, sentence, &label);
                assert_eq!(answer.header("cache-control"), Some("no-store"), "{label}");
                self.tally.differences.push(step.n);
            }
            Rule::AnswersAs(other) => {
                assert_ne!(step.status, Some(200), "{label}: v2 accepted this one");
                let mut expected = without_dropped_keys(self.fixture.step(other).json());
                // CI-4 (#651): the expectation of the step whose recording is used
                if let Some(list) = crate::compat_common::company_customers(self.fixture, other) {
                    expected = crate::compat_common::with_customers_of(
                        &expected,
                        &json!({ "customers": list }),
                        &format!(
                            "{label} ({})",
                            crate::compat_common::COMPANY_CUSTOMERS_REASON
                        ),
                    );
                }
                if expected.get("syncedAt").is_some() {
                    expected["syncedAt"] = json!(step.at);
                }
                assert_eq!(answer.status, 200, "{label}: {:.300}", answer.text);
                let actual = answer.json();
                // amended for #757 (RE-4c): rates[] is the publication (D-3, §4.9 R10)
                let expected = self.as_published(step, &actual, &expected).await;
                self.ids.learn(&expected, &actual);
                assert_same(&self.ids.to_actual(&expected), &actual, &label);
                self.tally.differences.push(step.n);
            }
            Rule::PartTwo => {
                let path = step.path().unwrap_or_default();
                let owner = self.fixture.sender(step).is_some_and(Person::is_owner);
                let sentence = match (path, owner) {
                    (RPC, true) => INVALID_REQUEST,
                    (RPC, false) => ROLE_NOT_ALLOWED_RPC,
                    _ => ROLE_NOT_ALLOWED_STAFF,
                };
                answer.assert_refused(400, sentence, &label);
                self.tally.part_two.push(step.n);
                if step.status == Some(200) && action_of(step).as_deref() == Some("saveTransaction")
                {
                    self.save_behind(step).await;
                }
            }
            Rule::Retired => {
                // known difference (design §9 removes the action): v3 answers the retired name
                // as it answers a name v2 does not know
                let unknown = self.unknown_name_answer(step).await;
                assert_eq!(
                    (answer.status, answer.text.as_str()),
                    (unknown.status, unknown.text.as_str()),
                    "{label}: answered as a name v2 does not know (RE-7, design §9)"
                );
                assert_eq!(
                    answer.header("cache-control"),
                    unknown.header("cache-control"),
                    "{label}: Cache-Control"
                );
                self.tally.differences.push(step.n);
                // what v2's accepted save did, the v3 way: the rate is published, as the owner
                // sets a rate, so the steps after it (saves at that rate, snapshots showing it)
                // keep v2's meaning. The kiosk page's `configure` is published by
                // `kiosk_http_common`'s replay (#756 RE-4b).
                if step.status == Some(200) {
                    self.retired_behind(step).await;
                }
            }
            Rule::ResolvedAs(resolution) => {
                assert_eq!(step.status, Some(200), "{label}: v2 accepted this one");
                assert_ne!(
                    step.json()["requestResolution"],
                    json!(resolution),
                    "{label}: listed as a difference, but v2 says the same"
                );
                assert_eq!(answer.status, 200, "{label}: {:.300}", answer.text);
                let mut recorded = step.json().clone();
                recorded["requestResolution"] = json!(resolution);
                self.hold_json_to(step, &answer, &recorded).await;
                self.tally.differences.push(step.n);
            }
            Rule::Blueprint(refusal) => {
                answer.assert_refused(refusal.status(), &refusal.to_string(), &label);
                assert_eq!(answer.header("cache-control"), Some("no-store"), "{label}");
                self.tally.blueprint.push(step.n);
                // a kiosk step is applied behind the routes by the kiosk's replay, which holds the
                // kiosk's audit log
                if step.status == Some(200) && step.path() != Some("/api/kiosk") {
                    self.accepted_behind(step).await;
                }
            }
            Rule::Widened(v2_refusal) => {
                assert_ne!(
                    answer.status, 500,
                    "{label}: widened caller hit 500: {:.300}",
                    answer.text
                );
                let is_v2_role_refusal = answer.status == v2_refusal.status()
                    && serde_json::from_str::<Value>(&answer.text)
                        .ok()
                        .and_then(|j| j.get("error").and_then(Value::as_str).map(str::to_owned))
                        .as_deref()
                        == Some(&v2_refusal.to_string());
                assert!(
                    !is_v2_role_refusal,
                    "{label}: RBAC §11.2 widened caller was still refused with v2's role refusal ({}): {:.300}",
                    v2_refusal, answer.text
                );
                self.tally.widened.push(step.n);
            }
            Rule::OwnerNoOpenShift(reason) => {
                let expected_sentence = if step.path() == Some("/api/kiosk") {
                    "noOpenShift"
                } else {
                    "กรุณาเปิดกะจากหน้าแอดมินก่อน"
                };
                assert_ne!(
                    step.error(),
                    Some(expected_sentence),
                    "{label} ({reason}): listed in OWNER_SAVE_NO_OPEN_SHIFT, but v2 says the same"
                );
                answer.assert_refused(400, expected_sentence, &format!("{label} ({reason})"));
                assert_eq!(answer.header("cache-control"), Some("no-store"), "{label}");
                self.tally.owner_no_open_shift.push(step.n);
                if step.path() == Some("/api/kiosk") {
                    // KioskReplay::step (in kiosk_http_common) applies an accepted owner kiosk
                    // step behind the routes and records its audit line when `tally.blueprint`
                    // contains `step.n`.
                    self.tally.blueprint.push(step.n);
                }
                // When the owner retries the same submissionId (#10/#11 in transactions.json,
                // #120/#121/#122 in engine_actions.json), apply v2's accepted saves behind the
                // routes once the last step of that submissionId has been answered so `save_behind`
                // does not reassign the uploaded file or record the submission under FR1 before the
                // retry steps run.
                fn submission_id_of(s: &Step) -> Option<&str> {
                    s.payload
                        .as_ref()
                        .and_then(|p| p.get("submissionId"))
                        .and_then(Value::as_str)
                }
                if let Some(sub_id) = submission_id_of(step) {
                    let more_retries = self
                        .fixture
                        .steps()
                        .iter()
                        .any(|s| s.n > step.n && submission_id_of(s) == Some(sub_id));
                    if !more_retries {
                        let accepted: Vec<Step> = self
                            .fixture
                            .steps()
                            .iter()
                            .filter(|s| {
                                s.n <= step.n
                                    && submission_id_of(s) == Some(sub_id)
                                    && s.status == Some(200)
                                    && s.path() != Some("/api/kiosk")
                            })
                            .cloned()
                            .collect();
                        for s in &accepted {
                            self.accepted_behind(s).await;
                        }
                    }
                } else if step.status == Some(200) && step.path() != Some("/api/kiosk") {
                    self.accepted_behind(step).await;
                }
            }
            Rule::QueueRefuses(code) => {
                assert_eq!(answer.status, 400, "{label}: {:.300}", answer.text);
                assert_eq!(
                    answer.json()["error"],
                    json!(code),
                    "{label}: {:.300}",
                    answer.text
                );
                self.tally.differences.push(step.n);
            }
            Rule::RecordedWith(edit) => {
                assert_eq!(
                    Some(answer.status),
                    step.status,
                    "{label}: {:.300}",
                    answer.text
                );
                let mut expected = without_dropped_keys(step.json());
                edit(&mut expected);
                let actual = answer.json();
                self.ids.learn(&expected, &actual);
                assert_same(&self.ids.to_actual(&expected), &actual, &label);
                self.tally.blueprint.push(step.n);
            }
            Rule::Excluded(_) => unreachable!("handled above"),
        }
        self.answers.insert(step.n, answer);
    }

    /// Re-expected for RE-7 #604 by the ticket author: the answer to `step` sent again with
    /// [`NO_SUCH_NAME`] in place of the retired name, by the same caller at the same clock.
    async fn unknown_name_answer(&self, step: &Step) -> Answer {
        let payload = step.payload.as_ref().map(|p| self.ids.to_actual(p));
        let mut call = call_of(self.fixture, step, &self.cookies, payload.as_ref());
        let body = String::from_utf8(call.body.clone()).expect("a recorded body is text");
        // CA-10 #626: any retired name, not only RE-7's
        let retired = action_of(step).expect("a retired step names its action");
        assert!(
            body.contains(&format!("\"{retired}\"")),
            "{}: the body names {retired}",
            self.label(step)
        );
        call.body = body
            .replace(
                &format!("\"{retired}\""),
                &format!("\"{}\"", unknown_name_for(&retired)),
            )
            .into_bytes();
        self.app.send(call).await
    }

    /// Re-expected for RE-7 #604 by the ticket author: the rate an accepted v2 `adminSaveRate`
    /// saved, published in its branch (`publish_rate_per_one`).
    async fn publish_behind(&mut self, step: &Step) {
        let payload = self
            .ids
            .to_actual(step.payload.as_ref().expect("a direct save has a payload"));
        let published = publish_rate_per_one(
            &self.desk.db.pool,
            self.branch_of(step).as_str(),
            &text_of(&payload, "code"),
            &text_of(&payload, "buy"),
            &text_of(&payload, "sell"),
        )
        .await;
        // the publication is stamped with the step's instant, as v2 stamped the saved row: the
        // later steps that name the rate's time (`opsStartService`'s `rateUpdated`) still match
        sqlx::query("UPDATE rate_publications SET published_at = $1 WHERE id = $2::uuid")
            .bind(time(&step.at))
            .bind(&published.id)
            .execute(&self.desk.db.pool)
            .await
            .expect("stamp the publication with the step's instant");
        self.tally.applied_behind.push(step.n);
    }

    /// A recorded answer re-expected for #757 (RE-4c): `rates` as the branch's current
    /// publication shows them (`rates_re4c_common::as_published`). Only the v2 snapshot shapes
    /// (`adminSnapshot`, `adminDeskConfig`, the six write answers) are, and they name their
    /// branch; any other answer (the kiosk's `rates`, RE-4b's) is left as recorded. An
    /// `adminSaveRate` answer does not show the rate it saved (D-3, R10).
    async fn as_published(&self, step: &Step, actual: &Value, expected: &Value) -> Value {
        // re-expected for RE-7 #604 by the ticket author: what the direct save wrote is not
        // shown (R10, §4.1, the kiosk's `manage`), and no direct save answers any more
        // (`Rule::Retired`)
        let mut expected = without_direct_saves(expected);
        // the kiosk's `manage`: one publication holds every line, so a saved currency's
        // `updated` is the branch's current publication's `published_at` (ruling Q3 on #604);
        // a v3 `configure` publishes nothing, so this is not v2's newest save
        if let Some(Value::Array(currencies)) = expected.get_mut("currencies") {
            let published: Option<String> = sqlx::query_scalar(
                "SELECT to_char(p.published_at AT TIME ZONE 'UTC', \
                    'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') \
                 FROM branches b JOIN rate_publications p ON p.id = b.current_publication_id \
                 WHERE b.id = $1",
            )
            .bind(self.branch_of(step).as_str())
            .fetch_optional(&self.desk.db.pool)
            .await
            .expect("the branch's current publication");
            if let Some(published) = published {
                for entry in currencies.iter_mut() {
                    let saved =
                        matches!(entry.get("updated"), Some(Value::String(t)) if !t.is_empty());
                    if saved {
                        entry["updated"] = json!(published);
                    }
                }
            }
        }
        let Some(branch) = actual.get("branchId").and_then(Value::as_str) else {
            return expected;
        };
        rates_re4c_common::as_published(&self.desk.db.pool, branch, &expected, None).await
    }

    // Re-expected for RE-7 #604 by the ticket author: `set_the_bank` is gone. It queued the
    // Bank of Thailand answer v2 recorded for an `adminSaveRate`, and no direct save asks the
    // bank any more (`Rule::Retired`).

    /// Status, headers and body against what v2 answered.
    pub async fn hold_to_recorded(&mut self, step: &Step, answer: &Answer) {
        let label = self.label(step);
        assert_eq!(
            Some(answer.status),
            step.status,
            "{label}: {:.300}",
            answer.text
        );
        let recorded = self.fixture.response_headers(step);
        assert_eq!(
            answer.header("cache-control"),
            recorded.get("cache-control").map(String::as_str),
            "{label}: Cache-Control"
        );
        let content_type = answer.header("content-type").unwrap_or_default();
        match recorded.get("content-type").map(String::as_str) {
            Some("application/json") => assert_eq!(content_type, "application/json", "{label}"),
            // v3's plain 404 says `text/plain` without the charset v2 added
            Some(other) => assert!(
                content_type.starts_with(other.split(';').next().unwrap_or(other)),
                "{label}: content type {content_type}, v2 {other}"
            ),
            None => panic!("{label}: v2 recorded no content type"),
        }

        if let Some(text) = &step.text {
            assert_eq!(&answer.text, text, "{label}");
            return;
        }
        if let Some(sentence) = step.error() {
            answer.assert_refused(step.status.unwrap_or(400), sentence, &label);
            return;
        }
        self.hold_json_to(step, answer, step.json()).await;
    }

    /// The JSON of an accepted answer against `recorded` (v2's, or v2's re-expected): ids paired,
    /// the keys v3 leaves out dropped. (Split out of `hold_to_recorded` for RE-7 #604's
    /// `Rule::ResolvedAs` by the ticket author; what it holds is unchanged.)
    async fn hold_json_to(&mut self, step: &Step, answer: &Answer, recorded: &Value) {
        let label = self.label(step);
        // RF-5 (#877): a key v3 adds by decision (snapshot_common::V3_ADDED_KEYS), shape-checked, then left out.
        // The action is the one the route read, whatever form the body took (a byte-order mark, a body too
        // long to write down and recorded only by how it begins).
        let action = action_of(step).or_else(|| {
            let sent = step.raw.as_deref().or(step.raw_start.as_deref())?;
            let (_, rest) = sent.split_once("\"action\":\"")?;
            rest.split_once('"').map(|(action, _)| action.to_string())
        });
        let actual = crate::snapshot_common::without_v3_added_keys(
            step.route.as_deref(),
            action.as_deref(),
            &answer.json(),
            &label,
        );
        // amended for #757 (RE-4c): rates[] is the publication (D-3, §4.9 R10)
        let mut expected = self
            .as_published(step, &actual, &without_dropped_keys(recorded))
            .await;
        // CI-4 (#651): a recorded answer whose `customers` v3 answers company-wide (COMPANY_CUSTOMERS)
        if let Some(list) = crate::compat_common::company_customers(self.fixture, step.n) {
            expected = crate::compat_common::with_customers_of(
                &expected,
                &serde_json::json!({ "customers": list }),
                &format!(
                    "{label} ({})",
                    crate::compat_common::COMPANY_CUSTOMERS_REASON
                ),
            );
        }
        // CA-10 #626 (design §6, CA-7e #1911): a shift the replay opened behind the routes is a
        // Blueprint count, and a verified OPEN count leaves the branch day READY, where v2's
        // switched-off branch said CLOSED.
        if expected["operations"]["phase"] == json!("CLOSED")
            && actual["operations"]["phase"] == json!("READY")
            && let Some(branch) = actual["branchId"].as_str()
            && self.opened_by_count(branch).await
        {
            expected["operations"]["phase"] = json!("READY");
        }
        // CA-10 #626 (ruling A, option A, beside the CA-7e READY case above): v3 needs an OPEN day to save and v2 had
        // no day phase, so a day this replay opened for a save (`open_day_for_save`) reads OPEN where v2 said CLOSED.
        // Only `operations.phase`, and only for such a day.
        if expected["operations"]["phase"] == json!("CLOSED")
            && actual["operations"]["phase"] == json!("OPEN")
            && let Some(branch) = actual["branchId"].as_str()
            && self.day_opened_for_a_save(branch).await
        {
            expected["operations"]["phase"] = json!("OPEN");
        }
        // CA-10 #626: such a shift is the counting teller's, not the owner's who sent the direct
        // action, and a count closes it: who opened and closed it, its request ids, the figures the
        // close compares (`expected`, `difference`) and the close's note are the Blueprint's. Its
        // opening, its count, its times and its status stay held to v2.
        if let (Some(want), Some(got)) = (
            expected["stock"]["shifts"].as_array().cloned(),
            actual["stock"]["shifts"].as_array(),
        ) && want.len() == got.len()
        {
            for (i, shift) in got.iter().enumerate() {
                if let Some(id) = shift["id"].as_str()
                    && self.counted_open(id).await
                {
                    for key in [
                        "staff",
                        "openRequest",
                        "closeRequest",
                        "closedBy",
                        "expected",
                        "difference",
                        "note",
                    ] {
                        if shift.get(key).is_some() {
                            expected["stock"]["shifts"][i][key] = shift[key].clone();
                        }
                    }
                }
            }
        }
        // LG-6.r6 #1046 (controller ruling): on `$.stock.movements`, `staff` is the user id for v2's username and
        // `requestId` is `""` (CA-10 retired requests, §7); `id` pairs via `IdMap` and the rest stays held to v2.
        if let (Some(want), Some(got)) = (
            expected["stock"]["movements"].as_array().cloned(),
            actual["stock"]["movements"].as_array(),
        ) && want.len() == got.len()
        {
            for (i, mov) in got.iter().enumerate() {
                // LG-6.r6 #1046 (controller ruling): `staff` in v3 is the user id for v2's username in the replay's user map.
                if let (Some(want_staff), Some(got_staff)) =
                    (want[i]["staff"].as_str(), mov["staff"].as_str())
                    && crate::snapshot_common::movement_staff_matches(
                        &self.desk.db.pool,
                        self.fixture,
                        got_staff,
                        want_staff,
                    )
                    .await
                {
                    expected["stock"]["movements"][i]["staff"] = mov["staff"].clone();
                }
                // LG-6.r6 #1046 (controller ruling): `requestId` on `$.stock.movements` is `""` in v3 (CA-10 §7).
                if !want[i]["requestId"].as_str().unwrap_or_default().is_empty()
                    && mov["requestId"] == json!("")
                {
                    expected["stock"]["movements"][i]["requestId"] = json!("");
                }
            }
        }
        // CA-10 #626: a CLOSE count whose figures differ from the shift's records the difference as
        // an exception (CA-3), where v2's direct close kept none.
        if let (Some(want), Some(got)) = (
            expected["operations"]["exceptions"].as_array().cloned(),
            actual["operations"]["exceptions"].as_array(),
        ) && got.len() > want.len()
        {
            let mut wanted = want.into_iter();
            let mut merged = Vec::new();
            for item in got {
                let request = item["requestId"].as_str().unwrap_or_default();
                if self.close_count(request).await {
                    merged.push(item.clone());
                } else if let Some(next) = wanted.next() {
                    merged.push(next);
                }
            }
            merged.extend(wanted);
            expected["operations"]["exceptions"] = Value::Array(merged);
        }
        // LG-6.r5 (#1044, BE controller's rulings A, issuecomment-6245895868, and A″, issuecomment-6245971463):
        // v3 funds the opening from SAFE; v2 never posted SAFE for it, LG-6.r5. Once SAFE has funded an opening it
        // prints a map where v2 printed null. Only the two listed recordings, only from the listed step on, only
        // `$.operations.safe`; every other key and every other recording stays held to v2.
        if LG6_R5_SAFE_FUNDED_AN_OPENING
            .iter()
            .any(|(name, first)| *name == self.fixture.name.as_str() && step.n >= *first)
            && expected["operations"]["safe"].is_null()
            && actual["operations"]["safe"].is_object()
        {
            expected["operations"]["safe"] = actual["operations"]["safe"].clone();
        }
        self.ids.learn(&expected, &actual);
        assert_same(&self.ids.to_actual(&expected), &actual, &label);
    }

    /// CA-10 #626: whether `branch` has a verified OPEN count, i.e. a shift opened the Blueprint way
    /// (the replay opens one behind the routes for an accepted `adminOpenShift`).
    async fn opened_by_count(&self, branch: &str) -> bool {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM cash_counts \
             WHERE branch_id = $1 AND kind = 'OPEN' AND verified_at IS NOT NULL)",
        )
        .bind(branch)
        .fetch_one(&self.desk.db.pool)
        .await
        .expect("read the branch's counts")
    }

    /// CA-10 #626 (ruling A): the branch day OPEN for a replayed save (`shift_holder_common::open_day`), remembered
    /// per test schema in [`DAYS_OPENED_FOR_A_SAVE`].
    async fn open_day_for_save(&self, branch: &str) {
        shift_holder_common::open_day(&self.desk.db.pool, branch).await;
        let schema = self.schema().await;
        DAYS_OPENED_FOR_A_SAVE
            .lock()
            .expect("the opened days")
            .insert((schema, branch.to_owned()));
    }

    /// CA-10 #626 (ruling A): whether this replay opened `branch`'s day for a save.
    async fn day_opened_for_a_save(&self, branch: &str) -> bool {
        let schema = self.schema().await;
        DAYS_OPENED_FOR_A_SAVE
            .lock()
            .expect("the opened days")
            .contains(&(schema, branch.to_owned()))
    }

    /// The test's own schema (each test migrates a fresh one).
    async fn schema(&self) -> String {
        sqlx::query_scalar("SELECT current_schema()")
            .fetch_one(&self.desk.db.pool)
            .await
            .expect("the test schema")
    }

    /// CA-10 #626: whether `request` names a CLOSE count (its id or its idempotency key).
    async fn close_count(&self, request: &str) -> bool {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM cash_counts \
             WHERE kind = 'CLOSE' AND (id::text = $1 OR idempotency_key = $1))",
        )
        .bind(request)
        .fetch_one(&self.desk.db.pool)
        .await
        .expect("read the counts")
    }

    /// CA-10 #626: whether shift `id` was opened by a verified OPEN count.
    async fn counted_open(&self, id: &str) -> bool {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM cash_counts \
             WHERE shift_id = $1 AND kind = 'OPEN' AND verified_at IS NOT NULL)",
        )
        .bind(id)
        .fetch_one(&self.desk.db.pool)
        .await
        .expect("read the shift's counts")
    }

    /// CA-10 #626: of a recorded `{currency: amount}`, the currencies the register holds (v2 let an
    /// opening name any code, e.g. `XXX`; a Blueprint count counts the register's).
    async fn registered(&self, amounts: &Value) -> Value {
        let codes: Vec<String> = sqlx::query_scalar("SELECT code FROM currencies")
            .fetch_all(&self.desk.db.pool)
            .await
            .expect("read the register");
        let kept: serde_json::Map<String, Value> = amounts
            .as_object()
            .map(|all| {
                all.iter()
                    .filter(|(code, _)| codes.contains(code))
                    .map(|(code, amount)| (code.clone(), amount.clone()))
                    .collect()
            })
            .unwrap_or_default();
        Value::Object(kept)
    }

    /// CA-10 #626: the id of `branch`'s OPEN shift, if any.
    async fn open_shift(&self, branch: &str) -> Option<String> {
        sqlx::query_scalar(
            "SELECT id FROM shifts WHERE branch_id = $1 AND status = 'OPEN' \
             ORDER BY opened_at DESC LIMIT 1",
        )
        .bind(branch)
        .fetch_optional(&self.desk.db.pool)
        .await
        .expect("read the branch's shifts")
    }

    fn branch_of(&self, step: &Step) -> BranchId {
        let sender = self.fixture.sender(step).expect("somebody sent it");
        if let Some(own) = &sender.branch_id {
            return BranchId::new(own.clone());
        }
        let from_query = step
            .route
            .as_deref()
            .and_then(|r| r.split_once("?branch="))
            .map(|(_, branch)| branch.to_string());
        let named = step
            .payload
            .as_ref()
            .and_then(|p| p.get("branchId"))
            .and_then(Value::as_str)
            .map(str::to_string);
        BranchId::new(from_query.or(named).unwrap_or_else(|| MAIN.to_string()))
    }

    /// A recorded upload, stored through `FileService` for whoever sent it; the
    /// key v3 made is paired with the one v2 answered.
    async fn upload_behind(&mut self, step: &Step) {
        let sender = self.fixture.sender(step).expect("somebody uploaded it");
        let (mime, bytes) = match step.upload.as_deref() {
            Some("png") => ("image/png", PNG),
            Some("pdf") => ("application/pdf", PDF),
            other => panic!("{}: no file of kind {other:?}", self.label(step)),
        };
        let key = self
            .desk
            .files
            .upload(
                &self.branch_of(step),
                &sender.id,
                mime,
                bytes,
                time(&step.at),
            )
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", self.label(step)));
        self.ids.learn(step.json(), &json!({ "uploadKey": key }));
        self.tally.applied_behind.push(step.n);
    }

    /// A step v2 accepted and the Blueprint refuses its caller (OR-4b): what it changed, applied
    /// behind the routes as recorded — the exchange, the customer reservation, the stock move.
    async fn accepted_behind(&mut self, step: &Step) {
        match action_of(step).as_deref() {
            Some("saveTransaction") => self.save_behind(step).await,
            Some("reserveCustomer") => self.reserve_behind(step).await,
            other => panic!(
                "{}: v2 accepted {other:?}; nothing applies it behind the routes",
                self.label(step)
            ),
        }
    }

    /// An accepted `reserveCustomer`, through the customer service as the route runs it; the id
    /// v3 made is paired with the one v2 answered.
    async fn reserve_behind(&mut self, step: &Step) {
        let payload = self
            .ids
            .to_actual(step.payload.as_ref().expect("a payload"));
        let command = ReserveCustomer {
            name: text_of(&payload, "name"),
            passport_no: text_of(&payload, "passportNo"),
            nationality: text_of(&payload, "nationality"),
            birth_date: text_of(&payload, "birthDate"),
            passport_expiry: text_of(&payload, "passportExpiry"),
            mode: text_of(&payload, "mode"),
        };
        let reserved = CustomerService::new(self.desk.db.branch_tx())
            .reserve(&self.branch_of(step), command, time(&step.at))
            .await
            .unwrap_or_else(|e| panic!("{}: applied behind the routes: {e:?}", self.label(step)));
        let actual = json!({ "customerId": reserved.customer_id, "existing": reserved.existing });
        let expected_existing =
            if crate::compat_common::same_customer_as(&self.fixture.name, step.n).is_some() {
                serde_json::Value::Bool(true)
            } else {
                step.json()["existing"].clone()
            };
        assert_eq!(
            actual["existing"],
            expected_existing,
            "{}: the reservation behind the routes is v2's",
            self.label(step)
        );
        // CI-4 (#651), held strictly when the step is held to the Blueprint (OR-4b C2): the
        // reservation behind the routes is the earlier step's customer (controller ruling on #767)
        if let Some(same) = crate::compat_common::same_customer_as(&self.fixture.name, step.n) {
            let earlier = text_of(
                &self.ids.to_actual(self.fixture.step(same.same_as).json()),
                "customerId",
            );
            assert_eq!(
                reserved.customer_id,
                earlier,
                "{} ({}): the reservation behind the routes is step {}'s customer",
                self.label(step),
                same.reason,
                same.same_as
            );
        }
        self.ids.learn(step.json(), &actual);
        self.ids
            .alias_customer(&text_of(step.json(), "customerId"), &reserved.customer_id);
        self.tally.applied_behind.push(step.n);
    }

    /// Re-expected for CA-10 #626 by the ticket author (design §6): what an accepted retired step
    /// did, the v3 way, so the steps after it keep v2's meaning. RE-7's direct rate save is
    /// published; a direct shift open or close is a Blueprint count (`shift_setup_common`); a stock
    /// move is written as the move it was. An accepted `ops*` step applies nothing: the request
    /// model it drove is gone.
    async fn retired_behind(&mut self, step: &Step) {
        match action_of(step).as_deref() {
            Some(RETIRED) => self.publish_behind(step).await,
            Some("adminOpenShift") => self.open_behind(step).await,
            Some("adminCloseShift") => self.close_behind(step).await,
            Some("adminMoveStock") => self.move_behind(step).await,
            _ => {}
        }
    }

    /// An accepted `adminOpenShift`: the branch's shift opened by a verified OPEN count of the
    /// recorded opening.
    async fn open_behind(&mut self, step: &Step) {
        let payload = self
            .ids
            .to_actual(step.payload.as_ref().expect("a payload"));
        let branch = self.branch_of(step);
        // v2 answered a repeated request as it answered the first: nothing more to open
        if self.open_shift(branch.as_str()).await.is_some() {
            return;
        }
        let opening = self.registered(&payload["opening"]).await;
        shift_setup_common::open_shift_by_count(
            &self.desk.db.pool,
            branch.as_str(),
            opening,
            time(&step.at),
        )
        .await;
        self.tally.applied_behind.push(step.n);
    }

    /// An accepted `adminCloseShift`: the branch's open shift closed by a verified CLOSE count of
    /// the recorded `counted`.
    async fn close_behind(&mut self, step: &Step) {
        let payload = self
            .ids
            .to_actual(step.payload.as_ref().expect("a payload"));
        let branch = self.branch_of(step);
        // v2 answered a repeated request as it answered the first: nothing left to close
        if self.open_shift(branch.as_str()).await.is_none() {
            return;
        }
        let counted = self.registered(&payload["counted"]).await;
        shift_setup_common::close_shift_by_count(
            &self.desk.db.pool,
            branch.as_str(),
            counted,
            time(&step.at),
        )
        .await;
        self.tally.applied_behind.push(step.n);
    }

    /// An accepted `adminMoveStock`: the move written on the branch's open shift as a moved
    /// `cash_transfers` row (LG-6.r6 #1046 ruling Q1), plus the shift's expected balance and the
    /// branch version.
    async fn move_behind(&mut self, step: &Step) {
        let payload = self
            .ids
            .to_actual(step.payload.as_ref().expect("a payload"));
        let branch = self.branch_of(step);
        let label = self.label(step);
        let code = text_of(&payload, "currency");
        let direction = text_of(&payload, "direction");
        let amount = match &payload["amount"] {
            Value::String(text) => text.trim().to_owned(),
            other => other.to_string(),
        };
        let (kind, from_shift, to_shift) = if direction == "IN" {
            ("SAFE_TO_DRAWER", false, true)
        } else {
            ("DRAWER_TO_SAFE", true, false)
        };
        let actor = &self
            .fixture
            .sender(step)
            .expect("somebody moved it")
            .username;
        let at = time(&step.at);
        let mut tx = self.desk.db.pool.begin().await.expect("begin");
        let shift: String = sqlx::query_scalar(
            "SELECT id FROM shifts WHERE branch_id = $1 AND status = 'OPEN' \
             ORDER BY opened_at DESC LIMIT 1",
        )
        .bind(branch.as_str())
        .fetch_one(&mut *tx)
        .await
        .unwrap_or_else(|e| panic!("{label}: the branch's open shift: {e}"));
        let mover: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT organization_id, $2, 'PENDING', 'test' FROM branches WHERE id = $1 \
             RETURNING id::text",
        )
        .bind(branch.as_str())
        .bind(actor)
        .fetch_one(&mut *tx)
        .await
        .unwrap_or_else(|e| panic!("{label}: mover user: {e}"));
        let transfer: Option<String> = sqlx::query_scalar(
            "INSERT INTO cash_transfers (organization_id, kind, from_branch_id, from_shift_id, \
                 to_shift_id, status, created_by, dispatched_at, dispatched_by, received_at, \
                 received_by, note, idempotency_key, created_at, updated_at) \
             SELECT organization_id, $2, id, $3, $4, 'RECEIVED', $5::uuid, $6, $5::uuid, $6, \
                 $5::uuid, $7, $8, $6, $6 \
             FROM branches WHERE id = $1 \
             ON CONFLICT (from_branch_id, idempotency_key) DO NOTHING \
             RETURNING id::text",
        )
        .bind(branch.as_str())
        .bind(kind)
        .bind(from_shift.then_some(shift.as_str()))
        .bind(to_shift.then_some(shift.as_str()))
        .bind(&mover)
        .bind(at)
        .bind(text_of(&payload, "reason").trim())
        .bind(text_of(&payload, "requestId"))
        .fetch_optional(&mut *tx)
        .await
        .unwrap_or_else(|e| panic!("{label}: the move: {e}"));
        // v2 answered a repeated request as it answered the first: nothing more to move
        let Some(transfer) = transfer else {
            return;
        };
        let face: Option<(i64, String)> = sqlx::query_as(
            "SELECT id, face_value::text FROM denominations \
             WHERE currency_code = $1 AND face_value IS NOT NULL AND $2::numeric % face_value = 0 \
             ORDER BY face_value DESC LIMIT 1",
        )
        .bind(&code)
        .bind(&amount)
        .fetch_optional(&mut *tx)
        .await
        .unwrap_or_else(|e| panic!("{label}: face value: {e}"));
        let (denomination, face) = match face {
            Some(f) => f,
            None => sqlx::query_as(
                "INSERT INTO denominations (currency_code, face_value, group_code) \
                 VALUES ($1, 0.01, 'ALL') RETURNING id, face_value::text",
            )
            .bind(&code)
            .fetch_one(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("{label}: 0.01 face value: {e}")),
        };
        sqlx::query(
            "INSERT INTO cash_transfer_lines (transfer_id, organization_id, currency, \
                 denomination_id, qty_sent, qty_received) \
             SELECT t.id, t.organization_id, $2, $3, ($4::numeric / $5::numeric)::int, \
                 ($4::numeric / $5::numeric)::int \
             FROM cash_transfers t WHERE t.id = $1::uuid",
        )
        .bind(&transfer)
        .bind(&code)
        .bind(denomination)
        .bind(&amount)
        .bind(&face)
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|e| panic!("{label}: the moved line: {e}"));
        {
            use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
            use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
            let (from, to) = if direction == "IN" {
                (
                    CashPlace::InTransit(transfer.clone()),
                    CashPlace::Drawer(shift.clone()),
                )
            } else {
                (
                    CashPlace::Drawer(shift.clone()),
                    CashPlace::InTransit(transfer.clone()),
                )
            };
            xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
                .post_in(
                    &mut tx,
                    &[PostingRequest {
                        from,
                        to,
                        currency: xc_kernel::money::CurrencyCode::parse(&code)
                            .expect("move currency"),
                        quantity: xc_kernel::money::ForeignAmount::from_decimal(
                            amount.parse().expect("move amount"),
                        ),
                        source: SourceRef {
                            source_type: SourceType::Transfer,
                            source_id: transfer.clone(),
                            source_key: code.clone(),
                        },
                        line: None,
                        reverses: None,
                    }],
                    None,
                )
                .await
                .unwrap_or_else(|e| panic!("{label}: the shift's balance: {e:?}"));
        }
        sqlx::query(
            "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 1) \
             ON CONFLICT (branch_id) DO UPDATE SET version = branch_versions.version + 1",
        )
        .bind(branch.as_str())
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|e| panic!("{label}: the branch version: {e}"));
        tx.commit().await.expect("commit the move");
        self.tally.applied_behind.push(step.n);
    }

    /// An accepted `saveTransaction`, applied through the save workflow as
    /// part two's arm will: `staff` is the caller's username.
    async fn save_behind(&mut self, step: &Step) {
        let sender = self.fixture.sender(step).expect("somebody saved it");
        let branch = self.branch_of(step);
        let saver =
            shift_holder_common::teller_of_open_shift(&self.desk.db.pool, branch.as_str()).await;
        let payload = self
            .ids
            .to_actual(step.payload.as_ref().expect("a payload"));
        let key_of = |file: &str| {
            payload
                .get(file)
                .and_then(|f| f.get("uploadKey"))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let passport = key_of("passport");
        let slip = key_of("slip");
        for key in [&passport, &slip].into_iter().flatten() {
            sqlx::query("UPDATE files SET owner_id = $1 WHERE key = $2")
                .bind(&saver)
                .bind(key)
                .execute(&self.desk.db.pool)
                .await
                .expect("assign the save's file to its saver");
        }
        let command = SaveTransaction {
            customer: ReserveCustomer {
                name: text_of(&payload, "name"),
                passport_no: text_of(&payload, "passportNo"),
                nationality: text_of(&payload, "nationality"),
                birth_date: text_of(&payload, "birthDate"),
                passport_expiry: text_of(&payload, "passportExpiry"),
                mode: text_of(&payload, "mode"),
            },
            submission_id: text_of(&payload, "submissionId"),
            customer_id: text_of(&payload, "customerId"),
            side: text_of(&payload, "type"),
            currency: text_of(&payload, "currency"),
            amount: text_of(&payload, "amount"),
            rate: text_of(&payload, "rate"),
            fee: text_of(&payload, "fee"),
            bank_fee: text_of(&payload, "bankFee"),
            method: text_of(&payload, "method"),
            payment_ref: text_of(&payload, "ref"),
            account: text_of(&payload, "account"),
            staff: sender.username.clone(),
            booking: text_of(&payload, "booking"),
            notes: text_of(&payload, "notes"),
            value_date: text_of(&payload, "valueDate"),
            paid_at: text_of(&payload, "paidAt"),
            payment_confirmed: payload["paymentConfirmed"] == json!(true),
            identity_confirmed: payload["identityConfirmed"] == json!(true),
            passport: passport.clone(),
            slip: slip.clone(),
            reference_rate: payload
                .get("referenceRate")
                .filter(|v| !v.is_null())
                .map(|_| text_of(&payload, "referenceRate")),
        };
        let saved = self
            .desk
            .workflow
            .save(&branch, command, &saver, time(&step.at))
            .await
            .unwrap_or_else(|e| panic!("{}: applied behind the routes: {e:?}", self.label(step)));
        for key in [&passport, &slip].into_iter().flatten() {
            sqlx::query("UPDATE files SET owner_id = $1 WHERE key = $2")
                .bind(&sender.id)
                .bind(key)
                .execute(&self.desk.db.pool)
                .await
                .expect("restore the save's file to its uploader");
        }
        assert_eq!(
            json!(saved.document_ref),
            step.json()["documentRef"],
            "{}: the exchange saved behind the routes is v2's",
            self.label(step)
        );
        self.tally.applied_behind.push(step.n);
    }

    pub async fn finish(self) {
        self.desk.finish().await;
    }
}

/// `text` with every id v2 made replaced by the one v3 made for it.
pub fn text_with_ids(ids: &IdMap, recorded: &Value, text: &str) -> String {
    let mut out = text.to_string();
    for id in crate::compat_common::server_ids(recorded) {
        if let Some(actual) = ids.actual(&id) {
            out = out.replace(&id, actual);
        }
    }
    out
}

/// LG-6.r5 (#1044): the source ids of the just-in-time postings below.
static JIT: AtomicU64 = AtomicU64::new(0);

async fn jit_post(
    pool: &sqlx::PgPool,
    from: xc_cash::domain::posting::CashPlace,
    to: xc_cash::domain::posting::CashPlace,
    amounts: &[(xc_kernel::money::CurrencyCode, xc_kernel::money::Money)],
) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPostings, PostingRequest};
    let mut tx = pool.begin().await.expect("begin");
    for (code, amount) in amounts {
        let n = JIT.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[PostingRequest {
                    from: from.clone(),
                    to: to.clone(),
                    currency: *code,
                    quantity: xc_kernel::money::ForeignAmount::from_decimal(
                        amount.to_string().parse().unwrap(),
                    ),
                    source: SourceRef {
                        source_type: SourceType::Opening,
                        source_id: format!("jit:{nanos}:{n}"),
                        source_key: "jit".into(),
                    },
                    line: None,
                    reverses: None,
                }],
                None,
            )
            .await
            .expect("jit post");
    }
    tx.commit().await.expect("commit");
}

/// LG-6.r5 (#1044): moves `amounts` from BANK into `branch`'s ledger SAFE (`OPENING`).
pub async fn fund_exact(
    pool: &sqlx::PgPool,
    branch: &BranchId,
    amounts: &[(xc_kernel::money::CurrencyCode, xc_kernel::money::Money)],
) {
    use xc_cash::domain::posting::CashPlace;
    jit_post(
        pool,
        CashPlace::Bank,
        CashPlace::Safe(branch.clone()),
        amounts,
    )
    .await;
}

/// LG-6.r5 (#1044): moves whatever `branch`'s ledger SAFE holds back to BANK (`OPENING`), so it holds nothing.
pub async fn sweep_safe(pool: &sqlx::PgPool, branch: &BranchId) {
    use xc_cash::domain::posting::{CashPlace, CashPostings};
    let mut tx = pool.begin().await.expect("begin");
    let held = xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .expected_in(&mut tx, &CashPlace::Safe(branch.clone()))
        .await
        .expect("SAFE balance");
    tx.rollback().await.expect("rollback");
    let left = held.non_zero();
    if !left.is_empty() {
        jit_post(
            pool,
            CashPlace::Safe(branch.clone()),
            CashPlace::Bank,
            &left,
        )
        .await;
    }
}
