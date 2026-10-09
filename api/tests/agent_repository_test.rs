//! Row AG-1 (`docs/design/xchange-v3.md` §5.4c, *Data migration —
//! `20261008120000_device_agents.sql`*, *Who writes `device_jobs.status`*, *Identity*).
//! Written before the implementation. Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! Part one is the migration, read straight from the database: the table, the four job
//! columns, the widened status CHECK (which also fixes the latent bug that a job could not be
//! stored as `DRAWER_OPENED`, `20261007180000_devices.sql:8`), the indexes, and the four
//! items of its data-migration section.
//!
//! Part two is `AgentRepository` / `PgAgentRepository` and `AgentToken`, whose names and
//! signatures are the ticket's (stubbed in this branch's first commit so that this file
//! compiles; the stubs answer errors or wrong values).
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration, TimeZone, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Row};
use xc_kernel::ids::BranchId;
use xchange::infrastructure::device_agents::agent_repository::{
    AgentRepository, AgentSeen, ClaimArgs, DeviceAgent, DrawerTarget, NewAgent,
};
use xchange::infrastructure::device_agents::agent_token::AgentToken;
use xchange::infrastructure::device_agents::pg_agent_repository::PgAgentRepository;
use xchange::infrastructure::devices::device_job::{JobKind, JobStatus};
use xchange::infrastructure::devices::device_repository::{DeviceRepository, NewJob};
use xchange::infrastructure::devices::pg_device_repository::PgDeviceRepository;

/// The migration of row AG-1 (§5.4c, [required] name).
const MIGRATION: &str = "20261008120000_device_agents.sql";
const MIGRATION_VERSION: i64 = 20261008120000;

const MAIN: &str = "MAIN";
const B9: &str = "B9";

/// Agent ids (UUIDs, lower case).
const A1: &str = "a0000000-0000-4000-8000-000000000001";
const A2: &str = "a0000000-0000-4000-8000-000000000002";
const A9: &str = "a0000000-0000-4000-8000-000000000009";
/// Lease ids.
const L0: &str = "1ea5e000-0000-4000-8000-000000000000";
const L1: &str = "1ea5e000-0000-4000-8000-000000000001";
const L2: &str = "1ea5e000-0000-4000-8000-000000000002";

/// The four statuses `20261007180000_devices.sql:8` admits today.
const TODAY: [&str; 4] = ["QUEUED", "FAILED", "SIMULATED_PRINTED", "COMMAND_SIMULATED"];

/// The older binary's statements on `device_jobs` (`pg_device_repository.rs:24-25`,
/// `:178-181`, `:240-241` at `d6084ea`).
const OLD_INSERT: &str = "INSERT INTO device_jobs \
     (id, request_id, request_hash, branch_id, ref, kind, status, actor, created_at, updated_at) \
     VALUES ($1, $2, 'h', $3, '', $4, $5, 'front1', $6, $6)";
const OLD_SELECT: &str = "SELECT id, request_id, request_hash, branch_id, ref, kind, status, actor, \
     created_at, updated_at FROM device_jobs WHERE branch_id = $1 AND id = $2";
const OLD_UPDATE: &str =
    "UPDATE device_jobs SET status = $1, updated_at = $2 WHERE branch_id = $3 AND id = $4";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

/// 2026-10-08 03:00:00 UTC plus `seconds`, whole milliseconds.
fn t(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_791_428_400, 0)
        .single()
        .expect("an instant")
        + Duration::seconds(seconds)
}

/// A status by its stored text; the two new ones exist only once AG-1 adds them.
fn st(text: &str) -> JobStatus {
    JobStatus::parse(text).unwrap_or_else(|| panic!("JobStatus::parse({text:?}) knows it"))
}

fn ms(millis: i64) -> Duration {
    Duration::milliseconds(millis)
}

