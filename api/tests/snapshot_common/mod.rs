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
//!
//! Re-expected for RE-7 #604 by the ticket author (design `docs/design/blueprint/rates.md` §8
//! migration 6, §9): the direct save and the `rates` table are gone. A recorded `adminSaveRate`
//! is a known difference (design §9 removes the action): its answer is not compared and its rate
//! is published instead (`publish_rate_per_one`, stamped with the step's instant), so later
//! steps see it; a recorded snapshot is held to `rateHistory: []` and no partner, with `updated`
//! the current publication's `published_at` (`without_direct_saves`, rates.md R10, §4.1; ruling
//! Q3 on #604).
#![allow(dead_code)]

#[path = "../rates_re4c_common/mod.rs"]
pub mod rates_re4c_common;

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
use xc_cash::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_customer::domain::customer::value_objects::customer_details::CustomerDetails;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta, Rate, Side, UnitSize};
use xc_rate::application::branch::service::BranchService;
use xc_rate::application::rate::service::RateService;
use xc_rate::domain::branch::commands::create_branch::CreateBranch;
use xc_rate::domain::rate::entities::rate_source::{RateSource, ReferenceRates};
use xc_shift::application::custody::service::{CustodyFactsPort, CustodyService};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::custody::entities::attendance::Attendance;
use xc_shift::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use xc_shift::domain::custody::entities::customer_hold::{CustomerHold, HoldStatus};
use xc_shift::domain::custody::entities::stored_event::StoredEvent;
use xc_shift::domain::custody::entities::tx_draft::{DraftStatus, TxDraft};
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::cash_target::CashTarget;
use xc_shift::domain::custody::value_objects::custody_event::EventKind;
use xc_shift::domain::custody::value_objects::custody_role::CustodyRole;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::custody::value_objects::cutoff::Cutoff;
use xc_shift::domain::custody::value_objects::entity::Entity;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::domain::custody::value_objects::service_window::ServiceWindow;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xchange::application::compat::abandon_request::AbandonRequest;
use xchange::application::compat::snapshot::{SnapshotBuilder, write_answer};
use xchange::application::compat::snapshot_sources::{
    MemberMark, NothingYet, SnapshotSources, SnapshotTransaction, SourceFuture,
};
use xchange::application::compat::v2_json::People;

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

/// Re-expected for RE-7 #604 by the ticket author: a recorded v2 answer (a snapshot, a write
/// answer cut from one, a desk config, the kiosk's `manage`) as v3 answers it once the direct
/// save and the `rates` table are gone. A known difference (design §9 removes the action,
/// rates.md R10, §4.1):
///
/// - `rateHistory` was the branch's `rates` rows, newest first, and is `[]`. The top-level
///   `updated` is the current publication's `published_at` (ruling Q3 on #604), which is the
///   recorded value wherever a replay publishes the step's rate at the step's time, so it is left
///   as recorded;
/// - a rate's `partnerName` / `partnerCost` were what v2's direct save wrote; v3 reads the partner
///   from a `'PARTNER'` reference line, which nothing enters yet (rates.md §4.1), so they are
///   `""` / `null`;
/// - the kiosk's `manage` lists each currency of the current publication per 1 unit: `buy` and
///   `sell` stay as recorded; a saved currency's `updated` is the publication's `published_at`
///   (one instant for every line; `rpc_common`'s `as_published` sets it from the branch's
///   current publication, because a v3 `configure` publishes nothing); what only a direct save
///   carried is gone: `requestId` is `""`, the partner `""` / `null`, `botReference` `null`.
///
/// The first holds for a snapshot (it has `rateHistory`), the second for the snapshot and the
/// desk config (v2's rate entries, which carry `requestId`), the third for `manage` (its
/// `currencies`); anything else is left as it is.
pub fn without_direct_saves(recorded: &Value) -> Value {
    let mut value = recorded.clone();
    let Some(map) = value.as_object_mut() else {
        return value;
    };
    if let Some(Value::Array(currencies)) = map.get_mut("currencies") {
        for entry in currencies
            .iter_mut()
            .filter(|c| c.get("requestId").is_some())
        {
            for (key, none) in [
                ("requestId", json!("")),
                ("partnerName", json!("")),
                ("partnerCost", Value::Null),
                ("botReference", Value::Null),
            ] {
                if entry.get(key).is_some() {
                    entry[key] = none;
                }
            }
        }
    }
    if map.contains_key("rateHistory") {
        map.insert("rateHistory".to_string(), json!([]));
    }
    if let Some(Value::Array(rates)) = map.get_mut("rates") {
        for rate in rates.iter_mut().filter(|r| r.get("requestId").is_some()) {
            if rate.get("partnerName").is_some() {
                rate["partnerName"] = json!("");
            }
            if rate.get("partnerCost").is_some() {
                rate["partnerCost"] = Value::Null;
            }
        }
    }
    value
}

