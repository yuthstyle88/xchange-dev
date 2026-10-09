//! Written before the implementation (ticket T11b). Do not edit, delete or weaken.
//!
//! The harness of `snapshot_test.rs`: a database per test, the services of
//! every context the snapshot reads, the two things a test controls (the
//! reference-rate source and the hand-made `SnapshotSources`), and the replay
//! of a recorded v2 scenario through those services.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing
//! database whose name ends in `_test`. When the variable is absent every test
//! FAILS; none of them skips. Each `TestDb` is a schema of its own with
//! `api/migrations/` applied.
//!
//! Nothing here goes through HTTP or a session: a recorded step is applied by
//! calling the service the dispatcher will call, with the step's `at` as `now`.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, NaiveDate, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use serde_json::{Map, Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use tokio::sync::Semaphore;
use xc_exchange::application::customer::service::CustomerService;
use xc_exchange::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::customer::value_objects::customer_details::CustomerDetails;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta, Rate, Side};
use xc_rate::application::branch::service::BranchService;
use xc_rate::application::rate::service::RateService;
use xc_rate::domain::branch::commands::create_branch::CreateBranch;
use xc_rate::domain::rate::commands::save_rate::SaveRate;
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xc_shift::application::custody::service::CustodyService;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::custody::entities::attendance::Attendance;
use xc_shift::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use xc_shift::domain::custody::entities::custody_request::{CustodyRequest, Decision};
use xc_shift::domain::custody::entities::customer_hold::{CustomerHold, HoldStatus};
use xc_shift::domain::custody::entities::stored_event::StoredEvent;
use xc_shift::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
use xc_shift::domain::custody::repositories::custody_ports::{CustodyPorts, OutsideFacts};
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::cash_target::CashTarget;
use xc_shift::domain::custody::value_objects::count_sheet::CountSheet;
use xc_shift::domain::custody::value_objects::custody_event::EventKind;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::cutoff::Cutoff;
use xc_shift::domain::custody::value_objects::denomination_row::DenominationRow;
use xc_shift::domain::custody::value_objects::entity::Entity;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::domain::custody::value_objects::request_kind::RequestKind;
use xc_shift::domain::custody::value_objects::request_status::{
    Reconciliation, RequestStatus, TransferStage,
};
use xc_shift::domain::custody::value_objects::signature::Signature;
use xc_shift::domain::shift::commands::close_shift::CloseShift;
use xc_shift::domain::shift::commands::move_stock::MoveStock;
use xc_shift::domain::shift::commands::open_shift::OpenShift;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;
use xchange::application::compat::abandon_request::AbandonRequest;
use xchange::application::compat::snapshot::{SnapshotBuilder, write_answer};
use xchange::application::compat::snapshot_sources::{
    MemberMark, NothingYet, SnapshotSources, SnapshotTransaction, SourceFuture,
};
use xchange::application::compat::v2_json::{People, PersonLabel};

use crate::compat_common::{Fixture, IdMap, Step};

/// Upper bound for anything that must finish.
pub const MUST_FINISH: Duration = Duration::from_secs(30);

pub const MAIN: &str = "MAIN";
pub const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";

/// The six actions that answer with the snapshot.
pub const WRITE_ACTIONS: [&str; 6] = [
    "adminSaveRate",
    "adminCreateBranch",
    "adminOpenShift",
    "adminMoveStock",
    "adminCloseShift",
    "adminAbandonRequest",
];

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A name no other test, test binary or run on this server has used.
pub fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

pub fn branch(id: &str) -> BranchId {
    BranchId::new(id)
}

/// A recorded time (`2026-10-06T05:00:00.000Z`).
pub fn time(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("{text} is not a time: {e}"))
        .with_timezone(&Utc)
}

/// The sentence of a refusal. Panics on success and on any error that is not
/// `AppError::Domain`.
pub fn refusal<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

// ── comparing two answers ────────────────────────────────────────────────────

/// Fails with the first place where the two differ: a 30 KB answer printed
/// twice tells nobody anything.
pub fn assert_same(expected: &Value, actual: &Value, label: &str) {
    if let Some(difference) = first_difference(expected, actual, "$") {
        panic!("{label}: {difference}");
    }
}

fn first_difference(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for key in e.keys() {
                if !a.contains_key(key) {
                    return Some(format!("{path}.{key} is missing; expected {}", e[key]));
                }
            }
            for key in a.keys() {
                if !e.contains_key(key) {
                    return Some(format!(
                        "{path}.{key} is not in v2's answer; got {}",
                        a[key]
                    ));
                }
            }
            e.iter()
                .find_map(|(key, value)| first_difference(value, &a[key], &format!("{path}.{key}")))
        }
        (Value::Array(e), Value::Array(a)) => {
            if e.len() != a.len() {
                return Some(format!(
                    "{path} has {} items, expected {}",
                    a.len(),
                    e.len()
                ));
            }
            e.iter()
                .zip(a)
                .enumerate()
                .find_map(|(i, (e, a))| first_difference(e, a, &format!("{path}[{i}]")))
        }
        (e, a) if e == a => None,
        (e, a) => Some(format!("{path}: expected {e}, got {a}")),
    }
}

/// A recorded answer without the keys v3 leaves out by decision: `row`,
/// `startRow`, `endRow` of a shift and `row`, `rawStatus` of a transaction.
/// Only those places are touched — a `row` anywhere else would still be compared.
pub fn without_dropped_keys(recorded: &Value) -> Value {
    let mut value = recorded.clone();
    let strip = |rows: Option<&mut Value>, keys: &[&str]| {
        if let Some(Value::Array(rows)) = rows {
            for row in rows {
                if let Value::Object(map) = row {
                    for key in keys {
                        map.remove(*key);
                    }
                }
            }
        }
    };
    strip(value.get_mut("transactions"), &["row", "rawStatus"]);
    strip(
        value.get_mut("stock").and_then(|s| s.get_mut("shifts")),
        &["row", "startRow", "endRow"],
    );
    value
}

