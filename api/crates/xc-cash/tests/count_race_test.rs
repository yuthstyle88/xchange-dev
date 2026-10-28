//! Staff corrections follow-up (#564 issuecomment-6460939428, item 1b): a verify and a reject racing on one waiting
//! count. The CHECK `cash_counts_verified_or_rejected` already stops both from landing; the loser must answer
//! `notWaiting` and change nothing, never a database error.
//!
//! Written before the implementation by the sub-controller (claude/studio-claude-4). Do not edit, delete or weaken
//! these tests; add your own in another file. Needs `TEST_DATABASE_URL` (a database whose name ends in `_test`);
//! nothing here skips.
//!
//! Each test holds the first operation open in one transaction, starts the second on another connection, waits until
//! PostgreSQL reports the second blocked by the first (`pg_blocking_pids`), then commits the first. So the second
//! always read the count before the first committed: the race is forced, not hoped for.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::domain::count::{CashCount, CountKind, CountLine, RejectError};
use xc_cash::domain::error::CashError;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

static LG4_FUND: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

struct Db {
    pool: PgPool,
    #[allow(dead_code)]
    migrator: Migrator,
}

impl Db {
    async fn fresh() -> Db {
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
        let schema = unique("count_race_test");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect to test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create test schema");
        admin.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to test schema");
        let migrator = Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations");
        migrator
            .run(&pool)
            .await
            .expect("every migration applies in version order on a fresh database");
        Db { pool, migrator }
    }

    async fn branch(&self, label: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&id)
            .bind(ORG)
            .execute(&self.pool)
            .await
            .expect("insert a branch");
        lg4_fund_safe(&self.pool, &id, "THB", "1000000.00").await;
        BranchId::new(id)
    }

    async fn person(&self, name: &str, roles: &[Role]) -> Actor {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        for role in roles {
            sqlx::query(
                "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
                 VALUES ($1::uuid, $2, $3::uuid, NULL)",
            )
            .bind(&id)
            .bind(role.as_str())
            .bind(ORG)
            .execute(&self.pool)
            .await
            .expect("insert role");
        }
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Actor::new(
            &id,
            ORG,
            roles
                .iter()
                .map(|r| ScopedRole::new(*r, None, from, None))
                .collect(),
            None,
        )
    }

    async fn denomination(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("denomination {currency} {face}: {e}"))
    }

    /// A person whose roles cover only `branch` (a branch-scoped `user_roles` row).
    async fn person_at(&self, name: &str, role: Role, branch: &BranchId) -> Actor {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
             VALUES ($1::uuid, $2, $3::uuid, $4)",
        )
        .bind(&id)
        .bind(role.as_str())
        .bind(ORG)
        .bind(branch.as_str())
        .execute(&self.pool)
        .await
        .expect("insert a branch role");
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Actor::new(
            &id,
            ORG,
            vec![ScopedRole::new(role, Some(branch.clone()), from, None)],
            None,
        )
    }

    async fn reject(
        &self,
        who: &Actor,
        count_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> Result<CashCount, RejectError> {
        let mut tx = self.pool.begin().await.expect("begin");
        let answer = self
            .counts()
            .reject_in(&mut tx, who, count_id, reason, now)
            .await;
        if answer.is_ok() {
            tx.commit().await.expect("commit");
        }
        answer
    }

    /// `rejected_by`, `rejected_at`, `reject_reason`, `verified_by` of a count, as stored.
    async fn stored(
        &self,
        id: &str,
    ) -> (
        Option<String>,
        Option<DateTime<Utc>>,
        Option<String>,
        Option<String>,
    ) {
        sqlx::query_as(
            "SELECT rejected_by::text, rejected_at, reject_reason, verified_by::text \
             FROM cash_counts WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("read the count")
    }

    /// The ids the `WAITING` list would show for `branch` (no verifier, no rejecter).
    async fn waiting(&self, branch: &BranchId) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT id::text FROM cash_counts \
             WHERE branch_id = $1 AND verified_by IS NULL AND rejected_by IS NULL ORDER BY created_at",
        )
        .bind(branch.as_str())
        .fetch_all(&self.pool)
        .await
        .expect("read the waiting counts")
    }

    fn counts(&self) -> CashCountService {
        CashCountService::pg(self.pool.clone())
    }

    async fn count_shifts_for_teller(&self, teller_id: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM shifts WHERE teller_id = $1::uuid")
            .bind(teller_id)
            .fetch_one(&self.pool)
            .await
            .expect("count shifts for teller")
    }
}

async fn lg4_fund_safe(pool: &PgPool, branch: &str, currency: &str, amount: &str) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
    let mut tx = pool.begin().await.expect("begin");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to: CashPlace::Safe(BranchId::new(branch)),
                currency: xc_kernel::money::CurrencyCode::parse(currency).expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Opening,
                    source_id: format!(
                        "lg4-fund:{branch}:{currency}:{}",
                        LG4_FUND.fetch_add(1, Ordering::Relaxed)
                    ),
                    source_key: "fund".to_string(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("fund the safe through the ledger");
    tx.commit().await.expect("commit");
}

