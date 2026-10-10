//! CA-1 (#617): the cash schema, migration M1 (`api/migrations/20261012100000_cash.sql`) and the teller written on
//! every shift. Design `docs/design/blueprint/cash-shift-transfer.md` (#582): §4.1 tables, §8 M1, §9.1 row CA-1,
//! the owner's answer A7 (face values: the current series).
//!
//! Written before the implementation by the ticket author (claude/89458e53). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.
//!
//! Signatures assumed:
//! - `ShiftService::open(&BranchId, OpenShift, actor: &str, now)` as on main; the actor is the opener id in either
//!   form (an employee id = `users.legacy_employee_id`, a `users.id`, or `"owner"`).
//! - `ShiftError::TellerUnknown` with the sentence [`TELLER_UNKNOWN`], returned as `AppError::Domain(sentence)`
//!   like every `ShiftError`.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use pos_infra::transactions::Transactor;
use serde_json::json;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::domain::shift::commands::open_shift::OpenShift;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

const M1: i64 = 20261012100000;
const M1_FILE: &str = "20261012100000_cash.sql";

/// The sentence of `ShiftError::TellerUnknown` ([required], design §8 M1 item 3: "no match → `tellerUnknown`").
const TELLER_UNKNOWN: &str = "ไม่พบผู้เปิดกะในรายชื่อผู้ใช้";

/// The tables §4.1 creates.
const NEW_TABLES: [&str; 7] = [
    "cash_counts",
    "cash_count_lines",
    "cash_count_totals",
    "handovers",
    "cash_transfers",
    "cash_transfer_lines",
    "cash_transfer_policy_versions",
];

/// A7: every note and coin of each central bank's current series, for the seven seeded currencies (§8 M1 item 2).
/// The edge values were settled by the owner on 2026-10-09 (#617): EUR 500 and JPY 2,000 are kept; the values in
/// [`LEFT_OUT`] are not seeded. The Blueprint names no face values; it has only
/// `Denomination(id PK, currency_code FK, face_value DECIMAL(20,4), group_code)` (Blueprint v2, Currency /
/// Denomination). Exactly these are seeded, one row each.
const FACE_VALUES: [(&str, &[&str]); 7] = [
    (
        "THB",
        &[
            "0.25", "0.5", "1", "2", "5", "10", "20", "50", "100", "500", "1000",
        ],
    ),
    (
        "USD",
        &[
            "0.01", "0.05", "0.1", "0.25", "1", "2", "5", "10", "20", "50", "100",
        ],
    ),
    (
        "EUR",
        &[
            "0.01", "0.02", "0.05", "0.1", "0.2", "0.5", "1", "2", "5", "10", "20", "50", "100",
            "200", "500",
        ],
    ),
    (
        "GBP",
        &[
            "0.01", "0.02", "0.05", "0.1", "0.2", "0.5", "1", "2", "5", "10", "20", "50",
        ],
    ),
    (
        "JPY",
        &[
            "1", "5", "10", "50", "100", "500", "1000", "2000", "5000", "10000",
        ],
    ),
    ("CNY", &["0.1", "0.5", "1", "5", "10", "20", "50", "100"]),
    (
        "KRW",
        &["10", "50", "100", "500", "1000", "5000", "10000", "50000"],
    ),
];

/// The values the owner left out on 2026-10-09 (#617): never seeded.
const LEFT_OUT: [(&str, &str); 6] = [
    ("THB", "0.01"),
    ("THB", "0.05"),
    ("THB", "0.1"),
    ("USD", "0.5"),
    ("KRW", "1"),
    ("KRW", "5"),
];

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 7, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

/// A schema of its own, migrated up to (not including) M1, or through everything.
struct Db {
    pool: PgPool,
    migrator: Migrator,
}

