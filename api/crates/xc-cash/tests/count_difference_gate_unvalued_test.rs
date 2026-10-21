//! Regression test for LG-3 (#925, PR #1121):
//! A non-zero difference that cannot be valued (no position, quantity 0, no average)
//! must give AwaitingFinance, never Clear.
//!
//! OR-7.1 #1020 (design `org-roles-branches.md` §3.3, §4.3), by #1020's ticket author: the gate's requester
//! is `access::Actor`, the B1-mapped MANAGER (BRANCH_MANAGER on B1). Every assertion is unchanged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::ledger_approval::count_difference::{CountDifferenceGate, GateResult};
use xc_cash::application::ledger_approval::negative_stock::NegativeStockSource;
use xc_cash::application::ledger_approval::register;
use xc_cash::domain::count::{CashCount, CountKind};
use xc_cash::infrastructure::inventory::repository::PgInventoryRepository;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

/// A user holding `role` on `branch` (the B1 mapping of a v3 employee, design §3.3).
fn staff(id: &str, role: Role, branch: &str) -> Actor {
    Actor::new(
        id,
        ORG,
        vec![ScopedRole::new(
            role,
            Some(BranchId::new(branch)),
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            None,
        )],
        None,
    )
}

#[derive(Default)]
struct FakeSource(Mutex<HashMap<String, String>>);

impl NegativeStockSource for FakeSource {
    fn version_hash<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        let found = self.0.lock().unwrap().get(object_id).cloned();
        Box::pin(async move {
            found.ok_or_else(|| AppError::Validation(format!("no source for {object_id}")))
        })
    }
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

async fn db() -> PgPool {
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
    let schema = unique("lg3_unvalued");
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
    Migrator::new(migrations_dir().as_path())
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("every migration applies");
    for b in ["B1", "B2"] {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(b)
            .execute(&pool)
            .await
            .expect("a branch");
    }
    pool
}

fn service(pool: &PgPool) -> Arc<ApprovalService> {
    let mut registry = ApprovalRegistry::new();
    register(&mut registry, pool.clone(), Arc::new(FakeSource::default()));
    Arc::new(ApprovalService::new(
        registry,
        Arc::new(PgApprovalRepository::new(pool.clone())),
        Arc::new(PgAuditLog::new(pool.clone())),
    ))
}

fn gate(pool: &PgPool) -> CountDifferenceGate {
    CountDifferenceGate::new(
        service(pool),
        Arc::new(PgInventoryRepository::new(pool.clone())),
    )
}

async fn user(pool: &PgPool, name: &str) -> String {
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(ORG)
    .bind(name)
    .fetch_one(pool)
    .await
    .expect("a user")
}

async fn count(pool: &PgPool, counter: &str, totals: &[(&str, &str)]) -> CashCount {
    let id: String = sqlx::query_scalar(
        "INSERT INTO cash_counts (organization_id, branch_id, shift_id, kind, counted_by, \
             counted_at, idempotency_key) \
         VALUES ($1::uuid, 'B1', NULL, 'CLOSE', $2::uuid, now(), $3) RETURNING id::text",
    )
    .bind(ORG)
    .bind(counter)
    .bind(unique("count"))
    .fetch_one(pool)
    .await
    .expect("a count");
    for (cur, diff) in totals {
        sqlx::query(
            "INSERT INTO cash_count_totals (count_id, organization_id, currency, counted, expected, \
                 difference) VALUES ($1::uuid, $2::uuid, $3, 100, 100 - $4::numeric, $4::numeric)",
        )
        .bind(&id)
        .bind(ORG)
        .bind(cur)
        .bind(diff)
        .execute(pool)
        .await
        .expect("a total");
    }
    let now = Utc::now();
    CashCount {
        id,
        organization_id: ORG.to_string(),
        branch_id: BranchId::new("B1"),
        shift_id: None,
        kind: CountKind::parse("CLOSE").expect("CLOSE"),
        counted_by: counter.to_string(),
        counted_at: now,
        verified_by: None,
        verified_at: None,
        note: None,
        idempotency_key: "k".to_string(),
        created_at: now,
        updated_at: now,
    }
}

async fn check(pool: &PgPool, c: &CashCount, by: &Actor) -> GateResult {
    let g = gate(pool);
    let mut tx = pool.begin().await.expect("begin");
    let out = g.check_in(&mut tx, c, by).await.expect("the gate");
    tx.commit().await.expect("commit");
    out
}

#[tokio::test]
async fn unvalued_foreign_difference_with_no_position_and_no_limit_awaits_finance() {
    let pool = db().await;
    let teller = user(&pool, "teller").await;
    let mgr = user(&pool, "manager").await;
    let by = staff(&mgr, Role::BranchManager, "B1");

    // 100 USD found, no USD position, no limit => AwaitingFinance
    let c = count(&pool, &teller, &[("USD", "100")]).await;
    let res = check(&pool, &c, &by).await;
    assert!(
        matches!(res, GateResult::AwaitingFinance(_)),
        "100 USD found, no USD position, no limit => AwaitingFinance, got {res:?}"
    );

    pool.close().await;
}

#[tokio::test]
async fn unvalued_foreign_difference_with_no_position_and_generous_limit_awaits_finance() {
    let pool = db().await;
    let teller = user(&pool, "teller").await;
    let mgr = user(&pool, "manager").await;
    let by = staff(&mgr, Role::BranchManager, "B1");

    sqlx::query(
        "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, \
             updated_by, updated_at) VALUES ($1::uuid, 'B1', 1000000, 'test', now())",
    )
    .bind(ORG)
    .execute(&pool)
    .await
    .expect("the limit");

    // Even with a generous limit, an unvalued foreign difference must never Clear
    let c = count(&pool, &teller, &[("USD", "100")]).await;
    let res = check(&pool, &c, &by).await;
    assert!(
        matches!(res, GateResult::AwaitingFinance(_)),
        "100 USD found, no USD position, generous limit => AwaitingFinance, got {res:?}"
    );

    pool.close().await;
}
