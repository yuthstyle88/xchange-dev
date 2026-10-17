//! CA-10 (#626): migration M3 (`api/migrations/20261216110000_retire_custody_requests.sql`), which drops the request
//! model and finishes M2's end state. Design `docs/design/blueprint/cash-shift-transfer.md` (#582): §6 (what is
//! retired), §8 M2 "CA-10's part of M2's end state" and §8 M3, §9.1 row CA-10. File name: the lane controller's
//! ruling of 2026-10-13 on #626 (the design's `20261030110000` sorts before M2).
//!
//! Written before the implementation by the ticket author (claude/c4d72c, for the BE lane controller claude/7ca80c).
//! Do not edit, delete or weaken these tests; add your own in another file.
//!
//! Each test stages a schema of its own, migrated up to (not including) M3, writes shifts the way today's binary
//! writes them, then applies M3 alone (`run_to`), so that no later migration can hide what M3 did.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.
//!
//! What M3 must do (§8):
//! - drop `custody_requests` and `custody_signatures` unconverted, `ops_state.custodian_id`, and `ops_state.enabled`
//!   (the lane controller's ruling Q2 on #626, 2026-10-15: CA-10 removes the gate that read it);
//! - drop `shifts_one_open_per_branch_v2` and `shifts.opened_by_count`;
//! - recreate `shifts_one_open_per_teller` over every OPEN shift (`WHERE status = 'OPEN'`), after the precheck
//!   `SELECT teller_id, count(*) FROM shifts WHERE status = 'OPEN' GROUP BY 1 HAVING count(*) > 1;` is empty, else
//!   `RAISE` (close the extra shifts first). A refused M3 changes nothing.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

/// M3 (§8, file name as ruled on #626).
const M3_FILE: &str = "20261216110000_retire_custody_requests.sql";
const M3: i64 = 20261216110000;

/// M3's precheck (§8), word for word.
const PRECHECK: &str =
    "SELECT teller_id, count(*) FROM shifts WHERE status = 'OPEN' GROUP BY 1 HAVING count(*) > 1";

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

/// A schema of its own, migrated up to (not including) M3.
struct Staged {
    pool: PgPool,
    admin_url: PgConnectOptions,
    schema: String,
    migrator: Migrator,
}

impl Staged {
    async fn before_m3() -> Staged {
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
        let schema = unique("ca10m3");
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
            .max_connections(2)
            .connect_with(options.clone().options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrator = Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations");
        assert!(
            migrator.version_exists(M3),
            "api/migrations/{M3_FILE} exists (the M3 of design §8, named as ruled on #626)"
        );
        migrator
            .run_to(M3 - 1, &pool)
            .await
            .expect("migrate up to the one before M3");
        Staged {
            pool,
            admin_url: options,
            schema,
            migrator,
        }
    }

    /// Applies M3 and nothing after it.
    async fn m3(&self) -> Result<(), String> {
        self.migrator
            .run_to(M3, &self.pool)
            .await
            .map_err(|e| e.to_string())
    }

    async fn exec(&self, sql: &str) -> Result<(), String> {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn table_exists(&self, table: &str) -> bool {
        self.count(&format!(
            "SELECT count(*) FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = '{table}'"
        ))
        .await
            == 1
    }

    async fn column_exists(&self, table: &str, column: &str) -> bool {
        self.count(&format!(
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = '{table}' AND column_name = '{column}'"
        ))
        .await
            == 1
    }

    /// The index's definition, or `None` when there is no such index in this schema.
    async fn index_def(&self, index: &str) -> Option<String> {
        sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT indexdef FROM pg_indexes WHERE schemaname = current_schema() AND indexname = '{index}'"
        )))
        .fetch_optional(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("read pg_indexes: {e}"))
    }

    async fn branch(&self, id: &str) {
        // The migrations seed MAIN; another branch is added as the binary adds one.
        self.exec(&format!(
            "INSERT INTO branches (id, name) VALUES ('{id}', '{id}') ON CONFLICT (id) DO NOTHING"
        ))
        .await
        .expect("insert a branch");
    }

    /// A user of the organisation; returns its id.
    async fn teller(&self, name: &str) -> String {
        sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT id, $1, 'ACTIVE', 'test' FROM organizations WHERE code = 'XC' RETURNING id::text",
        )
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .expect("insert a user of the XC organisation")
    }

    /// An OPEN shift as v2's path writes it today (no `opened_by_count`).
    async fn v2_open_shift(&self, id: &str, branch: &str, teller: &str) -> Result<(), String> {
        self.exec(&format!(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('{id}', '{branch}', 'OPEN', now(), '{teller}', '{id}', '{teller}'::uuid)"
        ))
        .await
    }

    async fn finish(self) {
        self.pool.close().await;
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(self.admin_url)
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {} CASCADE",
            self.schema
        )))
        .execute(&admin)
        .await
        .expect("drop the test schema");
        admin.close().await;
    }
}

