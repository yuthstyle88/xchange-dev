//! Shared by the shift service tests: a PostgreSQL schema per test, the v2
//! fixture, and the few conversions the tests need.
//! Provided with ticket T7 — do not edit.
//! CA-10 (#626, #2955): the setup moved off v2's deleted shift path onto SQL seeds (`seed_open_shift`,
//! `seed_close`, `seed_movement`) and the per-teller delta (`apply_delta`).
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use serde::Deserialize;
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money, MoneyDelta, Rate, Side};
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::shift::entities::shift::{Shift, ShiftStatus};
use xc_shift::domain::shift::entities::stock_movement::StockMovement;
use xc_shift::domain::shift::value_objects::balances::Balances;
use xc_shift::domain::shift::value_objects::stock_delta::{Direction, StockDelta};
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

#[path = "../shift_balances_double/mod.rs"]
pub mod shift_balances_double;
pub use shift_balances_double::ShiftBalancesDouble;

/// RE-7 #604 retires `Quote::compute`; a quote per 1 unit is `Quote::per_unit` at `UnitSize::ONE`
/// (re-expected by the ticket author, claude/07d3765f; the figures this test pins are unchanged).
fn per_one(
    side: xc_kernel::money::Side,
    amount: xc_kernel::money::Money,
    rate: xc_kernel::money::Rate,
    fee: xc_kernel::money::Money,
) -> Result<xc_kernel::money::Quote, xc_kernel::money::QuoteError> {
    let amount: rust_decimal::Decimal = amount.to_string().parse().expect("a decimal amount");
    xc_kernel::money::Quote::per_unit(
        side,
        xc_kernel::money::ForeignAmount::from_decimal(amount),
        rate,
        xc_kernel::money::UnitSize::ONE,
        fee,
    )
}

pub type Service = ShiftService<PgShiftRepository>;

/// Upper bound for anything that must finish.
pub const MUST_FINISH: Duration = Duration::from_secs(30);

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

/// Await `fut`, failing the test when it takes longer than [`MUST_FINISH`].
pub async fn within<T>(what: &str, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(MUST_FINISH, fut).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after {MUST_FINISH:?}: {what}"),
    }
}

/// A fixed instant, `seconds` after 2026-10-07 03:00:00 UTC, with milliseconds.
pub fn at(seconds: i64, millis: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 7, 3, 0, 0).unwrap()
        + chrono::Duration::seconds(seconds)
        + chrono::Duration::milliseconds(i64::from(millis))
}

/// Every opener id the xc-shift tests open a shift as (CA-1 #617: each must name a user).
pub const OPENERS: [&str; 21] = [
    "BE1", "BE2", "F1", "F2", "F3", "FIN1", "FIN2", "MGR1", "MON1", "RT1", "a", "b", "opener",
    "racer", "t10", "x", "replay", "somchai", "owner-1", "test", "owner",
];

pub struct TestDb {
    pub pool: PgPool,
    pub schema: String,
    pub balances: Arc<ShiftBalancesDouble>,
}

impl TestDb {
    /// A pool confined to a fresh schema with `api/migrations/` applied.
    ///
    /// `TEST_DATABASE_URL` must name an existing database whose name ends in
    /// `_test`. When it is absent the test FAILS; nothing here skips.
    pub async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The shift service tests need a real PostgreSQL and do not skip. Point it at an \
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

        let schema = unique("t7");
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
            .max_connections(16)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");

        for opener in OPENERS {
            seed_opener(&pool, opener).await;
        }