fn branch(id: &str) -> BranchId {
    BranchId::new(id)
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

/// A schema of the test's own.
struct Db {
    pool: PgPool,
    schema: String,
    migrator: Migrator,
}

impl Db {
    async fn empty() -> Db {
        let url = std::env::var("TEST_DATABASE_URL")
            .expect("TEST_DATABASE_URL names a database whose name ends in `_test`");
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("ag1");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the schema");
        let migrator = Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations");
        Db {
            pool,
            schema,
            migrator,
        }
    }

    /// Migrated up to (not including) the migration of row AG-1, with the branch B9.
    async fn before_ag1() -> Db {
        let db = Db::empty().await;
        db.migrator
            .run_to(MIGRATION_VERSION - 1, &db.pool)
            .await
            .expect("migrate up to the one before AG-1");
        db.seed_b9().await;
        db
    }

    /// Every migration applied, with the branch B9.
    async fn migrated() -> Db {
        let db = Db::empty().await;
        db.migrator
            .run(&db.pool)
            .await
            .expect("apply api/migrations");
        db.seed_b9().await;
        db
    }

    async fn seed_b9(&self) {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, 'Nine')")
            .bind(B9)
            .execute(&self.pool)
            .await
            .expect("insert the second branch");
    }

    async fn the_rest(&self) {
        self.migrator
            .run(&self.pool)
            .await
            .expect("apply the remaining migrations");
    }

    fn repo(&self) -> PgAgentRepository {
        PgAgentRepository::new(self.pool.clone())
    }

    async fn tx(&self) -> sqlx::Transaction<'static, sqlx::Postgres> {
        self.pool.begin().await.expect("begin")
    }

    /// A job as the older binary writes it.
    async fn job(&self, branch_id: &str, id: &str, kind: &str, status: &str, at: DateTime<Utc>) {
        sqlx::query(OLD_INSERT)
            .bind(id)
            .bind(format!("req-{id}"))
            .bind(branch_id)
            .bind(kind)
            .bind(status)
            .bind(at)
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("insert job {id} ({status}): {e}"));
    }

    /// A job leased to `agent` under `lease` until `until`.
    async fn leased_job(
        &self,
        branch_id: &str,
        id: &str,
        agent: &str,
        lease: &str,
        until: DateTime<Utc>,
    ) {
        self.job(
            branch_id,
            id,
            "DRAWER_REQUEST",
            "QUEUED",
            until - ms(12_000),
        )
        .await;
        sqlx::query(
            "UPDATE device_jobs SET status = 'LEASED', lease_id = $3::uuid, \
             leased_by = $4::uuid, lease_until = $5 WHERE branch_id = $1 AND id = $2",
        )
        .bind(branch_id)
        .bind(id)
        .bind(lease)
        .bind(agent)
        .bind(until)
        .execute(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("lease job {id}: {e}"));
    }

    /// An agent written straight into the table. `paired` gives it a token hash.
    async fn agent(&self, branch_id: &str, id: &str, name: &str, created: DateTime<Utc>) {
        sqlx::query(
            "INSERT INTO device_agents (id, branch_id, name, created_at, created_by) \
             VALUES ($1::uuid, $2, $3, $4, 'owner')",
        )
        .bind(id)
        .bind(branch_id)
        .bind(name)
        .bind(created)
        .execute(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("insert agent {id}: {e}"));
    }

    async fn set(&self, sql: &str, id: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .bind(id)
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql} for {id}: {e}"));
    }

    /// `(status, lease_id, leased_by, lease_until, outcome_reason, updated_at)` of a job.
    async fn job_row(&self, branch_id: &str, id: &str) -> JobRow {
        sqlx::query_as(
            "SELECT status, lease_id::text, leased_by::text, lease_until, outcome_reason, \
             updated_at FROM device_jobs WHERE branch_id = $1 AND id = $2",
        )
        .bind(branch_id)
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("read job {id}: {e}"))
    }

    async fn agent_text(&self, id: &str, column: &str) -> Option<String> {
        sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT {column}::text FROM device_agents WHERE id = $1::uuid"
        )))
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("read {column} of agent {id}: {e}"))
    }

    async fn finish(self) {
        self.pool.close().await;
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("reconnect");
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop the schema");
        admin.close().await;
    }
}

type JobRow = (
    String,
    Option<String>,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<String>,
    DateTime<Utc>,
);

/// `(column, data type, nullable)` of a table, in column order.
async fn columns(pool: &PgPool, table: &str) -> Vec<(String, String, bool)> {
    sqlx::query(
        "SELECT column_name::text, data_type::text, is_nullable::text FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
    )
    .bind(table)
    .fetch_all(pool)
    .await
    .expect("read information_schema.columns")
    .iter()
    .map(|r| {
        (
            r.get::<String, _>(0),
            r.get::<String, _>(1),
            r.get::<String, _>(2) == "YES",
        )
    })
    .collect()
}

fn seen<'a>(version: &'a str, drawer: Option<DrawerTarget<'a>>) -> AgentSeen<'a> {
    AgentSeen {
        version,
        build: "abc1234",
        os: "macos-14.6-arm64",
        host: "counter-1.local",
        drawer,
    }
}

const CUPS_POS80: DrawerTarget<'static> = DrawerTarget {
    transport: "CUPS",
    target: "POS80_",
};

fn ids(jobs: &[xchange::infrastructure::devices::device_job::DeviceJob]) -> Vec<String> {
    jobs.iter().map(|j| j.id.clone()).collect()
}

// ── part one: the migration ──────────────────────────────────────────────────

/// The file is where the design says and runs after the table it changes.
#[tokio::test(flavor = "multi_thread")]
async fn the_migration_file_is_where_the_design_says() {
    let names: BTreeSet<String> = std::fs::read_dir(migrations_dir())
        .expect("api/migrations")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(
        names.contains(MIGRATION),
        "api/migrations/{MIGRATION} exists: {names:?}"
    );
    assert!(MIGRATION > "20261007180000_devices.sql");
    assert_eq!(
        names.iter().filter(|n| n.contains("device_agents")).count(),
        1,
        "one migration for the agents: {names:?}"
    );
}

/// The table of §5.4c with its column types, and the four nullable job columns.
#[tokio::test(flavor = "multi_thread")]
async fn the_table_and_the_four_job_columns_are_as_designed() {
    let db = Db::migrated().await;
    let agents = columns(&db.pool, "device_agents").await;
    let expected: Vec<(&str, &str, bool)> = vec![
        ("id", "uuid", false),
        ("branch_id", "text", false),
        ("name", "text", false),
        ("created_at", "timestamp with time zone", false),
        ("created_by", "text", false),
        ("pair_code_hash", "text", true),
        ("pair_code_expires_at", "timestamp with time zone", true),
        ("token_hash", "text", true),
        ("paired_at", "timestamp with time zone", true),
        ("revoked_at", "timestamp with time zone", true),
        ("revoked_by", "text", true),
        ("last_seen_at", "timestamp with time zone", true),
        ("agent_version", "text", true),
        ("agent_build", "text", true),
        ("agent_os", "text", true),
        ("agent_host", "text", true),
        ("drawer_transport", "text", true),
        ("drawer_target", "text", true),
        ("claim_window_start", "timestamp with time zone", true),
        ("claim_count", "integer", false),
        // OR-1 (#550 D-1): every table carries the organisation.
        ("organization_id", "uuid", false),
    ];
    let got: BTreeSet<(String, String, bool)> = agents.into_iter().collect();
    let want: BTreeSet<(String, String, bool)> = expected
        .into_iter()
        .map(|(c, t, n)| (c.to_owned(), t.to_owned(), n))
        .collect();
    assert_eq!(got, want, "device_agents columns (name, type, nullable)");

    let jobs = columns(&db.pool, "device_jobs").await;
    for (column, kind) in [
        ("lease_id", "uuid"),
        ("leased_by", "uuid"),
        ("lease_until", "timestamp with time zone"),
        ("outcome_reason", "text"),
    ] {
        assert!(
            jobs.contains(&(column.to_owned(), kind.to_owned(), true)),
            "device_jobs.{column} {kind} NULL: {jobs:?}"
        );
    }
    assert_eq!(
        jobs.len(),
        15,
        "ten columns of today, four new and OR-1's organization_id: {jobs:?}"
    );

    // claim_count defaults to 0; leased_by refers to device_agents.
    db.agent(MAIN, A1, "Mac 1", t(0)).await;
    assert_eq!(db.agent_text(A1, "claim_count").await.as_deref(), Some("0"));
    db.job(MAIN, "j1", "DRAWER_REQUEST", "QUEUED", t(0)).await;
    let unknown_agent = sqlx::query(
        "UPDATE device_jobs SET leased_by = 'a0000000-0000-4000-8000-0000000000ff'::uuid \
         WHERE id = 'j1'",
    )
    .execute(&db.pool)
    .await;
    assert!(
        unknown_agent.is_err(),
        "leased_by must name an agent (foreign key)"
    );
    db.finish().await;
}