/// Re-expected for RE-7 #604 by the ticket author: sets `code` at `buy` / `sell` per 1 (as
/// v2's direct save took them) the v3 way, by publishing: every other line of `branch`'s
/// current publication is kept, and `code` is published per its `currencies.unit_size`.
/// Stands in for a v2 `adminSaveRate` a test or a recorded step needs applied.
pub async fn publish_rate_per_one(
    pool: &PgPool,
    branch: &str,
    code: &str,
    buy: &str,
    sell: &str,
) -> rates_re4c_common::Published {
    rates_re4c_common::set_counter_rate(pool, branch, code, buy, sell).await
}

/// Top-level keys v3 adds to a v2 answer by decision, per route and action, with the
/// exact fields each one's object must have. v2's pages ignore them; every other key
/// is still compared strictly both ways.
/// - RF-5 (#877, `docs/design/blueprint/rates.md` §4.10): the kiosk `rates` answer
///   appends `freshness`.
/// - #3473: the kiosk `get` answer appends `quoteId`.
pub const V3_ADDED_KEYS: &[(&str, &str, &str, &[&str])] = &[
    (
        "/api/kiosk",
        "rates",
        "freshness",
        &["publishedAt", "sourceDate", "stale"],
    ),
    ("/api/kiosk", "get", "quoteId", &[]),
];

/// `actual` without the keys [`V3_ADDED_KEYS`] allows on this route and action,
/// after checking each one present has its expected shape. The key
/// may be absent: the tests of the change that adds it require it.
pub fn without_v3_added_keys(
    route: Option<&str>,
    action: Option<&str>,
    actual: &Value,
    label: &str,
) -> Value {
    let mut actual = actual.clone();
    for (r, a, key, fields) in V3_ADDED_KEYS {
        if route != Some(*r) || action != Some(*a) {
            continue;
        }
        let Some(map) = actual.as_object_mut() else {
            continue;
        };
        if let Some(added) = map.remove(*key) {
            if fields.is_empty() {
                assert!(
                    added.is_string() || added.is_null(),
                    "{label}: $.{key} is a string or null; got {added}"
                );
            } else {
                let mut got: Vec<&str> = added
                    .as_object()
                    .unwrap_or_else(|| panic!("{label}: $.{key} is an object; got {added}"))
                    .keys()
                    .map(String::as_str)
                    .collect();
                got.sort_unstable();
                assert_eq!(got, *fields, "{label}: the fields of $.{key}");
            }
        }
    }
    actual
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
        install_test_pii_keys();

        let v2_hours = serde_json::json!({
            "mon": [{ "open": "11:00", "close": "23:00" }],
            "tue": [{ "open": "11:00", "close": "23:00" }],
            "wed": [{ "open": "11:00", "close": "23:00" }],
            "thu": [{ "open": "11:00", "close": "23:00" }],
            "fri": [{ "open": "11:00", "close": "23:00" }],
            "sat": [{ "open": "11:00", "close": "23:00" }],
            "sun": [{ "open": "11:00", "close": "23:00" }],
        });
        sqlx::query(
            "UPDATE branches SET timezone = 'Asia/Bangkok', opening_hours = $1 WHERE id = 'MAIN'",
        )
        .bind(&v2_hours)
        .execute(&pool)
        .await
        .expect("seed MAIN branch hours");

        Self { pool, schema }
    }

    pub fn branch_tx(&self) -> BranchTx {
        BranchTx::new(Transactor::new(self.pool.clone()))
    }

    /// Add a branch behind the service's back (`MAIN` is there already).
    pub async fn seed_branch(&self, id: &str, name: &str) {
        let v2_hours = serde_json::json!({
            "mon": [{ "open": "11:00", "close": "23:00" }],
            "tue": [{ "open": "11:00", "close": "23:00" }],
            "wed": [{ "open": "11:00", "close": "23:00" }],
            "thu": [{ "open": "11:00", "close": "23:00" }],
            "fri": [{ "open": "11:00", "close": "23:00" }],
            "sat": [{ "open": "11:00", "close": "23:00" }],
            "sun": [{ "open": "11:00", "close": "23:00" }],
        });
        sqlx::query("INSERT INTO branches (id, name, timezone, opening_hours) VALUES ($1, $2, 'Asia/Bangkok', $3)")
            .bind(id)
            .bind(name)
            .bind(&v2_hours)
            .execute(&self.pool)
            .await
            .expect("insert into branches");
        // LG-4 (#929, 1:1): a shift's opening now leaves the branch's ledger SAFE.
        lg4_fund_safe(&self.pool, id).await;
    }

    /// [`TestDb::seed_branch`] without the funded ledger SAFE (LG-6.r5 #1044): for a test that opens no shift
    /// and compares `operations.safe` with v2's `null`, as `rpc_common::Desk::open_without_cash` does for MAIN.
    pub async fn seed_branch_without_cash(&self, id: &str, name: &str) {
        let v2_hours = serde_json::json!({
            "mon": [{ "open": "11:00", "close": "23:00" }],
            "tue": [{ "open": "11:00", "close": "23:00" }],
            "wed": [{ "open": "11:00", "close": "23:00" }],
            "thu": [{ "open": "11:00", "close": "23:00" }],
            "fri": [{ "open": "11:00", "close": "23:00" }],
            "sat": [{ "open": "11:00", "close": "23:00" }],
            "sun": [{ "open": "11:00", "close": "23:00" }],
        });
        sqlx::query("INSERT INTO branches (id, name, timezone, opening_hours) VALUES ($1, $2, 'Asia/Bangkok', $3)")
            .bind(id)
            .bind(name)
            .bind(&v2_hours)
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

// CA-10 (#626, #2955): the core asks `CustodyFactsPort` (v2's `CustodyPorts` is no longer asked); the snapshot
// still must not ask it.
impl CustodyFactsPort for NoPorts {
    fn service_window_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _now: CustodyTime,
    ) -> BoxFuture<'a, AppResult<(ServiceWindow, i32)>> {
        Box::pin(async { never_asked() })
    }

    fn has_live_rate_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<(bool, String)>> {
        Box::pin(async { never_asked() })
    }

    fn transactions_summary_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<(Vec<String>, Option<String>)>> {
        Box::pin(async { never_asked() })
    }

    fn customer_exists_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _branch: &'a BranchId,
        _customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>> {
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
            Arc::new(ledger_shift_repository(db.pool.clone())),
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
        // amended for #757 (RE-4c, rates.md §4.9 R9, ruling of 2026-10-10): the row's own
        // unit; v2 recorded every rate per 1, so the row is stored per 1 unit
        unit_size: UnitSize::parse("1").expect("a unit"),
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
    /// Re-expected for RE-7 #604 by the ticket author: accepted `adminSaveRate` steps, whose
    /// answer is not compared (known difference: design §9 removes the action) and whose rate
    /// is published instead.
    pub retired: usize,
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
    /// Step number → the customer v3 answered (CI-4 #651, for SAME_CUSTOMER_AS).
    pub reserved: BTreeMap<usize, String>,
}