impl Db {
    async fn new() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip. \
                 Point it at an existing database whose name ends in `_test`."
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("ca1");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrator = Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations");
        assert!(
            migrator.version_exists(M1),
            "api/migrations/{M1_FILE} exists"
        );
        Db { pool, migrator }
    }

    async fn before_m1() -> Db {
        let db = Db::new().await;
        db.migrator
            .run_to(M1 - 1, &db.pool)
            .await
            .expect("migrate up to the one before M1");
        db
    }

    async fn fresh() -> Db {
        let db = Db::new().await;
        db.migrator
            .run(&db.pool)
            .await
            .expect("every migration applies in version order on a fresh database");
        db
    }

    async fn m1(&self) -> Result<(), String> {
        self.migrator
            .run_to(M1, &self.pool)
            .await
            .map_err(|e| e.to_string())
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn text(&self, sql: &str) -> Option<String> {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_optional(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn org(&self) -> String {
        self.text("SELECT id::text FROM organizations WHERE code = 'XC'")
            .await
            .expect("OR-1's organisation XC")
    }

    async fn branch(&self, id: &str) {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(id)
            .execute(&self.pool)
            .await
            .expect("insert a branch");
    }

    /// A user as OR-2's mirror writes it; `legacy` is the employee id (`None` for the owner's user).
    async fn user(&self, legacy: Option<&str>) -> String {
        let org = self.org().await;
        sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by, legacy_employee_id) \
             VALUES ($1::uuid, 'u', 'INVITED', 'test', $2) RETURNING id::text",
        )
        .bind(&org)
        .bind(legacy)
        .fetch_one(&self.pool)
        .await
        .expect("insert a user")
    }

    /// The owner's user: EXECUTIVE + ADMIN, organisation-wide, no legacy id (OR-2 migration 2 / `create_owner`).
    async fn owner_user(&self) -> String {
        let id = self.user(None).await;
        let org = self.org().await;
        for role in ["EXECUTIVE", "ADMIN"] {
            sqlx::query(
                "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
                 VALUES ($1::uuid, $2, $3::uuid, NULL)",
            )
            .bind(&id)
            .bind(role)
            .bind(&org)
            .execute(&self.pool)
            .await
            .expect("insert the owner's role");
        }
        id
    }

    /// An employee row as the pre-OR-7 account code writes it, with its mirrored user (OR-2).
    async fn employee(&self, id: &str, username: &str, branch: &str) -> String {
        sqlx::query(
            "INSERT INTO employees (id, username, name, role, branch_id, password_hash, active, must_change, \
             version, created_at) VALUES ($1, $2, $2, 'FRONT', $3, 'h', true, false, 1, now())",
        )
        .bind(id)
        .bind(username)
        .bind(branch)
        .execute(&self.pool)
        .await
        .expect("insert an employee");
        self.user(Some(id)).await
    }

    /// The owners row (pre-OR-7) with the given username.
    async fn owner_row(&self, username: &str) {
        sqlx::query(
            "INSERT INTO owners (id, username, name, password_hash, active, version, created_at) \
             VALUES ('owner-row', $1, 'Owner', 'h', true, 1, now())",
        )
        .bind(username)
        .execute(&self.pool)
        .await
        .expect("insert the owners row");
    }

    /// A shift row as the pre-CA-1 binary writes it.
    async fn old_shift(&self, id: &str, branch: &str, opened_by: &str, status: &str) {
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, open_request_id) \
             VALUES ($1, $2, $3, $4, $5, '', $1)",
        )
        .bind(id)
        .bind(branch)
        .bind(status)
        .bind(at(0))
        .bind(opened_by)
        .execute(&self.pool)
        .await
        .expect("insert a shift as the pre-CA-1 binary does");
    }

    async fn teller_of(&self, shift: &str) -> Option<String> {
        sqlx::query_scalar("SELECT teller_id::text FROM shifts WHERE id = $1")
            .bind(shift)
            .fetch_one(&self.pool)
            .await
            .expect("read the shift's teller")
    }

    fn service(&self) -> ShiftService<PgShiftRepository> {
        ShiftService::new(
            Arc::new(PgShiftRepository::new(self.pool.clone())),
            BranchTx::new(Transactor::new(self.pool.clone())),
        )
    }
}

fn open_cmd(n: u32) -> OpenShift {
    let request_id = format!("00000000-0000-4000-8000-{n:012x}");
    OpenShift::from_payload(&json!({ "requestId": request_id, "opening": { "THB": "1000" } }))
        .expect("a well-formed open payload")
}

// ── M1 on existing rows (§8 M1 items 1–2) ───────────────────────────────────

