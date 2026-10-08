//! Tests for branch lock concurrency and transaction boundary enforcement (ticket #86).
//!
//! Verifies that statements in `xc-rate` that must run inside the branch transaction
//! actually execute inside `BranchTx::run` on `tx`, rather than executing on the pool.
//!
//! Covers mutations X1 to X4:
//! - X1: in `BranchService::create`, the repeat look-up `find_by_request` runs inside the branch transaction.
//! - X2: in `RateService::save_rate`, the insert of the rate runs inside the branch transaction.
//! - X3: in `BranchService::create`, `count` runs inside the branch transaction.
//! - X4: in `BranchService::create`, `insert` runs inside the branch transaction.

mod common;

use common::*;
use sqlx::PgPool;

/// Takes the branch lock on `branch_id` from a separate transaction, matching `BranchTx::run`.
/// Returns the transaction handle and the PostgreSQL backend PID of the connection holding the lock.
async fn lock_branch(
    pool: &PgPool,
    branch_id: &str,
) -> (sqlx::Transaction<'static, sqlx::Postgres>, i32) {
    let mut tx = pool.begin().await.expect("begin lock transaction");
    let holder_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .expect("get backend pid");

    sqlx::query(
        "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 0) \
         ON CONFLICT (branch_id) DO NOTHING",
    )
    .bind(branch_id)
    .execute(&mut *tx)
    .await
    .expect("insert branch_versions");

    sqlx::query("SELECT version FROM branch_versions WHERE branch_id = $1 FOR UPDATE")
        .bind(branch_id)
        .execute(&mut *tx)
        .await
        .expect("select version for update");

    (tx, holder_pid)
}

/// Waits until a backend in this database is waiting for a lock and blocked by `holder_pid`.
/// Observes database state via `pg_locks`, `pg_stat_activity`, and `pg_blocking_pids` without sleep.
/// Fails fast if the spawned task exits early, surfacing the actual error (#415 diagnostic step).
async fn wait_until_waiting_for_branch_lock<T: std::fmt::Debug>(
    pool: &PgPool,
    holder_pid: i32,
    task: &mut tokio::task::JoinHandle<T>,
) {
    loop {
        if task.is_finished() {
            let res = task.await;
            panic!("command finished before waiting for branch lock: {res:?}");
        }

        let is_waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS(\
                SELECT 1 FROM pg_locks l \
                JOIN pg_stat_activity a ON l.pid = a.pid \
                WHERE a.datname = current_database() \
                  AND NOT l.granted \
                  AND $1 = ANY(pg_blocking_pids(a.pid))\
             )",
        )
        .bind(holder_pid)
        .fetch_one(pool)
        .await
        .expect("query lock status in wait_until_waiting_for_branch_lock");

        if is_waiting {
            break;
        }
        tokio::task::yield_now().await;
    }
}

// ── Nothing survives a refusal / rollback ───────────────────────────────────

#[tokio::test]
async fn branch_create_insert_runs_in_transaction() {
    let db = TestDb::start().await;

    // Set version of MAIN in branch_versions to i64::MAX so that BranchTx::run
    // fails with integer overflow when incrementing version after the closure.
    sqlx::query(
        "INSERT INTO branch_versions (branch_id, version) VALUES ('MAIN', $1) \
         ON CONFLICT (branch_id) DO UPDATE SET version = $1",
    )
    .bind(i64::MAX)
    .execute(&db.pool)
    .await
    .unwrap();

    let branches = db.branches();
    let req = request_id();
    let res = branches
        .create(
            CreateBranch {
                request_id: req.clone(),
                name: "Silom Branch".to_string(),
                from_branch: branch("MAIN"),
            },
            ACTOR,
            at(0),
        )
        .await;

    assert!(
        res.is_err(),
        "create must fail when version increment overflows"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM branches WHERE request_id = $1")
        .bind(&req)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "no branch row must survive when transaction rolls back"
    );

    db.finish().await;
}