/// §8 M3 item 1 and "CA-10's part of M2's end state": the request tables, `custodian_id`, `opened_by_count` and the
/// v2 per-branch index are gone after M3.
#[tokio::test]
async fn m3_drops_the_request_model_the_custodian_and_m2s_marker() {
    let db = Staged::before_m3().await;
    db.branch("MAIN").await;
    let ploy = db.teller("Ploy").await;
    db.v2_open_shift("SH-1", "MAIN", &ploy)
        .await
        .expect("an OPEN v2 shift before M3");
    assert!(
        db.table_exists("custody_requests").await,
        "M2's world still has the request tables"
    );
    assert!(
        db.column_exists("shifts", "opened_by_count").await,
        "M2 added opened_by_count"
    );

    db.m3()
        .await
        .expect("M3 applies when no teller holds two OPEN shifts");

    assert!(
        !db.table_exists("custody_requests").await,
        "custody_requests is dropped (A9, unconverted)"
    );
    assert!(
        !db.table_exists("custody_signatures").await,
        "custody_signatures is dropped (A9, unconverted)"
    );
    assert!(
        !db.column_exists("ops_state", "custodian_id").await,
        "ops_state.custodian_id is dropped with the custodian rule"
    );
    assert!(
        !db.column_exists("ops_state", "enabled").await,
        "ops_state.enabled is dropped with the Enable gate (#626 Q2)"
    );
    assert!(
        !db.column_exists("shifts", "opened_by_count").await,
        "shifts.opened_by_count is dropped (M2's end state)"
    );
    assert_eq!(
        db.index_def("shifts_one_open_per_branch_v2").await,
        None,
        "shifts_one_open_per_branch_v2 is dropped (M2's end state)"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM shifts WHERE id = 'SH-1' AND status = 'OPEN'")
            .await,
        1,
        "the shift open before M3 is still open after it"
    );
    db.finish().await;
}

/// §8: `shifts_one_open_per_teller` is recreated over every OPEN shift, so one teller cannot hold a second OPEN
/// shift anywhere, whichever path wrote it.
#[tokio::test]
async fn after_m3_one_teller_holds_at_most_one_open_shift_whoever_opened_it() {
    let db = Staged::before_m3().await;
    db.branch("MAIN").await;
    db.branch("B2").await;
    let ploy = db.teller("Ploy").await;
    db.v2_open_shift("SH-1", "MAIN", &ploy)
        .await
        .expect("an OPEN shift before M3");

    db.m3().await.expect("M3 applies");

    let def = db
        .index_def("shifts_one_open_per_teller")
        .await
        .expect("shifts_one_open_per_teller exists after M3");
    assert!(def.contains("UNIQUE"), "the teller index is unique: {def}");
    assert!(
        def.contains("(teller_id)"),
        "the teller index is on teller_id: {def}"
    );
    assert!(
        def.contains("'OPEN'"),
        "the teller index covers OPEN shifts: {def}"
    );
    assert!(
        !def.contains("opened_by_count"),
        "the teller index covers every OPEN shift, not only count-opened ones: {def}"
    );

    let second = db.v2_open_shift("SH-2", "B2", &ploy).await;
    let err = second.expect_err("a second OPEN shift for the same teller is refused after M3");
    assert!(
        err.contains("shifts_one_open_per_teller"),
        "refused by shifts_one_open_per_teller: {err}"
    );
    db.finish().await;
}