/// `value` without the top-level `keys`.
pub fn without_keys(value: &Value, keys: &[&str]) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !keys.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<Map<String, Value>>(),
        ),
        other => other.clone(),
    }
}

// ── the database ─────────────────────────────────────────────────────────────

pub struct TestDb {
    pub pool: PgPool,
    schema: String,
}

/// Schemas being migrated at once; every one opens its own pool.
static STARTING: Semaphore = Semaphore::const_new(4);

impl TestDb {
    /// A pool confined to a fresh schema with `api/migrations/` applied.
    pub async fn start() -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The snapshot tests need a real PostgreSQL and do not skip. Point it at an \
                 existing database whose name ends in `_test`, e.g.\n\
                 TEST_DATABASE_URL=postgres://postgres@localhost:5432/xchange_test"
            )
        });
        let options = PgConnectOptions::from_str(&url)
            .expect("TEST_DATABASE_URL is not a PostgreSQL connection URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "TEST_DATABASE_URL names the database '{database}', which does not end in `_test`. \
             Use a dedicated test database."
        );

        let schema = unique("t11b");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database — check TEST_DATABASE_URL");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");

        for opener in ["owner-1", "test"] {
            seed_opener(&pool, opener).await;
        }

        Self { pool, schema }
    }

    pub fn branch_tx(&self) -> BranchTx {
        BranchTx::new(Transactor::new(self.pool.clone()))
    }

    /// Add a branch behind the service's back (`MAIN` is there already).
    pub async fn seed_branch(&self, id: &str, name: &str) {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $2)")
            .bind(id)
            .bind(name)
            .execute(&self.pool)
            .await
            .expect("insert into branches");
    }

    /// Drop the schema. Reached only by a test that passed; a failed test
    /// leaves its schema behind for inspection.
    pub async fn finish(self) {
        self.pool.close().await;
        let url = std::env::var("TEST_DATABASE_URL").expect("checked in start");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("reconnect to drop the test schema");
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop the test schema");
        admin.close().await;
    }
}

// ── what a test controls ─────────────────────────────────────────────────────

/// A reference-rate source that answers what the test last gave it: the
/// `botReference` v2 recorded for the rate being saved.
#[derive(Default)]
pub struct RecordedSource {
    next: Mutex<Option<ReferenceRates>>,
}

impl RecordedSource {
    pub fn answer_next(&self, rates: ReferenceRates) {
        *self.next.lock().expect("the source's lock") = Some(rates);
    }
}

impl RateSource for RecordedSource {
    fn fetch(&self) -> BoxFuture<'_, AppResult<ReferenceRates>> {
        Box::pin(async move {
            self.next
                .lock()
                .expect("the source's lock")
                .clone()
                .ok_or_else(|| AppError::Domain("the test gave the source nothing".to_string()))
        })
    }
}

/// The custody ports. Reading the state and the log asks none of them.
pub struct NoPorts;

fn never_asked<T>() -> AppResult<T> {
    Err(AppError::Domain(
        "the snapshot must not ask the custody ports".to_string(),
    ))
}

impl CustodyPorts for NoPorts {
    fn outside_facts<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _open_shift_id: Option<&'a str>,
    ) -> BoxFuture<'a, AppResult<OutsideFacts>> {
        Box::pin(async { never_asked() })
    }

    fn customer_known<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async { never_asked() })
    }

    fn evidence_owned<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _owner_id: &'a str,
        _keys: &'a [String],
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async { never_asked() })
    }

    fn employee_role<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _employee_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<CustodyRole>>> {
        Box::pin(async { never_asked() })
    }
}

/// A `SnapshotSources` holding exactly what the test put in, per branch.
#[derive(Default)]
pub struct HandSources {
    transactions: Mutex<Vec<(String, SnapshotTransaction)>>,
    members: Mutex<Vec<(String, MemberMark)>>,
    people: Mutex<BTreeMap<String, People>>,
}

impl HandSources {
    pub fn add_transaction(&self, branch: &BranchId, transaction: SnapshotTransaction) {
        self.transactions
            .lock()
            .expect("lock")
            .push((branch.as_str().to_string(), transaction));
    }

    pub fn add_member(&self, branch: &BranchId, mark: MemberMark) {
        self.members
            .lock()
            .expect("lock")
            .push((branch.as_str().to_string(), mark));
    }

    pub fn set_people(&self, branch: &BranchId, people: People) {
        self.people
            .lock()
            .expect("lock")
            .insert(branch.as_str().to_string(), people);
    }

    pub fn has_transaction(&self, document_ref: &str) -> bool {
        self.transactions
            .lock()
            .expect("lock")
            .iter()
            .any(|(_, t)| t.document_ref == document_ref)
    }
}

impl SnapshotSources for HandSources {
    fn transactions<'a>(
        &'a self,
        branch: &'a BranchId,
    ) -> SourceFuture<'a, Vec<SnapshotTransaction>> {
        Box::pin(async move {
            Ok(self
                .transactions
                .lock()
                .expect("lock")
                .iter()
                .filter(|(b, _)| b == branch.as_str())
                .map(|(_, t)| t.clone())
                .collect())
        })
    }

    fn members<'a>(&'a self, branch: &'a BranchId) -> SourceFuture<'a, Vec<MemberMark>> {
        Box::pin(async move {
            Ok(self
                .members
                .lock()
                .expect("lock")
                .iter()
                .filter(|(b, _)| b == branch.as_str())
                .map(|(_, m)| m.clone())
                .collect())
        })
    }

    fn people<'a>(&'a self, branch: &'a BranchId) -> SourceFuture<'a, People> {
        Box::pin(async move {
            Ok(self
                .people
                .lock()
                .expect("lock")
                .get(branch.as_str())
                .cloned()
                .unwrap_or_default())
        })
    }
}

// ── the shop: every service the snapshot reads ───────────────────────────────

pub struct Shop {
    pub db: TestDb,
    pub branches: BranchService,
    pub rates: RateService,
    pub customers: CustomerService,
    pub shifts: ShiftService,
    pub custody: CustodyService,
    pub custody_repo: Arc<PgCustodyRepository>,
    pub source: Arc<RecordedSource>,
    pub hand: Arc<HandSources>,
}

