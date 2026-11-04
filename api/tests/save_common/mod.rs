//! Written before the implementation (ticket T8b). Do not edit, delete or weaken.
//!
//! Shared by the save tests: the database harness, a shop that can be prepared
//! step by step, and the v2 fixture.
//!
//! These tests need a real PostgreSQL. `TEST_DATABASE_URL` must name an existing
//! database whose name ends in `_test`. When the variable is absent every test
//! FAILS; none of them skips. Each `Shop` is a schema of its own with
//! `api/migrations/` applied and an upload directory of its own.
#![allow(dead_code)]

/// Publishing through the rate engine (#757 RE-4c): the helpers of `rates_re4c_common`, included
/// as an inline module rather than loaded as a file, so that a binary that also loads
/// `snapshot_common` (which loads `rates_re4c_common/mod.rs`) loads no file twice.
pub mod rates_re4c_common {
    #![allow(dead_code)]
    include!("../rates_re4c_common/publish.rs");
}

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use serde::Deserialize;
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use tokio::sync::Semaphore;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};

pub use xc_customer::application::customer::service::CustomerService;
pub use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
pub use xc_exchange::application::exchange::service::TransactionService;
pub use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
#[allow(unused_imports)]
pub use xc_exchange::domain::exchange::entities::transaction::Transaction;
pub use xc_exchange::infrastructure::exchange::transaction_repository::PgTransactionRepository;
pub use xc_kernel::mode::Mode;
pub use xc_shift::application::shift::service::ShiftService;
#[allow(unused_imports)]
pub use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;
pub use xchange::application::workflows::save_transaction::{
    SaveTransactionWorkflow, SavedTransaction,
};
pub use xchange::infrastructure::files::file_service::FileService;

/// Upper bound for anything that must finish.
pub const MUST_FINISH: Duration = Duration::from_secs(60);

pub const MAIN: &str = "MAIN";
/// Who saves, in every test that does not say otherwise.
pub const OWNER: &str = "owner";

// The sentences v2 answers with in this context.
pub const BAD_BRANCH: &str = "สาขาไม่ถูกต้อง";
pub const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
pub const BAD_ATTACHMENT: &str = "ไฟล์แนบไม่ถูกต้อง";
pub const ATTACHMENT_NOT_FOUND: &str = "ไม่พบไฟล์แนบของสาขานี้";
pub const ATTACHMENT_NOT_READY: &str = "ไฟล์แนบยังไม่พร้อม";
pub const BAD_TEXT: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";
pub const FILE_MISSING: &str = "แนบเอกสารไม่ครบ";
pub const NOT_THIS_PASSPORT: &str = "รหัสลูกค้าไม่ตรงกับพาสปอร์ต กรุณายืนยันข้อมูลลูกค้าใหม่";
pub const CHANGED: &str = "รายการเดิมมีข้อมูลเปลี่ยน ห้ามใช้รหัสส่งซ้ำกับข้อมูลใหม่";
pub const NO_OPEN_SHIFT: &str = "กรุณาเปิดกะจากหน้าแอดมินก่อน";
pub const RATE_MISMATCH: &str = "เรทไม่ตรงกับเรทหน้าร้านล่าสุด กรุณาตรวจสอบเรทก่อนยืนยันรายการ";
pub const NOT_ENOUGH_USD: &str = "ยอด USD ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน";
pub const NOT_ENOUGH_THB: &str = "ยอด THB ไม่เพียงพอ กรุณาตรวจนับหรือบันทึกรับเงินเข้าก่อน";

pub const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];
pub const PDF: &[u8] = b"%PDF-1.7 a slip\n";

/// A name no other test, test binary or run on this server has used.
pub fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

pub fn branch(id: &str) -> BranchId {
    BranchId::new(id)
}

pub fn code(text: &str) -> CurrencyCode {
    CurrencyCode::parse(text).unwrap_or_else(|_| panic!("{text} is a currency"))
}

pub fn money(text: &str) -> Money {
    Money::parse(text).unwrap_or_else(|e| panic!("{text} is an amount: {e:?}"))
}

/// 2026-10-06 05:00:00 UTC (noon in Bangkok) plus `seconds`, with microseconds
/// that are not whole milliseconds: what a wall clock hands the service.
pub fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_791_262_800 + seconds, 123_456_000)
        .single()
        .expect("a valid instant")
}

/// The same instant with everything below the millisecond dropped.
pub fn millis(t: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(t.timestamp_millis())
        .single()
        .expect("a valid instant")
}

pub fn instant(iso: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(iso)
        .unwrap_or_else(|e| panic!("{iso} is an instant: {e}"))
        .with_timezone(&Utc)
}

/// The sentence of a refusal. Panics on success and on any error that is not
/// `AppError::Domain` — a refusal reaches the screen as its sentence and nothing else.
pub fn refusal<T: std::fmt::Debug>(result: AppResult<T>) -> String {
    match result {
        Err(AppError::Domain(sentence)) => sentence,
        other => panic!("expected Err(AppError::Domain(<sentence>)), got {other:?}"),
    }
}

/// SHA-256 of `bytes` as PostgreSQL computes it, in lower-case hex.
pub async fn sha256(pool: &PgPool, bytes: &[u8]) -> String {
    sqlx::query_scalar("SELECT encode(sha256($1), 'hex')")
        .bind(bytes)
        .fetch_one(pool)
        .await
        .expect("PostgreSQL computes a SHA-256")
}

