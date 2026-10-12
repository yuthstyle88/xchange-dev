//! Up-front tests for LG-3 (#925; design `docs/design/blueprint/ledger-inventory.md` §5, §3.3): the
//! `NEGATIVE_STOCK` and `COUNT_DIFFERENCE` approval types and the count-difference gate. Written by
//! the ticket author before the implementation. Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! `NEGATIVE_STOCK`'s hash is an input (lane controller's ruling on #925, 2026-10-10): the subject asks a
//! `NegativeStockSource`, which these tests fake. Computing it over the source lines, and "an APPROVED
//! request lets the posting go negative", are LG-2 #924's and tested there.
//!
//! OR-7.1 #1020 (design `org-roles-branches.md` §3.3, §4.3) moved this file's actors 1:1 onto
//! `access::Actor`, by #1020's ticket author: each v3 employee is the B1-mapped user (MANAGER →
//! BRANCH_MANAGER, FRONT → TELLER, FINANCE → FINANCE) on the same branch, the owner is EXECUTIVE + ADMIN
//! organisation-wide. Every assertion is unchanged.
//!
//! The policy tests need no database. The gate tests need `TEST_DATABASE_URL` naming a database whose
//! name ends in `_test`; they fail without it, they do not skip.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use serde_json::json;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use sqlx::{Postgres, Transaction};
use xc_approval::application::approval_service::{ApprovalError, ApprovalService};
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::domain::request::{ApprovalRequest, Outcome, RequestId};
use xc_approval::domain::subject::ApprovalSubject;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::ledger_approval::count_difference::{
    COUNT_DIFFERENCE, CountDifferenceGate, CountDifferencePolicy, CountDifferenceSubject,
    GateResult,
};
use xc_cash::application::ledger_approval::negative_stock::{
    NEGATIVE_STOCK, NegativeStockPolicy, NegativeStockSource, NegativeStockSubject,
};
use xc_cash::application::ledger_approval::register;
use xc_cash::domain::count::{CashCount, CountKind};
use xc_cash::infrastructure::inventory::repository::PgInventoryRepository;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn since() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

/// A user holding `role` on `branch` (the B1 mapping of a v3 employee, design §3.3).
fn staff(id: &str, role: Role, branch: &str) -> Actor {
    Actor::new(
        id,
        ORG,
        vec![ScopedRole::new(
            role,
            Some(BranchId::new(branch)),
            since(),
            None,
        )],
        None,
    )
}

/// The v3 owner's user: EXECUTIVE and ADMIN, organisation-wide (§3.3).
fn owner(id: &str) -> Actor {
    Actor::new(
        id,
        ORG,
        vec![
            ScopedRole::new(Role::Executive, None, since(), None),
            ScopedRole::new(Role::Admin, None, since(), None),
        ],
        None,
    )
}

fn request(type_name: &str, branch: &str, requested_by: &str) -> ApprovalRequest {
    ApprovalRequest {
        id: RequestId::new("00000000-0000-0000-0000-0000000000aa"),
        type_name: type_name.to_string(),
        branch_id: Some(branch.to_string()),
        object_id: "object-1".to_string(),
        object_version_hash: "h".to_string(),
        policy_version: "p".to_string(),
        proposed_payload: json!({}),
        requested_by: requested_by.to_string(),
        requested_at: Utc.with_ymd_and_hms(2026, 10, 15, 3, 0, 0).unwrap(),
        status: "PENDING".to_string(),
        idempotency_key: "k".to_string(),
    }
}

// ── the policies (no database) ──────────────────────────────────────────────

#[test]
fn negative_stock_is_decided_only_by_the_branch_manager_of_the_requests_branch() {
    let p = NegativeStockPolicy;
    let r = request(NEGATIVE_STOCK, "B1", "teller-1");
    assert!(
        p.may_decide(&staff("mgr-1", Role::BranchManager, "B1"), &r)
            .is_ok()
    );
    for (who, label) in [
        (
            staff("mgr-2", Role::BranchManager, "B2"),
            "a branch manager of another branch",
        ),
        (staff("teller-2", Role::Teller, "B1"), "a teller"),
        (staff("fin-1", Role::Finance, "B1"), "FINANCE"),
        (owner("owner"), "the owner"),
    ] {
        assert!(
            p.may_decide(&who, &r).is_err(),
            "{label} may not decide NEGATIVE_STOCK"
        );
    }
    assert!(!p.allows_self_decision(&staff("mgr-1", Role::BranchManager, "B1")));
    assert!(!p.policy_version().is_empty());
}