impl<'f> Replay<'f> {
    pub async fn start(fixture: &'f Fixture) -> Replay<'f> {
        Replay {
            shop: Shop::open().await,
            fixture,
            ids: IdMap::new(),
            compared: Compared::default(),
            recorded_transactions: Vec::new(),
            reserved: BTreeMap::new(),
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
            // Re-expected for RE-7 #604 by the ticket author: a known difference (design §9
            // removes the action). v3 has no direct save, so v2's answer to it is not compared;
            // the rate the step saved is published instead, as a v3 owner sets a rate, so the
            // later steps still see it and still prove what they did.
            "adminSaveRate" => {
                self.publish_saved(step).await;
                self.compared.retired += 1;
            }
            "adminCreateBranch" => {
                let created = self.create_branch(step).await;
                self.compare_write(step, Some(created), None).await;
            }
            // CA-10 (#626, #2955): v2's admin open / move / close are deleted with the core. The recorded steps are
            // replayed straight into the tables, leaving exactly the rows v2's `ShiftService` wrote, in one branch
            // transaction as v2 did (setup only: what is compared is unchanged).
            "adminOpenShift" => {
                seed_opener(&self.shop.db.pool, &actor_of(self.fixture, step)).await;
                self.v2_admin_write(step, V2Write::Open).await;
                self.compare_write(step, None, None).await;
            }
            "adminMoveStock" => {
                self.v2_admin_write(step, V2Write::Move).await;
                self.compare_write(step, None, None).await;
            }
            "adminCloseShift" => {
                self.v2_admin_write(step, V2Write::Close).await;
                self.compare_write(step, None, None).await;
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
                self.compare_write(step, None, None).await;
            }
            "reserveCustomer" => self.reserve(step).await,
            "saveTransaction" => self.feed_save(step).await,
            "getConnectionStatus" | "documents" => {}
            other => panic!("{}: no rule for the action {other}", self.label(step)),
        }
    }

    /// The step's payload with the ids v2 made replaced by the ones v3 made.
    /// CA-10 (#626, #2955): one recorded v2 admin write (open, move or close a shift), replayed straight into the
    /// tables inside a branch transaction (which steps the branch version, as v2's command did).
    async fn v2_admin_write(&self, step: &Step, what: V2Write) {
        replay_v2_admin(
            &self.shop,
            &branch_of(self.fixture, step),
            what,
            &self.payload(step),
            &actor_of(self.fixture, step),
            time(&step.at),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: {e:?}", self.label(step)));
    }

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

    /// Re-expected for RE-7 #604 by the ticket author: the rate a recorded `adminSaveRate`
    /// saved (per 1, as v2 took it), published on top of the branch's current publication.
    async fn publish_saved(&mut self, step: &Step) {
        let payload = self.payload(step);
        let published = publish_rate_per_one(
            &self.shop.db.pool,
            branch_of(self.fixture, step).as_str(),
            &text_of(&payload, "code"),
            &number_text(&payload, "buy").expect("buy"),
            &number_text(&payload, "sell").expect("sell"),
        )
        .await;
        // ruling Q3 on #604: the snapshot's `updated` is the publication's `published_at`; it is
        // stamped with the step's instant, as v2 stamped the saved row
        sqlx::query("UPDATE rate_publications SET published_at = $1 WHERE id = $2::uuid")
            .bind(time(&step.at))
            .bind(&published.id)
            .execute(&self.shop.db.pool)
            .await
            .expect("stamp the publication with the step's instant");
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
        let actual =
            json!({"customerId": reservation.customer_id, "existing": reservation.existing});
        // CI-4 (#651): a decided customer difference (compat_common::SAME_CUSTOMER_AS), checked strictly.
        if let Some(same) = crate::compat_common::same_customer_as(&self.fixture.name, step.n) {
            let label = format!("{} ({})", self.label(step), same.reason);
            let earlier = self
                .reserved
                .get(&same.same_as)
                .unwrap_or_else(|| panic!("{label}: step {} was not replayed", same.same_as))
                .clone();
            let expected =
                crate::compat_common::same_customer_answer(step.json(), &earlier, &label);
            assert_same(&expected, &actual, &label);
            self.ids
                .alias_customer(&text_of(step.json(), "customerId"), &earlier);
        } else {
            assert_same(step.json(), &actual, &self.label(step));
        }
        self.reserved
            .insert(step.n, reservation.customer_id.clone());
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

    /// CI-4 (#651): a recorded answer whose `customers` v3 answers company-wide (compat_common::COMPANY_CUSTOMERS).
    fn decided_customers(&self, step: &Step, recorded: Value) -> Value {
        match crate::compat_common::company_customers(self.fixture, step.n) {
            Some(list) => crate::compat_common::with_customers_of(
                &recorded,
                &json!({ "customers": list }),
                &format!(
                    "{} ({})",
                    self.label(step),
                    crate::compat_common::COMPANY_CUSTOMERS_REASON
                ),
            ),
            None => recorded,
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
        // amended for #757 (RE-4c): rates[] is the publication (D-3, R10); re-expected for
        // RE-7 #604 by the ticket author: no `rates` rows (`without_direct_saves`)
        let recorded = rates_re4c_common::as_published(
            &self.shop.db.pool,
            branch_of(self.fixture, step).as_str(),
            &without_direct_saves(&without_dropped_keys(step.json())),
            None,
        )
        .await;
        let mut recorded = self.decided_customers(step, recorded);
        self.restate_stock_movements(&mut recorded, &actual).await;
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
        // amended for #757 (RE-4c): rates[] is the publication (D-3, R10, R3); re-expected for
        // RE-7 #604 by the ticket author: no partner (`without_direct_saves`)
        let recorded = rates_re4c_common::as_published(
            &self.shop.db.pool,
            branch_of(self.fixture, step).as_str(),
            &without_direct_saves(step.json()),
            None,
        )
        .await;
        self.ids.learn(&recorded, &actual);
        assert_same(&self.ids.to_actual(&recorded), &actual, &self.label(step));
        self.compared.desks += 1;
    }

    /// The answer of a write: the snapshot of the branch written to (of the new
    /// branch after `adminCreateBranch`) as `write_answer` cuts it.
    ///
    /// Two things in a recorded answer are not the builder's and are taken out
    /// of both sides: `requestResolution` (the dispatcher adds what the abandon
    /// workflow answered) and, on the staff path, the two links the staff view
    /// rewrites.
    ///
    /// Amended for #757 (RE-4c, D-3, §4.9 R10): `rates` is held to the publication of the
    /// branch (`rates_re4c_common::as_published`); `saved` is the code an `adminSaveRate` step
    /// saved, which the answer does not show.
    async fn compare_write(
        &mut self,
        step: &Step,
        answered_for: Option<BranchId>,
        saved: Option<&str>,
    ) {
        let here = answered_for.unwrap_or_else(|| branch_of(self.fixture, step));
        let snapshot = self
            .shop
            .builder()
            .admin_snapshot(&here, &actor_of(self.fixture, step), time(&step.at))
            .await
            .unwrap_or_else(|e| panic!("{}: {e:?}", self.label(step)));
        let mut actual = write_answer(&snapshot).to_value();
        let recorded = rates_re4c_common::as_published(
            &self.shop.db.pool,
            here.as_str(),
            // re-expected for RE-7 #604 by the ticket author: no `rates` rows
            &without_direct_saves(&without_keys(
                &without_dropped_keys(step.json()),
                &["requestResolution"],
            )),
            saved,
        )
        .await;
        let mut recorded = self.decided_customers(step, recorded);
        if step.path() == Some("/api/staff/rpc") {
            recorded = without_keys(&recorded, &["sheetUrl", "deskUrl"]);
            actual = without_keys(&actual, &["sheetUrl", "deskUrl"]);
        }
        self.restate_stock_movements(&mut recorded, &actual).await;
        self.ids.learn(&recorded, &actual);
        assert_same(&self.ids.to_actual(&recorded), &actual, &self.label(step));
        self.compared.writes += 1;
    }

    async fn restate_stock_movements(&self, expected: &mut Value, actual: &Value) {
        if let (Some(want), Some(got)) = (
            expected["stock"]["movements"].as_array().cloned(),
            actual["stock"]["movements"].as_array(),
        ) && want.len() == got.len()
        {
            for (i, mov) in got.iter().enumerate() {
                // LG-6.r6 #1046 (controller ruling): `staff` in v3 is the user id for v2's username in the replay's user map.
                if let (Some(want_staff), Some(got_staff)) =
                    (want[i]["staff"].as_str(), mov["staff"].as_str())
                    && movement_staff_matches(
                        &self.shop.db.pool,
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
    }
}

/// LG-6.r6 #1046 (controller ruling): whether `user_id` is the known user id for `username` in this replay.
pub async fn movement_staff_matches(
    pool: &PgPool,
    fixture: &Fixture,
    user_id: &str,
    username: &str,
) -> bool {
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT name, legacy_employee_id FROM users WHERE id::text = $1")
            .bind(user_id)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);
    let Some((name, legacy)) = row else {
        return false;
    };
    name == username
        || legacy.is_some_and(|leg| {
            leg == username
                || fixture
                    .people
                    .iter()
                    .any(|p| p.id == leg && p.username == username)
        })
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
    [Phase::Closed, Phase::Ready, Phase::Open, Phase::CutOff]
        .into_iter()
        .find(|p| p.as_str() == text)
        .unwrap_or_else(|| panic!("no phase {text}"))
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

pub fn state_of(ops: &Value) -> CustodyState {
    CustodyState {
        phase: phase_of(&text_of(ops, "phase")),
        target: ops.get("target").filter(|v| !v.is_null()).map(target_of),
        entity: ops.get("entity").filter(|v| !v.is_null()).map(|e| Entity {
            company: text_of(e, "company"),
            site: text_of(e, "site"),
            confirmed_by: text_of(e, "confirmedBy"),
            at: custody_at(&text_of(e, "at")),
        }),
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
        // CA-10 (#626): `enabled`, `custodian` and `requests` of a recorded v2 snapshot are not part of the state any
        // more (M3, Q6: the compat snapshot prints them as constants).
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
                       SELECT id, 'owner', 'PENDING', 'test' FROM o RETURNING id, organization_id) \
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
         SELECT id, $1, 'PENDING', 'test', $1 FROM organizations WHERE code = 'XC' \
         ON CONFLICT (legacy_employee_id) DO NOTHING",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("seed an opener's user");
}

/// CI-4 (#651): the PII keys every test process installs (U3: one `key_id:base64(32 bytes)` line). A test key,
/// never a real one. Added by the ticket author in CI-4's up-front commit.
pub const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

pub fn install_test_pii_keys() -> std::sync::Arc<xc_kernel::pii::PiiKeys> {
    let keys = std::sync::Arc::new(
        xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).expect("the test PII key parses"),
    );
    xc_customer::install_pii_keys(keys.clone()).expect("one test PII key per process");
    keys
}

/// The decrypted payload of a customer's ACTIVE passport identity: `{number, nationality, birthDate, expiry,
/// nameOnDocument}` (#554 §4.1), read the way only a test may.
pub async fn passport_payload(pool: &sqlx::PgPool, customer_id: &str) -> serde_json::Value {
    let (bytes, key_id): (Vec<u8>, String) = sqlx::query_as(
        "SELECT encrypted_value, key_id FROM customer_identities \
         WHERE customer_id = $1 AND type = 'PASSPORT' AND status = 'ACTIVE'",
    )
    .bind(customer_id)
    .fetch_one(pool)
    .await
    .expect("the customer's passport identity");
    let cipher = xc_kernel::pii::PiiCipher::new(install_test_pii_keys());
    let plain = cipher
        .decrypt(
            format!("{customer_id}PASSPORT").as_bytes(),
            &xc_kernel::pii::Sealed { key_id, bytes },
        )
        .expect("decrypt with the test key");
    serde_json::from_slice(&plain).expect("the passport payload is JSON")
}

/// CI-4 (#651): the LIVE customer whose ACTIVE passport carries this number, found by decrypting each passport
/// identity with the test key (a test's view; the product finds customers by lookup hash, #554 K4).
pub async fn live_customer_with_passport(pool: &sqlx::PgPool, number: &str) -> Option<String> {
    let rows: Vec<(String, Vec<u8>, String)> = sqlx::query_as(
        "SELECT i.customer_id, i.encrypted_value, i.key_id FROM customer_identities i \
         JOIN customers c ON c.id = i.customer_id \
         WHERE i.type = 'PASSPORT' AND i.status = 'ACTIVE' AND c.mode = 'LIVE' ORDER BY i.customer_id",
    )
    .fetch_all(pool)
    .await
    .expect("read the passport identities");
    let cipher = xc_kernel::pii::PiiCipher::new(install_test_pii_keys());
    for (customer_id, bytes, key_id) in rows {
        let plain = cipher
            .decrypt(
                format!("{customer_id}PASSPORT").as_bytes(),
                &xc_kernel::pii::Sealed { key_id, bytes },
            )
            .expect("decrypt with the test key");
        let payload: serde_json::Value =
            serde_json::from_slice(&plain).expect("the passport payload is JSON");
        if payload["number"].as_str() == Some(number) {
            return Some(customer_id);
        }
    }
    None
}

/// CI-4 (#651): gives an existing customer its ACTIVE passport identity, encrypted and hashed with the test key,
/// for tests that insert a customer row directly (#554 §4.1: the passport lives in `customer_identities`).
pub async fn seed_passport_identity(
    pool: &sqlx::PgPool,
    customer_id: &str,
    number: &str,
    nationality: &str,
) {
    use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, normalize};
    let keys = install_test_pii_keys();
    let normalized =
        normalize(IdentityType::Passport, number, Some(nationality)).expect("an alpha-3 passport");
    let hash = LookupHasher::new(&keys).hash(IdentityType::Passport, &normalized);
    let payload =
        serde_json::json!({"number": number, "nationality": nationality, "birthDate": null,
        "expiry": null, "nameOnDocument": null})
        .to_string();
    let sealed = PiiCipher::new(keys).encrypt(
        format!("{customer_id}PASSPORT").as_bytes(),
        payload.as_bytes(),
    );
    sqlx::query(
        "INSERT INTO customer_identities (id, organization_id, customer_id, type, encrypted_value, key_id, \
         lookup_hash, nationality) SELECT gen_random_uuid(), organization_id, id, 'PASSPORT', $2, $3, $4, $5 \
         FROM customers WHERE id = $1",
    )
    .bind(customer_id)
    .bind(&sealed.bytes)
    .bind(&sealed.key_id)
    .bind(hash.as_slice())
    .bind(nationality)
    .execute(pool)
    .await
    .expect("insert the passport identity");
}

#[path = "../lg4_fund/mod.rs"]
mod lg4_fund;
pub use lg4_fund::lg4_fund_safe;

/// A recorded v2 admin write on a shift (CA-10: replayed by SQL, its v2 command is deleted).
#[derive(Clone, Copy)]
pub enum V2Write {
    Open,
    Move,
    Close,
}

fn truncate_to_millis(t: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(t.timestamp_millis()).unwrap_or(t)
}

/// A recorded amount, a JSON text or number, as text for a `numeric` bind.
fn amount_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        other => panic!("a recorded amount, got {other}"),
    }
}

fn v2_text<'v>(payload: &'v Value, key: &str) -> &'v str {
    payload
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("the recorded payload has `{key}`: {payload}"))
}