/// A complete form for a cash purchase of 100 USD at 32.1 by the customer the
/// shop's `reserve` makes. `submission` must be 16 to 60 characters.
pub fn form(submission: &str, customer_id: &str, passport: &str) -> SaveTransaction {
    SaveTransaction {
        customer: person(),
        submission_id: submission.to_owned(),
        customer_id: customer_id.to_owned(),
        side: "BUY".to_owned(),
        currency: "USD".to_owned(),
        amount: "100".to_owned(),
        rate: "32.1".to_owned(),
        fee: "0".to_owned(),
        bank_fee: "0".to_owned(),
        method: "Cash".to_owned(),
        payment_ref: String::new(),
        account: "Cash drawer 1".to_owned(),
        staff: "EMP-001".to_owned(),
        booking: "WALK-IN".to_owned(),
        notes: String::new(),
        value_date: "2026-10-06".to_owned(),
        paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
        payment_confirmed: true,
        identity_confirmed: true,
        passport: Some(passport.to_owned()),
        slip: None,
        reference_rate: None,
    }
}

pub fn person() -> ReserveCustomer {
    ReserveCustomer {
        name: "Somchai Jaidee".to_owned(),
        passport_no: "AA1234567".to_owned(),
        nationality: "THA".to_owned(),
        birth_date: String::new(),
        passport_expiry: String::new(),
        mode: "LIVE".to_owned(),
    }
}

/// What a test compares of a branch: the open shift's expected balances
/// (`None` when no shift is open), without the zeroes, as `code -> "0.00"`.
pub type Seen = Option<BTreeMap<String, String>>;

pub struct Shop {
    pub pool: PgPool,
    /// The upload directory of this test.
    pub dir: PathBuf,
    pub files: FileService,
    pub shifts: ShiftService,
    pub customers: CustomerService,
    pub transactions: TransactionService,
    pub workflow: SaveTransactionWorkflow,
    schema: String,
    scratch: PathBuf,
}

/// Schemas being migrated at once; every one opens its own pool.
static STARTING: Semaphore = Semaphore::const_new(4);

impl Shop {
    /// A fresh schema with `api/migrations/` applied (so `MAIN` exists and
    /// nothing else), an empty upload directory, and every service on them.
    pub async fn start() -> Self {
        let _slot = STARTING.acquire().await.expect("semaphore is never closed");
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set.\n\
                 The save tests need a real PostgreSQL and do not skip. Point it at an \
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

        let schema = unique("t8b");
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

        // few connections each: every test of this file runs at once, and a
        // PostgreSQL out of the box takes a hundred
        let schema_options = options.options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(40)
            .acquire_timeout(MUST_FINISH)
            .connect_with(schema_options)
            .await
            .expect("connect to the test schema");

        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .unwrap_or_else(|e| panic!("read migrations from {}: {e}", migrations.display()))
            .run(&pool)
            .await
            .expect("apply api/migrations to the test schema");

        for opener in ["test"] {
            seed_opener(&pool, opener).await;
        }
        install_test_pii_keys();

        let scratch = std::env::temp_dir().join(format!("xc-{schema}"));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("create a scratch directory");
        let scratch = scratch
            .canonicalize()
            .expect("the scratch directory exists");
        let dir = scratch.join("uploads");

        let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
        let files = FileService::new(pool.clone(), dir.clone());
        let shifts = ShiftService::new(
            Arc::new(
                xc_cash::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository(
                    pool.clone(),
                ),
            ),
            branch_tx.clone(),
        );
        let customers = CustomerService::new(branch_tx.clone());
        let transactions = TransactionService::new(
            Arc::new(PgTransactionRepository::new(pool.clone())),
            branch_tx.clone(),
        );
        // [required] the workflow is built from exactly these three
        let workflow = SaveTransactionWorkflow::new(branch_tx, shifts.clone(), files.clone());

        Self {
            pool,
            dir,
            files,
            shifts,
            customers,
            transactions,
            workflow,
            schema,
            scratch,
        }
    }

