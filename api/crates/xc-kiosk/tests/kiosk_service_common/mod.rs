//! Written before the implementation (ticket T12b). Do not edit, delete or weaken.
//!
//! Shared by `kiosk_service_test.rs`: the database harness, the other contexts
//! as the tests control them (`FakePorts`), the v2 fixture and the function that
//! plays one recorded scenario against `KioskService`.
//!
//! The single `pub use` block below is every path of `xc-kiosk` these tests name.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing
//! database whose name ends in `_test`. When the variable is absent every test
//! FAILS; none of them skips. Each `TestDb` is a schema of its own with
//! `api/migrations/` applied, so tests run in parallel and the migration is
//! exercised from an empty schema every time. SQL in the migration and in the
//! implementation must therefore name tables without a schema prefix.
//!
//! Edited by the author of ticket #328 (row Q1, design §5.5.3): `list` takes a filter and a page
//! and answers a `QueuePage`, `accept` answers an `AcceptOutcome`. `play` prints the outcome in the
//! words of the 10-06 bundle, which the fixture recorded (see `accept_answer`).
//!
//! Edited by the author of ticket #756 (RE-4b, `docs/design/blueprint/rates.md` §4.9 R1, R4):
//! `KioskPorts` answers each currency's published line, so a `CounterRate` carries the line's
//! id, `unit_size` and minor unit. The fake prices per 1 with two-place amounts, as v2 did, and
//! its line id follows the pair of rates (`fake_line`): a save that changes a rate is a new line,
//! as a new publication is.
//!
//! Amended by the ticket author of #603 (RE-6, claude/e5ab6c): `configure` never asks the rate
//! context to save (item 1) and switches a currency on only when the branch's current
//! publication has it on both sides (item 2). `play` therefore, before a `configure` step that
//! switches a currency on and is accepted, publishes the step's rate for its branch (every line
//! published there so far, plus this one) and puts it in the fake counter, as v2's rate save
//! did; and it holds every `configure` step to no rate save. `publish_rates` writes the
//! publication as `../kiosk_publication_common/mod.rs` does; that module is not loaded here (a
//! test file loads it once, beside this one).
//!
//! Amended again by the ticket author of #603 (ruling of 2026-10-09): configure sets no rate, so
//! its `KIOSK_CURRENCY_CONFIGURED` line records `buy` and `sell` as `""`. `play` holds the
//! fixture's recorded configure lines to that (`v3_configure_line`); the fixture is unchanged.
//!
//! Re-expected for RE-7 #604 by the ticket author (claude/07d3765f): RE-7 removes
//! `KioskPorts::save_rate` (`docs/design/blueprint/rates.md` §9), so `FakePorts` no longer
//! implements it and no longer records rate saves (`RateSave`, `rate_saves`, `rate_save_fails`
//! are gone). #603 item 1 ("configure never asks the rate context to save") is now held by the
//! compiler: the port has no method to ask with. `play` leaves the fixture's `rateSaveFails`
//! field unread for that reason; the fixture is unchanged.
#![allow(dead_code, unused_imports)]

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Row};
use tokio::sync::Semaphore;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, MinorUnit, Money, Rate, Side, UnitSize};

pub use xc_kiosk::application::kiosk::service::KioskService;
pub use xc_kiosk::domain::kiosk::commands::configure_currency::ConfigureCurrency;
pub use xc_kiosk::domain::kiosk::commands::new_quote::NewQuote;
pub use xc_kiosk::domain::kiosk::commands::quote_input::QuoteInput;
pub use xc_kiosk::domain::kiosk::entities::kiosk_machine::KioskMachine;
pub use xc_kiosk::domain::kiosk::entities::kiosk_request::KioskRequest;
pub use xc_kiosk::domain::kiosk::repositories::kiosk_ports::KioskPorts;
pub use xc_kiosk::domain::kiosk::value_objects::accept_outcome::AcceptOutcome;
pub use xc_kiosk::domain::kiosk::value_objects::audit_line::{
    AuditLine, CUSTOMER_KIOSK, KioskAudit,
};
pub use xc_kiosk::domain::kiosk::value_objects::counter_facts::{
    CounterFacts, CounterLine, LineId,
};
pub use xc_kiosk::domain::kiosk::value_objects::counter_view::{CounterRate, CounterView};
pub use xc_kiosk::domain::kiosk::value_objects::currency_switch::CurrencySwitch;
pub use xc_kiosk::domain::kiosk::value_objects::customer_receipt::CustomerReceipt;
pub use xc_kiosk::domain::kiosk::value_objects::identified_request::IdentifiedRequest;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_actor::KioskActor;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_currencies::DEFAULT_CODES;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_error::KioskError;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_quote::KioskQuote;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_role::KioskRole;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_service_error::KioskServiceError;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_status::KioskStatus;
pub use xc_kiosk::domain::kiosk::value_objects::kiosk_time::KioskTime;
pub use xc_kiosk::domain::kiosk::value_objects::notifications::Notifications;
pub use xc_kiosk::domain::kiosk::value_objects::public_rates::PublicRates;
pub use xc_kiosk::domain::kiosk::value_objects::queue_filter::QueueFilter;
pub use xc_kiosk::domain::kiosk::value_objects::queue_page::QueuePage;
pub use xc_kiosk::domain::kiosk::value_objects::quote_limit::QuoteLimit;
pub use xc_kiosk::domain::kiosk::value_objects::receipt_image::ReceiptImage;
pub use xc_kiosk::domain::kiosk::value_objects::request_identity::RequestIdentity;
pub use xc_kiosk::domain::kiosk::value_objects::save_attempt::SaveAttempt;
pub use xc_kiosk::domain::kiosk::value_objects::transaction_link::TransactionLink;
pub use xc_kiosk::domain::kiosk::value_objects::work_times::WorkTimes;