impl Shop {
    pub async fn open() -> Shop {
        let db = TestDb::start().await;
        let source = Arc::new(RecordedSource::default());
        let shifts = ShiftService::new(
            Arc::new(PgShiftRepository::new(db.pool.clone())),
            db.branch_tx(),
        );
        let custody_repo = Arc::new(PgCustodyRepository::new(db.pool.clone()));
        let custody = CustodyService::new(
            custody_repo.clone(),
            shifts.clone(),
            Arc::new(NoPorts),
            db.branch_tx(),
        );
        Shop {
            branches: BranchService::new(db.branch_tx()),
            rates: RateService::new(db.branch_tx(), source.clone()),
            customers: CustomerService::new(db.branch_tx()),
            shifts,
            custody,
            custody_repo,
            source,
            hand: Arc::new(HandSources::default()),
            db,
        }
    }

    fn builder_with(&self, sources: Arc<dyn SnapshotSources>) -> SnapshotBuilder {
        SnapshotBuilder::new(
            self.branches.clone(),
            self.rates.clone(),
            self.customers.clone(),
            self.shifts.clone(),
            self.custody.clone(),
            sources,
        )
    }

    /// A builder that reads the hand-made sources of this shop.
    pub fn builder(&self) -> SnapshotBuilder {
        self.builder_with(self.hand.clone())
    }

    /// A builder with the stub: nothing has landed.
    pub fn builder_with_nothing_yet(&self) -> SnapshotBuilder {
        self.builder_with(Arc::new(NothingYet))
    }

    pub async fn finish(self) {
        self.db.finish().await;
    }
}

// ── reading a recorded step ──────────────────────────────────────────────────

fn text_of(value: &Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("`{key}` is not text in {value}"))
        .to_string()
}

fn optional_text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// A payload field v2 took as a number or as text, as the text the service takes.
fn number_text(value: &Value, key: &str) -> Option<String> {
    match value.get(key) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

fn array<'a>(value: &'a Value, key: &str) -> &'a Vec<Value> {
    value[key]
        .as_array()
        .unwrap_or_else(|| panic!("`{key}` is not an array in {value}"))
}

/// The branch a step is answered for: an employee's own whatever they name,
/// the owner's `payload.branchId`, `MAIN` when none is named.
pub fn branch_of(fixture: &Fixture, step: &Step) -> BranchId {
    let sender = fixture
        .sender(step)
        .unwrap_or_else(|| panic!("{}#{}: nobody sent it", fixture.name, step.n));
    if let Some(own) = &sender.branch_id {
        return branch(own);
    }
    let named = step
        .payload
        .as_ref()
        .and_then(|p| p.get("branchId"))
        .and_then(Value::as_str)
        .unwrap_or(MAIN);
    branch(named)
}

/// The username a step is recorded under: v2's `actor`.
pub fn actor_of(fixture: &Fixture, step: &Step) -> String {
    fixture
        .sender(step)
        .unwrap_or_else(|| panic!("{}#{}: nobody sent it", fixture.name, step.n))
        .username
        .clone()
}

/// The reference rates the Bank of Thailand "answered" when the rate of `code`
/// was saved: the `botReference` of that rate in the step's own answer.
pub fn recorded_reference(step: &Step, code: &str) -> ReferenceRates {
    let rate = array(step.json(), "rates")
        .iter()
        .find(|r| r["code"] == code)
        .unwrap_or_else(|| panic!("step {}: no rate {code} in the answer", step.n));
    let reference = &rate["botReference"];
    ReferenceRates {
        source: text_of(reference, "source"),
        fetched_at: text_of(reference, "fetchedAt"),
        kind: text_of(reference, "kind"),
        rows: array(reference, "rows").clone(),
    }
}

fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).unwrap_or_else(|_| panic!("{text} is not a register code"))
}

/// A recorded JSON number as hundredths.
fn minor_of(number: &Value) -> i128 {
    let n = number
        .as_f64()
        .unwrap_or_else(|| panic!("{number} is not a number"));
    (n * 100.0).round() as i128
}

fn money_of(number: &Value) -> Money {
    let minor = i64::try_from(minor_of(number)).expect("a recorded amount fits");
    Money::from_minor(minor).unwrap_or_else(|e| panic!("{number}: {e}"))
}

fn rate_of(number: &Value) -> Rate {
    Rate::parse(&number.to_string()).unwrap_or_else(|e| panic!("{number} is not a rate: {e}"))
}

/// One element of a recorded `transactions` as the plain data a source hands
/// over. `method` is given apart: the snapshot shows `Bank Transfer` where the
/// stored method is the old `SCB Transfer`.
pub fn transaction_of(recorded: &Value, method: &str) -> SnapshotTransaction {
    assert_eq!(
        recorded["status"], "COMPLETED",
        "v3 stores saved exchanges only"
    );
    SnapshotTransaction {
        document_ref: text_of(recorded, "id"),
        recorded_at: time(&text_of(recorded, "created")),
        value_date: NaiveDate::parse_from_str(&text_of(recorded, "date"), "%Y-%m-%d")
            .expect("a recorded value date"),
        side: match text_of(recorded, "type").as_str() {
            "BUY" => Side::Buy,
            "SELL" => Side::Sell,
            other => panic!("no side {other}"),
        },
        currency: text_of(recorded, "currency"),
        amount: money_of(&recorded["amount"]),
        rate: rate_of(&recorded["rate"]),
        reference_rate: recorded
            .get("referenceRate")
            .filter(|v| !v.is_null())
            .map(rate_of),
        fee: money_of(&recorded["fee"]),
        bank_fee: money_of(&recorded["bankFee"]),
        method: method.to_string(),
        slip_key: optional_text(recorded, "slipUrl").map(|url| {
            url.strip_prefix("/files/")
                .unwrap_or_else(|| panic!("{url} is not a file url"))
                .to_string()
        }),
        payment_ref: text_of(recorded, "paymentRef"),
        customer_id: text_of(recorded, "customerId"),
        customer_name: text_of(recorded, "customer"),
        staff: text_of(recorded, "staff"),
        notes: text_of(recorded, "notes"),
        shift_id: optional_text(recorded, "shiftId"),
    }
}