    /// Add a branch (the table is ticket T5a's; `MAIN`, named `XC-B`, is there already).
    pub async fn seed_branch(&self, id: &str, name: &str) {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $2)")
            .bind(id)
            .bind(name)
            .execute(&self.pool)
            .await
            .expect("insert into branches");
    }

    /// The counter rate of a currency from now on (the table is ticket T5a's).
    ///
    /// Amended for #757 (RE-4c, `docs/design/blueprint/rates.md` D-3, §4.9 R4): the counter rate is
    /// the line of the branch's current publication, no longer the newest `rates` row.
    ///
    /// Re-expected for RE-7 #604 by the ticket author (rates.md §8 migration 6 drops `rates`): the
    /// rate, per 1 as the direct save took it, is published × `currencies.unit_size` (as
    /// migration 4 converts a row) over the branch's current publication, with no `rates` row —
    /// the same rate set the newest rows published before.
    pub async fn set_rate(&self, branch_id: &str, currency: &str, buy: &str, sell: &str) {
        xc_testdb::seed_branch_currencies(&self.pool, branch_id, &[currency])
            .await
            .expect("seed branch currencies");
        rates_re4c_common::set_counter_rate(&self.pool, branch_id, currency, buy, sell).await;
    }

    fn opener_of(branch_id: &str) -> String {
        if branch_id == MAIN {
            "test".to_owned()
        } else {
            format!("test-{branch_id}")
        }
    }

    /// The teller of `branch_id`'s test shift: the owner's user at `MAIN` (whom `Shop::save` saves as), the
    /// branch's own opener elsewhere (one OPEN shift per teller). CA-10-save (#626): a save moves stock on the
    /// saver's own OPEN shift.
    pub async fn teller_of(&self, branch_id: &str) -> Option<String> {
        let key = if branch_id == MAIN {
            "owner"
        } else {
            &Self::opener_of(branch_id)
        };
        lookup_opener_user(&self.pool, key).await
    }

    pub async fn open_shift_id(&self, branch_id: &str) -> String {
        let teller = self
            .teller_of(branch_id)
            .await
            .unwrap_or_else(|| panic!("no opener seeded for {branch_id}"));
        self.shifts
            .open_of_teller(&teller)
            .await
            .expect("read the open shift")
            .expect("a shift is open")
            .id
    }

    pub async fn open_shift(&self, branch_id: &str, opening: &[(&str, &str)]) -> String {
        let opener = Self::opener_of(branch_id);
        if branch_id != MAIN {
            seed_opener(&self.pool, &opener).await;
        }
        seed_opener(&self.pool, OWNER).await;
        let teller: String = self.teller_of(branch_id).await.expect("the opener's user");
        let shift = format!("SH-{}", uuid::Uuid::new_v4());
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', $3, $4, '', $5, $6::uuid)",
        )
        .bind(&shift)
        .bind(branch_id)
        .bind(at(-60))
        .bind(&opener)
        .bind(unique("open-shift").replace('_', "-"))
        .bind(&teller)
        .execute(&self.pool)
        .await
        .expect("seed an OPEN shift");
        fund_ledger_drawer(&self.pool, &shift, opening).await;
        shift
    }

    /// Close the open shift without going through the count (the tables are ticket T7's).
    pub async fn close_shift(&self, branch_id: &str) {
        let done = sqlx::query(
            "UPDATE shifts SET status = 'CLOSED', closed_at = now(), closed_by = 'test', \
             close_request_id = $2 WHERE branch_id = $1 AND status = 'OPEN'",
        )
        .bind(branch_id)
        .bind(unique("close"))
        .execute(&self.pool)
        .await
        .expect("close the shift");
        assert_eq!(done.rows_affected(), 1, "a shift was open in {branch_id}");
    }

    pub async fn reserve(&self, branch_id: &str, who: ReserveCustomer) -> String {
        self.customers
            .reserve(&branch(branch_id), who, at(-30))
            .await
            .expect("reserve a customer")
            .customer_id
    }

    pub async fn upload(&self, branch_id: &str, owner: &str, mime: &str, bytes: &[u8]) -> String {
        let actor = if owner == OWNER {
            owner_user_of_branch(&self.pool, branch_id).await
        } else {
            opener_user(&self.pool, owner).await
        };
        self.files
            .upload(&branch(branch_id), &actor, mime, bytes, at(-10))
            .await
            .expect("upload a file")
    }

    /// `MAIN` ready for a cash purchase of dollars at 32.1 (sell 33): the rate, an
    /// open shift with `opening`, the customer of `person()`, a passport file of
    /// the owner. Returns (customer id, passport key).
    pub async fn ready(&self, opening: &[(&str, &str)]) -> (String, String) {
        self.set_rate(MAIN, "USD", "32.1", "33").await;
        self.open_shift(MAIN, opening).await;
        let customer = self.reserve(MAIN, person()).await;
        let passport = self.upload(MAIN, OWNER, "image/jpeg", JPEG).await;
        (customer, passport)
    }

    pub async fn save(
        &self,
        branch_id: &str,
        cmd: SaveTransaction,
        now: DateTime<Utc>,
    ) -> AppResult<SavedTransaction> {
        let owner = owner_user_of_branch(&self.pool, branch_id).await;
        tokio::time::timeout(
            MUST_FINISH,
            self.workflow.save(&branch(branch_id), cmd, &owner, now),
        )
        .await
        .expect("a save finishes")
    }

    pub async fn balances(&self, branch_id: &str) -> Seen {
        let teller = self.teller_of(branch_id).await?;
        let shift = self
            .shifts
            .open_of_teller(&teller)
            .await
            .expect("read the open shift")?;
        Some(
            shift
                .expected
                .non_zero()
                .into_iter()
                .map(|(c, m)| (c.as_str().to_owned(), m.to_string()))
                .collect(),
        )
    }

    /// The document counters, `key -> value`, keyed as v2 keys them (`document-<mode>-<date>`).
    ///
    /// Ticket #670 (TX-2, D-4; changed by its ticket author claude/f03345fe on the lane
    /// controller's ruling of 2026-10-10): v3 stores `document-<organization_id>-<mode>-<date>`.
    /// The v2 fixture records v2's key, so the organisation is taken out here, and only the
    /// organisation every save of these tests belongs to (OR-1's first) is accepted.
    /// `tx_compat_children_test.rs` pins the stored key itself.
    pub async fn counters(&self) -> BTreeMap<String, String> {
        const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
        let rows: Vec<(String, i64)> =
            sqlx::query_as("SELECT key, value FROM counters WHERE key LIKE 'document-%'")
                .fetch_all(&self.pool)
                .await
                .expect("read counters");
        rows.into_iter()
            .map(|(k, v)| {
                let rest = k
                    .strip_prefix(&format!("document-{ORG_1}-"))
                    .unwrap_or_else(|| panic!("counter {k} is not keyed by organisation {ORG_1}"));
                (format!("document-{rest}"), v.to_string())
            })
            .collect()
    }

    pub async fn refs(&self, branch_id: &str, mode: Mode) -> Vec<String> {
        self.transactions
            .list(&branch(branch_id), mode)
            .await
            .expect("list transactions")
            .into_iter()
            .map(|t| t.document_ref)
            .collect()
    }

    pub async fn count(&self, table: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }

    /// Everything a save may change, as one comparable text.
    pub async fn everything(&self) -> String {
        let mut out = String::new();
        for (table, order) in [
            ("counters", "key"),
            ("transactions", "document_ref"),
            ("transaction_files", "file_key, kind"),
            // ticket #670 (TX-2): a save writes these too, and a refused save none of them
            ("transaction_lines", "transaction_id, line_no"),
            ("transaction_payments", "transaction_id, payment_no"),
            ("receipts", "transaction_id, revision"),
            ("files", "key"),
            ("customers", "id"),
            ("ledger_entries", "id"),
        ] {
            let rows: Vec<String> = sqlx::query_scalar(AssertSqlSafe(format!(
                "SELECT row_to_json(t)::text FROM {table} t ORDER BY {order}"
            )))
            .fetch_all(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("read {table}: {e}"));
            out.push_str(&format!("{table}: {}\n", rows.join("\n  ")));
        }
        out
    }

    /// Make every INSERT and UPDATE on `table` fail until `mend` is called.
    pub async fn break_writes(&self, table: &str) {
        sqlx::query(
            "CREATE OR REPLACE FUNCTION t8b_refuse() RETURNS trigger LANGUAGE plpgsql AS \
             $$ BEGIN RAISE EXCEPTION 'test: this table refuses writes'; END $$",
        )
        .execute(&self.pool)
        .await
        .expect("create the refusing function");
        sqlx::query(AssertSqlSafe(format!(
            "CREATE TRIGGER t8b_refuse BEFORE INSERT OR UPDATE ON {table} \
             FOR EACH ROW EXECUTE FUNCTION t8b_refuse()"
        )))
        .execute(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("break {table}: {e}"));
    }

    pub async fn mend(&self, table: &str) {
        sqlx::query(AssertSqlSafe(format!("DROP TRIGGER t8b_refuse ON {table}")))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("mend {table}: {e}"));
    }

    /// Drop the schema and the scratch directory. Reached only by a test that
    /// passed; a failed test leaves both behind for inspection.
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
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