/// Upper bound for anything that must finish.
pub const MUST_FINISH: Duration = Duration::from_secs(30);

pub const MAIN: &str = "MAIN";
pub const B2: &str = "B2";
pub const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
pub const TOO_MANY_QUOTES: &str = "Too many requests. Please wait a moment and try again.";
pub const INVALID_IMAGE: &str = "Invalid receipt image.";
pub const NOT_READY: &str = "คิวนี้ไม่พร้อมบันทึกหรือบันทึกแล้ว";
pub const PNG: &str = "data:image/png;base64,iVBORw0KGgo=";
/// The 10-06 bundle's `accept` with nothing to claim (`queue-work.mjs:12`).
pub const NOTHING_TO_CLAIM: &str = "คิวนี้ไม่พร้อมรับเรื่อง กรุณาโหลดล่าสุด";

/// The 10-06 bundle's `accept` of a request somebody holds (`queue-work.mjs:12`).
pub fn held_by(holder: &str) -> String {
    format!("ออเดอร์นี้มี {holder} กำลังทำรายการแล้ว")
}

/// A name no other test, test binary or run on this server has used.
pub fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

pub fn branch(id: &str) -> BranchId {
    BranchId::new(id)
}

/// Await `fut`, failing the test when it takes longer than `limit`.
pub async fn within<T>(limit: Duration, what: &str, fut: impl Future<Output = T>) -> T {
    match tokio::time::timeout(limit, fut).await {
        Ok(value) => value,
        Err(_) => panic!("timed out after {limit:?}: {what}"),
    }
}

/// The sentence of a refusal. Panics on success and on any error that is not
/// `AppError::Domain` — a refusal reaches the screen as its sentence and nothing else.
pub fn refusal<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

// ── time ────────────────────────────────────────────────────────────────────

/// 2026-10-06 05:00:00 UTC, the fixture's `t0`, in milliseconds.
pub const T0: i64 = 1_791_262_800_000;

/// `T0 + ms`, plus 999 microseconds: what a wall clock hands the service. The
/// service must cut it to the millisecond, not round it.
pub fn at(ms: i64) -> DateTime<Utc> {
    exact(ms) + chrono::Duration::microseconds(999)
}

/// `T0 + ms` exactly.
pub fn exact(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(T0 + ms)
        .single()
        .expect("a valid instant")
}

// ── people ──────────────────────────────────────────────────────────────────

pub fn staff(username: &str, role: KioskRole) -> KioskActor {
    KioskActor::Staff {
        username: username.to_owned(),
        role,
    }
}

pub fn front1() -> KioskActor {
    staff("front1", KioskRole::Front)
}

pub fn front2() -> KioskActor {
    staff("front2", KioskRole::Front)
}

pub fn rates1() -> KioskActor {
    staff("rates1", KioskRole::Rates)
}

pub fn owner() -> KioskActor {
    staff("owner", KioskRole::Owner)
}

fn role(text: &str) -> KioskRole {
    match text {
        "OWNER" => KioskRole::Owner,
        "FRONT" => KioskRole::Front,
        "BACKEND" => KioskRole::Backend,
        "MANAGER" => KioskRole::Manager,
        "RATES" => KioskRole::Rates,
        _ => KioskRole::Other,
    }
}

// ── the other contexts, as the tests control them ───────────────────────────

pub fn rate(text: &str) -> Rate {
    Rate::parse(text).unwrap_or_else(|e| panic!("rate {text}: {e:?}"))
}

pub fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).unwrap_or_else(|e| panic!("currency {text}: {e:?}"))
}

/// The fake's line id: one per pair of rates (#756).
pub fn fake_line(buy: Rate, sell: Rate) -> LineId {
    let at_8 = |r: Rate| {
        let mut v = r.value();
        v.rescale(8);
        u128::try_from(v.mantissa()).expect("a rate is positive")
    };
    LineId((at_8(buy) << 64) | at_8(sell))
}

pub fn counter_rate(c: &str, buy: &str, sell: &str) -> CounterRate {
    CounterRate {
        code: code(c),
        buy: rate(buy),
        sell: rate(sell),
        unit_size: UnitSize::ONE,
        minor_unit: MinorUnit::TWO,
        line_id: fake_line(rate(buy), rate(sell)),
    }
}

/// Writes the fake's line as a row of `published_rate_lines` (with the approved request and the
/// one-line publication it needs), in the caller's transaction, unless it is there already:
/// `kiosk_requests.published_rate_line_id` references it (#756, migration 5). The branch's
/// current publication is not moved; the fake answers the lines itself.
pub async fn publish_line(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch: &BranchId,
    rate: &CounterRate,
) -> AppResult<()> {
    let line = rate.line_id.to_string();
    let there: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM published_rate_lines WHERE id = $1::uuid)")
            .bind(&line)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;
    if there {
        return Ok(());
    }
    let request = format!("{line}-request");
    sqlx::query(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
             object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
             status, idempotency_key) \
         SELECT gen_random_uuid(), organization_id, 'RATE_PUBLICATION', id, $2::uuid, 'test', \
             '1', '{}'::jsonb, 'test', now(), 'APPROVED', $3 \
         FROM branches WHERE id = $1",
    )
    .bind(branch.as_str())
    .bind(&line)
    .bind(&request)
    .execute(&mut **tx)
    .await
    .map_err(AppError::Database)?;
    sqlx::query(
        "INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, \
             approved_request_id, draft_id, published_by) \
         SELECT $2::uuid, b.organization_id, b.id, \
             COALESCE((SELECT max(version) FROM rate_publications WHERE branch_id = b.id), 0) + 1, \
             now(), r.id, NULL, 'test' \
         FROM branches b JOIN approval_requests r ON r.idempotency_key = $3 \
         WHERE b.id = $1",
    )
    .bind(branch.as_str())
    .bind(&line)
    .bind(&request)
    .execute(&mut **tx)
    .await
    .map_err(AppError::Database)?;
    sqlx::query(
        "INSERT INTO published_rate_lines (id, organization_id, publication_id, currency_code, \
             denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
         SELECT $1::uuid, organization_id, $1::uuid, $2, 'ALL', $3, $4, $5, \
             '{\"buy_mode\": \"MANUAL\", \"sell_mode\": \"MANUAL\"}'::jsonb \
         FROM rate_publications WHERE id = $1::uuid",
    )
    .bind(&line)
    .bind(rate.code.as_str())
    .bind(rate.unit_size.value())
    .bind(rate.buy.value())
    .bind(rate.sell.value())
    .execute(&mut **tx)
    .await
    .map_err(AppError::Database)?;
    Ok(())
}

