//! XC Counter, ticket CA-0 (`docs/design/xc-counter-app.md` §5.1, §5.3, §8.1): the agent repository's rules for
//! several counters per branch. Written before the implementation. Do not edit, delete or weaken; add tests in a
//! file of your own.
//!
//! | #   | Case | Expected |
//! |-----|------|----------|
//! | R1  | `active_agents_in(MAIN)` | MAIN's counters that are not revoked, PENDING ones included, oldest first then id; B9's and revoked ones left out |
//! | R2  | `has_claimable`, a `NULL`-target drawer job, the agent's `cash_drawer` NULL or false | `false` |
//! | R3  | the same job, `cash_drawer` true | `true` |
//! | R4  | a job targeting another counter | `false` for this one; `true` for the target, even with its `cash_drawer` NULL |
//! | R5  | `claim_in` | leases the oldest job **this** agent may claim; an older job targeting another counter is skipped and stays `QUEUED`; a no-drawer agent leaves `NULL`-target jobs alone |
//! | R6  | `claim_in`, housekeeping | stale `QUEUED` drawer jobs expire and ended leases lapse branch-wide, as today, whoever claims |
//! | R7  | `touch` / `pair` with `drawer: None` | `drawer_transport` and `drawer_target` become `NULL` (written as sent, §5.2; was `COALESCE`) |
//! | R8  | `revoke_in(X)`, another active counter Y with `cash_drawer` on | X's targeted `QUEUED` job and the job `LEASED` to X fail `AGENT_REVOKED`; Y's targeted job and the `NULL`-target `QUEUED` job stay `QUEUED`; the answer lists exactly the failed jobs |
//! | R9  | `revoke_in(X)`, the only other counter has `cash_drawer` NULL | the `NULL`-target `QUEUED` job fails too |
//! | R10 | `revoke_in(X)` | `user_sessions.counter_agent_id` = X is cleared; a session bound to Y keeps Y |
//! | R11 | the `PairedDeviceStore` revoke path (`revoke_in` then `after_revoke_in`, used by the owner's revoke) | the same jobs fail as in R8, each with a `FAILED` line; the bound session is cleared |
//!
//! Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`); nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, Duration, TimeZone, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_kernel::ids::BranchId;
use xchange::infrastructure::device_agents::agent_repository::{
    AgentRepository, AgentSeen, ClaimArgs, DrawerTarget,
};
use xchange::infrastructure::device_agents::pg_agent_repository::PgAgentRepository;
use xchange::infrastructure::paired_devices::store::PairedDeviceStore;

const MAIN: &str = "MAIN";
const B9: &str = "B9";

/// Counter ids (UUIDs, lower case).
const X: &str = "c0000000-0000-4000-8000-00000000000a";
const Y: &str = "c0000000-0000-4000-8000-00000000000b";
const Z: &str = "c0000000-0000-4000-8000-00000000000c";
const R: &str = "c0000000-0000-4000-8000-00000000000d";
const N9: &str = "c0000000-0000-4000-8000-000000000009";
const LEASE: &str = "1ea5e000-0000-4000-8000-0000000000aa";
const LEASE_NEW: &str = "1ea5e000-0000-4000-8000-0000000000bb";

/// 2026-11-06 03:00:00 UTC plus `seconds`.
fn t(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 6, 3, 0, 0)
        .single()
        .expect("an instant")
        + Duration::seconds(seconds)
}

