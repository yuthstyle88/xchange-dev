//! Up-front tests for LG-7.0 (#1054; design `docs/design/blueprint/ledger-inventory.md` §6.1): the
//! reconcile core, with fake checks. Written by the ticket author before the implementation. Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it
//! does not skip. Each test gets its own schema; no migration is needed.

use std::str::FromStr;
use std::sync::Arc;

use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use xc_cash::application::reconcile::{
    CheckResult, Offender, Reconcile, ReconcileCheck, ReconcileScope,
};

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

/// A pool on a fresh schema of the test database, holding one table `marks(n int)`.
async fn pool() -> PgPool {
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
    let schema = unique("lg70");
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
    sqlx::query("CREATE TABLE marks (n int)")
        .execute(&pool)
        .await
        .expect("create marks");
    pool
}

fn scope() -> ReconcileScope {
    ReconcileScope {
        organization_id: ORG.to_string(),
        branch: None,
        period: None,
    }
}

fn offender(key: &str) -> Offender {
    Offender {
        key: key.to_string(),
        detail: format!("detail of {key}"),
    }
}

/// A fake check answering a fixed list of offenders.
struct Fixed {
    id: u8,
    offenders: Vec<Offender>,
}

impl ReconcileCheck for Fixed {
    fn id(&self) -> u8 {
        self.id
    }
    fn run_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _scope: &'a ReconcileScope,
    ) -> BoxFuture<'a, AppResult<Vec<Offender>>> {
        let out = self.offenders.clone();
        Box::pin(async move { Ok(out) })
    }
}

/// A fake check that fails.
struct Failing(u8);

impl ReconcileCheck for Failing {
    fn id(&self) -> u8 {
        self.0
    }
    fn run_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _scope: &'a ReconcileScope,
    ) -> BoxFuture<'a, AppResult<Vec<Offender>>> {
        Box::pin(async { Err(AppError::Validation("the fake check fails".into())) })
    }
}

/// A fake check that reports what it sees inside the run's transaction as one offender, and, when
/// `commit_after` is set, then commits a row into `marks` from **another** connection.
struct Observer {
    id: u8,
    outside: PgPool,
    commit_after: bool,
}

impl ReconcileCheck for Observer {
    fn id(&self) -> u8 {
        self.id
    }
    fn run_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        _scope: &'a ReconcileScope,
    ) -> BoxFuture<'a, AppResult<Vec<Offender>>> {
        Box::pin(async move {
            let (marks, read_only, isolation): (i64, String, String) = sqlx::query_as(
                "SELECT (SELECT count(*) FROM marks), current_setting('transaction_read_only'), \
                        current_setting('transaction_isolation')",
            )
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;
            if self.commit_after {
                sqlx::query("INSERT INTO marks VALUES (1)")
                    .execute(&self.outside)
                    .await
                    .map_err(AppError::Database)?;
            }
            Ok(vec![Offender {
                key: format!("check {}", self.id),
                detail: format!("marks={marks} read_only={read_only} isolation={isolation}"),
            }])
        })
    }
}

/// A fake check that tries to write.
struct Writer(u8);

impl ReconcileCheck for Writer {
    fn id(&self) -> u8 {
        self.0
    }
    fn run_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        _scope: &'a ReconcileScope,
    ) -> BoxFuture<'a, AppResult<Vec<Offender>>> {
        Box::pin(async move {
            sqlx::query("INSERT INTO marks VALUES (2)")
                .execute(&mut **tx)
                .await
                .map_err(AppError::Database)?;
            Ok(Vec::new())
        })
    }
}

#[tokio::test]
async fn the_report_is_in_id_order_whatever_the_registration_order() {
    let pool = pool().await;
    let core = Reconcile::new(vec![
        Arc::new(Fixed {
            id: 5,
            offenders: vec![],
        }),
        Arc::new(Fixed {
            id: 1,
            offenders: vec![],
        }),
        Arc::new(Fixed {
            id: 3,
            offenders: vec![],
        }),
    ]);
    let report = core.run(&pool, &scope()).await.expect("a report");
    let ids: Vec<u8> = report.checks.iter().map(|c| c.id).collect();
    assert_eq!(ids, [1, 3, 5]);
    pool.close().await;
}