/// The people of a fixture by employee id, as the accounts would name them.
pub fn people_of(fixture: &Fixture) -> People {
    fixture
        .people
        .iter()
        .map(|p| {
            (
                p.id.clone(),
                PersonLabel {
                    name: p.name.clone(),
                    role: p.role.clone(),
                },
            )
        })
        .collect()
}

fn reserve_command(payload: &Value) -> ReserveCustomer {
    ReserveCustomer {
        name: text_of(payload, "name"),
        passport_no: text_of(payload, "passportNo"),
        nationality: text_of(payload, "nationality"),
        birth_date: text_of(payload, "birthDate"),
        passport_expiry: text_of(payload, "passportExpiry"),
        mode: text_of(payload, "mode"),
    }
}

// ── replaying a recorded scenario ────────────────────────────────────────────

/// What a replay compared, so a test can say how much it is supposed to be.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Compared {
    /// `adminSnapshot` answers of `/api/rpc`.
    pub snapshots: usize,
    /// `adminDeskConfig` answers of `/api/rpc`.
    pub desks: usize,
    /// Answers of the six write actions (either route).
    pub writes: usize,
    /// `reserveCustomer` answers.
    pub reservations: usize,
    /// `saveTransaction` steps fed into the hand-made sources.
    pub saves_fed: usize,
}

/// Replays the steps of one recorded file against a fresh shop.
pub struct Replay<'f> {
    pub shop: Shop,
    pub fixture: &'f Fixture,
    pub ids: IdMap,
    pub compared: Compared,
    /// The recorded `transactions` a `saveTransaction` step is fed from: those
    /// of the view the test will compare. Empty: saves are not fed.
    pub recorded_transactions: Vec<Value>,
}