#[test]
fn count_difference_is_decided_only_by_finance_of_the_requests_branch() {
    let p = CountDifferencePolicy;
    let r = request(COUNT_DIFFERENCE, "B1", "mgr-1");
    assert!(
        p.may_decide(&staff("fin-1", Role::Finance, "B1"), &r)
            .is_ok()
    );
    for (who, label) in [
        (
            staff("fin-2", Role::Finance, "B2"),
            "FINANCE of another branch (E-7)",
        ),
        (
            staff("mgr-2", Role::BranchManager, "B1"),
            "a branch manager",
        ),
        (staff("teller-2", Role::Teller, "B1"), "a teller"),
        (owner("owner"), "the owner"),
    ] {
        assert!(
            p.may_decide(&who, &r).is_err(),
            "{label} may not decide COUNT_DIFFERENCE"
        );
    }
    assert!(!p.allows_self_decision(&staff("fin-1", Role::Finance, "B1")));
    assert!(!p.policy_version().is_empty());
}

// ── the gate and the COUNT_DIFFERENCE flow (database) ───────────────────────

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
    let schema = unique("lg3");
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

/// A fake source of `NEGATIVE_STOCK` hashes: `object_id → hash`; a missing object fails.
#[derive(Default)]
struct FakeSource(Mutex<HashMap<String, String>>);

impl FakeSource {
    fn set(&self, object_id: &str, hash: &str) {
        self.0
            .lock()
            .unwrap()
            .insert(object_id.to_string(), hash.to_string());
    }
    fn forget(&self, object_id: &str) {
        self.0.lock().unwrap().remove(object_id);
    }
}

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

fn service(pool: &PgPool) -> Arc<ApprovalService> {
    service_with(pool, Arc::new(FakeSource::default()))
}

fn service_with(pool: &PgPool, source: Arc<FakeSource>) -> Arc<ApprovalService> {
    let mut registry = ApprovalRegistry::new();
    register(&mut registry, pool.clone(), source);
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

/// B1 holds 100 USD that cost 3,250.00: the average is 32.50.
async fn usd_position(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb, \
             version) VALUES ($1::uuid, 'B1', 'USD', 100, 3250.00, 1)",
    )
    .bind(ORG)
    .execute(pool)
    .await
    .expect("the USD position");
}

/// `cash_limits` for B1: `Some(limit)` (or `Some(None)` for a NULL limit); `None` = no row.
async fn limit(pool: &PgPool, value: Option<Option<&str>>) {
    if let Some(v) = value {
        sqlx::query(
            "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, \
                 updated_by, updated_at) VALUES ($1::uuid, 'B1', $2::numeric, 'test', now())",
        )
        .bind(ORG)
        .bind(v)
        .execute(pool)
        .await
        .expect("the limit");
    }
}

/// A CLOSE count of B1 counted by `counter`, with totals `(currency, difference)`.
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

async fn requests(pool: &PgPool) -> Vec<(String, String, String, Option<String>)> {
    sqlx::query_as(
        "SELECT id::text, type, object_id, branch_id FROM approval_requests ORDER BY requested_at",
    )
    .fetch_all(pool)
    .await
    .expect("read the requests")
}

async fn check(pool: &PgPool, c: &CashCount, by: &Actor) -> GateResult {
    let g = gate(pool);
    let mut tx = pool.begin().await.expect("begin");
    let out = g.check_in(&mut tx, c, by).await.expect("the gate");
    tx.commit().await.expect("commit");
    out
}

#[tokio::test]
async fn the_subjects_name_their_types() {
    let pool = db().await;
    assert_eq!(
        NegativeStockSubject::new(Arc::new(FakeSource::default())).type_name(),
        "NEGATIVE_STOCK"
    );
    assert_eq!(
        CountDifferenceSubject::new(pool.clone()).type_name(),
        "COUNT_DIFFERENCE"
    );
    pool.close().await;
}

#[tokio::test]
async fn a_difference_at_the_limit_is_clear_and_writes_nothing() {
    let pool = db().await;
    usd_position(&pool).await;
    limit(&pool, Some(Some("65.00"))).await;
    let teller = user(&pool, "teller").await;
    let mgr = user(&pool, "manager").await;
    // -2 USD at the average 32.50 = 65.00.
    let c = count(&pool, &teller, &[("USD", "-2"), ("THB", "0")]).await;
    assert_eq!(
        check(&pool, &c, &staff(&mgr, Role::BranchManager, "B1")).await,
        GateResult::Clear
    );
    assert_eq!(requests(&pool).await, vec![]);
    pool.close().await;
}