#[tokio::test]
async fn rate_save_insert_runs_in_transaction() {
    let db = TestDb::start().await;
    let b = branch("MAIN");

    // Set version of MAIN in branch_versions to i64::MAX so that BranchTx::run
    // fails with integer overflow when incrementing version after the closure.
    sqlx::query(
        "INSERT INTO branch_versions (branch_id, version) VALUES ($1, $2) \
         ON CONFLICT (branch_id) DO UPDATE SET version = $2",
    )
    .bind(b.as_str())
    .bind(i64::MAX)
    .execute(&db.pool)
    .await
    .unwrap();

    let rates = db.rates();
    let req = request_id();
    let cmd = SaveRate {
        request_id: req.clone(),
        code: "USD".to_string(),
        buy: "30.0000".to_string(),
        sell: "31.0000".to_string(),
        partner_name: String::new(),
        partner_cost: None,
        previous_updated: String::new(),
    };
    let res = rates.save_rate(&b, cmd, ACTOR, at(0)).await;

    assert!(
        res.is_err(),
        "save_rate must fail when version increment overflows"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM rates WHERE request_id = $1")
        .bind(&req)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "no rate row must survive when transaction rolls back"
    );

    db.finish().await;
}

// ── The lock is held while the command reads and writes ─────────────────────

#[tokio::test]
async fn branch_create_waits_for_branch_lock_and_writes_nothing_while_held() {
    let db = TestDb::start().await;
    let (lock_tx, holder_pid) = lock_branch(&db.pool, "MAIN").await;

    let branches = db.branches();
    let req = request_id();
    let cmd = CreateBranch {
        request_id: req.clone(),
        name: "Asok".to_string(),
        from_branch: branch("MAIN"),
    };

    let mut task = tokio::spawn(async move { branches.create(cmd, ACTOR, at(0)).await });

    within(
        MUST_FINISH,
        "wait for branch lock",
        wait_until_waiting_for_branch_lock(&db.pool, holder_pid, &mut task),
    )
    .await;
    assert!(
        !task.is_finished(),
        "command must be waiting for the branch lock"
    );

    let count_during: i64 =
        sqlx::query_scalar("SELECT count(*) FROM branches WHERE request_id = $1")
            .bind(&req)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(
        count_during, 0,
        "branch must not be written while branch lock is held"
    );

    lock_tx.rollback().await.unwrap();

    let made = within(MUST_FINISH, "finish create", task)
        .await
        .unwrap()
        .expect("create should succeed after lock released");
    assert_eq!(made.name, "Asok");

    let count_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM branches WHERE request_id = $1")
            .bind(&req)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(count_after, 1, "branch must be written after lock released");

    db.finish().await;
}

#[tokio::test]
async fn rate_save_waits_for_branch_lock_and_writes_nothing_while_held() {
    let db = TestDb::start().await;
    let b = branch("MAIN");
    let (lock_tx, holder_pid) = lock_branch(&db.pool, b.as_str()).await;

    let rates = db.rates();
    let req = request_id();
    let b_clone = b.clone();
    let cmd = SaveRate {
        request_id: req.clone(),
        code: "EUR".to_string(),
        buy: "35.0000".to_string(),
        sell: "36.0000".to_string(),
        partner_name: String::new(),
        partner_cost: None,
        previous_updated: String::new(),
    };

    let mut task = tokio::spawn(async move { rates.save_rate(&b_clone, cmd, ACTOR, at(0)).await });

    within(
        MUST_FINISH,
        "wait for branch lock",
        wait_until_waiting_for_branch_lock(&db.pool, holder_pid, &mut task),
    )
    .await;
    assert!(
        !task.is_finished(),
        "command must be waiting for the branch lock"
    );

    let count_during: i64 = sqlx::query_scalar("SELECT count(*) FROM rates WHERE request_id = $1")
        .bind(&req)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        count_during, 0,
        "rate must not be written while branch lock is held"
    );

    lock_tx.rollback().await.unwrap();

    within(MUST_FINISH, "finish save_rate", task)
        .await
        .unwrap()
        .expect("save_rate should succeed after lock released");

    let count_after: i64 = sqlx::query_scalar("SELECT count(*) FROM rates WHERE request_id = $1")
        .bind(&req)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(count_after, 1, "rate must be written after lock released");

    db.finish().await;
}