fn branch(id: &str) -> BranchId {
    BranchId::new(id)
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

struct Db {
    pool: PgPool,
    schema: String,
}

impl Db {
    async fn migrated() -> Db {
        let url = std::env::var("TEST_DATABASE_URL")
            .expect("TEST_DATABASE_URL names a database whose name ends in `_test`");
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = xc_testdb::unique_schema_name("xcc0");
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
        Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, 'Nine')")
            .bind(B9)
            .execute(&pool)
            .await
            .expect("insert B9");
        Db { pool, schema }
    }

    fn repo(&self) -> PgAgentRepository {
        PgAgentRepository::new(self.pool.clone())
    }

    async fn tx(&self) -> sqlx::Transaction<'static, sqlx::Postgres> {
        self.pool.begin().await.expect("begin")
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// A counter, paired (it has a token hash), with `cash_drawer` as given (`None` = not set).
    async fn counter(&self, branch_id: &str, id: &str, created: i64, cash_drawer: Option<bool>) {
        sqlx::query(
            "INSERT INTO device_agents (id, branch_id, name, created_at, created_by, token_hash, \
             paired_at, last_seen_at, drawer_transport, drawer_target, cash_drawer) \
             VALUES ($1::uuid, $2, 'Mac', $3, 'owner', $1, $3, $3, 'CUPS', 'POS80_', $4)",
        )
        .bind(id)
        .bind(branch_id)
        .bind(t(created))
        .bind(cash_drawer)
        .execute(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("insert counter {id}: {e}"));
    }

    /// A drawer job `QUEUED` at `t(created)` targeting `target` (`None` = any drawer counter).
    async fn job(&self, branch_id: &str, id: &str, created: i64, target: Option<&str>) {
        sqlx::query(
            "INSERT INTO device_jobs (id, request_id, request_hash, branch_id, ref, kind, status, \
             actor, created_at, updated_at, target_agent_id) \
             VALUES ($1, $1, 'h', $2, '', 'DRAWER_REQUEST', 'QUEUED', 'front1', $3, $3, $4::uuid)",
        )
        .bind(id)
        .bind(branch_id)
        .bind(t(created))
        .bind(target)
        .execute(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("insert job {id}: {e}"));
    }

    /// Job `id` leased to `agent` until `t(until)`.
    async fn lease(&self, id: &str, agent: &str, until: i64) {
        sqlx::query(
            "UPDATE device_jobs SET status = 'LEASED', lease_id = $3::uuid, leased_by = $2::uuid, \
             lease_until = $4 WHERE id = $1",
        )
        .bind(id)
        .bind(agent)
        .bind(LEASE)
        .bind(t(until))
        .execute(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("lease job {id}: {e}"));
    }

    /// `(status, outcome_reason)` of a job.
    async fn job_row(&self, id: &str) -> (String, Option<String>) {
        sqlx::query_as("SELECT status, outcome_reason FROM device_jobs WHERE id = $1")
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
        .unwrap_or_else(|e| panic!("read {column} of {id}: {e}"))
    }

    /// A staff session of a new user in MAIN, bound to `counter`; answers its `user_sessions.hash`.
    async fn bound_session(&self, counter: &str) -> String {
        let org: String = sqlx::query_scalar(
            "SELECT id::text FROM organizations ORDER BY created_at, id LIMIT 1",
        )
        .fetch_one(&self.pool)
        .await
        .expect("the migrations seed the organisation");
        let mut conn = self.pool.acquire().await.expect("a connection");
        let user = xc_testdb::people::seed_person(&mut conn, &org, "teller", &[])
            .await
            .expect("seed a user");
        let token = xc_testdb::sessions::seed_session(&mut conn, &user, Some(MAIN), t(86_400))
            .await
            .expect("seed a session");
        let hash: String = sqlx::query_scalar(
            "UPDATE user_sessions SET counter_agent_id = $2::uuid \
             WHERE hash = encode(sha256(convert_to($1, 'UTF8')), 'hex') RETURNING hash",
        )
        .bind(&token)
        .bind(counter)
        .fetch_one(&self.pool)
        .await
        .expect("bind the session");
        hash
    }

    async fn session_counter(&self, hash: &str) -> Option<String> {
        sqlx::query_scalar("SELECT counter_agent_id::text FROM user_sessions WHERE hash = $1")
            .bind(hash)
            .fetch_one(&self.pool)
            .await
            .expect("read the session")
    }

    async fn failed_lines(&self) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT detail->>'job' FROM device_events WHERE kind = 'FAILED' \
             AND detail->>'reason' = 'AGENT_REVOKED' ORDER BY detail->>'job'",
        )
        .fetch_all(&self.pool)
        .await
        .expect("read lines")
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