#[tokio::test]
async fn a_difference_over_the_limit_awaits_finance_once_per_count() {
    let pool = db().await;
    usd_position(&pool).await;
    limit(&pool, Some(Some("64.99"))).await;
    let teller = user(&pool, "teller").await;
    let mgr = user(&pool, "manager").await;
    let c = count(&pool, &teller, &[("USD", "-2")]).await;
    let by = staff(&mgr, Role::BranchManager, "B1");
    let first = check(&pool, &c, &by).await;
    let GateResult::AwaitingFinance(id) = first.clone() else {
        panic!("65.00 > 64.99 awaits FINANCE: {first:?}");
    };
    let rows = requests(&pool).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, id.as_str().to_string());
    assert_eq!(rows[0].1, "COUNT_DIFFERENCE");
    assert_eq!(rows[0].2, c.id, "the request's object is the count");
    assert_eq!(rows[0].3.as_deref(), Some("B1"));
    assert_eq!(
        check(&pool, &c, &by).await,
        first,
        "the same count answers the same request"
    );
    assert_eq!(requests(&pool).await.len(), 1);
    pool.close().await;
}

#[tokio::test]
async fn the_difference_is_valued_net_across_currencies() {
    let pool = db().await;
    usd_position(&pool).await;
    let teller = user(&pool, "teller").await;
    let mgr = user(&pool, "manager").await;
    let by = staff(&mgr, Role::BranchManager, "B1");
    // -2 USD (-65.00) and +70 THB (+70.00): Σ = +5.00 (design §5.2: |Σ difference valued at average|).
    limit(&pool, Some(Some("5.00"))).await;
    let c = count(&pool, &teller, &[("USD", "-2"), ("THB", "70")]).await;
    assert_eq!(
        check(&pool, &c, &by).await,
        GateResult::Clear,
        "|+5.00| ≤ 5.00"
    );
    sqlx::query("UPDATE cash_limits SET count_difference_limit_thb = 4.99")
        .execute(&pool)
        .await
        .expect("lower the limit");
    let c2 = count(&pool, &teller, &[("USD", "-2"), ("THB", "70")]).await;
    assert!(
        matches!(check(&pool, &c2, &by).await, GateResult::AwaitingFinance(_)),
        "|+5.00| > 4.99"
    );
    pool.close().await;
}

#[tokio::test]
async fn a_null_limit_or_no_row_sends_every_non_zero_difference_to_finance() {
    let pool = db().await;
    usd_position(&pool).await;
    let teller = user(&pool, "teller").await;
    let mgr = user(&pool, "manager").await;
    let by = staff(&mgr, Role::BranchManager, "B1");
    // No row.
    let c = count(&pool, &teller, &[("THB", "0.01")]).await;
    assert!(
        matches!(check(&pool, &c, &by).await, GateResult::AwaitingFinance(_)),
        "no row"
    );
    // A NULL limit.
    limit(&pool, Some(None)).await;
    let c2 = count(&pool, &teller, &[("THB", "-0.01")]).await;
    assert!(
        matches!(check(&pool, &c2, &by).await, GateResult::AwaitingFinance(_)),
        "NULL"
    );
    // A zero difference is always clear.
    let c3 = count(&pool, &teller, &[("USD", "0"), ("THB", "0")]).await;
    assert_eq!(check(&pool, &c3, &by).await, GateResult::Clear, "zero");
    pool.close().await;
}

