//! #4136 K1: `ShiftLifecycleHooks`, several hooks as one. Design `docs/design/blueprint/queue-day-close.md` §2 row
//! 10 and §3.1: every method calls each hook in order, on the caller's transaction, and answers the first error
//! without calling the hooks after it.
//!
//! Written before the implementation by the ticket author (claude/studio-claude-9). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test` (only for a transaction; no schema is
//! touched); nothing here skips.

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{PgPool, Postgres, Transaction};
use xc_kernel::ids::BranchId;
use xc_kernel::shift_hook::{ShiftLifecycleHook, ShiftLifecycleHooks};

/// What each hook saw: `(hook, method, branch, teller, txid)`. The teller is `-` for `on_day_closed`.
type Log = Arc<Mutex<Vec<(String, &'static str, String, String, i64)>>>;

struct Recording {
    name: &'static str,
    log: Log,
    fail: bool,
}

impl Recording {
    fn hook(name: &'static str, log: &Log, fail: bool) -> Arc<dyn ShiftLifecycleHook> {
        Arc::new(Self {
            name,
            log: log.clone(),
            fail,
        })
    }

    fn record<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        method: &'static str,
        branch: &'a BranchId,
        teller: &'a str,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let txid: i64 = sqlx::query_scalar("SELECT txid_current()")
                .fetch_one(&mut **tx)
                .await?;
            self.log.lock().expect("log lock").push((
                self.name.to_owned(),
                method,
                branch.as_str().to_owned(),
                teller.to_owned(),
                txid,
            ));
            if self.fail {
                return Err(AppError::Domain(format!("{} failed", self.name)));
            }
            Ok(())
        })
    }
}

impl ShiftLifecycleHook for Recording {
    fn on_opened<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        teller_user_id: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        self.record(tx, "on_opened", branch, teller_user_id)
    }

    fn on_closed<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        teller_user_id: &'a str,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        self.record(tx, "on_closed", branch, teller_user_id)
    }

    fn on_day_closed<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        _now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        self.record(tx, "on_day_closed", branch, "-")
    }
}

async fn pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!("TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip.")
    });
    let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
    assert!(
        options.get_database().unwrap_or("").ends_with("_test"),
        "TEST_DATABASE_URL must name a database ending in `_test`"
    );
    PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("connect to the test database")
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 21, 12, 0, 0).unwrap()
}

async fn txid(tx: &mut Transaction<'_, Postgres>) -> i64 {
    sqlx::query_scalar("SELECT txid_current()")
        .fetch_one(&mut **tx)
        .await
        .expect("txid")
}

/// Calls `method` on `hooks`.
async fn call(
    hooks: &ShiftLifecycleHooks,
    method: &'static str,
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
) -> AppResult<()> {
    match method {
        "on_opened" => hooks.on_opened(tx, branch, "teller-1", now()).await,
        "on_closed" => hooks.on_closed(tx, branch, "teller-1", now()).await,
        "on_day_closed" => hooks.on_day_closed(tx, branch, now()).await,
        other => panic!("unknown method {other}"),
    }
}

const METHODS: [&str; 3] = ["on_opened", "on_closed", "on_day_closed"];

/// Row 10: each method calls `a` then `b`, with the same branch and teller, on the caller's transaction.
#[tokio::test]
async fn row10_every_method_calls_each_hook_in_order_on_the_callers_transaction() {
    let pool = pool().await;
    let branch = BranchId::new("XC-K1");
    for method in METHODS {
        let log: Log = Arc::default();
        let hooks = ShiftLifecycleHooks::new(vec![
            Recording::hook("a", &log, false),
            Recording::hook("b", &log, false),
        ]);
        let mut tx = pool.begin().await.expect("begin");
        let mine = txid(&mut tx).await;

        call(&hooks, method, &mut tx, &branch)
            .await
            .unwrap_or_else(|e| panic!("{method}: {e}"));
        tx.rollback().await.expect("rollback");

        let teller = if method == "on_day_closed" {
            "-"
        } else {
            "teller-1"
        };
        let seen = log.lock().expect("log lock").clone();
        assert_eq!(
            seen,
            vec![
                (
                    "a".to_owned(),
                    method,
                    "XC-K1".to_owned(),
                    teller.to_owned(),
                    mine
                ),
                (
                    "b".to_owned(),
                    method,
                    "XC-K1".to_owned(),
                    teller.to_owned(),
                    mine
                ),
            ],
            "{method}: a then b, on the caller's transaction"
        );
    }
}

/// Row 10: the first error is answered, and the hooks after it are not called.
#[tokio::test]
async fn row10_the_first_error_stops_the_rest() {
    let pool = pool().await;
    let branch = BranchId::new("XC-K1");
    for method in METHODS {
        let log: Log = Arc::default();
        let hooks = ShiftLifecycleHooks::new(vec![
            Recording::hook("a", &log, false),
            Recording::hook("b", &log, true),
            Recording::hook("c", &log, false),
        ]);
        let mut tx = pool.begin().await.expect("begin");

        let answer = call(&hooks, method, &mut tx, &branch).await;
        tx.rollback().await.expect("rollback");

        match answer {
            Err(AppError::Domain(msg)) => assert_eq!(msg, "b failed", "{method}: b's error"),
            other => panic!("{method}: expected b's error, got {other:?}"),
        }
        let names: Vec<String> = log
            .lock()
            .expect("log lock")
            .iter()
            .map(|(name, ..)| name.clone())
            .collect();
        assert_eq!(
            names,
            vec!["a", "b"],
            "{method}: c is not called after b fails"
        );
    }
}

/// Row 10: no hooks is a hook that does nothing.
#[tokio::test]
async fn row10_an_empty_composite_answers_ok() {
    let pool = pool().await;
    let branch = BranchId::new("XC-K1");
    let hooks = ShiftLifecycleHooks::new(Vec::new());
    for method in METHODS {
        let mut tx = pool.begin().await.expect("begin");
        call(&hooks, method, &mut tx, &branch)
            .await
            .unwrap_or_else(|e| panic!("{method}: {e}"));
        tx.rollback().await.expect("rollback");
    }
}