#[derive(Default)]
struct World {
    counters: BTreeMap<String, CounterView>,
    audit_fails: bool,
    /// Calls of `counter`, the form that reads committed data.
    counter_reads: usize,
    /// Calls of `counter_in`, the form that reads inside the caller's transaction.
    counter_reads_in: usize,
}

/// Branches, shifts, counter rates and the audit log, in the tests' hands.
///
/// `record_in` writes the line into the table `test_audit` **through the
/// transaction it is given**, so a line is there exactly when the caller's
/// transaction commits. There is no rate save to fake: RE-7 #604 removed
/// `KioskPorts::save_rate`.
pub struct FakePorts {
    world: Mutex<World>,
}

impl FakePorts {
    /// The two branches every fixture scenario starts from.
    pub fn standard() -> Arc<Self> {
        let ports = Self {
            world: Mutex::new(World::default()),
        };
        for (id, view) in fixture()["world"].as_object().expect("world") {
            ports.set(
                id,
                CounterView {
                    branch_name: text(&view["name"]),
                    open: view["open"].as_bool().expect("open"),
                    rates: view["rates"]
                        .as_array()
                        .expect("rates")
                        .iter()
                        .map(|r| {
                            counter_rate(&text(&r["code"]), &text(&r["buy"]), &text(&r["sell"]))
                        })
                        .collect(),
                },
            );
        }
        Arc::new(ports)
    }

    fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().expect("the fake's lock is not poisoned")
    }

    pub fn set(&self, branch: &str, view: CounterView) {
        self.world().counters.insert(branch.to_owned(), view);
    }

    pub fn set_open(&self, branch: &str, open: bool) {
        self.world()
            .counters
            .get_mut(branch)
            .expect("a branch the fake knows")
            .open = open;
    }

    pub fn set_rate(&self, branch: &str, new: CounterRate) {
        let mut world = self.world();
        let view = world
            .counters
            .get_mut(branch)
            .expect("a branch the fake knows");
        match view.rates.iter_mut().find(|r| r.code == new.code) {
            Some(old) => *old = new,
            None => view.rates.push(new),
        }
    }

    /// From now on `record_in` fails after it was called (the line is not written).
    pub fn audit_fails(&self, fails: bool) {
        self.world().audit_fails = fails;
    }

    /// How often the counter was read outside a transaction, and inside one.
    pub fn counter_reads(&self) -> (usize, usize) {
        let world = self.world();
        (world.counter_reads, world.counter_reads_in)
    }
}

fn quote_json(q: &KioskQuote) -> Value {
    json!({
        "type": match q.side { Side::Buy => "BUY", Side::Sell => "SELL" },
        "currency": q.currency.as_str(),
        "rate": q.rate.to_string(),
        "foreign": q.foreign.to_string(),
        "thb": q.thb.to_string(),
        "inputAmount": q.input_amount.to_string(),
        "paymentMethod": q.payment_method.as_str(),
    })
}

impl KioskPorts for FakePorts {
    fn branch_name<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<Option<String>>> {
        Box::pin(async move {
            Ok(self
                .world()
                .counters
                .get(branch.as_str())
                .map(|view| view.branch_name.clone()))
        })
    }

    fn counter<'a>(
        &'a self,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<CounterView>>> {
        Box::pin(async move {
            let mut world = self.world();
            world.counter_reads += 1;
            Ok(world.counters.get(branch.as_str()).cloned())
        })
    }

    fn counter_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<CounterView>>> {
        Box::pin(async move {
            let view = {
                let mut world = self.world();
                world.counter_reads_in += 1;
                world.counters.get(branch.as_str()).cloned()
            };
            // #756: a quote stores its line's id, a foreign key; the line must exist.
            if let Some(view) = &view {
                for rate in &view.rates {
                    publish_line(tx, branch, rate).await?;
                }
            }
            Ok(view)
        })
    }

    fn record_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        line: &'a AuditLine,
        at: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if self.world().audit_fails {
                return Err(AppError::Internal("the audit log is down".to_owned()));
            }
            let detail = match &line.what {
                KioskAudit::Request { quote } => json!({ "quote": quote_json(quote) }),
                KioskAudit::Accepted | KioskAudit::IdChecked => json!({}),
                KioskAudit::Completed { document_ref } => json!({ "documentRef": document_ref }),
                KioskAudit::CurrencyConfigured { enabled, buy, sell } => {
                    json!({ "enabled": enabled, "buy": buy, "sell": sell })
                }
                // Amended by the author of KC-D (claude/studio-claude-10, `kiosk-cancel.md` §3.3):
                // the cancel's line, `{from, reason, note}` as `detail_of` writes it.
                KioskAudit::Cancelled { from, reason } => json!({
                    "from": from.as_str(),
                    "reason": reason.as_str(),
                    "note": reason.note(),
                }),
            };
            sqlx::query(
                "INSERT INTO test_audit (branch_id, actor, kind, subject, detail, at) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(branch.as_str())
            .bind(&line.actor)
            .bind(line.what.kind())
            .bind(&line.subject)
            .bind(detail.to_string())
            .bind(at)
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    }

    /// #769: `KioskPorts::check_stale_in` has no default body, so this fake implements it. It
    /// admits every quote, the behaviour the fake had through the default.
    #[allow(clippy::too_many_arguments)]
    fn check_stale_in<'a>(
        &'a self,
        _tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &'a BranchId,
        _line_id: LineId,
        _currency: CurrencyCode,
        _side: Side,
        _thb: Money,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async { Ok(true) })
    }
}