/// The latent bug of `main` (`20261007180000_devices.sql:8`): a job could not be stored as
/// `DRAWER_OPENED` although `JobStatus::DrawerOpened` exists. Through the device repository
/// that `process` uses.
#[tokio::test(flavor = "multi_thread")]
async fn a_job_can_be_stored_as_drawer_opened() {
    let db = Db::migrated().await;
    let repo = PgDeviceRepository::new(db.pool.clone());
    let main = branch(MAIN);
    let mut tx = db.tx().await;
    let stored = repo
        .insert_job(
            &mut tx,
            &main,
            &NewJob {
                id: "j-open",
                request_id: "cccccccc-0000-4000-8000-000000000001",
                request_hash: "h",
                reference: "",
                kind: JobKind::DrawerRequest,
                actor: "front1",
                at: t(0),
            },
        )
        .await
        .expect("insert a drawer job");
    assert!(stored);
    repo.set_job_status(&mut tx, &main, "j-open", JobStatus::DrawerOpened, t(1))
        .await
        .expect("a drawer job can record that the drawer opened");
    tx.commit().await.expect("commit");

    let mut tx = db.tx().await;
    let job = repo
        .find_job(&mut tx, &main, "j-open")
        .await
        .expect("read it back")
        .expect("the job");
    assert_eq!(job.status, JobStatus::DrawerOpened);
    tx.rollback().await.expect("rollback");
    db.finish().await;
}

/// The widened CHECK admits `LEASED`, `DRAWER_OPENED`, `EXPIRED` beside the four of today,
/// and still refuses any other text (DRV-2's `SENDING` was never adopted).
#[tokio::test(flavor = "multi_thread")]
async fn the_status_check_admits_the_three_new_statuses_and_nothing_else() {
    let db = Db::migrated().await;
    db.job(MAIN, "j1", "DRAWER_REQUEST", "QUEUED", t(0)).await;
    for status in TODAY
        .iter()
        .chain(["LEASED", "DRAWER_OPENED", "EXPIRED"].iter())
    {
        sqlx::query(OLD_UPDATE)
            .bind(status)
            .bind(t(1))
            .bind(MAIN)
            .bind("j1")
            .execute(&db.pool)
            .await
            .unwrap_or_else(|e| panic!("status {status} is admitted: {e}"));
    }
    for status in ["SENDING", "queued", ""] {
        let refused = sqlx::query(OLD_UPDATE)
            .bind(status)
            .bind(t(1))
            .bind(MAIN)
            .bind("j1")
            .execute(&db.pool)
            .await;
        assert!(refused.is_err(), "status {status:?} is refused");
    }
    db.finish().await;
}

/// Data-migration items 1 and 2: every existing row keeps every column and its status and
/// gets no lease; the design's count before deploy names only today's four statuses; no
/// agent exists afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn existing_jobs_keep_everything_and_get_no_lease() {
    let db = Db::before_ag1().await;
    for (i, status) in TODAY.iter().enumerate() {
        db.job(
            MAIN,
            &format!("old-{i}"),
            "DRAWER_REQUEST",
            status,
            t(i as i64),
        )
        .await;
        db.job(
            B9,
            &format!("old-r{i}"),
            "PRINT_RECEIPT",
            status,
            t(i as i64),
        )
        .await;
    }
    let counted: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, count(*) FROM device_jobs GROUP BY status ORDER BY status")
            .fetch_all(&db.pool)
            .await
            .expect("the design's count before deploy");
    assert!(
        counted.iter().all(|(s, _)| TODAY.contains(&s.as_str())),
        "{counted:?}"
    );
    let before: Vec<String> =
        sqlx::query_scalar("SELECT to_jsonb(j)::text FROM device_jobs j ORDER BY branch_id, id")
            .fetch_all(&db.pool)
            .await
            .expect("read the rows");

    db.the_rest().await;

    let after: Vec<String> = sqlx::query_scalar(
        "SELECT (to_jsonb(j) - 'lease_id' - 'leased_by' - 'lease_until' - 'outcome_reason' \
         - 'organization_id')::text \
         FROM device_jobs j ORDER BY branch_id, id",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read the rows again");
    assert_eq!(after, before, "every old column of every row is unchanged");
    let leased: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM device_jobs WHERE lease_id IS NOT NULL OR leased_by IS NOT NULL \
         OR lease_until IS NOT NULL OR outcome_reason IS NOT NULL",
    )
    .fetch_one(&db.pool)
    .await
    .expect("count");
    assert_eq!(leased, 0, "no row gets a lease or a reason");
    let agents: i64 = sqlx::query_scalar("SELECT count(*) FROM device_agents")
        .fetch_one(&db.pool)
        .await
        .expect("count agents");
    assert_eq!(
        agents, 0,
        "the table starts empty: every branch keeps today's path"
    );
    db.finish().await;
}