fn args<'a>(agent: &'a str, now: i64) -> ClaimArgs<'a> {
    ClaimArgs {
        agent_id: agent,
        lease_id: LEASE_NEW,
        now: t(now),
        stale_before: t(now - 10),
        lease_until: t(now + 10),
    }
}

fn seen(drawer: Option<DrawerTarget<'_>>) -> AgentSeen<'_> {
    AgentSeen {
        version: "2.0.0",
        build: "abc1234",
        os: "macos-15-arm64",
        host: "counter-1.local",
        drawer,
    }
}

/// R1.
#[tokio::test(flavor = "multi_thread")]
async fn active_agents_in_lists_the_branch_s_live_counters_oldest_first() {
    let db = Db::migrated().await;
    db.counter(MAIN, Y, 2, Some(true)).await;
    db.counter(MAIN, X, 1, None).await;
    db.counter(MAIN, Z, 1, Some(false)).await;
    db.counter(MAIN, R, 0, Some(true)).await;
    db.counter(B9, N9, 0, Some(true)).await;
    db.exec(&format!(
        "UPDATE device_agents SET revoked_at = now(), revoked_by = 'owner' WHERE id = '{R}'"
    ))
    .await;
    db.exec(&format!(
        "UPDATE device_agents SET token_hash = NULL, paired_at = NULL, pair_code_hash = 'p', \
         pair_code_expires_at = now() + interval '10 minutes' WHERE id = '{Z}'"
    ))
    .await;

    let mut tx = db.tx().await;
    let listed: Vec<String> = db
        .repo()
        .active_agents_in(&mut tx, &branch(MAIN))
        .await
        .expect("list")
        .into_iter()
        .map(|a| a.id)
        .collect();
    tx.rollback().await.expect("rollback");
    assert_eq!(
        listed,
        [X, Z, Y],
        "oldest first, then id; the PENDING Z counts; revoked R and B9's counter do not"
    );
    db.finish().await;
}

/// R2, R3, R4.
#[tokio::test(flavor = "multi_thread")]
async fn has_claimable_sees_only_jobs_the_agent_may_claim() {
    let db = Db::migrated().await;
    let repo = db.repo();
    db.counter(MAIN, X, 0, None).await;
    db.counter(MAIN, Y, 0, Some(false)).await;
    db.counter(MAIN, Z, 0, Some(true)).await;
    db.job(MAIN, "any", 20, None).await;
    let now = t(25);

    assert!(
        !repo
            .has_claimable(&branch(MAIN), X, now)
            .await
            .expect("probe"),
        "R2: cash_drawer not set = off"
    );
    assert!(
        !repo
            .has_claimable(&branch(MAIN), Y, now)
            .await
            .expect("probe"),
        "R2: cash_drawer off"
    );
    assert!(
        repo.has_claimable(&branch(MAIN), Z, now)
            .await
            .expect("probe"),
        "R3: cash_drawer on"
    );

    db.exec("UPDATE device_jobs SET target_agent_id = 'c0000000-0000-4000-8000-00000000000a' WHERE id = 'any'")
        .await;
    assert!(
        !repo
            .has_claimable(&branch(MAIN), Z, now)
            .await
            .expect("probe"),
        "R4: a job for X is not Z's"
    );
    assert!(
        repo.has_claimable(&branch(MAIN), X, now)
            .await
            .expect("probe"),
        "R4: the target may claim it, whatever its cash_drawer"
    );
    db.finish().await;
}

/// R5, R6.
#[tokio::test(flavor = "multi_thread")]
async fn claim_in_leases_the_oldest_job_this_agent_may_claim() {
    let db = Db::migrated().await;
    let repo = db.repo();
    db.counter(MAIN, X, 0, Some(true)).await;
    db.counter(MAIN, Y, 0, Some(true)).await;
    db.counter(MAIN, Z, 0, None).await;
    db.job(MAIN, "stale", 5, Some(Y)).await;
    db.job(MAIN, "for-y", 21, Some(Y)).await;
    db.job(MAIN, "any", 22, None).await;
    db.job(MAIN, "for-x", 23, Some(X)).await;
    db.job(MAIN, "ended", 1, Some(Y)).await;
    db.lease("ended", Y, 19).await;

    let mut tx = db.tx().await;
    let none = repo
        .claim_in(&mut tx, &branch(MAIN), &args(Z, 20))
        .await
        .expect("claim by Z");
    assert_eq!(
        none.leased, None,
        "R5: Z has no drawer and no job targets it"
    );
    let ids = |jobs: &[xchange::infrastructure::devices::device_job::DeviceJob]| -> Vec<String> {
        jobs.iter().map(|j| j.id.clone()).collect()
    };
    assert_eq!(ids(&none.expired), ["stale"], "R6: expiry is branch-wide");
    assert_eq!(ids(&none.lapsed), ["ended"], "R6: a lapse is branch-wide");

    let step = repo
        .claim_in(&mut tx, &branch(MAIN), &args(X, 24))
        .await
        .expect("claim by X");
    assert_eq!(
        step.leased.map(|j| j.id).as_deref(),
        Some("any"),
        "R5: X skips the older job for Y and takes the oldest it may claim"
    );
    tx.commit().await.expect("commit");
    assert_eq!(
        db.job_row("for-y").await.0,
        "QUEUED",
        "R5: Y's job waits for Y"
    );
    assert_eq!(db.job_row("for-x").await.0, "QUEUED");
    db.finish().await;
}

/// R7.
#[tokio::test(flavor = "multi_thread")]
async fn touch_and_pair_write_the_drawer_as_sent() {
    let db = Db::migrated().await;
    let repo = db.repo();
    db.counter(MAIN, X, 0, Some(true)).await;

    repo.touch(X, &seen(None), t(30))
        .await
        .expect("touch")
        .expect("X is active");
    assert_eq!(db.agent_text(X, "drawer_transport").await, None);
    assert_eq!(db.agent_text(X, "drawer_target").await, None);

    let tcp = DrawerTarget {
        transport: "TCP",
        target: "10.0.0.5:9100",
    };
    repo.touch(X, &seen(Some(tcp)), t(31))
        .await
        .expect("touch")
        .expect("X is active");
    assert_eq!(
        db.agent_text(X, "drawer_transport").await.as_deref(),
        Some("TCP")
    );

    db.exec(&format!(
        "UPDATE device_agents SET pair_code_hash = 'code-x', pair_code_expires_at = '{}' WHERE id = '{X}'",
        t(600).to_rfc3339()
    ))
    .await;
    AgentRepository::pair(&repo, "code-x", "token-x2", &seen(None), t(40))
        .await
        .expect("pair")
        .expect("the code is live");
    assert_eq!(
        db.agent_text(X, "drawer_transport").await,
        None,
        "pair writes as sent too"
    );
    assert_eq!(db.agent_text(X, "drawer_target").await, None);
    db.finish().await;
}

/// The jobs R8–R11 start from: X is being revoked, Y is another counter of MAIN.
async fn revoke_fixture(db: &Db, y_cash_drawer: Option<bool>) -> (String, String) {
    db.counter(MAIN, X, 0, Some(true)).await;
    db.counter(MAIN, Y, 0, y_cash_drawer).await;
    db.counter(B9, N9, 0, Some(true)).await;
    db.job(MAIN, "q-x", 1, Some(X)).await;
    db.job(MAIN, "l-x", 2, None).await;
    db.lease("l-x", X, 30).await;
    db.job(MAIN, "q-y", 3, Some(Y)).await;
    db.job(MAIN, "q-any", 4, None).await;
    db.job(B9, "q-9", 4, None).await;
    let x_session = db.bound_session(X).await;
    let y_session = db.bound_session(Y).await;
    (x_session, y_session)
}

/// R8, R10.
#[tokio::test(flavor = "multi_thread")]
async fn revoke_in_fails_the_counter_s_jobs_and_keeps_the_branch_queue_for_another_drawer() {
    let db = Db::migrated().await;
    let (x_session, y_session) = revoke_fixture(&db, Some(true)).await;

    let mut tx = db.tx().await;
    let failed = AgentRepository::revoke_in(&db.repo(), &mut tx, &branch(MAIN), X, "owner", t(10))
        .await
        .expect("revoke")
        .expect("X is MAIN's active counter");
    tx.commit().await.expect("commit");

    let mut failed: Vec<String> = failed.into_iter().map(|j| j.id).collect();
    failed.sort();
    assert_eq!(failed, ["l-x", "q-x"], "R8: only X's jobs");
    for id in ["l-x", "q-x"] {
        assert_eq!(
            db.job_row(id).await,
            ("FAILED".to_owned(), Some("AGENT_REVOKED".to_owned()))
        );
    }
    assert_eq!(db.job_row("q-y").await.0, "QUEUED");
    assert_eq!(
        db.job_row("q-any").await.0,
        "QUEUED",
        "R8: Y can still take it"
    );
    assert_eq!(db.job_row("q-9").await.0, "QUEUED");
    assert_eq!(db.session_counter(&x_session).await, None, "R10");
    assert_eq!(
        db.session_counter(&y_session).await.as_deref(),
        Some(Y),
        "R10"
    );
    db.finish().await;
}

/// R9.
#[tokio::test(flavor = "multi_thread")]
async fn revoke_in_fails_the_branch_queue_when_no_drawer_counter_remains() {
    let db = Db::migrated().await;
    revoke_fixture(&db, None).await;

    let mut tx = db.tx().await;
    let failed = AgentRepository::revoke_in(&db.repo(), &mut tx, &branch(MAIN), X, "owner", t(10))
        .await
        .expect("revoke")
        .expect("X is MAIN's active counter");
    tx.commit().await.expect("commit");

    let mut failed: Vec<String> = failed.into_iter().map(|j| j.id).collect();
    failed.sort();
    assert_eq!(failed, ["l-x", "q-any", "q-x"], "Y has no drawer set");
    assert_eq!(
        db.job_row("q-y").await.0,
        "QUEUED",
        "a job for Y waits for Y"
    );
    assert_eq!(db.job_row("q-9").await.0, "QUEUED");
    db.finish().await;
}

/// R11.
#[tokio::test(flavor = "multi_thread")]
async fn the_owner_s_revoke_path_follows_the_same_rule() {
    let db = Db::migrated().await;
    let (x_session, y_session) = revoke_fixture(&db, Some(true)).await;
    let repo = db.repo();

    let mut tx = db.tx().await;
    assert!(
        PairedDeviceStore::revoke_in(&repo, &mut tx, &branch(MAIN), X, "owner", t(10))
            .await
            .expect("revoke")
    );
    PairedDeviceStore::after_revoke_in(&repo, &mut tx, &branch(MAIN), X, "owner", t(10))
        .await
        .expect("after revoke");
    tx.commit().await.expect("commit");

    assert_eq!(db.job_row("q-x").await.0, "FAILED");
    assert_eq!(db.job_row("l-x").await.0, "FAILED");
    assert_eq!(db.job_row("q-y").await.0, "QUEUED");
    assert_eq!(db.job_row("q-any").await.0, "QUEUED");
    assert_eq!(
        db.failed_lines().await,
        ["l-x", "q-x"],
        "one FAILED line per failed job"
    );
    assert_eq!(db.session_counter(&x_session).await, None);
    assert_eq!(db.session_counter(&y_session).await.as_deref(), Some(Y));
    db.finish().await;
}