impl<'f> Replay<'f> {
    pub async fn start(fixture: &'f Fixture) -> Replay<'f> {
        Replay {
            shop: Shop::open().await,
            fixture,
            ids: IdMap::new(),
            compared: Compared::default(),
            recorded_transactions: Vec::new(),
        }
    }

    fn label(&self, step: &Step) -> String {
        format!(
            "{}#{} ({} as {})",
            self.fixture.name,
            step.n,
            step.action.as_deref().unwrap_or("-"),
            step.sent_as
        )
    }

    /// Every step in the order recorded.
    pub async fn run(&mut self) {
        let fixture = self.fixture;
        for step in fixture.steps() {
            self.step(step).await;
        }
    }

    /// Steps `from..=to`, counted from 1.
    pub async fn run_range(&mut self, from: usize, to: usize) {
        let fixture = self.fixture;
        for n in from..=to {
            self.step(fixture.step(n)).await;
        }
    }

    /// Applies one step, or compares what it read. A step this harness has no
    /// rule for fails the test: nothing is skipped by accident.
    pub async fn step(&mut self, step: &Step) {
        if !step.is_request() {
            return self.generator_step(step);
        }
        let path = step.path().unwrap_or_default().to_string();
        let on_rpc = path == "/api/rpc";
        let on_staff_rpc = path == "/api/staff/rpc";
        if !on_rpc && !on_staff_rpc {
            // uploads, the employees door, the documents page: other tickets'
            assert!(
                matches!(
                    path.as_str(),
                    "/api/upload" | "/api/staff/upload" | "/documents" | "/api/employees"
                ),
                "{}: no rule for the route {path}",
                self.label(step)
            );
            return;
        }
        let action = step.action.clone().unwrap_or_default();
        if step.status != Some(200) {
            // a refusal changed nothing; the ones this ticket owns have tests of their own
            return;
        }
        match action.as_str() {
            "adminSnapshot" if on_rpc => self.compare_snapshot(step).await,
            "adminDeskConfig" if on_rpc => self.compare_desk(step).await,
            // the staff path's view of the two reads is T11s's
            "adminSnapshot" | "adminDeskConfig" => {}
            "adminSaveRate" => {
                self.save_rate(step).await;
                self.compare_write(step, None).await;
            }
            "adminCreateBranch" => {
                let created = self.create_branch(step).await;
                self.compare_write(step, Some(created)).await;
            }
            "adminOpenShift" => {
                let cmd = OpenShift::from_payload(&self.payload(step)).expect("a recorded opening");
                seed_opener(&self.shop.db.pool, &actor_of(self.fixture, step)).await;
                self.shop
                    .shifts
                    .open(
                        &branch_of(self.fixture, step),
                        cmd,
                        &actor_of(self.fixture, step),
                        time(&step.at),
                    )
                    .await
                    .unwrap_or_else(|e| panic!("{}: {e:?}", self.label(step)));
                self.compare_write(step, None).await;
            }
            "adminMoveStock" => {
                let cmd =
                    MoveStock::from_payload(&self.payload(step)).expect("a recorded movement");
                self.shop
                    .shifts
                    .move_stock(
                        &branch_of(self.fixture, step),
                        cmd,
                        &actor_of(self.fixture, step),
                        time(&step.at),
                    )
                    .await
                    .unwrap_or_else(|e| panic!("{}: {e:?}", self.label(step)));
                self.compare_write(step, None).await;
            }
            "adminCloseShift" => {
                let cmd = CloseShift::from_payload(&self.payload(step)).expect("a recorded close");
                self.shop
                    .shifts
                    .close(
                        &branch_of(self.fixture, step),
                        cmd,
                        &actor_of(self.fixture, step),
                        time(&step.at),
                    )
                    .await
                    .unwrap_or_else(|e| panic!("{}: {e:?}", self.label(step)));
                self.compare_write(step, None).await;
            }
            "adminAbandonRequest" => {
                let resolution = AbandonRequest::new(self.shop.db.branch_tx())
                    .abandon(
                        &branch_of(self.fixture, step),
                        &self.payload(step),
                        &actor_of(self.fixture, step),
                        time(&step.at),
                    )
                    .await
                    .unwrap_or_else(|e| panic!("{}: {e:?}", self.label(step)));
                assert_eq!(
                    step.json()["requestResolution"],
                    resolution.as_str(),
                    "{}",
                    self.label(step)
                );
                self.compare_write(step, None).await;
            }
            "reserveCustomer" => self.reserve(step).await,
            "saveTransaction" => self.feed_save(step).await,
            "getConnectionStatus" | "documents" => {}
            other if other.starts_with("ops") => {}
            other => panic!("{}: no rule for the action {other}", self.label(step)),
        }
    }

    /// The step's payload with the ids v2 made replaced by the ones v3 made.
    fn payload(&self, step: &Step) -> Value {
        self.ids
            .to_actual(step.payload.as_ref().unwrap_or(&Value::Null))
    }

    fn generator_step(&mut self, step: &Step) {
        // "a members row for customer CX-00000002, verified_at 2026-10-06T04:00:00.000Z"
        let sql = step.sql.as_deref().unwrap_or_default();
        let rest = sql
            .strip_prefix("a members row for customer ")
            .unwrap_or_else(|| panic!("{}: no rule for the row `{sql}`", self.label(step)));
        let (customer_id, verified) = rest
            .split_once(", verified_at ")
            .unwrap_or_else(|| panic!("{}: `{sql}`", self.label(step)));
        self.shop.hand.add_member(
            &branch(MAIN),
            MemberMark {
                customer_id: customer_id.to_string(),
                verified_at: Some(time(verified)),
            },
        );
    }

    async fn save_rate(&mut self, step: &Step) {
        let payload = self.payload(step);
        let code = text_of(&payload, "code");
        self.shop
            .source
            .answer_next(recorded_reference(step, &code));
        let cmd = SaveRate {
            request_id: text_of(&payload, "requestId"),
            code,
            buy: number_text(&payload, "buy").expect("buy"),
            sell: number_text(&payload, "sell").expect("sell"),
            partner_name: optional_text(&payload, "partnerName").unwrap_or_default(),
            partner_cost: number_text(&payload, "partnerCost"),
            previous_updated: text_of(&payload, "previousUpdated"),
        };
        self.shop
            .rates
            .save_rate(
                &branch_of(self.fixture, step),
                cmd,
                &actor_of(self.fixture, step),
                time(&step.at),
            )
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", self.label(step)));
    }

    async fn create_branch(&mut self, step: &Step) -> BranchId {
        let payload = self.payload(step);
        let cmd = CreateBranch {
            request_id: text_of(&payload, "requestId"),
            name: text_of(&payload, "name"),
            from_branch: branch_of(self.fixture, step),
        };
        self.shop
            .branches
            .create(cmd, &actor_of(self.fixture, step), time(&step.at))
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", self.label(step)))
            .id
    }

    async fn reserve(&mut self, step: &Step) {
        let reservation = self
            .shop
            .customers
            .reserve(
                &branch_of(self.fixture, step),
                reserve_command(&self.payload(step)),
                time(&step.at),
            )
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", self.label(step)));
        assert_same(
            step.json(),
            &json!({"customerId": reservation.customer_id, "existing": reservation.existing}),
            &self.label(step),
        );
        self.compared.reservations += 1;
    }

    /// What the save workflow will do that a snapshot shows: the customer's
    /// details are confirmed, and the exchange is listed. The exchange itself is
    /// fed from the recorded view — the save is not on main.
    async fn feed_save(&mut self, step: &Step) {
        if self.recorded_transactions.is_empty() {
            return;
        }
        let payload = self.payload(step);
        let document_ref = text_of(step.json(), "documentRef");
        if self.shop.hand.has_transaction(&document_ref) {
            return; // the same submission again
        }
        let here = branch_of(self.fixture, step);
        let details = CustomerDetails::parse(&reserve_command(&payload)).expect("recorded details");
        let customer_id = text_of(&payload, "customerId");
        let (b, id, reference) = (here.clone(), customer_id, document_ref.clone());
        self.shop
            .customers
            .branch_tx()
            .run(&here, move |tx| {
                Box::pin(async move {
                    CustomerService::record_exchange_in(tx, &b, &id, &details, &reference).await
                })
            })
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", self.label(step)));

        let recorded = self
            .recorded_transactions
            .iter()
            .find(|t| t["id"] == document_ref.as_str());
        // a DEMO save is in no view: nothing to list
        if let Some(recorded) = recorded {
            let recorded = self.ids.to_actual(recorded);
            self.shop.hand.add_transaction(
                &here,
                transaction_of(&recorded, &text_of(&payload, "method")),
            );
            self.compared.saves_fed += 1;
        }
    }

    async fn compare_snapshot(&mut self, step: &Step) {
        let actual = self
            .shop
            .builder()
            .admin_snapshot(
                &branch_of(self.fixture, step),
                &actor_of(self.fixture, step),
                time(&step.at),
            )
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", self.label(step)))
            .to_value();
        let recorded = without_dropped_keys(step.json());
        self.ids.learn(&recorded, &actual);
        assert_same(&self.ids.to_actual(&recorded), &actual, &self.label(step));
        self.compared.snapshots += 1;
    }

    async fn compare_desk(&mut self, step: &Step) {
        let actual = self
            .shop
            .builder()
            .desk_config(&branch_of(self.fixture, step))
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", self.label(step)))
            .to_value();
        self.ids.learn(step.json(), &actual);
        assert_same(&self.ids.to_actual(step.json()), &actual, &self.label(step));
        self.compared.desks += 1;
    }

    /// The answer of a write: the snapshot of the branch written to (of the new
    /// branch after `adminCreateBranch`) as `write_answer` cuts it.
    ///
    /// Two things in a recorded answer are not the builder's and are taken out
    /// of both sides: `requestResolution` (the dispatcher adds what the abandon
    /// workflow answered) and, on the staff path, the two links the staff view
    /// rewrites.
    async fn compare_write(&mut self, step: &Step, answered_for: Option<BranchId>) {
        let here = answered_for.unwrap_or_else(|| branch_of(self.fixture, step));
        let snapshot = self
            .shop
            .builder()
            .admin_snapshot(&here, &actor_of(self.fixture, step), time(&step.at))
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", self.label(step)));
        let mut actual = write_answer(&snapshot).to_value();
        let mut recorded = without_keys(&without_dropped_keys(step.json()), &["requestResolution"]);
        if step.path() == Some("/api/staff/rpc") {
            recorded = without_keys(&recorded, &["sheetUrl", "deskUrl"]);
            actual = without_keys(&actual, &["sheetUrl", "deskUrl"]);
        }
        self.ids.learn(&recorded, &actual);
        assert_same(&self.ids.to_actual(&recorded), &actual, &self.label(step));
        self.compared.writes += 1;
    }

    // ── feeding what a recorded view shows, where no step can be replayed ────

    /// Makes the shifts and movements a recorded view shows, through the shift
    /// service, at the recorded times and by the recorded people. What exchanges
    /// moved is applied as one delta per shift (`expected − opening −
    /// movements`), as the save workflow will through `apply_delta_in`.
    /// Answers recorded shift id → the id v3 made.
    pub async fn feed_stock(&mut self, here: &BranchId, view: &Value) -> BTreeMap<String, String> {
        let mut made = BTreeMap::new();
        let movements = array(&view["stock"], "movements");
        for shift in array(&view["stock"], "shifts") {
            let recorded_id = text_of(shift, "id");
            seed_opener(&self.shop.db.pool, &text_of(shift, "staff")).await;
            let opened = self
                .shop
                .shifts
                .open(
                    here,
                    OpenShift::from_payload(&json!({
                        "requestId": shift["openRequest"],
                        "opening": shift["opening"],
                    }))
                    .expect("a recorded opening"),
                    &text_of(shift, "staff"),
                    time(&text_of(shift, "openedAt")),
                )
                .await
                .expect("open the recorded shift");
            made.insert(recorded_id.clone(), opened.id.clone());

            let mut moved: BTreeMap<String, i128> = BTreeMap::new();
            for movement in movements
                .iter()
                .filter(|m| m["shiftId"] == recorded_id.as_str())
            {
                let sign = if movement["direction"] == "IN" { 1 } else { -1 };
                *moved.entry(text_of(movement, "currency")).or_default() +=
                    sign * minor_of(&movement["amount"]);
                self.shop
                    .shifts
                    .move_stock(
                        here,
                        MoveStock::from_payload(&json!({
                            "requestId": movement["requestId"],
                            "shiftId": opened.id,
                            "currency": movement["currency"],
                            "direction": movement["direction"],
                            "amount": movement["amount"],
                            "reason": movement["reason"],
                        }))
                        .expect("a recorded movement"),
                        &text_of(movement, "staff"),
                        time(&text_of(movement, "created")),
                    )
                    .await
                    .expect("make the recorded movement");
            }

            if shift["status"] != "CLOSED" {
                continue;
            }
            let exchanged: Vec<(CurrencyCode, MoneyDelta)> = CurrencyCode::all()
                .iter()
                .map(|c| {
                    let key = c.as_str();
                    let rest = minor_of(&shift["expected"][key])
                        - minor_of(&shift["opening"][key])
                        - moved.get(key).copied().unwrap_or(0);
                    (*c, MoneyDelta::from_minor(rest))
                })
                .filter(|(_, delta)| !delta.is_zero())
                .collect();
            if !exchanged.is_empty() {
                let (shifts, b) = (self.shop.shifts.clone(), here.clone());
                let delta = StockDelta::of(&exchanged);
                self.shop
                    .shifts
                    .branch_tx()
                    .run(here, move |tx| {
                        Box::pin(async move { shifts.apply_delta_in(tx, &b, &delta).await })
                    })
                    .await
                    .expect("apply what the exchanges moved");
            }
            self.shop
                .shifts
                .close(
                    here,
                    CloseShift::from_payload(&json!({
                        "requestId": shift["closeRequest"],
                        "shiftId": opened.id,
                        "expected": shift["expected"],
                        "counted": shift["counted"],
                        "note": shift["note"],
                    }))
                    .expect("a recorded close"),
                    &text_of(shift, "closedBy"),
                    time(&text_of(shift, "closedAt")),
                )
                .await
                .expect("close the recorded shift");
        }
        made
    }

    /// Stores a recorded `operations` as the custody state and log of `here`,
    /// through the custody repository — what the custody service reads back.
    pub async fn feed_custody(&mut self, here: &BranchId, operations: &Value) {
        let state = state_of(operations);
        let events = events_of(operations);
        let (repo, b) = (self.shop.custody_repo.clone(), here.clone());
        self.shop
            .db
            .branch_tx()
            .run(here, move |tx| {
                Box::pin(async move {
                    // what the branch holds now is what the store compares with
                    let before = repo.load_state_in(tx, &b).await?;
                    repo.store_state_in(tx, &b, &before, &state).await?;
                    repo.append_events_in(tx, &b, &events).await
                })
            })
            .await
            .expect("store the recorded custody state");
    }
}

/// `value` with every key of `ids` replaced by its value, inside strings too.
pub fn with_ids(value: &Value, ids: &BTreeMap<String, String>) -> Value {
    let mut text = value.to_string();
    for (recorded, actual) in ids {
        text = text.replace(recorded.as_str(), actual.as_str());
    }
    serde_json::from_str(&text).expect("still JSON")
}

// ── a recorded `operations` as the state the custody service hands over ──────
//
// The same reading `compat_v2_json_test.rs` does; a test file cannot use
// another's functions.

fn custody_at(text: &str) -> CustodyTime {
    CustodyTime::from_epoch_millis(time(text).timestamp_millis())
}

fn optional_custody_time(value: &Value, key: &str) -> Option<CustodyTime> {
    value.get(key).and_then(Value::as_str).map(custody_at)
}

fn balances_of(object: &Value) -> Balances {
    let map = object
        .as_object()
        .unwrap_or_else(|| panic!("{object} is not an object"));
    assert_eq!(map.len(), 153, "v2 prints every register code");
    let pairs: Vec<(CurrencyCode, Money)> =
        map.iter().map(|(c, n)| (code(c), money_of(n))).collect();
    Balances::of(&pairs)
}

fn optional_balances(value: &Value, key: &str) -> Option<Balances> {
    value.get(key).filter(|v| !v.is_null()).map(balances_of)
}

fn delta_of(object: &Value) -> StockDelta {
    let map = object
        .as_object()
        .unwrap_or_else(|| panic!("{object} is not an object"));
    assert_eq!(map.len(), 153, "v2 prints every register code");
    let pairs: Vec<(CurrencyCode, MoneyDelta)> = map
        .iter()
        .map(|(c, n)| (code(c), MoneyDelta::from_minor(minor_of(n))))
        .collect();
    StockDelta::of(&pairs)
}

fn optional_delta(value: &Value, key: &str) -> Option<StockDelta> {
    value.get(key).filter(|v| !v.is_null()).map(delta_of)
}

fn role_of(text: &str) -> CustodyRole {
    match text {
        "OWNER" => CustodyRole::Owner,
        "FRONT" => CustodyRole::Front,
        "FINANCE" => CustodyRole::Finance,
        "MONITOR" => CustodyRole::Monitor,
        "MANAGER" => CustodyRole::Manager,
        "BACKEND" => CustodyRole::Backend,
        _ => CustodyRole::Other,
    }
}

fn phase_of(text: &str) -> Phase {
    [
        Phase::Closed,
        Phase::Ready,
        Phase::Open,
        Phase::CutOff,
        Phase::Counting,
    ]
    .into_iter()
    .find(|p| p.as_str() == text)
    .unwrap_or_else(|| panic!("no phase {text}"))
}

fn kind_of(text: &str) -> RequestKind {
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
    .unwrap_or_else(|| panic!("no request kind {text}"))
}

fn event_kind_of(text: &str) -> EventKind {
    [
        EventKind::SetEntity,
        EventKind::Enable,
        EventKind::ClockIn,
        EventKind::ClockOut,
        EventKind::CashTarget,
        EventKind::ServiceOpen,
        EventKind::CutOff,
        EventKind::Propose,
        EventKind::HoldCustomer,
        EventKind::ReleaseCustomer,
        EventKind::Reject,
        EventKind::Sign,
        EventKind::Dispatch,
        EventKind::Commit,
        EventKind::Reconcile,
    ]
    .into_iter()
    .find(|k| k.as_str() == text)
    .unwrap_or_else(|| panic!("no event kind {text}"))
}

fn target_of(value: &Value) -> CashTarget {
    CashTarget {
        amounts: balances_of(&value["amounts"]),
        by: text_of(value, "by"),
        at: custody_at(&text_of(value, "at")),
    }
}

fn cutoff_of(value: &Value) -> Cutoff {
    Cutoff {
        at: custody_at(&text_of(value, "at")),
        by: text_of(value, "by"),
        shift_id: text_of(value, "shiftId"),
        last_ref: optional_text(value, "lastRef"),
        pending: array(value, "pending")
            .iter()
            .map(|p| p.as_str().expect("a pending ref is text").to_string())
            .collect(),
    }
}

fn sheet_of(request: &Value, totals: &str, rows: &str) -> Option<CountSheet> {
    let totals = optional_balances(request, totals)?;
    let denominations = array(request, rows)
        .iter()
        .map(|row| DenominationRow {
            currency: code(&text_of(row, "currency")),
            denomination: money_of(&row["denomination"]),
            count: u32::try_from(row["count"].as_u64().expect("a count")).expect("a small count"),
        })
        .collect();
    Some(CountSheet {
        totals,
        denominations,
    })
}

fn decision_of(value: &Value, text_key: &str) -> Decision {
    Decision {
        by: text_of(value, "by"),
        at: custody_at(&text_of(value, "at")),
        text: text_of(value, text_key),
    }
}

fn request_of(r: &Value) -> CustodyRequest {
    CustodyRequest {
        id: text_of(r, "id"),
        request_id: text_of(r, "requestId"),
        payload_hash: text_of(r, "hash"),
        kind: kind_of(&text_of(r, "kind")),
        status: match text_of(r, "status").as_str() {
            "PENDING" => RequestStatus::Pending,
            "REJECTED" => RequestStatus::Rejected,
            "COMPLETED" => RequestStatus::Completed,
            other => panic!("no request status {other}"),
        },
        at: custody_at(&text_of(r, "at")),
        sender: text_of(r, "sender"),
        receiver: optional_text(r, "receiver"),
        note: text_of(r, "note"),
        evidence_keys: array(r, "evidenceKeys")
            .iter()
            .map(|k| k.as_str().expect("a key is text").to_string())
            .collect(),
        signatures: array(r, "signatures")
            .iter()
            .map(|s| Signature {
                employee_id: text_of(s, "employeeId"),
                role: role_of(&text_of(s, "role")),
                at: custody_at(&text_of(s, "at")),
            })
            .collect(),
        shift_id: optional_text(r, "shiftId"),
        counted: sheet_of(r, "counted", "denominations"),
        target: r.get("target").map(target_of),
        target_difference: optional_delta(r, "targetDifference"),
        expected: optional_balances(r, "expected"),
        variance: optional_delta(r, "variance"),
        cutoff: r.get("cutoff").map(cutoff_of),
        safe_count: sheet_of(r, "safeCount", "safeDenominations"),
        safe_expected: optional_balances(r, "safeExpected"),
        safe_variance: optional_delta(r, "safeVariance"),
        currency: optional_text(r, "currency").map(|c| code(&c)),
        amount: r.get("amount").map(money_of),
        stage: optional_text(r, "stage").map(|s| match s.as_str() {
            "IN_TRANSIT" => TransferStage::InTransit,
            "RECEIVED" => TransferStage::Received,
            other => panic!("no stage {other}"),
        }),
        dispatched_at: optional_custody_time(r, "dispatchedAt"),
        dispatched_by: optional_text(r, "dispatchedBy"),
        rejected: r.get("rejected").map(|d| decision_of(d, "reason")),
        completed_at: optional_custody_time(r, "completedAt"),
        reconciliation: optional_text(r, "reconciliation").map(|s| match s.as_str() {
            "PENDING" => Reconciliation::Pending,
            "CHECKED" => Reconciliation::Checked,
            other => panic!("no reconciliation {other}"),
        }),
        reconciled: r.get("reconciled").map(|d| decision_of(d, "note")),
    }
}

pub fn state_of(ops: &Value) -> CustodyState {
    CustodyState {
        enabled: ops["enabled"].as_bool().expect("enabled"),
        phase: phase_of(&text_of(ops, "phase")),
        safe: optional_balances(ops, "safe"),
        target: ops.get("target").filter(|v| !v.is_null()).map(target_of),
        entity: ops.get("entity").filter(|v| !v.is_null()).map(|e| Entity {
            company: text_of(e, "company"),
            site: text_of(e, "site"),
            confirmed_by: text_of(e, "confirmedBy"),
            at: custody_at(&text_of(e, "at")),
        }),
        custodian: optional_text(ops, "custodian"),
        attendance: array(ops, "attendance")
            .iter()
            .map(|a| Attendance {
                id: text_of(a, "id"),
                employee_id: text_of(a, "employeeId"),
                role: role_of(&text_of(a, "role")),
                clock_in: custody_at(&text_of(a, "in")),
                clock_out: optional_custody_time(a, "out"),
            })
            .collect(),
        requests: array(ops, "requests").iter().map(request_of).collect(),
        cutoff: ops.get("cutoff").filter(|v| !v.is_null()).map(cutoff_of),
        exceptions: array(ops, "exceptions")
            .iter()
            .map(|x| CashException {
                id: text_of(x, "id"),
                request_id: text_of(x, "requestId"),
                status: match text_of(x, "status").as_str() {
                    "OPEN" => ExceptionStatus::Open,
                    "REVIEWED" => ExceptionStatus::Reviewed,
                    other => panic!("no exception status {other}"),
                },
                target_difference: optional_delta(x, "targetDifference"),
                cash_variance: optional_delta(x, "cashVariance"),
                safe_variance: optional_delta(x, "safeVariance"),
                note: text_of(x, "note"),
                at: custody_at(&text_of(x, "at")),
                reviewed_by: optional_text(x, "reviewedBy"),
                reviewed_at: optional_custody_time(x, "reviewedAt"),
            })
            .collect(),
        holds: array(ops, "holds")
            .iter()
            .map(|h| CustomerHold {
                id: text_of(h, "id"),
                customer_id: text_of(h, "customerId"),
                reason: text_of(h, "reason"),
                by: text_of(h, "by"),
                at: custody_at(&text_of(h, "at")),
                status: match text_of(h, "status").as_str() {
                    "OPEN" => HoldStatus::Open,
                    "RELEASED" => HoldStatus::Released,
                    other => panic!("no hold status {other}"),
                },
                reviewed_by: optional_text(h, "reviewedBy"),
                reviewed_at: optional_custody_time(h, "reviewedAt"),
                note: optional_text(h, "note"),
            })
            .collect(),
        drafts: array(ops, "drafts")
            .iter()
            .map(|d| TxDraft {
                id: text_of(d, "id"),
                shift_id: text_of(d, "shiftId"),
                employee_id: text_of(d, "employeeId"),
                side: match text_of(d, "type").as_str() {
                    "BUY" => Side::Buy,
                    "SELL" => Side::Sell,
                    other => panic!("no side {other}"),
                },
                currency: code(&text_of(d, "currency")),
                amount: money_of(&d["amount"]),
                at: custody_at(&text_of(d, "at")),
                status: match text_of(d, "status").as_str() {
                    "OPEN" => DraftStatus::Open,
                    "COMPLETED" => DraftStatus::Completed,
                    other => panic!("no draft status {other}"),
                },
                document_ref: optional_text(d, "documentRef"),
            })
            .collect(),
        service_opened_at: optional_custody_time(ops, "serviceOpenedAt"),
    }
}

pub fn events_of(ops: &Value) -> Vec<StoredEvent> {
    array(ops, "events")
        .iter()
        .map(|e| StoredEvent {
            id: text_of(e, "id"),
            kind: event_kind_of(&text_of(e, "type")),
            at: time(&text_of(e, "at")),
            actor: text_of(e, "actor"),
            role: role_of(&text_of(e, "role")),
            details: e["details"].clone(),
        })
        .collect()
}

/// CA-1 (#617): a shift's opener must name a user (`shifts.teller_id`, design #582 §8 M1 item 3). The user an
/// opener id resolves to: `users.legacy_employee_id = id`, or, for `"owner"`, the organisation-wide EXECUTIVE +
/// ADMIN user. Seeded once per id; no test's expectation depends on it. Added by the ticket author in CA-1's
/// up-front commit.
pub async fn seed_opener(pool: &PgPool, id: &str) {
    if id == "owner" {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users u WHERE u.legacy_employee_id IS NULL AND EXISTS \
             (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL))",
        )
        .fetch_one(pool)
        .await
        .expect("look for the owner's user");
        if !exists {
            sqlx::query(
                "WITH o AS (SELECT id FROM organizations WHERE code = 'XC'), \
                 u AS (INSERT INTO users (organization_id, name, status, created_by) \
                       SELECT id, 'owner', 'INVITED', 'test' FROM o RETURNING id, organization_id) \
                 INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
                 SELECT u.id, r.code, u.organization_id, NULL FROM u, (VALUES ('EXECUTIVE'), ('ADMIN')) AS r(code)",
            )
            .execute(pool)
            .await
            .expect("seed the owner's user");
        }
        return;
    }
    sqlx::query(
        "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
         SELECT id, $1, 'INVITED', 'test', $1 FROM organizations WHERE code = 'XC' \
         ON CONFLICT (legacy_employee_id) DO NOTHING",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("seed an opener's user");
}