// ── #603: a rate publication behind the service ─────────────────────────────

/// #603 (ruling of 2026-10-09): a recorded `KIOSK_CURRENCY_CONFIGURED` line as v3 writes it, with
/// `buy` and `sell` `""`; every other line as recorded.
pub fn v3_configure_lines(events: &Value) -> Value {
    let mut events = events.clone();
    for line in events.as_array_mut().into_iter().flatten() {
        if line["kind"] == json!("KIOSK_CURRENCY_CONFIGURED") {
            line["buy"] = json!("");
            line["sell"] = json!("");
        }
    }
    events
}

/// #603 (RE-6): the branch's next rate publication, holding `lines` (code → buy, sell) at the
/// currency's register unit, made the branch's current one, as RE-1's publish leaves it.
pub async fn publish_rates(
    pool: &PgPool,
    branch: &str,
    at: DateTime<Utc>,
    lines: &BTreeMap<String, (String, String)>,
) {
    let mut tx = pool.begin().await.expect("begin the publication");
    let request: String = sqlx::query_scalar(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, \
         policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
         VALUES (gen_random_uuid(), 'RATE_PUBLICATION', $1, 'seeded', 'seeded', '1', '{}'::jsonb, \
         'test', $2, 'APPROVED', gen_random_uuid()::text, '00000000-0000-0000-0000-000000000001') RETURNING id::text",
    )
    .bind(branch)
    .bind(at)
    .fetch_one(&mut *tx)
    .await
    .expect("the approval request");
    let publication: String = sqlx::query_scalar(
        "INSERT INTO rate_publications (id, branch_id, version, published_at, \
         approved_request_id, draft_id, published_by) \
         SELECT gen_random_uuid(), $1, COALESCE(MAX(version), 0) + 1, $2, $3::uuid, NULL, 'test' \
         FROM rate_publications WHERE branch_id = $1 RETURNING id::text",
    )
    .bind(branch)
    .bind(at)
    .bind(&request)
    .fetch_one(&mut *tx)
    .await
    .expect("the publication");
    for (code, (buy, sell)) in lines {
        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, \
             denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
             VALUES (gen_random_uuid(), $1::uuid, $2, 'ALL', \
             (SELECT unit_size FROM currencies WHERE code = $2), $3::numeric, $4::numeric, \
             '{\"partner\": null}'::jsonb)",
        )
        .bind(&publication)
        .bind(code)
        .bind(buy)
        .bind(sell)
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|e| panic!("the line {code}: {e}"));
    }
    sqlx::query("UPDATE branches SET current_publication_id = $1::uuid WHERE id = $2")
        .bind(&publication)
        .bind(branch)
        .execute(&mut *tx)
        .await
        .expect("the branch's current publication");
    tx.commit().await.expect("commit the publication");
}

/// #603: `publish_rates` of one currency.
pub async fn publish_one(
    pool: &PgPool,
    branch: &str,
    at: DateTime<Utc>,
    c: &str,
    buy: &str,
    sell: &str,
) {
    let lines = BTreeMap::from([(c.to_owned(), (buy.to_owned(), sell.to_owned()))]);
    publish_rates(pool, branch, at, &lines).await;
}

// ── the database ────────────────────────────────────────────────────────────

pub struct TestDb {
    /// A pool of the test's own, for looking at what was stored.
    pub pool: PgPool,
    /// The pool the service works on.
    service_pool: PgPool,
    schema: String,
}

/// Schemas being migrated at once; every one opens its own pools.
static STARTING: Semaphore = Semaphore::const_new(4);

fn options() -> PgConnectOptions {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "TEST_DATABASE_URL is not set.\n\
             The xc-kiosk service tests need a real PostgreSQL and do not skip. Point it at an \
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
    options
}

impl TestDb {
    /// A fresh schema with `api/migrations/` applied, the branch `B2` beside
    /// `MAIN`, and the table the fake audit log writes to.
    pub async fn start() -> Self {
        Self::start_with(8).await
    }

    /// The same, with the service confined to `connections` connections.
    pub async fn start_with(connections: u32) -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let options = options();
        let schema = unique("t12b");
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

        let in_schema = options.options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(MUST_FINISH)
            .connect_with(in_schema.clone())
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");
        sqlx::query("INSERT INTO branches (id, name) VALUES ('B2', 'Second')")
            .execute(&pool)
            .await
            .expect("insert the second branch");
        sqlx::query(
            "CREATE TABLE test_audit (seq bigserial PRIMARY KEY, branch_id text NOT NULL, \
             actor text NOT NULL, kind text NOT NULL, subject text NOT NULL, \
             detail text NOT NULL, at timestamptz NOT NULL)",
        )
        .execute(&pool)
        .await
        .expect("create the fake audit log");

        // A statement that goes round the transaction it should be in needs a second
        // connection; with one connection it cannot get one and fails after this timeout.
        let service_pool = PgPoolOptions::new()
            .max_connections(connections)
            .acquire_timeout(if connections == 1 {
                Duration::from_secs(10)
            } else {
                MUST_FINISH
            })
            .connect_with(in_schema)
            .await
            .expect("connect the service to the test schema");