#[tokio::test]
async fn branch_create_count_runs_inside_branch_lock() {
    let db = TestDb::start().await;

    // Seed 18 branches so MAIN (from migration) + 18 seeded = 19 branches total
    for i in 1..19 {
        db.seed_branch(&format!("B{i:02}"), &format!("Branch {i:02}"))
            .await;
    }
    assert_eq!(db.count("branches").await, 19);

    // Hold the lock on MAIN from a separate transaction
    let (mut lock_tx, holder_pid) = lock_branch(&db.pool, "MAIN").await;

    let branches = db.branches();
    let req = request_id();
    let cmd = CreateBranch {
        request_id: req.clone(),
        name: "Branch 20".to_string(),
        from_branch: branch("MAIN"),
    };

    let mut task = tokio::spawn(async move { branches.create(cmd, ACTOR, at(0)).await });

    within(
        MUST_FINISH,
        "wait for branch lock",
        wait_until_waiting_for_branch_lock(&db.pool, holder_pid, &mut task),
    )
    .await;
    assert!(
        !task.is_finished(),
        "command must be waiting for the branch lock"
    );

    // While the command is waiting, insert the 20th branch from our transaction and commit
    sqlx::query(
        "INSERT INTO branches (id, name, created_at, created_by, request_id) \
         VALUES ($1, $2, now(), $3, $4)",
    )
    .bind("B19")
    .bind("Branch 19")
    .bind(ACTOR)
    .bind(request_id())
    .execute(&mut *lock_tx)
    .await
    .unwrap();

    lock_tx.commit().await.unwrap();

    // Now the command unblocks. Because count runs inside the transaction after acquiring the lock,
    // it must see 20 branches and refuse with TOO_MANY_BRANCHES.
    let res = within(MUST_FINISH, "await create", task).await.unwrap();
    let sentence = refusal(res);
    assert_eq!(sentence, TOO_MANY_BRANCHES);

    let count_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM branches WHERE request_id = $1")
            .bind(&req)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(count_after, 0, "rejected branch must not be inserted");

    db.finish().await;
}

#[tokio::test]
async fn branch_create_find_by_request_runs_inside_branch_lock() {
    let db = TestDb::start().await;

    // Hold the lock on MAIN from a separate transaction
    let (mut lock_tx, holder_pid) = lock_branch(&db.pool, "MAIN").await;

    let branches = db.branches();
    let req = request_id();
    let cmd = CreateBranch {
        request_id: req.clone(),
        name: "Intended Name".to_string(),
        from_branch: branch("MAIN"),
    };

    let mut task = tokio::spawn(async move { branches.create(cmd, ACTOR, at(0)).await });

    within(
        MUST_FINISH,
        "wait for branch lock",
        wait_until_waiting_for_branch_lock(&db.pool, holder_pid, &mut task),
    )
    .await;
    assert!(
        !task.is_finished(),
        "command must be waiting for the branch lock"
    );

    // While the command is waiting, insert a branch with the same request_id but a different name, and commit
    sqlx::query(
        "INSERT INTO branches (id, name, created_at, created_by, request_id) \
         VALUES ($1, $2, now(), $3, $4)",
    )
    .bind("B-CONCURRENT")
    .bind("Different Name")
    .bind(ACTOR)
    .bind(&req)
    .execute(&mut *lock_tx)
    .await
    .unwrap();

    lock_tx.commit().await.unwrap();

    // Now the command unblocks. Because find_by_request runs inside the transaction after acquiring the lock,
    // it sees the existing request_id with different name and refuses with REQUEST_CHANGED.
    let res = within(MUST_FINISH, "await create", task).await.unwrap();
    let sentence = refusal(res);
    assert_eq!(sentence, REQUEST_CHANGED);

    db.finish().await;
}