fn cmd(
    branch: &BranchId,
    shift: Option<&str>,
    kind: CountKind,
    lines: Vec<CountLine>,
) -> SubmitCount {
    SubmitCount {
        branch: branch.clone(),
        shift_id: shift.map(str::to_owned),
        kind,
        lines,
        note: None,
        idempotency_key: unique("count"),
    }
}

/// THB 1000 × 2 + 100 × 3 = 2,300.
async fn thb_2300(db: &Db) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 3),
    ]
}

/// A waiting OPEN count of 2,300 THB by `teller`.
async fn waiting_open(db: &Db, branch: &BranchId, teller: &Actor) -> CashCount {
    let lines = thb_2300(db).await;
    db.counts()
        .submit(teller, cmd(branch, None, CountKind::Open, lines), at(0))
        .await
        .expect("an OPEN count")
}

struct Floor {
    db: Db,
    branch: BranchId,
    teller: Actor,
    manager: Actor,
}

async fn floor() -> Floor {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    Floor {
        db,
        branch,
        teller,
        manager,
    }
}

/// Waits until a backend of this database is blocked by `pid` (the first operation's connection).
async fn until_blocked_by(pool: &PgPool, pid: i32) {
    for _ in 0..400 {
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid))",
        )
        .bind(pid)
        .fetch_one(pool)
        .await
        .expect("read pg_stat_activity");
        if n > 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!(
        "the second operation never waited on the first: it did not read the count before the commit"
    );
}

async fn backend_pid(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> i32 {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut **tx)
        .await
        .expect("read pg_backend_pid")
}

const MUST_FINISH: std::time::Duration = std::time::Duration::from_secs(60);

// A verify that read the count before a reject committed answers notWaiting; the count stays rejected and no shift
// opens.
#[tokio::test]
async fn a_verify_that_loses_to_a_reject_is_not_waiting() {
    let f = floor().await;
    let count = waiting_open(&f.db, &f.branch, &f.teller).await;
    let checker = f.db.person("Nok", &[Role::BranchManager]).await;

    let mut first = f.db.pool.begin().await.expect("begin");
    let pid = backend_pid(&mut first).await;
    f.db.counts()
        .reject_in(&mut first, &f.manager, &count.id, "เงินไม่ครบ", at(60))
        .await
        .expect("the reject itself passes");

    let pool = f.db.pool.clone();
    let id = count.id.clone();
    let second = tokio::spawn(async move {
        let mut tx = pool.begin().await.expect("begin");
        CashCountService::pg(pool.clone())
            .verify_in(&mut tx, &checker, &id, at(61))
            .await
            .map(|_| ())
    });
    until_blocked_by(&f.db.pool, pid).await;
    first.commit().await.expect("commit the reject");

    let answer = tokio::time::timeout(MUST_FINISH, second)
        .await
        .expect("the verify finishes")
        .expect("the verify task does not panic");
    match answer {
        Err(AppError::Domain(m)) if m == CashError::NotWaiting.to_string() => {}
        other => panic!("the losing verify answers CashError::NotWaiting, got {other:?}"),
    }
    let (rejected_by, _, reason, verified_by) = f.db.stored(&count.id).await;
    assert_eq!(rejected_by.as_deref(), Some(f.manager.user_id.as_str()));
    assert_eq!(reason.as_deref(), Some("เงินไม่ครบ"));
    assert_eq!(verified_by, None, "the count is not also verified");
    assert_eq!(
        f.db.count_shifts_for_teller(&f.teller.user_id).await,
        0,
        "no shift opened"
    );
}

// A reject that read the count before a verify committed answers NotWaiting; the count stays verified, unrejected.
#[tokio::test]
async fn a_reject_that_loses_to_a_verify_is_not_waiting() {
    let f = floor().await;
    let count = waiting_open(&f.db, &f.branch, &f.teller).await;
    let checker = f.db.person("Nok", &[Role::BranchManager]).await;

    let mut first = f.db.pool.begin().await.expect("begin");
    let pid = backend_pid(&mut first).await;
    f.db.counts()
        .verify_in(&mut first, &checker, &count.id, at(60))
        .await
        .expect("the verify itself passes");

    let pool = f.db.pool.clone();
    let id = count.id.clone();
    let manager = f.manager.clone();
    let second = tokio::spawn(async move {
        let mut tx = pool.begin().await.expect("begin");
        CashCountService::pg(pool.clone())
            .reject_in(&mut tx, &manager, &id, "why", at(61))
            .await
            .map(|_| ())
    });
    until_blocked_by(&f.db.pool, pid).await;
    first.commit().await.expect("commit the verify");

    let answer = tokio::time::timeout(MUST_FINISH, second)
        .await
        .expect("the reject finishes")
        .expect("the reject task does not panic");
    assert!(
        matches!(answer, Err(RejectError::NotWaiting)),
        "the losing reject answers RejectError::NotWaiting, got {answer:?}"
    );
    let (rejected_by, _, _, verified_by) = f.db.stored(&count.id).await;
    assert_eq!(rejected_by, None, "the count is not also rejected");
    assert_eq!(verified_by.as_deref(), Some(checker.user_id.as_str()));
}