#[tokio::test]
async fn existing_shifts_get_the_teller_of_their_opener() {
    let db = Db::before_m1().await;
    db.branch("A").await;
    db.branch("B").await;
    let alice = db.user(Some("E-ALICE")).await;
    let owner = db.owner_user().await;
    db.old_shift("S1", "A", "E-ALICE", "CLOSED").await;
    db.old_shift("S2", "A", "owner", "OPEN").await;
    db.old_shift("S3", "B", "E-ALICE", "OPEN").await;
    db.m1().await.expect("M1 applies");
    assert_eq!(
        db.teller_of("S1").await,
        Some(alice.clone()),
        "employee → its user"
    );
    assert_eq!(
        db.teller_of("S2").await,
        Some(owner),
        "\"owner\" → the EXECUTIVE + ADMIN user"
    );
    assert_eq!(
        db.teller_of("S3").await,
        Some(alice),
        "the same employee at another branch"
    );
    assert_eq!(
        db.count(
            "SELECT count(*) FROM information_schema.columns WHERE table_schema = current_schema() \
                  AND table_name = 'shifts' AND column_name = 'teller_id' AND is_nullable = 'NO'"
        )
        .await,
        1,
        "teller_id is NOT NULL after M1"
    );
}

#[tokio::test]
async fn a_shift_whose_opener_maps_to_nobody_stops_m1_naming_the_shift() {
    let db = Db::before_m1().await;
    db.branch("A").await;
    db.user(Some("E-ALICE")).await;
    db.old_shift("S-ORPHAN", "A", "E-GONE", "CLOSED").await;
    let err = db
        .m1()
        .await
        .expect_err("M1 must RAISE, never guess a teller");
    assert!(err.contains("S-ORPHAN"), "the error names the shift: {err}");
    assert_eq!(
        db.count(
            "SELECT count(*) FROM information_schema.columns WHERE table_schema = current_schema() \
                  AND table_name = 'shifts' AND column_name = 'teller_id'"
        )
        .await,
        0,
        "nothing of M1 is left behind"
    );
}

#[tokio::test]
async fn an_owner_opened_shift_with_no_owner_user_stops_m1() {
    let db = Db::before_m1().await;
    db.branch("A").await;
    db.old_shift("S1", "A", "owner", "CLOSED").await;
    assert!(db.m1().await.is_err(), "no owner user: M1 RAISEs");
}

#[tokio::test]
async fn an_owner_opened_shift_with_two_owner_users_stops_m1() {
    let db = Db::before_m1().await;
    db.branch("A").await;
    db.owner_user().await;
    db.owner_user().await;
    db.old_shift("S1", "A", "owner", "CLOSED").await;
    assert!(db.m1().await.is_err(), "two owner users: M1 RAISEs");
}

#[tokio::test]
async fn without_an_owner_opened_shift_m1_needs_no_owner_user() {
    // A fresh install: the migrations run before the owner exists (PR 636).
    let db = Db::before_m1().await;
    db.branch("A").await;
    db.user(Some("E-ALICE")).await;
    db.old_shift("S1", "A", "E-ALICE", "CLOSED").await;
    db.m1().await.expect("M1 applies with no owner user");
}

#[tokio::test]
async fn a_fresh_database_applies_every_migration_in_version_order() {
    let db = Db::fresh().await;
    assert_eq!(
        db.count("SELECT count(*) FROM users").await,
        0,
        "nothing seeded a user: M1 applied without the owner's"
    );
}

#[tokio::test]
async fn m1_leaves_the_request_model_untouched() {
    let db = Db::fresh().await;
    for (table, column) in [
        ("custody_requests", "id"),
        ("custody_signatures", "request_id"),
        ("ops_state", "custodian_id"),
    ] {
        assert_eq!(
            db.count(&format!(
                "SELECT count(*) FROM information_schema.columns WHERE table_schema = current_schema() \
                 AND table_name = '{table}' AND column_name = '{column}'"
            ))
            .await,
            1,
            "{table}.{column} stays until CA-10"
        );
    }
}

#[tokio::test]
async fn m1_keeps_the_per_branch_open_shift_index() {
    // The index swap is CA-3's M2, not M1. CA-3 (#619): read the schema as M1 leaves it, since M2 now follows it.
    let db = Db::before_m1().await;
    db.m1().await.expect("M1 applies");
    assert_eq!(
        db.count(
            "SELECT count(*) FROM pg_indexes WHERE schemaname = current_schema() \
                  AND indexname = 'shifts_one_open_per_branch'"
        )
        .await,
        1
    );
}

// ── the tables (§4.1) ───────────────────────────────────────────────────────