#[tokio::test]
async fn a_check_with_offenders_is_not_ok_and_keeps_them_in_order() {
    let pool = pool().await;
    let core = Reconcile::new(vec![Arc::new(Fixed {
        id: 2,
        offenders: vec![offender("B1:USD"), offender("B2:EUR")],
    })]);
    let report = core.run(&pool, &scope()).await.expect("a report");
    assert_eq!(
        report.checks,
        vec![CheckResult {
            id: 2,
            ok: false,
            offenders: vec![offender("B1:USD"), offender("B2:EUR")],
        }]
    );
    pool.close().await;
}

#[tokio::test]
async fn a_check_without_offenders_is_ok() {
    let pool = pool().await;
    let core = Reconcile::new(vec![
        Arc::new(Fixed {
            id: 1,
            offenders: vec![],
        }),
        Arc::new(Fixed {
            id: 4,
            offenders: vec![offender("p-1")],
        }),
    ]);
    let report = core.run(&pool, &scope()).await.expect("a report");
    assert_eq!(
        report.checks,
        vec![
            CheckResult {
                id: 1,
                ok: true,
                offenders: vec![]
            },
            CheckResult {
                id: 4,
                ok: false,
                offenders: vec![offender("p-1")]
            },
        ]
    );
    pool.close().await;
}

#[tokio::test]
async fn a_failing_check_fails_the_run_with_its_error() {
    let pool = pool().await;
    let core = Reconcile::new(vec![
        Arc::new(Fixed {
            id: 1,
            offenders: vec![],
        }),
        Arc::new(Failing(2)),
    ]);
    match core.run(&pool, &scope()).await {
        Err(AppError::Validation(m)) => assert_eq!(m, "the fake check fails"),
        other => panic!("the check's error, got {other:?}"),
    }
    pool.close().await;
}

#[tokio::test]
async fn every_check_runs_in_one_read_only_repeatable_read_snapshot() {
    let pool = pool().await;
    // Check 1 commits a mark from outside after reading; check 2 must not see it, because both read
    // one snapshot taken before the commit.
    let core = Reconcile::new(vec![
        Arc::new(Observer {
            id: 1,
            outside: pool.clone(),
            commit_after: true,
        }),
        Arc::new(Observer {
            id: 2,
            outside: pool.clone(),
            commit_after: false,
        }),
    ]);
    let report = core.run(&pool, &scope()).await.expect("a report");
    let seen: Vec<&str> = report
        .checks
        .iter()
        .map(|c| c.offenders[0].detail.as_str())
        .collect();
    assert_eq!(
        seen,
        [
            "marks=0 read_only=on isolation=repeatable read",
            "marks=0 read_only=on isolation=repeatable read",
        ],
        "one snapshot, read only, repeatable read"
    );
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM marks")
        .fetch_one(&pool)
        .await
        .expect("count marks");
    assert_eq!(after, 1, "the outside commit happened");
    pool.close().await;
}

#[tokio::test]
async fn a_check_that_writes_fails_and_nothing_is_kept() {
    let pool = pool().await;
    let core = Reconcile::new(vec![Arc::new(Writer(3))]);
    assert!(
        core.run(&pool, &scope()).await.is_err(),
        "a write inside the read-only run fails"
    );
    let marks: i64 = sqlx::query_scalar("SELECT count(*) FROM marks")
        .fetch_one(&pool)
        .await
        .expect("count marks");
    assert_eq!(marks, 0, "nothing was written");
    pool.close().await;
}

#[tokio::test]
async fn standard_registers_only_checks_of_section_6_in_increasing_order() {
    let pool = pool().await;
    // Holds whatever LG-7.1–7.5 have registered so far: each id is one of design §6's five, once.
    // The checks may fail on an empty schema, so only the registration is read.
    match Reconcile::standard().run(&pool, &scope()).await {
        Ok(report) => {
            let ids: Vec<u8> = report.checks.iter().map(|c| c.id).collect();
            assert!(
                ids.windows(2).all(|w| w[0] < w[1]),
                "increasing, no repeat: {ids:?}"
            );
            assert!(
                ids.iter().all(|i| (1..=5).contains(i)),
                "ids of §6: {ids:?}"
            );
        }
        Err(AppError::Database(_)) => {
            // A registered check's query needs the ledger tables, which this schema lacks.
        }
        Err(other) => panic!("unexpected error: {other:?}"),
    }
    pool.close().await;
}