/// §7 "several tellers at once": with the v2 per-branch index gone, two tellers hold two OPEN shifts at one
/// branch after M3.
#[tokio::test]
async fn after_m3_two_tellers_hold_two_open_shifts_at_one_branch() {
    let db = Staged::before_m3().await;
    db.branch("MAIN").await;
    let ploy = db.teller("Ploy").await;
    let nok = db.teller("Nok").await;
    db.v2_open_shift("SH-1", "MAIN", &ploy)
        .await
        .expect("an OPEN shift before M3");

    db.m3().await.expect("M3 applies");

    db.v2_open_shift("SH-2", "MAIN", &nok)
        .await
        .expect("a second teller opens a shift at the same branch after M3");
    assert_eq!(
        db.count("SELECT count(*) FROM shifts WHERE branch_id = 'MAIN' AND status = 'OPEN'")
            .await,
        2
    );
    db.finish().await;
}

/// §8: the precheck refuses M3 while a teller holds two OPEN shifts (M2 allowed that for v2-opened shifts at two
/// branches), and a refused M3 leaves the schema as it was.
#[tokio::test]
async fn m3_is_refused_while_a_teller_holds_two_open_shifts_and_changes_nothing() {
    let db = Staged::before_m3().await;
    db.branch("MAIN").await;
    db.branch("B2").await;
    let ploy = db.teller("Ploy").await;
    db.v2_open_shift("SH-1", "MAIN", &ploy)
        .await
        .expect("a v2 OPEN shift at MAIN");
    db.v2_open_shift("SH-2", "B2", &ploy)
        .await
        .expect("M2's per-branch v2 index lets the same teller hold a v2 OPEN shift at B2");
    assert_eq!(
        db.count(&format!("SELECT count(*) FROM ({PRECHECK}) p"))
            .await,
        1
    );

    let refused = db
        .m3()
        .await
        .expect_err("M3 RAISEs while the precheck finds a teller with two OPEN shifts");
    // §8: the precheck refuses first (`RAISE`, close the extra shifts), not the teller index's build failing on
    // the duplicate.
    assert!(
        !refused.contains("could not create unique index")
            && !refused.contains("duplicate key")
            && !refused.contains("23505"),
        "M3 is refused by its precheck's RAISE, not by a unique violation while building the index: {refused}"
    );

    assert!(
        db.table_exists("custody_requests").await,
        "a refused M3 keeps custody_requests"
    );
    assert!(
        db.table_exists("custody_signatures").await,
        "a refused M3 keeps custody_signatures"
    );
    assert!(
        db.column_exists("ops_state", "custodian_id").await,
        "a refused M3 keeps custodian_id"
    );
    assert!(
        db.column_exists("ops_state", "enabled").await,
        "a refused M3 keeps enabled"
    );
    assert!(
        db.column_exists("shifts", "opened_by_count").await,
        "a refused M3 keeps opened_by_count"
    );
    assert!(
        db.index_def("shifts_one_open_per_branch_v2")
            .await
            .is_some(),
        "a refused M3 keeps the v2 index"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM _sqlx_migrations WHERE version = 20261216110000")
            .await,
        0,
        "a refused M3 is not recorded as applied"
    );

    db.exec("UPDATE shifts SET status = 'CLOSED', closed_at = now(), closed_by = 'test' WHERE id = 'SH-2'")
        .await
        .expect("close the extra shift");
    db.m3()
        .await
        .expect("M3 applies once the extra shift is closed");
    assert!(!db.table_exists("custody_requests").await);
    db.finish().await;
}