#[tokio::test]
async fn finance_of_the_branch_decides_and_a_changed_count_is_stale() {
    let pool = db().await;
    usd_position(&pool).await;
    let teller = user(&pool, "teller").await;
    let mgr = user(&pool, "manager").await;
    let fin = user(&pool, "finance").await;
    let by = staff(&mgr, Role::BranchManager, "B1");
    let approvals = service(&pool);

    let c = count(&pool, &teller, &[("THB", "10")]).await;
    let GateResult::AwaitingFinance(id) = check(&pool, &c, &by).await else {
        panic!("no limit: awaits FINANCE");
    };
    // The branch manager who asked cannot decide; another manager cannot either (not FINANCE).
    let mut tx = pool.begin().await.expect("begin");
    let refused = approvals
        .decide(
            &mut tx,
            &id,
            &staff(&user(&pool, "mgr2").await, Role::BranchManager, "B1"),
            Outcome::Approved,
            "ok",
            "k1",
        )
        .await;
    assert!(
        matches!(refused, Err(ApprovalError::PolicyRefused { .. })),
        "{refused:?}"
    );
    drop(tx);

    // A changed total makes the request stale.
    sqlx::query("UPDATE cash_count_totals SET difference = 11 WHERE count_id = $1::uuid")
        .bind(&c.id)
        .execute(&pool)
        .await
        .expect("change the count");
    let mut tx = pool.begin().await.expect("begin");
    let stale = approvals
        .decide(
            &mut tx,
            &id,
            &staff(&fin, Role::Finance, "B1"),
            Outcome::Approved,
            "ok",
            "k2",
        )
        .await;
    assert!(
        matches!(stale, Err(ApprovalError::StaleVersion(_))),
        "{stale:?}"
    );
    drop(tx);

    // Back as it was: FINANCE of B1 approves.
    sqlx::query("UPDATE cash_count_totals SET difference = 10 WHERE count_id = $1::uuid")
        .bind(&c.id)
        .execute(&pool)
        .await
        .expect("restore the count");
    let mut tx = pool.begin().await.expect("begin");
    approvals
        .decide(
            &mut tx,
            &id,
            &staff(&fin, Role::Finance, "B1"),
            Outcome::Approved,
            "ok",
            "k3",
        )
        .await
        .expect("FINANCE of the branch approves");
    tx.commit().await.expect("commit");
    let status: String =
        sqlx::query_scalar("SELECT status FROM approval_requests WHERE id = $1::uuid")
            .bind(id.as_str())
            .fetch_one(&pool)
            .await
            .expect("status");
    assert_eq!(status, "APPROVED");
    pool.close().await;
}

#[tokio::test]
async fn negative_stock_uses_the_sources_hash_stale_when_it_moves_refused_when_it_fails() {
    let pool = db().await;
    let source = Arc::new(FakeSource::default());
    let approvals = service_with(&pool, source.clone());
    let teller = staff(&user(&pool, "teller").await, Role::Teller, "B1");
    let mgr = staff(&user(&pool, "manager").await, Role::BranchManager, "B1");
    let shortfall = json!({ "shortfall": [{ "currency": "USD", "quantity": "-20" }] });

    // Unchanged hash: the branch manager approves.
    source.set("transaction:tx-1", "h1");
    let mut tx = pool.begin().await.expect("begin");
    let r1 = approvals
        .request(
            &mut tx,
            NEGATIVE_STOCK,
            "transaction:tx-1",
            &shortfall,
            &teller,
            "ns-1",
        )
        .await
        .expect("the teller asks");
    approvals
        .decide(&mut tx, &r1.id, &mgr, Outcome::Approved, "ok", "d-1")
        .await
        .expect("the branch manager approves an unchanged source");
    tx.commit().await.expect("commit");

    // The source moved after the request: stale.
    source.set("transaction:tx-2", "h1");
    let mut tx = pool.begin().await.expect("begin");
    let r2 = approvals
        .request(
            &mut tx,
            NEGATIVE_STOCK,
            "transaction:tx-2",
            &shortfall,
            &teller,
            "ns-2",
        )
        .await
        .expect("the teller asks");
    tx.commit().await.expect("commit");
    source.set("transaction:tx-2", "h2");
    let mut tx = pool.begin().await.expect("begin");
    let stale = approvals
        .decide(&mut tx, &r2.id, &mgr, Outcome::Approved, "ok", "d-2")
        .await;
    assert!(
        matches!(stale, Err(ApprovalError::StaleVersion(_))),
        "{stale:?}"
    );
    drop(tx);

    // The source cannot answer: the decision is refused and the request stays PENDING.
    source.forget("transaction:tx-2");
    let mut tx = pool.begin().await.expect("begin");
    let failed = approvals
        .decide(&mut tx, &r2.id, &mgr, Outcome::Approved, "ok", "d-3")
        .await;
    assert!(failed.is_err(), "{failed:?}");
    drop(tx);
    let status: String =
        sqlx::query_scalar("SELECT status FROM approval_requests WHERE id = $1::uuid")
            .bind(r2.id.as_str())
            .fetch_one(&pool)
            .await
            .expect("status");
    assert_eq!(status, "PENDING");
    pool.close().await;
}