        Self {
            pool,
            schema,
            balances: Arc::new(ShiftBalancesDouble::new()),
        }
    }

    pub fn branch_tx(&self) -> BranchTx {
        BranchTx::new(Transactor::new(self.pool.clone()))
    }

    pub fn service(&self) -> Service {
        ShiftService::new(
            Arc::new(PgShiftRepository::with_balances(
                self.pool.clone(),
                self.balances.clone(),
            )),
            self.branch_tx(),
        )
    }

    /// A branch that exists: a row in `branches` (the table of ticket T5).
    pub async fn branch(&self, label: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $2)")
            .bind(&id)
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert a test branch into `branches`");
        BranchId::new(id)
    }

    /// What the screen does when it gives up on a request (ticket T5's table).
    pub async fn abandon(&self, branch: &BranchId, request_id: &str) {
        sqlx::query(
            "INSERT INTO abandoned_requests (branch_id, request_id, at, actor) \
             VALUES ($1, $2, now(), 'test')",
        )
        .bind(branch.as_str())
        .bind(request_id)
        .execute(&self.pool)
        .await
        .expect("insert into `abandoned_requests`");
    }

    pub async fn version(&self, branch: &BranchId) -> i64 {
        self.branch_tx()
            .version(branch)
            .await
            .expect("read the branch version")
    }

    /// Run `apply_delta_for_teller_in` the way a teller's save does: inside a `BranchTx::run` of its own.
    /// CA-10 (#626, #2955): restates the deleted per-branch `apply_delta_in`; `teller` is a `users.id`.
    pub async fn apply_delta(
        &self,
        branch: &BranchId,
        teller: &str,
        delta: StockDelta,
    ) -> AppResult<String> {
        let service = self.service();
        let balances = self.balances.clone();
        let (b, t) = (branch.clone(), teller.to_owned());
        self.branch_tx()
            .run(branch, move |tx| {
                Box::pin(async move {
                    let shift_id = service
                        .apply_delta_for_teller_in(tx, &b, &t, &delta)
                        .await?;
                    balances.apply_delta(&shift_id, &delta);
                    Ok(shift_id)
                })
            })
            .await
    }

    /// The `users.id` an opener id of [`OPENERS`] was seeded as.
    pub async fn teller(&self, opener: &str) -> String {
        sqlx::query_scalar("SELECT id::text FROM users WHERE legacy_employee_id = $1")
            .bind(opener)
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("the user seeded for opener {opener}: {e}"))
    }

    /// CA-10 (#626, #2955): an OPEN shift written straight into `shifts` and the `ShiftBalances` double, the way a
    /// verified OPEN count leaves one, with no `opened_by_count` (M3 drops it). `opening` is `(code, amount)`;
    /// `expected` starts equal to it. Returns the shift's id. Replaces v2's `ShiftService::open` as the suites' setup.
    pub async fn seed_open_shift(
        &self,
        branch: &BranchId,
        opener: &str,
        open_request_id: &str,
        opening: &[(&str, &str)],
        opened_at: DateTime<Utc>,
    ) -> String {
        let id = format!("SH-{}", uuid::Uuid::new_v4());
        let teller = self.teller(opener).await;
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', $3, $4, '', $5, $6::uuid)",
        )
        .bind(&id)
        .bind(branch.as_str())
        .bind(opened_at)
        .bind(opener)
        .bind(open_request_id)
        .bind(&teller)
        .execute(&self.pool)
        .await
        .expect("seed an OPEN shift");
        self.balances.seed_opening(&id, opening);
        id
    }

    /// CA-10 (#626, #2955): closes a seeded shift straight in the tables and the `ShiftBalances` double, the way a
    /// verified CLOSE count leaves it: `counted` per currency, `difference = counted - expected`. Replaces v2's
    /// `ShiftService::close` as setup.
    pub async fn seed_close(
        &self,
        shift_id: &str,
        closed_by: &str,
        close_request_id: &str,
        counted: &[(&str, &str)],
        note: &str,
        closed_at: DateTime<Utc>,
    ) {
        sqlx::query(
            "UPDATE shifts SET status = 'CLOSED', closed_at = $1, closed_by = $2, close_note = $3, \
             close_request_id = $4 WHERE id = $5",
        )
        .bind(closed_at)
        .bind(closed_by)
        .bind(note)
        .bind(close_request_id)
        .bind(shift_id)
        .execute(&self.pool)
        .await
        .expect("seed a close");
        self.balances.seed_close(shift_id, counted);
    }

    pub async fn drop_schema(self) {
        let schema = self.schema.clone();
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&self.pool)
            .await
            .expect("drop the test schema");
        self.pool.close().await;
    }
}

// ── conversions ─────────────────────────────────────────────────────────────

pub fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).unwrap_or_else(|_| panic!("{text} is not in the register"))
}

pub fn money(text: &str) -> Money {
    Money::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

/// The sentence of a business refusal; panics on any other kind of error.
pub fn refusal<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected AppError::Domain(<sentence>), got {other:?}"),
    }
}

/// `Ok` → `None`, a business refusal → its sentence, anything else → panic.
pub fn outcome<T>(result: AppResult<T>) -> (Option<T>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(AppError::Domain(sentence)) => (None, Some(sentence)),
        Err(other) => panic!("neither accepted nor refused with a sentence: {other:?}"),
    }
}

fn plain(minor: i128) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.unsigned_abs();
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
}