/// Data-migration item 3: the older binary's insert, read and status write work on the new
/// schema.
#[tokio::test(flavor = "multi_thread")]
async fn the_older_binarys_statements_work_on_the_new_schema() {
    let db = Db::migrated().await;
    db.job(MAIN, "j-old", "PRINT_RECEIPT", "QUEUED", t(0)).await;
    let row = sqlx::query(OLD_SELECT)
        .bind(MAIN)
        .bind("j-old")
        .fetch_one(&db.pool)
        .await
        .expect("the older read");
    assert_eq!(row.get::<String, _>("status"), "QUEUED");
    sqlx::query(OLD_UPDATE)
        .bind("SIMULATED_PRINTED")
        .bind(t(1))
        .bind(MAIN)
        .bind("j-old")
        .execute(&db.pool)
        .await
        .expect("the older status write");
    db.finish().await;
}

/// Data-migration item 4 and the indexes: one active agent per branch is the database's rule
/// (a revoked one does not count; another branch is free); the two hashes are unique; the
/// drawer transport is `CUPS` or `TCP`; the two partial indexes of `device_jobs` exist with
/// their predicates; narrowing the CHECK back fails while a row holds a new status.
#[tokio::test(flavor = "multi_thread")]
async fn the_rules_the_schema_holds() {
    let db = Db::migrated().await;
    db.agent(MAIN, A1, "Mac 1", t(0)).await;
    let second = sqlx::query(
        "INSERT INTO device_agents (id, branch_id, name, created_at, created_by) \
         VALUES ($1::uuid, 'MAIN', 'Mac 2', $2, 'owner')",
    )
    .bind(A2)
    .bind(t(1))
    .execute(&db.pool)
    .await;
    assert!(second.is_err(), "a second active agent in MAIN is refused");
    db.set(
        "UPDATE device_agents SET revoked_at = now(), revoked_by = 'owner' WHERE id = $1::uuid",
        A1,
    )
    .await;
    db.agent(MAIN, A2, "Mac 2", t(2)).await;
    db.agent(B9, A9, "Mac 9", t(2)).await;

    db.set(
        "UPDATE device_agents SET token_hash = 'same', pair_code_hash = 'code' WHERE id = $1::uuid",
        A2,
    )
    .await;
    for column in ["token_hash", "pair_code_hash"] {
        let duplicate = sqlx::query(AssertSqlSafe(format!(
            "UPDATE device_agents SET {column} = (SELECT {column} FROM device_agents WHERE id = $2::uuid) \
             WHERE id = $1::uuid"
        )))
        .bind(A9)
        .bind(A2)
        .execute(&db.pool)
        .await;
        assert!(duplicate.is_err(), "{column} is unique");
    }
    for (transport, ok) in [
        ("CUPS", true),
        ("TCP", true),
        ("USB", false),
        ("cups", false),
    ] {
        let done =
            sqlx::query("UPDATE device_agents SET drawer_transport = $2 WHERE id = $1::uuid")
                .bind(A9)
                .bind(transport)
                .execute(&db.pool)
                .await;
        assert_eq!(done.is_ok(), ok, "drawer_transport {transport}");
    }

    let defs: Vec<(String, String)> = sqlx::query_as(
        "SELECT indexname::text, indexdef FROM pg_indexes \
         WHERE schemaname = current_schema() AND tablename = 'device_jobs'",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read pg_indexes");
    let def = |name: &str| {
        defs.iter()
            .find(|(n, _)| n == name)
            .map(|(_, d)| d.clone())
            .unwrap_or_else(|| panic!("index {name} exists: {defs:?}"))
    };
    let queued = def("device_jobs_drawer_queued_idx");
    assert!(
        queued.contains("(branch_id, created_at)")
            && queued.contains("'QUEUED'")
            && queued.contains("'DRAWER_REQUEST'"),
        "{queued}"
    );
    let leased = def("device_jobs_leased_idx");
    assert!(
        leased.contains("(branch_id, lease_until)") && leased.contains("'LEASED'"),
        "{leased}"
    );

    db.job(MAIN, "j-new", "DRAWER_REQUEST", "QUEUED", t(3))
        .await;
    sqlx::query(OLD_UPDATE)
        .bind("LEASED")
        .bind(t(4))
        .bind(MAIN)
        .bind("j-new")
        .execute(&db.pool)
        .await
        .expect("lease it");
    let narrowed = sqlx::query(
        "ALTER TABLE device_jobs ADD CONSTRAINT device_jobs_today_only CHECK \
         (status IN ('QUEUED', 'FAILED', 'SIMULATED_PRINTED', 'COMMAND_SIMULATED'))",
    )
    .execute(&db.pool)
    .await;
    assert!(
        narrowed.is_err(),
        "what `down` loses: the old CHECK cannot come back while a row holds a new status"
    );
    db.finish().await;
}

// ── part two: JobStatus, AgentToken, AgentRepository ────────────────────────

#[test]
fn job_status_reads_and_writes_the_two_new_statuses() {
    for text in [
        "QUEUED",
        "FAILED",
        "SIMULATED_PRINTED",
        "COMMAND_SIMULATED",
        "DRAWER_OPENED",
        "LEASED",
        "EXPIRED",
    ] {
        assert_eq!(
            JobStatus::parse(text).map(|s| s.as_str()),
            Some(text),
            "{text} round-trips"
        );
    }
    assert_eq!(JobStatus::parse("SENDING"), None);
}

/// §5.4c *Identity*: the token is `xcda_` + 64 lower-case hex digits, never the same twice;
/// the code is 10 Crockford base32 characters shown `XXXXX-XXXXX`; only SHA-256 hex is
/// stored; a typed code is read without its dash, case-insensitively.
#[test]
fn agent_token_builds_and_hashes_the_two_secrets() {
    let token = AgentToken::new_token();
    let hex = token
        .strip_prefix("xcda_")
        .unwrap_or_else(|| panic!("{token:?}"));
    assert_eq!(hex.len(), 64, "{token}");
    assert!(
        hex.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "{token}"
    );
    assert_ne!(AgentToken::new_token(), token, "a new token every time");

    const CROCKFORD: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let code = AgentToken::new_pairing_code();
    assert_eq!(code.len(), 11, "{code:?}");
    assert_eq!(&code[5..6], "-", "{code}");
    assert!(
        code.chars()
            .filter(|c| *c != '-')
            .all(|c| CROCKFORD.contains(c)),
        "{code}"
    );
    assert_ne!(
        AgentToken::new_pairing_code(),
        code,
        "a new code every time"
    );

    assert_eq!(
        AgentToken::hash("abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "lower-case hex SHA-256"
    );
    let stored = AgentToken::hash("ABCDE0FGHJ");
    assert_eq!(AgentToken::code_hash("ABCDE-0FGHJ"), Some(stored.clone()));
    assert_eq!(AgentToken::code_hash("abcde0fghj"), Some(stored.clone()));
    assert_eq!(AgentToken::code_hash(" abcde-0fghj "), Some(stored));
    for bad in [
        "ABCDE-0FGH",
        "ABCDE-0FGHJK",
        "ABCDE-0FGHU",
        "ABCDE-0FGHI",
        "",
    ] {
        assert_eq!(AgentToken::code_hash(bad), None, "{bad:?} is not a code");
    }
}

/// An active agent is one not revoked, paired or not; another branch's does not count.
#[tokio::test(flavor = "multi_thread")]
async fn has_active_agent_in_sees_only_unrevoked_agents_of_the_branch() {
    let db = Db::migrated().await;
    let repo = db.repo();
    let mut tx = db.tx().await;
    assert!(
        !repo
            .has_active_agent_in(&mut tx, &branch(MAIN))
            .await
            .expect("ask")
    );
    tx.rollback().await.expect("rollback");

    db.agent(B9, A9, "Mac 9", t(0)).await;
    db.agent(MAIN, A1, "Mac 1", t(0)).await;
    let mut tx = db.tx().await;
    assert!(
        repo.has_active_agent_in(&mut tx, &branch(MAIN))
            .await
            .expect("ask"),
        "an agent added and not yet paired is active"
    );
    tx.rollback().await.expect("rollback");

    db.set(
        "UPDATE device_agents SET revoked_at = now(), revoked_by = 'owner' WHERE id = $1::uuid",
        A1,
    )
    .await;
    let mut tx = db.tx().await;
    assert!(
        !repo
            .has_active_agent_in(&mut tx, &branch(MAIN))
            .await
            .expect("ask")
    );
    tx.rollback().await.expect("rollback");
    db.finish().await;
}

/// `insert_agent` answers `false`, writing nothing, while the branch has an active agent —
/// a PENDING one included — and `true` once it is revoked; `find` and `list` read it back.
#[tokio::test(flavor = "multi_thread")]
async fn insert_agent_keeps_one_active_agent_per_branch() {
    let db = Db::migrated().await;
    let repo = db.repo();
    let new = |id: &'static str, name: &'static str| NewAgent {
        id,
        name,
        created_by: "owner",
        pair_code_hash: id,
        pair_code_expires_at: t(600),
        at: t(0),
    };
    let mut tx = db.tx().await;
    assert!(
        repo.insert_agent(&mut tx, &branch(MAIN), &new(A1, "Mac 1"))
            .await
            .expect("add")
    );
    assert!(
        !repo
            .insert_agent(&mut tx, &branch(MAIN), &new(A2, "Mac 2"))
            .await
            .expect("add"),
        "the first is PENDING and still counts"
    );
    assert!(
        repo.insert_agent(&mut tx, &branch(B9), &new(A9, "Mac 9"))
            .await
            .expect("add")
    );
    tx.commit().await.expect("commit");

    let first = repo.find(A1).await.expect("find").expect("A1");
    assert_eq!(
        (
            first.branch_id.as_str(),
            first.name.as_str(),
            first.created_by.as_str()
        ),
        (MAIN, "Mac 1", "owner")
    );
    assert_eq!(first.pair_code_expires_at, Some(t(600)));
    assert_eq!((first.paired_at, first.revoked_at), (None, None));
    assert_eq!(
        repo.find(A2).await.expect("find"),
        None,
        "the refused one was not stored"
    );

    db.set(
        "UPDATE device_agents SET revoked_at = now(), revoked_by = 'owner' WHERE id = $1::uuid",
        A1,
    )
    .await;
    let mut tx = db.tx().await;
    assert!(
        repo.insert_agent(&mut tx, &branch(MAIN), &new(A2, "Mac 2"))
            .await
            .expect("add")
    );
    tx.commit().await.expect("commit");

    let listed: Vec<String> = repo
        .list()
        .await
        .expect("list")
        .into_iter()
        .map(|a: DeviceAgent| a.id)
        .collect();
    assert_eq!(
        listed,
        [A9, A1, A2],
        "by branch id (B9 < MAIN), then oldest, then id"
    );
    db.finish().await;
}

/// `pair` takes a live code once: an expired or used code, or a revoked agent's, finds
/// nothing; a paired agent is found by its token hash; a new code (`set_pairing`) re-pairs
/// and the old token hash finds nothing.
#[tokio::test(flavor = "multi_thread")]
async fn pair_takes_a_live_code_once_and_a_new_code_re_pairs() {
    let db = Db::migrated().await;
    let repo = db.repo();
    db.agent(MAIN, A1, "Mac 1", t(0)).await;
    let mut tx = db.tx().await;
    assert!(
        repo.set_pairing(&mut tx, &branch(MAIN), A1, "code-1", t(600))
            .await
            .expect("set a code")
    );
    assert!(
        !repo
            .set_pairing(&mut tx, &branch(B9), A1, "code-x", t(600))
            .await
            .expect("another branch's agent"),
        "set_pairing names the agent's own branch"
    );
    tx.commit().await.expect("commit");

    let s = seen("1.0.0", None);
    assert_eq!(
        repo.pair("code-1", "tok-1", &s, t(600))
            .await
            .expect("pair"),
        None,
        "expired at 600"
    );
    let paired = repo
        .pair("code-1", "tok-1", &s, t(599))
        .await
        .expect("pair")
        .expect("a live code pairs");
    assert_eq!(paired.id, A1);
    assert_eq!(paired.paired_at, Some(t(599)));
    assert_eq!(paired.last_seen_at, Some(t(599)));
    assert_eq!(paired.agent_version.as_deref(), Some("1.0.0"));
    assert_eq!(paired.pair_code_expires_at, None, "the code is used up");
    assert_eq!(db.agent_text(A1, "pair_code_hash").await, None);
    assert_eq!(
        db.agent_text(A1, "token_hash").await.as_deref(),
        Some("tok-1")
    );
    assert_eq!(
        repo.pair("code-1", "tok-x", &s, t(10)).await.expect("pair"),
        None,
        "used once"
    );
    assert_eq!(
        repo.find_by_token_hash("tok-1")
            .await
            .expect("find")
            .map(|a| a.id),
        Some(A1.to_owned())
    );

    let mut tx = db.tx().await;
    assert!(
        repo.set_pairing(&mut tx, &branch(MAIN), A1, "code-2", t(1200))
            .await
            .expect("new code")
    );
    tx.commit().await.expect("commit");
    assert!(
        repo.pair("code-2", "tok-2", &s, t(700))
            .await
            .expect("re-pair")
            .is_some()
    );
    assert_eq!(
        repo.find_by_token_hash("tok-1").await.expect("find"),
        None,
        "the old token stops"
    );
    assert!(
        repo.find_by_token_hash("tok-2")
            .await
            .expect("find")
            .is_some()
    );

    let mut tx = db.tx().await;
    assert!(
        repo.set_pairing(&mut tx, &branch(MAIN), A1, "code-3", t(1800))
            .await
            .expect("code")
    );
    tx.commit().await.expect("commit");
    db.set(
        "UPDATE device_agents SET revoked_at = now(), revoked_by = 'owner' WHERE id = $1::uuid",
        A1,
    )
    .await;
    assert_eq!(
        repo.pair("code-3", "tok-3", &s, t(800))
            .await
            .expect("pair"),
        None,
        "revoked"
    );
    let revoked = repo
        .find_by_token_hash("tok-2")
        .await
        .expect("find")
        .expect("kept for audit");
    assert!(
        revoked.revoked_at.is_some(),
        "a revoked agent is still found, revoked"
    );
    let mut tx = db.tx().await;
    assert!(
        !repo
            .set_pairing(&mut tx, &branch(MAIN), A1, "code-4", t(1800))
            .await
            .expect("code"),
        "a revoked agent gets no new code"
    );
    tx.rollback().await.expect("rollback");
    db.finish().await;
}

/// `touch` writes what the agent says and counts its claims in a window of 60 seconds.
#[tokio::test(flavor = "multi_thread")]
async fn touch_counts_claims_in_a_window_of_sixty_seconds() {
    let db = Db::migrated().await;
    let repo = db.repo();
    db.agent(MAIN, A1, "Mac 1", t(0)).await;
    let s = seen("1.0.1", Some(CUPS_POS80));
    let w = repo
        .touch(A1, &s, t(10))
        .await
        .expect("touch")
        .expect("active");
    assert_eq!((w.count, w.window_start), (1, t(10)));
    let w = repo
        .touch(A1, &s, t(40))
        .await
        .expect("touch")
        .expect("active");
    assert_eq!((w.count, w.window_start), (2, t(10)));
    let w = repo
        .touch(A1, &s, t(70) - ms(1))
        .await
        .expect("touch")
        .expect("active");
    assert_eq!(
        (w.count, w.window_start),
        (3, t(10)),
        "still the first window"
    );
    let w = repo
        .touch(A1, &s, t(70))
        .await
        .expect("touch")
        .expect("active");
    assert_eq!(
        (w.count, w.window_start),
        (1, t(70)),
        "a new window at 60 s"
    );

    let agent = repo.find(A1).await.expect("find").expect("A1");
    assert_eq!(agent.last_seen_at, Some(t(70)));
    assert_eq!(agent.agent_version.as_deref(), Some("1.0.1"));
    assert_eq!(agent.agent_build.as_deref(), Some("abc1234"));
    assert_eq!(agent.agent_os.as_deref(), Some("macos-14.6-arm64"));
    assert_eq!(agent.agent_host.as_deref(), Some("counter-1.local"));
    assert_eq!(agent.drawer_transport.as_deref(), Some("CUPS"));
    assert_eq!(agent.drawer_target.as_deref(), Some("POS80_"));

    let s = seen("1.0.2", None);
    repo.touch(A1, &s, t(71))
        .await
        .expect("touch")
        .expect("active");
    let agent = repo.find(A1).await.expect("find").expect("A1");
    assert_eq!(
        agent.drawer_target.as_deref(),
        Some("POS80_"),
        "no drawer named: kept"
    );

    db.set(
        "UPDATE device_agents SET revoked_at = now(), revoked_by = 'owner' WHERE id = $1::uuid",
        A1,
    )
    .await;
    assert_eq!(
        repo.touch(A1, &s, t(72)).await.expect("touch"),
        None,
        "revoked"
    );
    db.finish().await;
}

/// The probe and the claim (§5.4c *Claim* steps 4 and 5): stale `QUEUED` drawer jobs expire,
/// ended leases lapse, the oldest fresh drawer job is leased; receipts and other branches
/// are never touched. "Stale" is strictly before `stale_before`; "ended" strictly before `now`.
#[tokio::test(flavor = "multi_thread")]
async fn claim_in_expires_lapses_and_leases_the_oldest() {
    let db = Db::migrated().await;
    let repo = db.repo();
    db.agent(MAIN, A1, "Mac 1", t(0)).await;
    db.agent(B9, A9, "Mac 9", t(0)).await;
    let now = t(20);

    assert!(
        !repo.has_claimable(&branch(MAIN), now).await.expect("probe"),
        "nothing yet"
    );
    db.job(MAIN, "r-1", "PRINT_RECEIPT", "QUEUED", t(11)).await;
    assert!(
        !repo.has_claimable(&branch(MAIN), now).await.expect("probe"),
        "a receipt is not"
    );
    db.leased_job(MAIN, "lease-now", A1, L0, now).await;
    assert!(
        !repo.has_claimable(&branch(MAIN), now).await.expect("probe"),
        "a lease ending exactly now has not ended"
    );
    db.leased_job(MAIN, "lease-old", A1, L0, now - ms(1)).await;
    assert!(
        repo.has_claimable(&branch(MAIN), now).await.expect("probe"),
        "an ended lease"
    );

    db.job(MAIN, "q-stale", "DRAWER_REQUEST", "QUEUED", t(10) - ms(1))
        .await;
    db.job(MAIN, "q-edge", "DRAWER_REQUEST", "QUEUED", t(10))
        .await;
    db.job(MAIN, "q-later", "DRAWER_REQUEST", "QUEUED", t(12))
        .await;
    db.job(B9, "q-9", "DRAWER_REQUEST", "QUEUED", t(0)).await;
    assert!(repo.has_claimable(&branch(B9), now).await.expect("probe"));

    let mut tx = db.tx().await;
    let step = repo
        .claim_in(
            &mut tx,
            &branch(MAIN),
            &ClaimArgs {
                agent_id: A1,
                lease_id: L1,
                now,
                stale_before: t(10),
                lease_until: t(30),
            },
        )
        .await
        .expect("claim");
    tx.commit().await.expect("commit");

    assert_eq!(ids(&step.expired), ["q-stale"]);
    assert_eq!(ids(&step.lapsed), ["lease-old"]);
    let leased = step.leased.expect("a job is leased");
    assert_eq!(
        (leased.id.as_str(), leased.status),
        ("q-edge", st("LEASED"))
    );
    assert!(step.expired.iter().all(|j| j.status == st("EXPIRED")));
    assert!(step.lapsed.iter().all(|j| j.status == JobStatus::Failed));

    let (status, lease, by, until, reason, updated) = db.job_row(MAIN, "q-edge").await;
    assert_eq!(
        (
            status.as_str(),
            lease.as_deref(),
            by.as_deref(),
            until,
            reason,
            updated
        ),
        ("LEASED", Some(L1), Some(A1), Some(t(30)), None, now)
    );
    let (status, _, _, _, reason, updated) = db.job_row(MAIN, "q-stale").await;
    assert_eq!((status.as_str(), reason, updated), ("EXPIRED", None, now));
    let (status, lease, _, _, reason, _) = db.job_row(MAIN, "lease-old").await;
    assert_eq!(
        (status.as_str(), lease.as_deref(), reason.as_deref()),
        ("FAILED", Some(L0), Some("LEASE_LAPSED")),
        "a lapse keeps its lease id, so a late report can still be matched"
    );
    assert_eq!(db.job_row(MAIN, "lease-now").await.0, "LEASED");
    assert_eq!(
        db.job_row(MAIN, "q-later").await.0,
        "QUEUED",
        "one job per claim"
    );
    assert_eq!(
        db.job_row(MAIN, "r-1").await.0,
        "QUEUED",
        "receipts are not the agent's"
    );
    assert_eq!(
        db.job_row(B9, "q-9").await.0,
        "QUEUED",
        "another branch is not touched"
    );
    db.finish().await;
}

/// `lease_of_in` reads a job's lease and the reason it was queued with (from its queue line);
/// `report_in` moves a job only from the named status under the same lease.
#[tokio::test(flavor = "multi_thread")]
async fn report_in_moves_only_from_the_named_status_under_the_same_lease() {
    let db = Db::migrated().await;
    let repo = db.repo();
    db.agent(MAIN, A1, "Mac 1", t(0)).await;
    db.leased_job(MAIN, "j-1", A1, L1, t(30)).await;
    sqlx::query(
        "INSERT INTO device_events (branch_id, actor, kind, ref, detail, at) VALUES \
         ('MAIN', 'front1', 'DRAWER_REQUEST', '', '{\"job\":\"j-1\",\"reason\":\"ทอนเงิน\"}', $1), \
         ('MAIN', 'front1', 'DRAWER_REQUEST', '', '{\"job\":\"j-x\",\"reason\":\"อื่น\"}', $1)",
    )
    .bind(t(18))
    .execute(&db.pool)
    .await
    .expect("the queue lines");
    let main = branch(MAIN);

    let mut tx = db.tx().await;
    let lease = repo
        .lease_of_in(&mut tx, &main, "j-1")
        .await
        .expect("read")
        .expect("the job");
    assert_eq!(lease.job.status, st("LEASED"));
    assert_eq!(
        (
            lease.lease_id.as_deref(),
            lease.leased_by.as_deref(),
            lease.lease_until
        ),
        (Some(L1), Some(A1), Some(t(30)))
    );
    assert_eq!(lease.queue_reason.as_deref(), Some("ทอนเงิน"));
    assert_eq!(
        repo.lease_of_in(&mut tx, &branch(B9), "j-1")
            .await
            .expect("read"),
        None
    );

    for (lease, from, to) in [
        (L2, st("LEASED"), JobStatus::DrawerOpened),
        (L1, JobStatus::Queued, JobStatus::DrawerOpened),
        (L1, JobStatus::Failed, JobStatus::DrawerOpened),
    ] {
        assert!(
            !repo
                .report_in(&mut tx, &main, "j-1", lease, from, to, None, t(21))
                .await
                .expect("report"),
            "{lease} {from:?} → {to:?} does not move a LEASED job under {L1}"
        );
    }
    assert!(
        repo.report_in(
            &mut tx,
            &main,
            "j-1",
            L1,
            st("LEASED"),
            JobStatus::DrawerOpened,
            Some(""),
            t(21)
        )
        .await
        .expect("report")
    );
    assert!(
        !repo
            .report_in(
                &mut tx,
                &main,
                "j-1",
                L1,
                st("LEASED"),
                JobStatus::DrawerOpened,
                Some(""),
                t(22)
            )
            .await
            .expect("report"),
        "once"
    );
    tx.commit().await.expect("commit");
    let (status, _, _, _, _, updated) = db.job_row(MAIN, "j-1").await;
    assert_eq!((status.as_str(), updated), ("DRAWER_OPENED", t(21)));

    // a late report replaces a lapse; a failure of another reason is not a lapse
    db.leased_job(MAIN, "j-2", A1, L2, t(30)).await;
    db.leased_job(MAIN, "j-3", A1, L2, t(30)).await;
    sqlx::query(
        "UPDATE device_jobs SET status = 'FAILED', outcome_reason = \
         CASE id WHEN 'j-2' THEN 'LEASE_LAPSED' ELSE 'NO_PULSE' END WHERE id IN ('j-2', 'j-3')",
    )
    .execute(&db.pool)
    .await
    .expect("fail them");
    let mut tx = db.tx().await;
    assert!(
        repo.report_in(
            &mut tx,
            &main,
            "j-2",
            L2,
            JobStatus::Failed,
            JobStatus::DrawerOpened,
            Some(""),
            t(40)
        )
        .await
        .expect("late report")
    );
    assert!(
        !repo
            .report_in(
                &mut tx,
                &main,
                "j-3",
                L2,
                JobStatus::Failed,
                JobStatus::DrawerOpened,
                Some(""),
                t(40)
            )
            .await
            .expect("report"),
        "only a LEASE_LAPSED failure is replaced"
    );
    tx.commit().await.expect("commit");
    let (status, _, _, _, reason, _) = db.job_row(MAIN, "j-2").await;
    assert_eq!(
        (status.as_str(), reason.as_deref()),
        ("DRAWER_OPENED", Some(""))
    );
    db.finish().await;
}

/// `revoke_in` (§5.4c *Identity* item 5): the agent is revoked by the actor at `now`; the
/// branch's `QUEUED` drawer jobs fail `AGENT_REVOKED`; receipts, `LEASED` jobs and another
/// branch's jobs stay; a second revoke, or another branch's agent, finds nothing.
#[tokio::test(flavor = "multi_thread")]
async fn revoke_in_fails_the_branch_s_queued_drawer_jobs_and_leaves_leases() {
    let db = Db::migrated().await;
    let repo = db.repo();
    db.agent(MAIN, A1, "Mac 1", t(0)).await;
    db.agent(B9, A9, "Mac 9", t(0)).await;
    db.job(MAIN, "q-1", "DRAWER_REQUEST", "QUEUED", t(1)).await;
    db.job(MAIN, "q-2", "DRAWER_REQUEST", "QUEUED", t(2)).await;
    db.job(MAIN, "r-1", "PRINT_RECEIPT", "QUEUED", t(1)).await;
    db.job(MAIN, "f-1", "DRAWER_REQUEST", "FAILED", t(1)).await;
    db.leased_job(MAIN, "l-1", A1, L1, t(12)).await;
    db.job(B9, "q-9", "DRAWER_REQUEST", "QUEUED", t(1)).await;

    let mut tx = db.tx().await;
    assert_eq!(
        repo.revoke_in(&mut tx, &branch(B9), A1, "owner", t(5))
            .await
            .expect("revoke"),
        None,
        "A1 is not B9's"
    );
    let failed = repo
        .revoke_in(&mut tx, &branch(MAIN), A1, "owner", t(5))
        .await
        .expect("revoke")
        .expect("A1 is MAIN's active agent");
    tx.commit().await.expect("commit");
    let mut failed_ids = ids(&failed);
    failed_ids.sort();
    assert_eq!(failed_ids, ["q-1", "q-2"]);
    assert!(failed.iter().all(|j| j.status == JobStatus::Failed));
    for id in ["q-1", "q-2"] {
        let (status, _, _, _, reason, updated) = db.job_row(MAIN, id).await;
        assert_eq!(
            (status.as_str(), reason.as_deref(), updated),
            ("FAILED", Some("AGENT_REVOKED"), t(5))
        );
    }
    assert_eq!(db.job_row(MAIN, "r-1").await.0, "QUEUED");
    assert_eq!(
        db.job_row(MAIN, "l-1").await.0,
        "LEASED",
        "a lease is left to lapse"
    );
    assert_eq!(db.job_row(MAIN, "f-1").await.4, None);
    assert_eq!(db.job_row(B9, "q-9").await.0, "QUEUED");

    let agent = repo.find(A1).await.expect("find").expect("A1");
    assert_eq!(agent.revoked_at, Some(t(5)));
    assert_eq!(agent.revoked_by.as_deref(), Some("owner"));
    let mut tx = db.tx().await;
    assert_eq!(
        repo.revoke_in(&mut tx, &branch(MAIN), A1, "owner", t(6))
            .await
            .expect("revoke"),
        None,
        "revoked once"
    );
    tx.rollback().await.expect("rollback");
    db.finish().await;
}