// ── the v2 fixture ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct Fixture {
    /// Who saved in v2: the owner.
    pub actor: String,
    pub files: BTreeMap<String, FxFile>,
    pub counts: BTreeMap<String, usize>,
    pub sentences: Vec<String>,
    pub scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
pub struct FxFile {
    pub mime: String,
    pub head: String,
    pub pad: usize,
}

impl FxFile {
    pub fn bytes(&self) -> Vec<u8> {
        let mut out: Vec<u8> = (0..self.head.len() / 2)
            .map(|i| u8::from_str_radix(&self.head[2 * i..2 * i + 2], 16).expect("hex"))
            .collect();
        out.resize(out.len() + self.pad, 0);
        out
    }
}

#[derive(Debug, Deserialize)]
pub struct Scenario {
    pub name: String,
    pub steps: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FxSave {
    pub note: String,
    pub at: String,
    pub branch: String,
    pub form: BTreeMap<String, Value>,
    pub passport: Option<String>,
    pub slip: Option<String>,
    pub v2_error: Option<String>,
    pub saved: Option<FxSaved>,
    pub after: Option<FxAfter>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FxSaved {
    pub mode: String,
    pub document_ref: String,
    pub passport: String,
    pub slip: Option<String>,
    pub receipt_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FxAfter {
    pub balances: Seen,
    pub live: Vec<String>,
    pub demo: Vec<String>,
    /// live document number -> v2's id of the shift it belongs to
    pub shift_of: BTreeMap<String, Option<String>>,
    pub customers: Vec<FxCustomer>,
    pub counters: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct FxCustomer {
    pub id: String,
    pub name: String,
    pub birth_date: Option<String>,
    pub passport_expiry: Option<String>,
    pub last_document_ref: Option<String>,
    pub mode: String,
}

pub fn fixture() -> Fixture {
    serde_json::from_str(include_str!("../fixtures/save/v2_save_cases.json"))
        .expect("tests/fixtures/save/v2_save_cases.json parses")
}

fn text(form: &BTreeMap<String, Value>, key: &str) -> String {
    match form.get(key) {
        Some(Value::String(s)) => s.clone(),
        None | Some(Value::Null) => String::new(),
        Some(other) => panic!("fixture form.{key} is {other}"),
    }
}

/// The command a fixture form is; the two files are given as keys.
pub fn command(
    form: &BTreeMap<String, Value>,
    passport: Option<String>,
    slip: Option<String>,
) -> SaveTransaction {
    SaveTransaction {
        customer: ReserveCustomer {
            name: text(form, "name"),
            passport_no: text(form, "passportNo"),
            nationality: text(form, "nationality"),
            birth_date: text(form, "birthDate"),
            passport_expiry: text(form, "passportExpiry"),
            mode: text(form, "mode"),
        },
        submission_id: text(form, "submissionId"),
        customer_id: text(form, "customerId"),
        side: text(form, "type"),
        currency: text(form, "currency"),
        amount: text(form, "amount"),
        rate: text(form, "rate"),
        fee: text(form, "fee"),
        bank_fee: text(form, "bankFee"),
        method: text(form, "method"),
        payment_ref: text(form, "ref"),
        account: text(form, "account"),
        staff: text(form, "staff"),
        booking: text(form, "booking"),
        notes: text(form, "notes"),
        value_date: text(form, "valueDate"),
        paid_at: text(form, "paidAt"),
        payment_confirmed: form.get("paymentConfirmed") == Some(&Value::Bool(true)),
        identity_confirmed: form.get("identityConfirmed") == Some(&Value::Bool(true)),
        passport,
        slip,
        reference_rate: form.get("referenceRate").map(|v| match v {
            Value::String(s) => s.clone(),
            other => panic!("fixture form.referenceRate is {other}"),
        }),
    }
}

/// CA-1 (#617): a shift's opener must name a user (`shifts.teller_id`, design #582 §8 M1 item 3).
/// Resolved per OR-7.m H1 (§3):
/// 1. `k` is a `users.id` (`users.id::text = k`) -> that user;
/// 2. `k == "owner"` -> the organisation-wide user with an `EXECUTIVE` role and `branch_id` NULL (oldest `created_at`);
/// 3. otherwise -> the XC organisation's user with `name = k` and `created_by = 'test'` (oldest), created as
///    `name = k`, `status = 'PENDING'`, `created_by = 'test'` if missing.
async fn lookup_opener_user(pool: &PgPool, k: &str) -> Option<String> {
    if let Some(id) =
        sqlx::query_scalar::<_, String>("SELECT id::text FROM users WHERE id::text = $1")
            .bind(k)
            .fetch_optional(pool)
            .await
            .expect("check if opener key is a users.id")
    {
        return Some(id);
    }
    if k == "owner" {
        return sqlx::query_scalar::<_, String>(
            "SELECT u.id::text FROM users u WHERE EXISTS \
             (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
             ORDER BY u.created_at, u.id LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .expect("read the owner's user id");
    }
    sqlx::query_scalar::<_, String>(
        "SELECT u.id::text FROM users u JOIN organizations o ON o.id = u.organization_id \
         WHERE o.code = 'XC' AND u.name = $1 AND u.created_by = 'test' \
         ORDER BY u.created_at, u.id LIMIT 1",
    )
    .bind(k)
    .fetch_optional(pool)
    .await
    .expect("read the opener's user id")
}

async fn opener_user(pool: &PgPool, k: &str) -> String {
    if let Some(id) = lookup_opener_user(pool, k).await {
        return id;
    }
    if k == "owner" {
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
        return lookup_opener_user(pool, k)
            .await
            .expect("read seeded owner user");
    }
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         SELECT id, $1, 'PENDING', 'test' FROM organizations WHERE code = 'XC' \
         RETURNING id::text",
    )
    .bind(k)
    .fetch_one(pool)
    .await
    .unwrap_or_else(|e| panic!("the user of opener {k}: {e}"))
}

async fn owner_user_of_branch(pool: &PgPool, branch_id: &str) -> String {
    if let Some(id) = sqlx::query_scalar::<_, String>(
        "SELECT u.id::text FROM users u \
         JOIN branches b ON b.id = $1 AND b.organization_id = u.organization_id \
         WHERE EXISTS (SELECT 1 FROM user_roles r \
                       WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
         ORDER BY u.created_at, u.id LIMIT 1",
    )
    .bind(branch_id)
    .fetch_optional(pool)
    .await
    .expect("read branch owner user id")
    {
        return id;
    }
    opener_user(pool, OWNER).await
}

pub async fn seed_opener(pool: &PgPool, id: &str) {
    let _ = opener_user(pool, id).await;
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

// ── decided customer differences in the save scenarios (CI-4 #651) ───────────

/// A scenario `reserve` (1-based step) that v2 answered with a new customer and v3 answers with the customer of
/// an earlier step of the same scenario. Lane controller's ruling, 2026-10-09: these five, each with its cause;
/// with the three replay uses (compat_common::SAME_CUSTOMER_AS) the cap is eight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SameScenarioCustomer {
    pub scenario: &'static str,
    pub step: usize,
    pub same_as: usize,
    pub reason: &'static str,
}

pub const SAME_SCENARIO_CUSTOMER: [SameScenarioCustomer; 5] = [
    SameScenarioCustomer {
        scenario: "numbers follow one another through a day, live and demo apart",
        step: 4,
        same_as: 3,
        reason: "owner 2026-10-09: DEMO shares LIVE",
    },
    SameScenarioCustomer {
        scenario: "the same submission live and demo is two exchanges",
        step: 4,
        same_as: 3,
        reason: "owner 2026-10-09: DEMO shares LIVE",
    },
    SameScenarioCustomer {
        scenario: "the number is counted for the whole shop, not per branch",
        step: 7,
        same_as: 3,
        reason: "K1 (customer per company)",
    },
    SameScenarioCustomer {
        scenario: "the same submission in two branches is two exchanges",
        step: 7,
        same_as: 3,
        reason: "K1 (customer per company)",
    },
    SameScenarioCustomer {
        scenario: "a rate of another branch does not count",
        step: 7,
        same_as: 3,
        reason: "K1 (customer per company)",
    },
];

/// The decided difference of this scenario step, if any. A use without a reason panics.
pub fn same_scenario_customer(scenario: &str, step: usize) -> Option<SameScenarioCustomer> {
    let found = SAME_SCENARIO_CUSTOMER
        .iter()
        .find(|s| s.scenario == scenario && s.step == step)
        .copied();
    if let Some(s) = found {
        assert!(
            !s.reason.trim().is_empty(),
            "{scenario} step {step}: needs its reason"
        );
    }
    found
}

/// Holds a decided step strictly: v3 answered the earlier step's customer and said it already existed.
/// Panics, naming the step, otherwise.
pub fn hold_same_scenario_customer(got: &str, existing: bool, earlier: &str, label: &str) {
    assert_eq!(got, earlier, "{label}: not the earlier step's customer");
    assert!(
        existing,
        "{label}: v3 must answer that the customer already existed"
    );
}

// ── decided receipt differences in the save scenarios (#5315, #5349) ─────────

/// A save whose receipt v3 prints differently from v2, on purpose. #5315 (R1, R3, R5) makes `THB Total` the
/// settled figure (principal ± fee ± cash rounding) and adds the signed cash-rounding row on a cash save whose
/// rounding is not zero. v2 printed the principal as the total and had no rounding row. Ruled per save on #5349
/// (2026-10-28): every one of these differs only in those two rows; the `Staff` row is v2's on all of them.
/// A decided difference, listed in `docs/design/xchange-v3-known-differences.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettledTotalReceipt {
    pub scenario: &'static str,
    pub step: usize,
    /// The `THB Total` v3 prints, without " THB".
    pub total: &'static str,
    /// The signed cash-rounding row, when the save has one.
    pub rounding: Option<&'static str>,
    /// The SHA-256 in v2's recorded `receipt_url`, which this entry replaces.
    pub v2_receipt: &'static str,
    /// The SHA-256 of the receipt v3 stores.
    pub v3_receipt: &'static str,
}

pub const RECEIPT_SETTLED_TOTAL: [SettledTotalReceipt; 17] = [
    SettledTotalReceipt {
        scenario: "a fee moves with the baht",
        step: 4,
        total: "3,184.50",
        rounding: None,
        v2_receipt: "d2e4c20958a519f14e5c1d221016d43869485c90d482464b2e3a59bfd77f1139",
        v3_receipt: "f7f07731a74e82b80f8f44f2fa21faa87b659503cc0fc09014b849c8ae1e2928",
    },
    SettledTotalReceipt {
        scenario: "a fee moves with the baht",
        step: 5,
        total: "3,310.00",
        rounding: None,
        v2_receipt: "9a07d8d78c5e287486fd6d9546eb5997f636c0a8f939399cbc52892c26064163",
        v3_receipt: "d88a1e787172527240c6f81675751ce12ed30b43525c0865ed6d6a94ac08180b",
    },
    SettledTotalReceipt {
        scenario: "paid by transfer: only the currency moves",
        step: 5,
        total: "3,305.00",
        rounding: None,
        v2_receipt: "2c32f51f00a859aaad8f8ee01f8972c76661029f9102f07a3ad353bf0e529ec2",
        v3_receipt: "987700decdcb9bf178f5cd0740ca9f6e7a6e6e8ea8bab93a8644e81a37a9246e",
    },
    SettledTotalReceipt {
        scenario: "numbers follow one another through a day, live and demo apart",
        step: 7,
        total: "32.25",
        rounding: Some("+0.15"),
        v2_receipt: "53067d44263991099778e16c45629ef56778893d59dff066ff18a5bcbce78ec5",
        v3_receipt: "e5744267d0b79f4bde8e6ba3ee502bb3bb2e66eb595500784dffb7a416727191",
    },
    SettledTotalReceipt {
        scenario: "the number is counted for the whole shop, not per branch",
        step: 10,
        total: "64.25",
        rounding: Some("+0.05"),
        v2_receipt: "0fa92a7ef880ec1421757e495f20b1908fae93533518516a4ee509c17dd2461d",
        v3_receipt: "86c22ca50f8854c06ee026440b38a3cb96b7da4c51ab3e9f6aa18b69d5130ece",
    },
    SettledTotalReceipt {
        scenario: "the day turns at Bangkok midnight",
        step: 5,
        total: "32.25",
        rounding: Some("+0.15"),
        v2_receipt: "ee14a7f615454850af4332da43259b25d4891fd0c17b37228a69eb4071ae6110",
        v3_receipt: "c4b1a3527fa8b3e3e1e76eeab12ccf4c34e07cb1334c2809d0d9b6a0a03e2ad6",
    },
    SettledTotalReceipt {
        scenario: "the day turns at Bangkok midnight",
        step: 6,
        total: "64.25",
        rounding: Some("+0.05"),
        v2_receipt: "34d6038d6cbeeae6bfab669124083e21b9fd680e2a24e29c192cd09df0144088",
        v3_receipt: "23aafe905e84d5a35f98a4f501e0f6ebb7c5986670550c03e0b0cfadeff966d0",
    },
    SettledTotalReceipt {
        scenario: "sending a saved exchange again changes nothing and answers the same",
        step: 4,
        total: "3,208.75",
        rounding: None,
        v2_receipt: "fd769d9713606b3ec9b37895292d90b30c61ed6e6efd3a4a2199882dad054095",
        v3_receipt: "fe1325bfacc3234a38ea5ea667c7010e71361ee611135423137acb166db205fb",
    },
    SettledTotalReceipt {
        scenario: "sending a saved exchange again changes nothing and answers the same",
        step: 5,
        total: "3,208.75",
        rounding: None,
        v2_receipt: "fd769d9713606b3ec9b37895292d90b30c61ed6e6efd3a4a2199882dad054095",
        v3_receipt: "fe1325bfacc3234a38ea5ea667c7010e71361ee611135423137acb166db205fb",
    },
    SettledTotalReceipt {
        scenario: "sending a saved exchange again changes nothing and answers the same",
        step: 6,
        total: "3,208.75",
        rounding: None,
        v2_receipt: "fd769d9713606b3ec9b37895292d90b30c61ed6e6efd3a4a2199882dad054095",
        v3_receipt: "fe1325bfacc3234a38ea5ea667c7010e71361ee611135423137acb166db205fb",
    },
    SettledTotalReceipt {
        scenario: "sending a saved exchange again changes nothing and answers the same",
        step: 7,
        total: "3,208.75",
        rounding: None,
        v2_receipt: "fd769d9713606b3ec9b37895292d90b30c61ed6e6efd3a4a2199882dad054095",
        v3_receipt: "fe1325bfacc3234a38ea5ea667c7010e71361ee611135423137acb166db205fb",
    },
    SettledTotalReceipt {
        scenario: "a fee lowers what a buy takes out",
        step: 5,
        total: "3,200.00",
        rounding: None,
        v2_receipt: "5a99b544d39f32192741fa0abc4cf9e4cfb141c8c53eac5d99195a5fa10f8954",
        v3_receipt: "b1c08957fa46bdde285599952d9aaa6201d4e5578fefd30f9df693f57cb40fe1",
    },
    SettledTotalReceipt {
        scenario: "a save writes the customer as the form has it",
        step: 5,
        total: "32.25",
        rounding: Some("+0.15"),
        v2_receipt: "55c29172e200b62127330bb73b089b15e8aa844883ff13240207f490a6bd0ca9",
        v3_receipt: "3e18a1ff2ff95f4bc3801c628d8f57deeb2d3318271e891d03f24fb77faa8b76",
    },
    SettledTotalReceipt {
        scenario: "a reference rate is kept and changes nothing else",
        step: 5,
        total: "32.25",
        rounding: Some("+0.15"),
        v2_receipt: "55c29172e200b62127330bb73b089b15e8aa844883ff13240207f490a6bd0ca9",
        v3_receipt: "3e18a1ff2ff95f4bc3801c628d8f57deeb2d3318271e891d03f24fb77faa8b76",
    },
    SettledTotalReceipt {
        scenario: "an exchange belongs to the shift that was open",
        step: 7,
        total: "64.25",
        rounding: Some("+0.05"),
        v2_receipt: "34d6038d6cbeeae6bfab669124083e21b9fd680e2a24e29c192cd09df0144088",
        v3_receipt: "23aafe905e84d5a35f98a4f501e0f6ebb7c5986670550c03e0b0cfadeff966d0",
    },
    SettledTotalReceipt {
        scenario: "several currencies in one shift",
        step: 9,
        total: "2.50",
        rounding: Some("+0.01"),
        v2_receipt: "e84eab6e90fce2bd13ec72c933bf8d1dd1ecf1e0666437dfce4aea4e78614f5b",
        v3_receipt: "899b6de651fc799340764a4fc8c9b3b71f9c8213d6c9154132856f5d337dbc90",
    },
    SettledTotalReceipt {
        scenario: "several currencies in one shift",
        step: 10,
        total: "2.50",
        rounding: Some("-0.05"),
        v2_receipt: "8ba58c95515d5294e41c5fb65c263022e7d5f057be3de790496d2307006a1547",
        v3_receipt: "46a817585562d89a8db755aba03155150385f0e245a8b606bd7edb8ff6b47703",
    },
];

/// The decided receipt difference of this scenario step, if any.
pub fn settled_total_receipt(scenario: &str, step: usize) -> Option<SettledTotalReceipt> {
    RECEIPT_SETTLED_TOTAL
        .iter()
        .find(|r| r.scenario == scenario && r.step == step)
        .copied()
}

/// Holds a decided receipt strictly: it replaces v2's recorded receipt, it is the receipt v3 stored, it is the
/// pinned v3 receipt byte for byte, and it prints the entry's total and rounding row. Panics, naming the step.
pub fn hold_settled_total_receipt(
    entry: &SettledTotalReceipt,
    v2_url: &str,
    url: &str,
    stored_receipt: &[u8],
    label: &str,
) {
    assert_eq!(
        v2_url,
        format!("/files/receipt-{}", entry.v2_receipt),
        "{label}: the entry does not replace v2's recorded receipt"
    );
    hold_receipt_hash(url, stored_receipt, label);
    assert_eq!(
        url,
        format!("/files/receipt-{}", entry.v3_receipt),
        "{label}: not the decided v3 receipt"
    );
    let html = std::str::from_utf8(stored_receipt).expect("the receipt is UTF-8");
    assert!(
        html.contains(&format!("<dt>THB Total</dt><dd>{} THB</dd>", entry.total)),
        "{label}: THB Total is not {}",
        entry.total
    );
    match entry.rounding {
        Some(r) => assert!(
            html.contains(&format!("Cash rounding</dt><dd>{r}</dd>")),
            "{label}: no cash-rounding row {r}"
        ),
        None => assert!(
            !html.contains("Cash rounding"),
            "{label}: a cash-rounding row where none is due"
        ),
    }
}

/// Holds the receipt address of a save for an aliased customer: `/files/receipt-<SHA-256 of the stored receipt>`.
pub fn hold_receipt_hash(url: &str, stored_receipt: &[u8], label: &str) {
    use sha2::{Digest, Sha256};
    assert_eq!(
        url,
        format!("/files/receipt-{:x}", Sha256::digest(stored_receipt)),
        "{label}: the receipt URL is not the hash of the stored receipt"
    );
}

/// CI-4 (#651): one accepted save of a scenario, in order: the customer it was written to and what it wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenSave {
    pub customer: String,
    pub document_ref: String,
    pub name: String,
    pub birth_date: Option<String>,
    pub passport_expiry: Option<String>,
}

/// CI-4 (#651), the `after.customers` of a scenario with a decided customer difference (lane controller
/// 2026-10-09): every aliased entry is folded into its earlier customer, whose name, birth date, expiry and
/// last document become those of the newest save to that customer; its mode stays its own. Every other entry
/// is v2's, unchanged.
pub fn fold_aliased_customers(
    after: &[FxCustomer],
    aliases: &BTreeMap<String, String>,
    saves: &[WrittenSave],
) -> Vec<FxCustomer> {
    after
        .iter()
        .filter(|c| !aliases.contains_key(&c.id))
        .map(|c| {
            let mut c = c.clone();
            if aliases.values().any(|a| a == &c.id)
                && let Some(newest) = saves.iter().rev().find(|s| s.customer == c.id)
            {
                c.name = newest.name.clone();
                c.birth_date = newest.birth_date.clone();
                c.passport_expiry = newest.passport_expiry.clone();
                c.last_document_ref = Some(newest.document_ref.clone());
            }
            c
        })
        .collect()
}

static LEDGER_FUND: AtomicU64 = AtomicU64::new(0);

/// R1 (TX-6 #674, lane controller's ruling: compat saves post through the ledger): moves each
/// `(currency, amount)` of a v2 shift's opening from the BANK into that shift's ledger drawer
/// (`OPENING`), as `tx5_common`'s `lg4_fund` does for the route harness.
pub async fn fund_ledger_drawer(pool: &PgPool, shift: &str, opening: &[(&str, &str)]) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
    let mut tx = pool.begin().await.expect("begin");
    for (currency, amount) in opening {
        let quantity: rust_decimal::Decimal = amount.parse().expect("an amount");
        if quantity.is_zero() {
            continue;
        }
        let n = LEDGER_FUND.fetch_add(1, Ordering::Relaxed);
        xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[PostingRequest {
                    from: CashPlace::Bank,
                    to: CashPlace::Drawer(shift.to_owned()),
                    currency: CurrencyCode::parse(currency).expect("a currency"),
                    quantity: xc_kernel::money::ForeignAmount::from_decimal(quantity),
                    source: SourceRef {
                        source_type: SourceType::Opening,
                        source_id: format!("r1-fund:{shift}:{currency}:{n}"),
                        source_key: "fund".to_string(),
                    },
                    line: None,
                    reverses: None,
                }],
                None,
            )
            .await
            .expect("fund the drawer through the ledger");
    }
    tx.commit().await.expect("commit");
}