/// Non-zero balances as the fixture writes them: code → `"1234.50"`.
pub fn shown_balances(b: &Balances) -> BTreeMap<String, String> {
    b.non_zero()
        .into_iter()
        .map(|(c, m)| (c.as_str().to_owned(), plain(m.minor_i128())))
        .collect()
}

/// Non-zero signed amounts as the fixture writes them: code → `"-0.50"`.
pub fn shown_delta(d: &StockDelta) -> BTreeMap<String, String> {
    d.non_zero()
        .into_iter()
        .map(|(c, m): (CurrencyCode, MoneyDelta)| (c.as_str().to_owned(), plain(m.minor())))
        .collect()
}

pub fn shown_shift(s: &Shift) -> ShiftSeen {
    ShiftSeen {
        status: match s.status {
            ShiftStatus::Open => "OPEN".to_owned(),
            ShiftStatus::Closed => "CLOSED".to_owned(),
        },
        opening: shown_balances(&s.opening),
        expected: shown_balances(&s.expected),
        counted: s.counted.as_ref().map(shown_balances),
        difference: s.difference.as_ref().map(shown_delta),
        note: s.note.clone(),
    }
}

pub fn shown_movement(m: &StockMovement, shift_ids: &[String]) -> MovementSeen {
    MovementSeen {
        shift: shift_ids
            .iter()
            .position(|id| *id == m.shift_id)
            .map(|i| i as i64)
            .unwrap_or(-1),
        currency: m.currency.as_str().to_owned(),
        direction: match m.direction {
            Direction::In => "IN".to_owned(),
            Direction::Out => "OUT".to_owned(),
        },
        amount: plain(m.amount.minor_i128()),
        reason: m.reason.clone(),
    }
}

/// The stock change of one completed transaction, as ticket T8 will compute it.
pub fn tx_delta(t: &Tx) -> StockDelta {
    let side = match t.r#type.as_str() {
        "BUY" => Side::Buy,
        "SELL" => Side::Sell,
        other => panic!("fixture type {other}"),
    };
    let amount = money(&t.amount);
    let quote = per_one(
        side,
        amount,
        Rate::parse(&t.rate).expect("fixture rate"),
        money(&t.fee),
    )
    .expect("fixture quote");
    StockDelta::for_tx(side, code(&t.currency), amount, &quote, t.method == "Cash")
        .expect("fixture delta")
}

// ── the fixture ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseCase {
    pub action: String,
    pub payload: Value,
    pub parse_error: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct Tx {
    pub r#type: String,
    pub currency: String,
    pub amount: String,
    pub rate: String,
    pub fee: String,
    pub method: String,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct ShiftSeen {
    pub status: String,
    pub opening: BTreeMap<String, String>,
    pub expected: BTreeMap<String, String>,
    pub counted: Option<BTreeMap<String, String>>,
    pub difference: Option<BTreeMap<String, String>>,
    pub note: String,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct MovementSeen {
    pub shift: i64,
    pub currency: String,
    pub direction: String,
    pub amount: String,
    pub reason: String,
}

#[derive(Deserialize)]
pub struct After {
    pub shifts: Vec<ShiftSeen>,
    pub movements: Vec<MovementSeen>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub r#do: String,
    pub payload: Option<Value>,
    pub tx: Option<Tx>,
    pub request_id: Option<String>,
    pub resolution: Option<String>,
    pub v2_error: Option<String>,
    pub after: After,
}

#[derive(Deserialize)]
pub struct Scenario {
    pub name: String,
    pub steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fixture {
    pub register: Vec<String>,
    pub sentences: BTreeMap<String, u32>,
    pub parse_cases: Vec<ParseCase>,
    pub scenarios: Vec<Scenario>,
}

pub fn fixture() -> Fixture {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v2_shift_cases.json");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("v2_shift_cases.json has the expected shape")
}

/// A payload of the fixture with `"$SHIFT:n"` replaced by the id of the n-th
/// shift this scenario opened.
pub fn resolved(payload: &Value, shift_ids: &[String]) -> Value {
    let mut p = payload.clone();
    if let Some(text) = p.get("shiftId").and_then(Value::as_str) {
        if let Some(n) = text.strip_prefix("$SHIFT:") {
            let n: usize = n.parse().expect("shift index");
            let id = shift_ids
                .get(n)
                .unwrap_or_else(|| panic!("the scenario has not opened shift {n} yet"));
            p["shiftId"] = Value::String(id.clone());
        }
    }
    p
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