#[tokio::test]
async fn branch_create_uncommitted_row_does_not_affect_command_after_rollback() {
    let db = TestDb::start().await;

    // Hold the lock on MAIN from a separate transaction
    let (mut lock_tx, holder_pid) = lock_branch(&db.pool, "MAIN").await;

    let branches = db.branches();
    let req = request_id();
    let cmd = CreateBranch {
        request_id: req.clone(),
        name: "New Branch".to_string(),
        from_branch: branch("MAIN"),
    };

    let mut task = tokio::spawn(async move { branches.create(cmd, ACTOR, at(0)).await });

    within(
        MUST_FINISH,
        "wait for branch lock",
        wait_until_waiting_for_branch_lock(&db.pool, holder_pid, &mut task),
    )
    .await;
    assert!(
        !task.is_finished(),
        "command must be waiting for the branch lock"
    );

    // In the second connection, change what the command would read without committing
    sqlx::query(
        "INSERT INTO branches (id, name, created_at, created_by, request_id) \
         VALUES ($1, $2, now(), $3, $4)",
    )
    .bind("B-UNCOMMITTED")
    .bind("Uncommitted Branch")
    .bind(ACTOR)
    .bind(&req)
    .execute(&mut *lock_tx)
    .await
    .unwrap();

    // Release by rolling back
    lock_tx.rollback().await.unwrap();

    // The command unblocks and succeeds normally with its intended name
    let made = within(MUST_FINISH, "await create", task)
        .await
        .unwrap()
        .expect("create should succeed after uncommitted row is rolled back");
    assert_eq!(made.name, "New Branch");

    db.finish().await;
}

#[tokio::test]
async fn branch_create_count_runs_on_transaction_connection() {
    let db = TestDb::start().await;

    // Acquire 9 of the pool's 10 connections so only 1 remains for BranchTx::run
    let mut conns = Vec::new();
    for _ in 0..9 {
        conns.push(db.pool.acquire().await.unwrap());
    }

    let branches = db.branches();
    let req = request_id();
    let cmd = CreateBranch {
        request_id: req,
        name: "Conn Test".to_string(),
        from_branch: branch("MAIN"),
    };

    // If count runs on self.pool instead of tx, it tries to acquire a second connection
    // from the exhausted pool and times out when the pool's acquire_timeout (MUST_FINISH = 20s)
    // expires. A bound of 5 seconds safely separates a normal execution from pool exhaustion
    // under heavy CPU load.
    let res = within(
        std::time::Duration::from_secs(5),
        "create must not need extra pool connections during transaction",
        branches.create(cmd, ACTOR, at(0)),
    )
    .await;

    assert!(
        res.is_ok(),
        "create should succeed when only 1 connection is available"
    );

    drop(conns);
    db.finish().await;
}

#[tokio::test]
async fn branch_create_find_by_request_runs_on_transaction_connection() {
    let db = TestDb::start().await;

    let req = request_id();
    // Seed an existing branch with this request_id
    sqlx::query(
        "INSERT INTO branches (id, name, created_at, created_by, request_id) \
         VALUES ('B-EXIST', 'Existing Branch', now(), $1, $2)",
    )
    .bind(ACTOR)
    .bind(&req)
    .execute(&db.pool)
    .await
    .unwrap();

    // Acquire 9 of the pool's 10 connections so only 1 remains for BranchTx::run
    let mut conns = Vec::new();
    for _ in 0..9 {
        conns.push(db.pool.acquire().await.unwrap());
    }

    let branches = db.branches();
    let cmd = CreateBranch {
        request_id: req,
        name: "Existing Branch".to_string(),
        from_branch: branch("MAIN"),
    };

    // If find_by_request runs on self.pool instead of tx, it tries to acquire a second connection
    // from the exhausted pool and times out when the pool's acquire_timeout (MUST_FINISH = 20s)
    // expires. A bound of 5 seconds safely separates a normal execution from pool exhaustion
    // under heavy CPU load.
    let res = within(
        std::time::Duration::from_secs(5),
        "find_by_request must not need extra pool connections during transaction",
        branches.create(cmd, ACTOR, at(0)),
    )
    .await;

    assert!(
        res.is_ok(),
        "create should return existing branch when only 1 connection is available"
    );

    drop(conns);
    db.finish().await;
}