/// A3 (kiosk) (TX-6 #674, lane controller's ruling R2, design quote-transaction.md §4.6.1): a kiosk request is
/// saved at its own quote, which the kiosk asks for through `QuoteService` with channel KIOSK when the customer
/// quotes (TX-7, `xc-kiosk` `customer.rs:118-135`). This is that call as `PgKioskPorts::quote_in` makes it, for a
/// test's `KioskPorts` fake: one line, no fee, the kiosk as the actor.
pub async fn kiosk_quote_in(
    pool: &PgPool,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    branch_id: &BranchId,
    currency: CurrencyCode,
    side: xc_kernel::money::Side,
    quantity: xc_kernel::money::ForeignAmount,
    now: DateTime<Utc>,
) -> AppResult<Option<String>> {
    use xchange::application::workflows::exchange::quote::{
        QuoteRequest, QuoteRequestLine, QuoteService,
    };
    let request = QuoteRequest {
        branch_id: branch_id.clone(),
        customer_id: None,
        channel: xc_exchange::domain::quote::quote::QuoteChannel::Kiosk,
        lines: vec![QuoteRequestLine {
            currency,
            shop_side: side,
            quantity,
            fee_thb: None,
            denomination_group: xc_rate::domain::currency::denomination_groups::GroupCode::all(),
        }],
        payment_method: Some(
            xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod::Cash,
        ),
    };
    let kiosk = xchange::application::workflows::save_step::SaveActor {
        id: xc_kiosk::domain::kiosk::value_objects::audit_line::CUSTOMER_KIOSK.to_owned(),
        username: xc_kiosk::domain::kiosk::value_objects::audit_line::CUSTOMER_KIOSK.to_owned(),
        employee_id: None,
        standing: xc_auth::domain::access::StandingKind::Staff(
            xc_auth::domain::access::Role::Admin,
        ),
    };
    let quote = QuoteService::new(pool.clone())
        .quote_in(tx, request, &kiosk, now)
        .await?;
    Ok(Some(quote.id.to_string()))
}