#[tokio::test]
async fn every_new_table_carries_the_organisation() {
    let db = Db::fresh().await;
    for table in NEW_TABLES {
        assert_eq!(
            db.count(&format!(
                "SELECT count(*) FROM information_schema.columns WHERE table_schema = current_schema() \
                 AND table_name = '{table}' AND column_name = 'organization_id' AND is_nullable = 'NO'"
            ))
            .await,
            1,
            "{table}.organization_id NOT NULL"
        );
    }
}

#[tokio::test]
async fn amounts_are_numeric_20_4() {
    let db = Db::fresh().await;
    for (table, column) in [
        ("cash_count_totals", "counted"),
        ("cash_count_totals", "expected"),
        ("cash_count_totals", "difference"),
    ] {
        assert_eq!(
            db.text(&format!(
                "SELECT data_type || '(' || numeric_precision || ',' || numeric_scale || ')' \
                 FROM information_schema.columns WHERE table_schema = current_schema() \
                 AND table_name = '{table}' AND column_name = '{column}'"
            ))
            .await
            .as_deref(),
            Some("numeric(20,4)"),
            "{table}.{column}"
        );
    }
}

#[tokio::test]
async fn person_columns_reference_users_never_employees() {
    let db = Db::fresh().await;
    for (table, column) in [
        ("shifts", "teller_id"),
        ("cash_counts", "counted_by"),
        ("cash_counts", "verified_by"),
        ("handovers", "receiver_id"),
        ("cash_transfers", "created_by"),
        ("cash_transfers", "dispatched_by"),
        ("cash_transfers", "received_by"),
        ("cash_transfers", "reconciled_by"),
        ("cash_transfer_policy_versions", "set_by"),
    ] {
        let target = db
            .text(&format!(
                "SELECT ccu.table_name::text FROM information_schema.key_column_usage kcu \
                 JOIN information_schema.referential_constraints rc \
                   ON rc.constraint_name = kcu.constraint_name AND rc.constraint_schema = kcu.constraint_schema \
                 JOIN information_schema.constraint_column_usage ccu \
                   ON ccu.constraint_name = rc.unique_constraint_name AND ccu.constraint_schema = rc.unique_constraint_schema \
                 WHERE kcu.table_schema = current_schema() AND kcu.table_name = '{table}' AND kcu.column_name = '{column}'"
            ))
            .await;
        assert_eq!(
            target.as_deref(),
            Some("users"),
            "{table}.{column} → users(id)"
        );
        assert_eq!(
            db.text(&format!(
                "SELECT data_type FROM information_schema.columns WHERE table_schema = current_schema() \
                 AND table_name = '{table}' AND column_name = '{column}'"
            ))
            .await
            .as_deref(),
            Some("uuid"),
            "{table}.{column} is uuid"
        );
    }
}

#[tokio::test]
async fn a_count_naming_another_organisations_branch_is_refused() {
    let db = Db::fresh().await;
    db.branch("A").await;
    let counter = db.user(Some("E-C")).await;
    db.exec(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES ('00000000-0000-0000-0000-0000000000b2', 'OTHER', 'Asia/Bangkok', now())",
    )
    .await;
    let refused = sqlx::query(
        "INSERT INTO cash_counts (id, organization_id, branch_id, kind, counted_by, counted_at, idempotency_key) \
         VALUES (gen_random_uuid(), '00000000-0000-0000-0000-0000000000b2', 'A', 'SAFE', $1::uuid, now(), 'k1')",
    )
    .bind(&counter)
    .execute(&db.pool)
    .await;
    assert!(
        refused.is_err(),
        "branch A belongs to XC, not OTHER: the composite FK refuses"
    );
    let org = db.org().await;
    sqlx::query(
        "INSERT INTO cash_counts (id, organization_id, branch_id, kind, counted_by, counted_at, idempotency_key) \
         VALUES (gen_random_uuid(), $1::uuid, 'A', 'SAFE', $2::uuid, now(), 'k1')",
    )
    .bind(&org)
    .bind(&counter)
    .execute(&db.pool)
    .await
    .expect("the same count in its own organisation is accepted");
}

