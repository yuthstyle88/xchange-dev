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
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.
#![allow(dead_code)]

use std::collections::BTreeMap;
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
use xc_auth::domain::account::value_objects::password_hasher::PasswordHasher;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_auth::infrastructure::account::account_repository::PgAccountRepository;
use xc_exchange::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_kernel::ids::BranchId;
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;
use xchange::application::workflows::save_transaction::SaveTransactionWorkflow;
use xchange::bootstrap::AppState;
use xchange::infrastructure::files::file_service::FileService;
use xchange::presentation::http::rpc::RpcClock;

use crate::common::settings;
use crate::compat_common::{Fixture, IdMap, Person, Step};
use crate::snapshot_common::{TestDb, assert_same, recorded_reference, time, without_dropped_keys};

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

pub mod gap_custody;
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
    /// Anything starting `ops`.
    Custody,
    /// `saveTransaction`.
    Save,
}

impl Family {
    pub const ALL: [Family; 3] = [Family::Documents, Family::Custody, Family::Save];

    /// The family `action` belongs to; `None` for a name of part one and for a
    /// name nobody knows.
    pub fn of(action: &str) -> Option<Family> {
        if action == "saveTransaction" {
            Some(Family::Save)
        } else if action == "documents" {
            Some(Family::Documents)
        } else if action.starts_with("ops") {
            Some(Family::Custody)
        } else {
            None
        }
    }

    /// Whether the family's ticket says its file answers its names.
    pub fn wired(self) -> bool {
        match self {
            Family::Documents => gap_documents::WIRED,
            Family::Custody => gap_custody::WIRED,
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
    scratch: PathBuf,
}

impl Desk {
    /// A fresh schema with `api/migrations/` applied (so `MAIN` exists) and the
    /// owner `owner`, as after `xchange create-owner --username owner`.
    pub async fn open() -> Desk {
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
            Arc::new(PgShiftRepository::new(db.pool.clone())),
            db.branch_tx(),
        );
        let workflow = SaveTransactionWorkflow::new(db.branch_tx(), shifts, files.clone());
        Desk {
            db,
            clock: Arc::new(TestClock(Mutex::new(time("2026-10-06T05:00:00.000Z")))),
            bank: Arc::new(Bank::default()),
            files,
            workflow,
            scratch,
        }
    }

    /// The server's state, its upload directory the one [`Desk::files`] keeps
    /// files in: a save through the routes finds the files uploaded behind
    /// them (P2-S).
    pub fn state(&self) -> AppState {
        let uploads = self.scratch.join("uploads");
        let uploads = uploads.to_str().expect("a scratch path in UTF-8");
        AppState::new(
            Arc::new(settings(&[("APP_FILES__DIR", uploads)])),
            self.db.pool.clone(),
        )
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

    /// Switch custody on for `branch` behind the routes (the owner's door to it
    /// is `/api/employees`, ticket T4c).
    pub async fn custody_on(&self, branch: &str) {
        sqlx::query(
            "INSERT INTO ops_state (branch_id, enabled, phase) VALUES ($1, true, 'CLOSED') \
             ON CONFLICT (branch_id) DO UPDATE SET enabled = true",
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
#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    /// Sent through HTTP and held to what v2 answered.
    Replay,
    /// Sent through HTTP and held to what was decided instead of v2's answer:
    /// this status and sentence.
    Differs { status: u16, sentence: &'static str },
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
}

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
}

impl Tally {
    pub fn total(&self) -> usize {
        self.replayed.len() + self.differences.len() + self.part_two.len() + self.excluded.len()
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

impl<'f> Replay<'f> {
    /// A desk with the people of `fixture` employed and signed in.
    pub async fn start(desk: Desk, app: App, fixture: &'f Fixture) -> Replay<'f> {
        desk.employ(&fixture.people).await;
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
            "/api/upload" | "/api/staff/upload" => {
                return Rule::Excluded("an upload: the file routes are another ticket's");
            }
            // any other path is sent as it was and held to what v2 answered
            _ => return Rule::Replay,
        };
        let Some(action) = action_of(step) else {
            return Rule::Replay;
        };
        if !is_part_two(&action) {
            return Rule::Replay;
        }
        // a part-two name is this part's to answer exactly when Policy refuses
        // it, or when the route refuses the request before any name is judged
        let judged_by_name = step.method.as_deref() == Some("POST")
            && step.request_headers.is_empty()
            && step.payload.as_ref().is_none_or(Value::is_object);
        let actor = actor_of(self.fixture, step);
        let allowed = Policy::allow(actor.as_ref(), entry, &Action::new(action)).is_ok();
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
        self.set_the_bank(step);
        let payload = step.payload.as_ref().map(|p| self.ids.to_actual(p));
        let call = call_of(self.fixture, step, &self.cookies, payload.as_ref());
        let answer = self.app.send(call).await;
        let label = self.label(step);

        match rule {
            Rule::Replay => {
                self.hold_to_recorded(step, &answer);
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
                if expected.get("syncedAt").is_some() {
                    expected["syncedAt"] = json!(step.at);
                }
                assert_eq!(answer.status, 200, "{label}: {:.300}", answer.text);
                let actual = answer.json();
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
            Rule::Excluded(_) => unreachable!("handled above"),
        }
        self.answers.insert(step.n, answer);
    }

    /// What the Bank of Thailand answers during this step: the reference v2
    /// recorded in the answer of an accepted `adminSaveRate`; no answer where
    /// v2 recorded that the bank gave none; otherwise a day with no row.
    fn set_the_bank(&self, step: &Step) {
        if step.action.as_deref() != Some("adminSaveRate") {
            return;
        }
        if step.error() == Some(BANK_DOWN) {
            self.desk.bank.go_down();
        } else if step.status == Some(200) {
            let code = text_of(step.payload.as_ref().expect("a payload"), "code");
            self.desk.bank.answer_next(recorded_reference(step, &code));
        } else {
            // refused for another reason: the bank was up
            self.desk.bank.answer_nothing_special();
        }
    }

    /// Status, headers and body against what v2 answered.
    pub fn hold_to_recorded(&mut self, step: &Step, answer: &Answer) {
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
        let actual = answer.json();
        let expected = without_dropped_keys(step.json());
        self.ids.learn(&expected, &actual);
        assert_same(&self.ids.to_actual(&expected), &actual, &label);
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

    /// An accepted `saveTransaction`, applied through the save workflow as
    /// part two's arm will: `staff` is the caller's username.
    async fn save_behind(&mut self, step: &Step) {
        let sender = self.fixture.sender(step).expect("somebody saved it");
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
            passport: key_of("passport"),
            slip: key_of("slip"),
            reference_rate: payload
                .get("referenceRate")
                .filter(|v| !v.is_null())
                .map(|_| text_of(&payload, "referenceRate")),
        };
        let saved = self
            .desk
            .workflow
            .save(&self.branch_of(step), command, &sender.id, time(&step.at))
            .await
            .unwrap_or_else(|e| panic!("{}: applied behind the routes: {e:?}", self.label(step)));
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