/// A3 (kiosk): quotes kiosk request `id` of `branch_id` through [`kiosk_quote_in`] and writes the quote's id on
/// the request, as the kiosk does (`KioskRepository::set_quote_id_in`). The request's own rate, line and unit are
/// left as they are.
pub async fn quote_kiosk_request(
    pool: &PgPool,
    branch_id: &str,
    id: &str,
    currency: &str,
    side: xc_kernel::money::Side,
    quantity: &str,
    now: DateTime<Utc>,
) -> String {
    let mut tx = pool.begin().await.expect("begin");
    let quote_id = kiosk_quote_in(
        pool,
        &mut tx,
        &branch(branch_id),
        code(currency),
        side,
        xc_kernel::money::ForeignAmount::from_decimal(quantity.parse().expect("a quantity")),
        now,
    )
    .await
    .expect("the kiosk's quote")
    .expect("a quote id");
    let done = sqlx::query(
        "UPDATE kiosk_requests SET quote_id = $1::uuid WHERE branch_id = $2 AND id = $3",
    )
    .bind(&quote_id)
    .bind(branch_id)
    .bind(id)
    .execute(&mut *tx)
    .await
    .expect("write the request's quote");
    assert_eq!(
        done.rows_affected(),
        1,
        "kiosk request {branch_id}/{id} exists"
    );
    tx.commit().await.expect("commit");
    quote_id
}