#[tokio::test]
async fn a_count_verified_by_its_counter_is_refused_by_the_table() {
    let db = Db::fresh().await;
    db.branch("A").await;
    let counter = db.user(Some("E-C")).await;
    let org = db.org().await;
    let refused = sqlx::query(
        "INSERT INTO cash_counts (id, organization_id, branch_id, kind, counted_by, counted_at, \
         verified_by, verified_at, idempotency_key) \
         VALUES (gen_random_uuid(), $1::uuid, 'A', 'SAFE', $2::uuid, now(), $2::uuid, now(), 'k1')",
    )
    .bind(&org)
    .bind(&counter)
    .execute(&db.pool)
    .await;
    assert!(refused.is_err(), "CHECK (verified_by <> counted_by)");
}

#[tokio::test]
async fn a_transfer_has_the_five_states_and_no_sixth() {
    let db = Db::fresh().await;
    db.branch("A").await;
    let creator = db.user(Some("E-C")).await;
    let org = db.org().await;
    for (n, status) in [
        "DRAFT",
        "APPROVED",
        "DISPATCHED",
        "RECEIVED",
        "RECONCILED",
        "CANCELLED",
    ]
    .iter()
    .enumerate()
    {
        let res = sqlx::query(
            "INSERT INTO cash_transfers (id, organization_id, kind, from_branch_id, status, created_by, idempotency_key) \
             VALUES (gen_random_uuid(), $1::uuid, 'DRAWER_TO_SAFE', 'A', $2, $3::uuid, $4)",
        )
        .bind(&org)
        .bind(status)
        .bind(&creator)
        .bind(format!("k{n}"))
        .execute(&db.pool)
        .await;
        assert_eq!(res.is_ok(), *status != "CANCELLED", "status {status}");
    }
}

#[tokio::test]
async fn a_count_line_takes_a_whole_non_negative_quantity_of_one_face_value() {
    let db = Db::fresh().await;
    db.branch("A").await;
    let counter = db.user(Some("E-C")).await;
    let org = db.org().await;
    let count: String = sqlx::query_scalar(
        "INSERT INTO cash_counts (id, organization_id, branch_id, kind, counted_by, counted_at, idempotency_key) \
         VALUES (gen_random_uuid(), $1::uuid, 'A', 'SAFE', $2::uuid, now(), 'k1') RETURNING id::text",
    )
    .bind(&org)
    .bind(&counter)
    .fetch_one(&db.pool)
    .await
    .expect("a count");
    let note: i64 = sqlx::query_scalar(
        "SELECT id FROM denominations WHERE currency_code = 'THB' AND face_value = 1000",
    )
    .fetch_one(&db.pool)
    .await
    .expect("the 1,000 baht note");
    let line = |qty: i32| {
        sqlx::query(
            "INSERT INTO cash_count_lines (count_id, organization_id, currency, denomination_id, qty) \
             VALUES ($1::uuid, $2::uuid, 'THB', $3, $4)",
        )
        .bind(count.clone())
        .bind(org.clone())
        .bind(note)
        .bind(qty)
    };
    assert!(line(-1).execute(&db.pool).await.is_err(), "qty >= 0");
    line(3).execute(&db.pool).await.expect("3 notes");
}

// ── face values (A7) ────────────────────────────────────────────────────────

#[tokio::test]
async fn the_face_values_are_exactly_the_current_series_the_owner_settled() {
    let db = Db::fresh().await;
    for (currency, want) in FACE_VALUES {
        let got: Vec<String> = sqlx::query_scalar(
            "SELECT trim_scale(face_value)::text FROM denominations \
             WHERE currency_code = $1 AND face_value IS NOT NULL ORDER BY face_value",
        )
        .bind(currency)
        .fetch_all(&db.pool)
        .await
        .expect("read the face values");
        assert_eq!(
            got, want,
            "{currency}: exactly the settled list, one row each, in order"
        );
    }
    for (currency, value) in LEFT_OUT {
        assert_eq!(
            db.count(&format!(
                "SELECT count(*) FROM denominations WHERE currency_code = '{currency}' \
                 AND trim_scale(face_value) = {value}"
            ))
            .await,
            0,
            "{currency} {value} is left out (owner, #617)"
        );
    }
    assert_eq!(
        db.count(
            "SELECT count(*) FROM denominations WHERE face_value IS NOT NULL \
             AND currency_code NOT IN ('THB','USD','EUR','GBP','JPY','CNY','KRW')"
        )
        .await,
        0,
        "only the seven seeded currencies get face values"
    );
}