        Self {
            pool,
            service_pool,
            schema,
        }
    }

    pub fn branch_tx(&self) -> BranchTx {
        BranchTx::new(Transactor::new(self.service_pool.clone()))
    }

    pub fn service(&self, ports: Arc<FakePorts>) -> KioskService {
        KioskService::new(self.branch_tx(), ports)
    }

    /// A transaction as the transaction save would hand it over: on the
    /// service's pool, holding the branch's lock.
    pub async fn begin(&self) -> sqlx::Transaction<'static, sqlx::Postgres> {
        self.service_pool.begin().await.expect("begin")
    }

    pub async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    /// Every stored request, as the fixture writes `rows`.
    pub async fn rows(&self) -> Value {
        let rows = sqlx::query(
            "SELECT branch_id, id, status, staff, document_ref, submission_id \
             FROM kiosk_requests ORDER BY branch_id, id",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read kiosk_requests");
        Value::Array(
            rows.iter()
                .map(|r| {
                    json!({
                        "branch": r.get::<String, _>(0), "id": r.get::<String, _>(1),
                        "status": r.get::<String, _>(2), "staff": r.get::<Option<String>, _>(3),
                        "documentRef": r.get::<Option<String>, _>(4),
                        "submissionId": r.get::<Option<String>, _>(5),
                    })
                })
                .collect(),
        )
    }

    /// The stored status of one request; `None` when there is no such row.
    pub async fn status(&self, branch_id: &str, id: &str) -> Option<String> {
        sqlx::query_scalar("SELECT status FROM kiosk_requests WHERE branch_id = $1 AND id = $2")
            .bind(branch_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .expect("read kiosk_requests")
    }

    /// Every stored switch, as the fixture writes `switches`.
    pub async fn switches(&self) -> Value {
        let rows = sqlx::query(
            "SELECT branch_id, code::text, enabled FROM kiosk_currencies ORDER BY branch_id, code",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read kiosk_currencies");
        Value::Array(
            rows.iter()
                .map(|r| {
                    json!({ "branch": r.get::<String, _>(0), "code": r.get::<String, _>(1).trim(),
                            "enabled": r.get::<bool, _>(2) })
                })
                .collect(),
        )
    }

    /// The audit lines written after line `after`, as the fixture writes
    /// `events`, with the number of the last one and their times.
    pub async fn audit_since(&self, after: i64) -> (Value, i64, Vec<DateTime<Utc>>) {
        let rows = sqlx::query(
            "SELECT seq, branch_id, actor, kind, subject, detail, at FROM test_audit \
             WHERE seq > $1 ORDER BY seq",
        )
        .bind(after)
        .fetch_all(&self.pool)
        .await
        .expect("read test_audit");
        let mut last = after;
        let mut times = Vec::new();
        let lines = rows
            .iter()
            .map(|r| {
                last = r.get::<i64, _>(0);
                times.push(r.get::<DateTime<Utc>, _>(6));
                let mut line = json!({
                    "branch": r.get::<String, _>(1), "actor": r.get::<String, _>(2),
                    "kind": r.get::<String, _>(3), "subject": r.get::<String, _>(4),
                });
                let detail: Value =
                    serde_json::from_str(&r.get::<String, _>(5)).expect("detail is JSON");
                for (k, v) in detail.as_object().expect("detail is an object") {
                    line[k] = v.clone();
                }
                line
            })
            .collect();
        (Value::Array(lines), last, times)
    }

    /// Takes the lock `BranchTx` takes on `branch_id`, on a connection of its
    /// own, and keeps it until the returned guard is released.
    pub async fn hold_branch(&self, branch_id: &str) -> HeldBranch {
        let mut tx = self.pool.begin().await.expect("begin");
        sqlx::query(
            "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 0) \
             ON CONFLICT (branch_id) DO NOTHING",
        )
        .bind(branch_id)
        .execute(&mut *tx)
        .await
        .expect("create the lock row");
        tx.commit().await.expect("commit the lock row");
        let mut tx = self.pool.begin().await.expect("begin");
        sqlx::query("SELECT version FROM branch_versions WHERE branch_id = $1 FOR UPDATE")
            .bind(branch_id)
            .execute(&mut *tx)
            .await
            .expect("lock the branch");
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .expect("pid");
        HeldBranch { tx, pid }
    }

    /// Waits until some other connection is blocked by the lock `held` holds —
    /// read from PostgreSQL itself, not guessed from elapsed time.
    pub async fn until_blocked_by(&self, held: &HeldBranch) {
        within(MUST_FINISH, "a command waits for the branch lock", async {
            loop {
                let blocked: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid))",
                )
                .bind(held.pid)
                .fetch_one(&self.pool)
                .await
                .expect("read pg_stat_activity");
                if blocked > 0 {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
    }

    /// Drop the schema. Reached only by a test that passed; a failed test
    /// leaves its schema behind for inspection.
    pub async fn finish(self) {
        self.pool.close().await;
        self.service_pool.close().await;
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options())
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

pub struct HeldBranch {
    tx: sqlx::Transaction<'static, sqlx::Postgres>,
    pid: i32,
}

impl HeldBranch {
    pub async fn release(self) {
        self.tx.commit().await.expect("release the branch lock");
    }
}

// ── building commands ───────────────────────────────────────────────────────

/// A quote of `amount` of `currency`; ids `q1`, `q2`, … are enough for a hand-written test.
pub fn quote_of(id: &str, side: Side, currency: &str, amount: &str) -> NewQuote {
    NewQuote {
        input: QuoteInput {
            side: Some(side),
            currency: CurrencyCode::parse(currency).ok(),
            amount: Money::parse(amount).ok(),
            payment_method: None,
        },
        new_id: id.to_owned(),
        client_ip: "203.0.113.7".to_owned(),
    }
}

/// Buying 100 dollars from the customer, the plainest quote there is.
pub fn buy100(id: &str) -> NewQuote {
    quote_of(id, Side::Buy, "USD", "100")
}

pub fn from_ip(mut quote: NewQuote, ip: &str) -> NewQuote {
    quote.client_ip = ip.to_owned();
    quote
}

pub fn configure_of(c: &str, enabled: bool, buy: &str, sell: &str) -> ConfigureCurrency {
    ConfigureCurrency {
        code: c.to_owned(),
        enabled,
        buy: buy.to_owned(),
        sell: sell.to_owned(),
        previous_updated: String::new(),
        partner_name: String::new(),
        partner_cost: None,
        request_id: unique("req"),
    }
}

/// The save of the transaction that `buy100` asks for.
pub fn save100(submission_id: &str) -> SaveAttempt {
    SaveAttempt {
        live: true,
        side: Side::Buy,
        currency: code("USD"),
        amount: Money::parse("100").expect("100"),
        method: "Cash".to_owned(),
        submission_id: submission_id.to_owned(),
    }
}

pub fn link(submission_id: &str, document_ref: &str) -> TransactionLink {
    TransactionLink {
        submission_id: submission_id.to_owned(),
        document_ref: document_ref.to_owned(),
    }
}

/// Quote `id` at 0 ms, confirm at 4,000, `front1` accepts at 5,000 and confirms
/// the passport check at 6,000. Returns after the step named by `upto`
/// (`"quoted"`, `"waiting"`, `"accepted"`, `"checked"`).
pub async fn bring(service: &KioskService, branch_id: &str, id: &str, upto: &str) {
    let b = branch(branch_id);
    service
        .quote(&b, buy100(id), at(0))
        .await
        .expect("the quote is made");
    if upto == "quoted" {
        return;
    }
    service.confirm(&b, id, at(4000)).await.expect("confirmed");
    if upto == "waiting" {
        return;
    }
    service
        .accept(&b, &front1(), id, at(5000))
        .await
        .expect("accepted");
    if upto == "accepted" {
        return;
    }
    service
        .identity_checked(&b, &front1(), id, true, at(6000))
        .await
        .expect("passport checked");
    assert_eq!(upto, "checked", "bring: unknown step {upto}");
}

// ── the v2 fixture ──────────────────────────────────────────────────────────

pub fn fixture() -> &'static Value {
    static FIXTURE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    FIXTURE.get_or_init(|| {
        serde_json::from_str(include_str!("../fixtures/v2_kiosk_service_cases.json"))
            .expect("tests/fixtures/v2_kiosk_service_cases.json parses")
    })
}

fn text(v: &Value) -> String {
    v.as_str()
        .unwrap_or_else(|| panic!("the fixture has {v} where a text was expected"))
        .to_owned()
}

fn opt_text(v: &Value) -> Option<String> {
    v.as_str().map(str::to_owned)
}

fn side(v: &Value) -> Option<Side> {
    match v.as_str() {
        Some("BUY") => Some(Side::Buy),
        Some("SELL") => Some(Side::Sell),
        _ => None,
    }
}

fn actor_of(who: &str) -> KioskActor {
    let person = &fixture()["people"][who];
    if person.is_null() {
        assert_eq!(who, "customer", "the fixture has nobody called {who}");
        return KioskActor::Customer;
    }
    staff(&text(&person["username"]), role(&text(&person["role"])))
}

pub fn identity_json(i: &RequestIdentity) -> Value {
    json!({ "id": i.id, "queue": i.queue, "orderNo": i.order_no,
            "transactionNo": i.transaction_no, "status": i.status.as_str() })
}

/// `{png: n}` is a PNG data URL of exactly n characters; `{text}` is sent as it is.
pub fn image_of(spec: &Value) -> String {
    match spec["text"].as_str() {
        Some(literal) => literal.to_owned(),
        None => {
            let prefix = "data:image/png;base64,";
            let n = spec["png"].as_u64().expect("image.png") as usize;
            format!("{prefix}{}", "A".repeat(n - prefix.len()))
        }
    }
}

/// Play one fixture scenario against a fresh schema and return every place where
/// the service answered, stored or logged something else than v2 (or than the
/// step's `v3` entry, where v3 differs on purpose).
pub async fn play(name: &str) -> Vec<String> {
    let scenario = fixture()["scenarios"]
        .as_array()
        .expect("scenarios")
        .iter()
        .find(|s| s["name"] == name)
        .unwrap_or_else(|| panic!("the fixture has no scenario '{name}'"));
    let db = TestDb::start().await;
    let ports = FakePorts::standard();
    let service = db.service(ports.clone());
    let mut wrong = Vec::new();
    let mut audit_seen = 0;
    // #603: per branch, the lines published so far
    let mut published: BTreeMap<String, BTreeMap<String, (String, String)>> = BTreeMap::new();

    for (i, step) in scenario["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .enumerate()
    {
        let op = text(&step["op"]);
        let ms = step["at"].as_i64().expect("at");
        let now = at(ms);
        let b = branch(&text(&step["branch"]));
        let who = text(&step["who"]);
        let actor = actor_of(&who);
        let id = step["id"].as_str().unwrap_or("");
        let here = format!("'{name}' step {} ({op} by {who} at {ms} ms)", i + 1);

        if op == "counter" {
            if let Some(open) = step["open"].as_bool() {
                ports.set_open(b.as_str(), open);
            }
            for r in step["rates"].as_array().into_iter().flatten() {
                ports.set_rate(
                    b.as_str(),
                    counter_rate(&text(&r["code"]), &text(&r["buy"]), &text(&r["sell"])),
                );
            }
            continue;
        }

        if op == "configure" && step["enabled"].as_bool() == Some(true) {
            let v3_error = step["v3"].get("error").is_some_and(|e| !e.is_null());
            if !v3_error && step["error"].is_null() {
                let (c, buy, sell) = (text(&step["code"]), text(&step["buy"]), text(&step["sell"]));
                let lines = published.entry(b.as_str().to_owned()).or_default();
                lines.insert(c.clone(), (buy.clone(), sell.clone()));
                publish_rates(&db.pool, b.as_str(), now, lines).await;
                ports.set_rate(b.as_str(), counter_rate(&c, &buy, &sell));
            }
        }
        let got: AppResult<Value> = match op.as_str() {
            "rates" => service.public_rates(&b).await.map(|r| {
                json!({
                    "branchId": r.branch_id.as_str(), "branchName": r.branch_name, "open": r.open,
                    "rates": r.rates.iter().map(|x| json!({ "code": x.code.as_str(),
                        "buy": x.buy.to_string(), "sell": x.sell.to_string() })).collect::<Vec<_>>(),
                })
            }),
            "quote" => {
                let cmd = NewQuote {
                    input: QuoteInput {
                        side: side(&step["type"]),
                        currency: step["currency"]
                            .as_str()
                            .and_then(|c| CurrencyCode::parse(c).ok()),
                        amount: step["amount"].as_str().and_then(|a| Money::parse(a).ok()),
                        payment_method: opt_text(&step["paymentMethod"]),
                    },
                    new_id: text(&step["newId"]),
                    client_ip: "203.0.113.7".to_owned(),
                };
                service.quote(&b, cmd, now).await.map(|q| {
                    let mut out = identity_json(&q.identity);
                    out["quote"] = quote_json(&q.request.quote);
                    out["created"] = json!(q.request.created.epoch_millis() - T0);
                    out
                })
            }
            "confirm" => service
                .confirm(&b, id, now)
                .await
                .map(|i| identity_json(&i)),
            "status" => service.status(&b, id, now).await.map(|i| identity_json(&i)),
            "identityChecked" => service
                .identity_checked(
                    &b,
                    &actor,
                    id,
                    step["confirmed"].as_bool().expect("confirmed"),
                    now,
                )
                .await
                .map(|i| identity_json(&i)),
            "customerReceipt" => service.customer_receipt(&b, id, now).await.map(|r| {
                let mut out = identity_json(&r.identity);
                out["image"] = match r.image {
                    Some(image) => json!({ "length": image.len() }),
                    None => Value::Null,
                };
                out
            }),
            "notifications" => service.notifications(&b, &actor).await.map(|n| {
                json!({ "branchName": n.branch_name, "count": n.count, "latest": n.latest })
            }),
            "list" => service.list(&b, &actor, QueueFilter::All, 0, now).await.map(|page| {
                Value::Array(
                    page.items
                        .iter()
                        .map(|x| {
                            let mut out = identity_json(&x.identity);
                            out["staff"] = json!(x.request.staff);
                            out["created"] = json!(x.request.created.epoch_millis() - T0);
                            out["quote"] = quote_json(&x.request.quote);
                            out
                        })
                        .collect(),
                )
            }),
            "accept" => {
                let outcome = service.accept(&b, &actor, id, now).await;
                if let Ok(AcceptOutcome::Claimed(ref times)) = outcome {
                    if let Some(started) = times.started_at {
                        let _ = sqlx::query(
                            "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, kiosk_request_id, counter_teller_id, issued_at, served_at, idempotency_key) \
                             VALUES (gen_random_uuid(), '00000000-0000-0000-0000-000000000001'::uuid, $1, current_date, 'WALK_IN', (SELECT COALESCE(MAX(sequence), 0) + 1 FROM queue_tickets), 'A001', 'SERVING', $2, NULL, now(), $3, $4) \
                             ON CONFLICT (branch_id, kiosk_request_id) DO UPDATE SET served_at = COALESCE(queue_tickets.served_at, EXCLUDED.served_at)",
                        )
                        .bind(b.as_str())
                        .bind(id)
                        .bind(exact(started.epoch_millis() - T0))
                        .bind(format!("ticket-{}-{id}", b.as_str()))
                        .execute(&db.pool)
                        .await;
                    }
                }
                outcome.and_then(accept_answer)
            }
            "get" => service.get(&b, &actor, id).await.map(|r| {
                let queue = KioskMachine::identity(Some(&r), kiosk_time(now))
                    .expect("identity of a stored request")
                    .queue;
                json!({ "id": r.id, "queue": queue, "quote": quote_json(&r.quote) })
            }),
            "publishReceipt" => service
                .publish_receipt(&b, &actor, id, &image_of(&step["image"]), now)
                .await
                .map(|()| Value::Null),
            "manage" => service.manage(&b, &actor).await.map(|all| {
                let codes: Vec<&str> = all.iter().map(|s| s.code.as_str()).collect();
                let listed: Vec<&str> = fixture()["limits"]["currencies"]
                    .as_array()
                    .expect("currencies")
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                if codes != listed {
                    wrong.push(format!(
                        "{here}: manage lists {} currencies in another order or number than v2's {}",
                        codes.len(),
                        listed.len()
                    ));
                }
                json!({ "enabled": all.iter().filter(|s| s.enabled).map(|s| s.code.as_str()).collect::<Vec<_>>() })
            }),
            "configure" => {
                let cmd = ConfigureCurrency {
                    code: text(&step["code"]),
                    enabled: step["enabled"].as_bool().expect("enabled"),
                    buy: text(&step["buy"]),
                    sell: text(&step["sell"]),
                    previous_updated: opt_text(&step["previousUpdated"]).unwrap_or_default(),
                    partner_name: opt_text(&step["partnerName"]).unwrap_or_default(),
                    partner_cost: opt_text(&step["partnerCost"]),
                    request_id: unique("req"),
                };
                let out = service.configure(&b, &actor, cmd, now).await;
                out.map(|()| Value::Null)
            }
            "checkSave" | "complete" => {
                let save = SaveAttempt {
                    live: step["live"].as_bool().unwrap_or(true),
                    side: side(&step["type"]).expect("type"),
                    currency: code(&text(&step["currency"])),
                    amount: Money::parse(&text(&step["amount"])).expect("amount"),
                    method: text(&step["method"]),
                    submission_id: text(&step["submissionId"]),
                };
                // The transaction save hands its own transaction over.
                let mut tx = db.begin().await;
                let out = if op == "checkSave" {
                    service.check_save_in(&mut tx, &b, id, &save, &actor).await
                } else {
                    let l = link(&save.submission_id, &text(&step["documentRef"]));
                    service.complete_in(&mut tx, &b, id, &l, &actor, now).await
                };
                if op == "complete" && out.is_ok() {
                    let _ = sqlx::query(
                        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, kiosk_request_id, counter_teller_id, issued_at, completed_at, idempotency_key) \
                         VALUES (gen_random_uuid(), '00000000-0000-0000-0000-000000000001'::uuid, $1, current_date, 'WALK_IN', (SELECT COALESCE(MAX(sequence), 0) + 1 FROM queue_tickets), 'A001', 'COMPLETED', $2, NULL, now(), $3, $4) \
                         ON CONFLICT (branch_id, kiosk_request_id) DO UPDATE SET completed_at = COALESCE(queue_tickets.completed_at, EXCLUDED.completed_at)",
                    )
                    .bind(b.as_str())
                    .bind(id)
                    .bind(exact(ms))
                    .bind(format!("ticket-{}-{id}", b.as_str()))
                    .execute(&mut *tx)
                    .await;
                }
                match &out {
                    Ok(()) => tx.commit().await.expect("commit"),
                    Err(_) => tx.rollback().await.expect("rollback"),
                }
                out.map(|()| Value::Null)
            }
            other => panic!("{here}: the fixture has an operation this test does not know: {other}"),
        };

        // `v3` holds what v3 answers and stores where it differs from v2 on purpose.
        let v3 = &step["v3"];
        let outcome = if v3.get("error").is_some() || v3.get("answer").is_some() {
            v3
        } else {
            step
        };
        let expected = |key: &str| {
            if v3[key].is_null() {
                &step[key]
            } else {
                &v3[key]
            }
        };
        let want_error = outcome["error"].as_str();
        match (&got, want_error) {
            (Ok(answer), None) => {
                if answer != &outcome["answer"] {
                    wrong.push(format!(
                        "{here}: answered {answer}, expected {}",
                        outcome["answer"]
                    ));
                }
            }
            (Err(AppError::Domain(sentence)), Some(want)) if sentence == want => {}
            (got, want) => wrong.push(format!("{here}: answered {got:?}, expected error {want:?}")),
        }

        let (lines, last, times) = db.audit_since(audit_seen).await;
        audit_seen = last;
        let want_lines = if v3["error"].is_null() {
            v3_configure_lines(&step["events"])
        } else {
            json!([])
        };
        if lines != want_lines {
            wrong.push(format!(
                "{here}: wrote the audit lines {lines}, expected {want_lines}"
            ));
        }
        if times.iter().any(|t| *t != exact(ms)) {
            wrong.push(format!(
                "{here}: an audit line carries {times:?}, expected {}",
                exact(ms)
            ));
        }
        let rows = db.rows().await;
        if &rows != expected("rows") {
            wrong.push(format!(
                "{here}: stored requests are {rows}, expected {}",
                expected("rows")
            ));
        }
        let switches = db.switches().await;
        if switches != step["switches"] {
            wrong.push(format!(
                "{here}: stored switches are {switches}, expected {}",
                step["switches"]
            ));
        }
        // #603 item 1 (configure asks for no rate save) needs no check here since RE-7 #604:
        // `KioskPorts` has no `save_rate` to call.
    }

    if wrong.is_empty() {
        db.finish().await;
    }
    wrong
}

/// An `AcceptOutcome` in the words the 10-06 bundle answers with (`queue-work.mjs:6-13`): a claim and
/// the holder again are answers with the times, in milliseconds after `T0`; `HeldBy` and
/// `Unavailable` are its two refusals, as the route says them (row K-HTTP). The service writes nothing
/// for the last three.
pub fn accept_answer(outcome: AcceptOutcome) -> AppResult<Value> {
    let answer = |claimed: bool, t: WorkTimes| {
        json!({
            "claimed": claimed,
            "startedAt": t.started_at.map(|s| s.epoch_millis() - T0),
            "completedAt": t.completed_at.map(|c| c.epoch_millis() - T0),
            "durationSeconds": t.duration_seconds(),
        })
    };
    match outcome {
        AcceptOutcome::Claimed(t) => Ok(answer(true, t)),
        AcceptOutcome::AlreadyMine(t) => Ok(answer(false, t)),
        AcceptOutcome::HeldBy(holder) => Err(AppError::Domain(held_by(&holder))),
        AcceptOutcome::Unavailable => Err(AppError::Domain(NOTHING_TO_CLAIM.to_owned())),
    }
}

/// `now` as the machine counts it.
pub fn kiosk_time(now: DateTime<Utc>) -> KioskTime {
    KioskTime::from_epoch_millis(now.timestamp_millis())
}