/// What v2's `ShiftService::{open_in, move_stock_in, close_in}` wrote for a step that answered 200: the shift row
/// (`SH-<uuid>`, its teller the opener's user), the movement (`MV-<uuid>`) and the expected figure it moved, or
/// the close (counted per currency, `difference = counted - expected`, the rest 0).
/// CA-10 (#626, #2955): one v2 admin write (open, move or close a shift with v2's payload), replayed straight
/// into the tables inside a branch transaction, which steps the branch version as v2's command did. Answers the
/// id of the row it wrote: the shift for an open or a close, the movement for a move.
pub async fn replay_v2_admin(
    shop: &Shop,
    branch: &BranchId,
    what: V2Write,
    payload: &Value,
    actor: &str,
    at: DateTime<Utc>,
) -> AppResult<String> {
    let (b, payload, actor, at) = (
        branch.clone(),
        payload.clone(),
        actor.to_owned(),
        truncate_to_millis(at),
    );
    shop.db
        .branch_tx()
        .run(branch, move |tx| {
            Box::pin(async move { v2_admin_write_in(tx, &b, what, &payload, &actor, at).await })
        })
        .await
}

async fn v2_admin_write_in(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    what: V2Write,
    payload: &Value,
    actor: &str,
    at: DateTime<Utc>,
) -> AppResult<String> {
    match what {
        V2Write::Open => {
            let teller: String = if actor == "owner" {
                sqlx::query_scalar(
                    "SELECT u.id::text FROM users u WHERE u.legacy_employee_id IS NULL AND EXISTS \
                     (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' \
                     AND r.branch_id IS NULL) ORDER BY u.created_at LIMIT 1",
                )
                .fetch_one(&mut **tx)
                .await?
            } else {
                sqlx::query_scalar("SELECT id::text FROM users WHERE legacy_employee_id = $1")
                    .bind(actor)
                    .fetch_one(&mut **tx)
                    .await?
            };
            let id = format!("SH-{}", uuid::Uuid::new_v4());
            sqlx::query(
                "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, \
                 teller_id) VALUES ($1, $2, 'OPEN', $3, $4, '', $5, $6::uuid)",
            )
            .bind(&id)
            .bind(branch.as_str())
            .bind(at)
            .bind(actor)
            .bind(v2_text(payload, "requestId"))
            .bind(&teller)
            .execute(&mut **tx)
            .await?;
            if let Some(opening) = payload.get("opening").and_then(Value::as_object) {
                use rust_decimal::Decimal;
                use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
                use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
                for (code_str, amount) in opening {
                    let qty: Decimal = amount_text(amount).parse().expect("opening decimal");
                    if qty > Decimal::ZERO
                        && let Ok(currency) = CurrencyCode::parse(code_str)
                    {
                        xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
                            .post_in(
                                tx,
                                &[PostingRequest {
                                    from: CashPlace::Bank,
                                    to: CashPlace::Drawer(id.clone()),
                                    currency,
                                    quantity: xc_kernel::money::ForeignAmount::from_decimal(qty),
                                    source: SourceRef {
                                        source_type: SourceType::Opening,
                                        source_id: format!("v2-open:{id}"),
                                        source_key: code_str.clone(),
                                    },
                                    line: None,
                                    reverses: None,
                                }],
                                None,
                            )
                            .await?;
                    }
                }
            }
            Ok(id)
        }
        V2Write::Move => {
            let shift = v2_text(payload, "shiftId");
            let (code_str, direction) =
                (v2_text(payload, "currency"), v2_text(payload, "direction"));
            let amount = amount_text(payload.get("amount").unwrap_or(&Value::Null));
            let (kind, from_shift, to_shift) = if direction == "IN" {
                ("SAFE_TO_DRAWER", None, Some(shift))
            } else {
                ("DRAWER_TO_SAFE", Some(shift), None)
            };
            let face: Option<(i64, String)> = sqlx::query_as(
                "SELECT id, face_value::text FROM denominations \
                 WHERE currency_code = $1 AND face_value IS NOT NULL AND $2::numeric % face_value = 0 \
                 ORDER BY face_value DESC LIMIT 1",
            )
            .bind(code_str)
            .bind(&amount)
            .fetch_optional(&mut **tx)
            .await?;
            let (denomination, face) = match face {
                Some(f) => f,
                None => {
                    sqlx::query_as(
                        "INSERT INTO denominations (currency_code, face_value, group_code) \
                         VALUES ($1, 0.01, 'ALL') RETURNING id, face_value::text",
                    )
                    .bind(code_str)
                    .fetch_one(&mut **tx)
                    .await?
                }
            };
            let mover: String = sqlx::query_scalar(
                "INSERT INTO users (organization_id, name, status, created_by) \
                 SELECT organization_id, $2, 'PENDING', 'test' FROM branches WHERE id = $1 \
                 RETURNING id::text",
            )
            .bind(branch.as_str())
            .bind(actor)
            .fetch_one(&mut **tx)
            .await?;
            let transfer: String = sqlx::query_scalar(
                "INSERT INTO cash_transfers (organization_id, kind, from_branch_id, from_shift_id, \
                     to_shift_id, status, created_by, dispatched_at, dispatched_by, received_at, \
                     received_by, note, idempotency_key, created_at, updated_at) \
                 SELECT organization_id, $2, id, $3, $4, 'RECEIVED', $5::uuid, $6, $5::uuid, $6, \
                     $5::uuid, $7, $8, $6, $6 \
                 FROM branches WHERE id = $1 RETURNING id::text",
            )
            .bind(branch.as_str())
            .bind(kind)
            .bind(from_shift)
            .bind(to_shift)
            .bind(&mover)
            .bind(at)
            .bind(v2_text(payload, "reason"))
            .bind(v2_text(payload, "requestId"))
            .fetch_one(&mut **tx)
            .await?;
            sqlx::query(
                "INSERT INTO cash_transfer_lines (transfer_id, organization_id, currency, \
                     denomination_id, qty_sent, qty_received) \
                 SELECT t.id, t.organization_id, $2, $3, ($4::numeric / $5::numeric)::int, \
                     ($4::numeric / $5::numeric)::int \
                 FROM cash_transfers t WHERE t.id = $1::uuid",
            )
            .bind(&transfer)
            .bind(code_str)
            .bind(denomination)
            .bind(&amount)
            .bind(&face)
            .execute(&mut **tx)
            .await?;
            {
                use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
                use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
                let (from, to) = if direction == "IN" {
                    (
                        CashPlace::InTransit(transfer.clone()),
                        CashPlace::Drawer(shift.to_owned()),
                    )
                } else {
                    (
                        CashPlace::Drawer(shift.to_owned()),
                        CashPlace::InTransit(transfer.clone()),
                    )
                };
                xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
                    .post_in(
                        tx,
                        &[PostingRequest {
                            from,
                            to,
                            currency: code(code_str),
                            quantity: xc_kernel::money::ForeignAmount::from_decimal(
                                amount.parse().expect("move decimal"),
                            ),
                            source: SourceRef {
                                source_type: SourceType::Transfer,
                                source_id: transfer.clone(),
                                source_key: code_str.to_owned(),
                            },
                            line: None,
                            reverses: None,
                        }],
                        None,
                    )
                    .await?;
            }
            Ok(format!("{transfer}:{code_str}"))
        }
        V2Write::Close => {
            let shift = v2_text(payload, "shiftId");
            sqlx::query(
                "UPDATE shifts SET status = 'CLOSED', closed_at = $1, closed_by = $2, close_note = $3, \
                 close_request_id = $4 WHERE id = $5",
            )
            .bind(at)
            .bind(actor)
            .bind(payload.get("note").and_then(Value::as_str).unwrap_or_default())
            .bind(v2_text(payload, "requestId"))
            .bind(shift)
            .execute(&mut **tx)
            .await?;
            let count_id: String = sqlx::query_scalar(
                "WITH counter AS ( \
                     INSERT INTO users (organization_id, name, status, created_by) \
                     SELECT organization_id, 'v2-close-counter', 'PENDING', 'test' FROM branches WHERE id = $1 \
                     RETURNING id, organization_id \
                 ), \
                 verifier AS ( \
                     INSERT INTO users (organization_id, name, status, created_by) \
                     SELECT organization_id, 'v2-close-verifier', 'PENDING', 'test' FROM counter \
                     RETURNING id \
                 ) \
                 INSERT INTO cash_counts (organization_id, branch_id, shift_id, kind, counted_by, counted_at, \
                     verified_by, verified_at, idempotency_key) \
                 SELECT counter.organization_id, $1, $2, 'CLOSE', counter.id, $3, verifier.id, $3, $4 \
                 FROM counter, verifier \
                 RETURNING id::text",
            )
            .bind(branch.as_str())
            .bind(shift)
            .bind(at)
            .bind(v2_text(payload, "requestId"))
            .fetch_one(&mut **tx)
            .await?;
            if let Some(counted) = payload.get("counted").and_then(Value::as_object) {
                for (code_str, amount) in counted {
                    sqlx::query(
                        "INSERT INTO cash_count_totals (count_id, organization_id, currency, counted) \
                         SELECT c.id, c.organization_id, $2, $3::numeric \
                         FROM cash_counts c WHERE c.id = $1::uuid \
                         ON CONFLICT (count_id, currency) DO UPDATE SET counted = EXCLUDED.counted",
                    )
                    .bind(&count_id)
                    .bind(code_str)
                    .bind(amount_text(amount))
                    .execute(&mut **tx)
                    .await?;
                }
            }
            sqlx::query(
                "INSERT INTO cash_count_totals (count_id, organization_id, currency, counted) \
                 SELECT c.id, c.organization_id, a.currency, 0 \
                 FROM cash_counts c \
                 JOIN cash_accounts a ON a.kind = 'DRAWER' AND a.shift_id = c.shift_id \
                 WHERE c.id = $1::uuid \
                 ON CONFLICT (count_id, currency) DO NOTHING",
            )
            .bind(&count_id)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "UPDATE cash_count_totals cct \
                 SET expected = COALESCE(( \
                         SELECT SUM(e.quantity_signed) \
                         FROM cash_accounts a \
                         JOIN ledger_entries e ON e.account_id = a.id \
                         WHERE a.kind = 'DRAWER' AND a.shift_id = $2 AND a.currency = cct.currency \
                     ), 0), \
                     difference = cct.counted - COALESCE(( \
                         SELECT SUM(e.quantity_signed) \
                         FROM cash_accounts a \
                         JOIN ledger_entries e ON e.account_id = a.id \
                         WHERE a.kind = 'DRAWER' AND a.shift_id = $2 AND a.currency = cct.currency \
                     ), 0) \
                 WHERE cct.count_id = $1::uuid",
            )
            .bind(&count_id)
            .bind(shift)
            .execute(&mut **tx)
            .await?;
            Ok(shift.to_owned())
        }
    }
}