/// A3 (kiosk): a kiosk request `id` that `staff` accepted and whose passport they checked, for a cash `side` of
/// `amount` `currency` at `rate` per 1 (unit 1) for `thb`, carrying the kiosk's own quote from
/// [`quote_kiosk_request`] — a request as the kiosk leaves it since TX-7.
#[allow(clippy::too_many_arguments)]
pub async fn checked_kiosk_request_with_quote(
    pool: &PgPool,
    branch_id: &str,
    id: &str,
    staff: &str,
    side: xc_kernel::money::Side,
    currency: &str,
    amount: &str,
    rate: &str,
    thb: &str,
    now: DateTime<Utc>,
) -> String {
    sqlx::query(
        "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, \
         rate, foreign_amount, thb, input_amount, payment_method, client_ip, unit_size) \
         VALUES ($1, $2, 'ID_CHECKED', now(), $3, $4, $5, $6::numeric, $7::numeric, $8::numeric, \
         $7::numeric, 'Cash', '203.0.113.7', 1)",
    )
    .bind(branch_id)
    .bind(id)
    .bind(staff)
    .bind(match side {
        xc_kernel::money::Side::Buy => "BUY",
        xc_kernel::money::Side::Sell => "SELL",
    })
    .bind(currency)
    .bind(rate)
    .bind(amount)
    .bind(thb)
    .execute(pool)
    .await
    .expect("insert a checked kiosk request");
    quote_kiosk_request(pool, branch_id, id, currency, side, amount, now).await
}