#[tokio::test]
async fn every_face_value_fits_its_currencys_minor_unit() {
    let db = Db::fresh().await;
    assert_eq!(
        db.count(
            "SELECT count(*) FROM denominations d JOIN currencies c ON c.code = d.currency_code \
             WHERE d.face_value IS NOT NULL AND scale(trim_scale(d.face_value)) > c.minor_unit"
        )
        .await,
        0
    );
}

// ── the binary writes the teller (§8 M1 item 3) ─────────────────────────────

#[tokio::test]
async fn opening_a_shift_writes_the_teller_from_either_form_of_the_opener() {
    let db = Db::fresh().await;
    let branches = ["A", "B", "C"];
    for b in branches {
        db.branch(b).await;
    }
    let alice = db.user(Some("E-ALICE")).await;
    let owner = db.owner_user().await;
    let service = db.service();
    let cases = [
        ("A", "E-ALICE", alice.clone()),
        ("B", "owner", owner),
        ("C", alice.as_str(), alice.clone()),
    ];
    for (n, (branch, opener, teller)) in cases.into_iter().enumerate() {
        let shift = service
            .open(
                &BranchId::new(branch),
                open_cmd(n as u32 + 1),
                opener,
                at(n as i64),
            )
            .await
            .unwrap_or_else(|e| panic!("open by {opener}: {e:?}"));
        assert_eq!(
            db.teller_of(&shift.id).await,
            Some(teller),
            "opener {opener}"
        );
    }
}

#[tokio::test]
async fn an_opener_with_no_user_is_refused_and_no_shift_is_written() {
    let db = Db::fresh().await;
    db.branch("A").await;
    db.user(Some("E-ALICE")).await;
    let refused = db
        .service()
        .open(&BranchId::new("A"), open_cmd(1), "E-NOBODY", at(0))
        .await;
    match refused {
        Err(AppError::Domain(sentence)) => assert_eq!(sentence, TELLER_UNKNOWN),
        other => panic!("expected AppError::Domain({TELLER_UNKNOWN:?}), got {other:?}"),
    }
    assert_eq!(db.count("SELECT count(*) FROM shifts").await, 0);
    assert_eq!(db.count("SELECT count(*) FROM shift_balances").await, 0);
}

#[tokio::test]
async fn owner_with_no_owner_user_is_refused() {
    let db = Db::fresh().await;
    db.branch("A").await;
    let refused = db
        .service()
        .open(&BranchId::new("A"), open_cmd(1), "owner", at(0))
        .await;
    assert!(
        matches!(&refused, Err(AppError::Domain(s)) if s == TELLER_UNKNOWN),
        "got {refused:?}"
    );
    assert_eq!(db.count("SELECT count(*) FROM shifts").await, 0);
}

#[tokio::test]
async fn existing_shifts_opened_through_the_rpc_route_name_their_opener_by_username() {
    // `/api/rpc` passes `caller.username` as the opener (`compat/rpc.rs`), and v2 prints it as `staff`.
    let db = Db::before_m1().await;
    db.branch("A").await;
    let bob = db.employee("E-BOB", "bob", "A").await;
    let owner = db.owner_user().await;
    db.owner_row("boss").await;
    db.old_shift("S1", "A", "bob", "CLOSED").await;
    db.old_shift("S2", "A", "boss", "CLOSED").await;
    db.m1().await.expect("M1 applies");
    assert_eq!(
        db.teller_of("S1").await,
        Some(bob),
        "an employee's username → its user"
    );
    assert_eq!(
        db.teller_of("S2").await,
        Some(owner),
        "the owner's username → the owner's user"
    );
}

#[tokio::test]
async fn opening_a_shift_by_username_writes_the_same_teller() {
    let db = Db::fresh().await;
    db.branch("A").await;
    db.branch("B").await;
    let bob = db.employee("E-BOB", "bob", "A").await;
    let owner = db.owner_user().await;
    db.owner_row("boss").await;
    let service = db.service();
    for (n, (branch, opener, teller)) in [("A", "bob", bob), ("B", "boss", owner)]
        .into_iter()
        .enumerate()
    {
        let shift = service
            .open(
                &BranchId::new(branch),
                open_cmd(n as u32 + 1),
                opener,
                at(n as i64),
            )
            .await
            .unwrap_or_else(|e| panic!("open by {opener}: {e:?}"));
        assert_eq!(
            db.teller_of(&shift.id).await,
            Some(teller),
            "opener {opener}"
        );
    }
}
