//! Up-front tests for LG-5 (#928):
//! Count difference gate in CashCountService::verify (design `docs/design/blueprint/ledger-inventory.md` §5, §3.3).
//!
//! Tests every row of #928 §8 (behaviour table), §9 (edge cases) and §7 (error codes & Display).
//!
//! Written by the test-writer before the implementation. Do not edit, delete or weaken.
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::request::{Outcome, RequestId};
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, Refusal, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::ledger_approval::count_difference::register as register_count_difference;
use xc_cash::domain::count::{CashCount, CountKind, CountLine};
use xc_cash::domain::error::CashError;
use xc_kernel::ids::BranchId;
use xc_testdb::people::{RoleRow, seed_person};

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a valid decimal")
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
        let schema = unique("gate_test");
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

    async fn set_cash_limit(&self, branch: &BranchId, limit_thb: Option<Decimal>) {
        sqlx::query(
            "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
             VALUES ($1::uuid, $2, $3, 'test', now()) \
             ON CONFLICT (organization_id, branch_id) DO UPDATE SET count_difference_limit_thb = $3, updated_at = now()",
        )
        .bind(ORG)
        .bind(branch.as_str())
        .bind(limit_thb)
        .execute(&self.pool)
        .await
        .expect("set cash limit");
    }

    async fn person(&self, name: &str, roles: &[Role]) -> Actor {
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let role_rows: Vec<RoleRow> = roles
            .iter()
            .map(|r| RoleRow {
                role_code: r.as_str(),
                branch_id: None,
                valid_from: from,
                valid_to: None,
            })
            .collect();
        let mut conn = self.pool.acquire().await.expect("acquire connection");
        let id = seed_person(&mut conn, ORG, name, &role_rows)
            .await
            .expect("seed person");
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

    fn counts(&self) -> CashCountService {
        CashCountService::pg(self.pool.clone())
    }

    async fn shift_status(&self, id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM shifts WHERE id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("read shift status")
    }

    async fn count_verified_by(&self, id: &str) -> Option<String> {
        sqlx::query_scalar("SELECT verified_by::text FROM cash_counts WHERE id = $1::uuid")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("read count verified_by")
    }

    async fn cash_exceptions_for_count(&self, count_id: &str) -> Vec<serde_json::Value> {
        let rows: Vec<(serde_json::Value,)> =
            sqlx::query_as("SELECT data FROM cash_exceptions WHERE data->>'requestId' = $1")
                .bind(count_id)
                .fetch_all(&self.pool)
                .await
                .expect("read cash exceptions");
        rows.into_iter().map(|(d,)| d).collect()
    }

    async fn approval_requests_for_count(
        &self,
        count_id: &str,
    ) -> Vec<(String, String, String, serde_json::Value)> {
        sqlx::query_as(
            "SELECT id::text, type, status, proposed_payload FROM approval_requests \
             WHERE type = 'COUNT_DIFFERENCE' AND object_id = $1",
        )
        .bind(count_id)
        .fetch_all(&self.pool)
        .await
        .expect("read approval requests")
    }
}

fn approval_service(pool: &PgPool) -> ApprovalService {
    let mut registry = ApprovalRegistry::new();
    register_count_difference(&mut registry, pool.clone());
    ApprovalService::new(
        registry,
        Arc::new(PgApprovalRepository::new(pool.clone())),
        Arc::new(PgAuditLog::new(pool.clone())),
    )
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

/// A teller's shift opened with 2,300 THB expected.
async fn open_shift(
    db: &Db,
    branch: &BranchId,
    teller: &Actor,
    manager: &Actor,
) -> (String, CashCount) {
    let lines = thb_2300(db).await;
    let counts = db.counts();
    let open_count = counts
        .submit(teller, cmd(branch, None, CountKind::Open, lines), at(0))
        .await
        .expect("an OPEN count");
    let verified = counts
        .verify(manager, &open_count.id, at(60))
        .await
        .expect("the OPEN count verifies");
    let shift_id = verified.shift_id.clone().expect("shift opened");
    (shift_id, verified)
}

// ── §8 Row 1: difference 0 ──────────────────────────────────────────────────

#[tokio::test]
async fn verify_count_difference_zero_verifies_without_approval_request() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let (shift_id, _) = open_shift(&db, &branch, &teller, &manager).await;
    let counts = db.counts();

    // CLOSE count exactly matching 2,300 expected (difference = 0)
    let close = counts
        .submit(
            &teller,
            cmd(
                &branch,
                Some(&shift_id),
                CountKind::Close,
                thb_2300(&db).await,
            ),
            at(600),
        )
        .await
        .expect("submit close count");

    let verified = counts
        .verify(&manager, &close.id, at(660))
        .await
        .expect("count with zero difference verifies cleanly");

    assert_eq!(
        verified.verified_by.as_deref(),
        Some(manager.user_id.as_str())
    );
    assert_eq!(db.shift_status(&shift_id).await, "CLOSED");

    let requests = db.approval_requests_for_count(&close.id).await;
    assert!(
        requests.is_empty(),
        "difference 0 must not create any approval request: {requests:?}"
    );

    let exceptions = db.cash_exceptions_for_count(&close.id).await;
    assert!(
        exceptions.is_empty(),
        "difference 0 must not write cash exceptions: {exceptions:?}"
    );
}

// ── §8 Row 2: valued difference ≤ branch limit ──────────────────────────────

#[tokio::test]
async fn verify_count_difference_within_branch_limit_verifies_without_approval_request() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let (shift_id, _) = open_shift(&db, &branch, &teller, &manager).await;
    let counts = db.counts();

    // Branch limit = 500.00 THB
    db.set_cash_limit(&branch, Some(d("500.00"))).await;

    // Count 2,200 THB against 2,300 expected (difference = -100 THB, abs <= 500)
    let lines = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 2),
    ];
    let close = counts
        .submit(
            &teller,
            cmd(&branch, Some(&shift_id), CountKind::Close, lines),
            at(600),
        )
        .await
        .expect("submit short close count");

    let verified = counts
        .verify(&manager, &close.id, at(660))
        .await
        .expect("difference <= limit must verify cleanly");

    assert_eq!(
        verified.verified_by.as_deref(),
        Some(manager.user_id.as_str())
    );
    assert_eq!(db.shift_status(&shift_id).await, "CLOSED");

    let exceptions = db.cash_exceptions_for_count(&close.id).await;
    assert_eq!(
        exceptions.len(),
        1,
        "exactly one cash_exceptions row for difference"
    );
    let exc = &exceptions[0];
    assert_eq!(exc["status"], "OPEN");
    assert_eq!(exc["note"], format!("COUNT {}", close.id));
    assert_eq!(exc["cashVariance"]["THB"], -10000);
    assert!(exc["safeVariance"].is_null());
    assert_eq!(exc["at"], at(660).timestamp_millis());

    let requests = db.approval_requests_for_count(&close.id).await;
    assert!(
        requests.is_empty(),
        "difference within limit must not create approval request: {requests:?}"
    );
}

// ── §8 Row 3: difference > limit or ≠ 0 with no limit ───────────────────────

#[tokio::test]
async fn verify_count_difference_exceeding_branch_limit_refuses_awaiting_finance() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let (shift_id, _) = open_shift(&db, &branch, &teller, &manager).await;
    let counts = db.counts();

    // Branch limit = 50.00 THB
    db.set_cash_limit(&branch, Some(d("50.00"))).await;

    // Count 2,200 against 2,300 expected (diff = -100 THB, abs 100 > limit 50)
    let lines = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 2),
    ];
    let close = counts
        .submit(
            &teller,
            cmd(&branch, Some(&shift_id), CountKind::Close, lines),
            at(600),
        )
        .await
        .expect("submit short close count");

    let res = counts.verify(&manager, &close.id, at(660)).await;

    // The call must be refused with AwaitingFinance(id)
    match res {
        Err(AppError::Domain(msg)) => {
            let requests = db.approval_requests_for_count(&close.id).await;
            assert_eq!(requests.len(), 1, "exactly one PENDING request created");
            let (req_id, req_type, req_status, payload) = &requests[0];
            assert_eq!(req_type, "COUNT_DIFFERENCE");
            assert_eq!(req_status, "PENDING");
            assert_eq!(payload["count_id"], close.id);
            assert_eq!(
                d(payload["net_difference_thb"].as_str().unwrap()),
                d("-100"),
                "net_difference_thb matches -100 as decimal"
            );

            // Display contains the request ID and matches CashError::AwaitingFinance
            assert!(
                msg.contains(req_id),
                "refusal message '{msg}' must contain request id '{req_id}'"
            );
            assert_eq!(
                msg,
                CashError::AwaitingFinance(req_id.clone()).to_string(),
                "refusal reaches caller as AppError::Domain of CashError::AwaitingFinance"
            );
        }
        other => panic!("expected AppError::Domain with AwaitingFinance refusal, got {other:?}"),
    }

    // Shift must stay OPEN and count must stay unverified (§8)
    assert_eq!(db.shift_status(&shift_id).await, "OPEN");
    let verified_by = db.count_verified_by(&close.id).await;
    assert!(verified_by.is_none());

    // No cash_exceptions written (§8)
    let exceptions = db.cash_exceptions_for_count(&close.id).await;
    assert!(
        exceptions.is_empty(),
        "no cash exceptions on refusal: {exceptions:?}"
    );

    // verify writes the stored differences first, the gate reads them after (§3 item 2)
    let diff: Option<Decimal> = sqlx::query_scalar(
        "SELECT difference FROM cash_count_totals WHERE count_id = $1::uuid AND currency = 'THB'",
    )
    .bind(&close.id)
    .fetch_one(&db.pool)
    .await
    .expect("stored difference in cash_count_totals");
    assert_eq!(
        diff,
        Some(d("-100")),
        "stored difference must be persisted on refusal"
    );
}

#[tokio::test]
async fn verify_count_difference_with_no_branch_limit_refuses_awaiting_finance() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let (shift_id, _) = open_shift(&db, &branch, &teller, &manager).await;
    let counts = db.counts();

    // No cash_limits row configured for this branch (V-1: non-zero difference sends to FINANCE)
    let lines = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 2),
    ];
    let close = counts
        .submit(
            &teller,
            cmd(&branch, Some(&shift_id), CountKind::Close, lines),
            at(600),
        )
        .await
        .expect("submit close count");

    let res = counts.verify(&manager, &close.id, at(660)).await;

    match res {
        Err(AppError::Domain(msg)) => {
            let requests = db.approval_requests_for_count(&close.id).await;
            assert_eq!(
                requests.len(),
                1,
                "one PENDING request created when no limit exists"
            );
            let (req_id, _, req_status, _) = &requests[0];
            assert_eq!(req_status, "PENDING");
            assert!(msg.contains(req_id));
            assert_eq!(msg, CashError::AwaitingFinance(req_id.clone()).to_string());
        }
        other => panic!("expected AppError::Domain with AwaitingFinance, got {other:?}"),
    }

    assert_eq!(db.shift_status(&shift_id).await, "OPEN");
    let verified_by = db.count_verified_by(&close.id).await;
    assert!(verified_by.is_none());
    assert!(db.cash_exceptions_for_count(&close.id).await.is_empty());
}

// ── §8 Row 4: verifying again while request is pending ──────────────────────

#[tokio::test]
async fn verifying_again_while_request_is_pending_returns_same_refusal_and_no_second_request() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let (shift_id, _) = open_shift(&db, &branch, &teller, &manager).await;
    let counts = db.counts();

    let lines = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 2),
    ];
    let close = counts
        .submit(
            &teller,
            cmd(&branch, Some(&shift_id), CountKind::Close, lines),
            at(600),
        )
        .await
        .expect("submit close count");

    // First verify: triggers gate, commits PENDING request, refuses AwaitingFinance(id)
    let req_id1 = match counts.verify(&manager, &close.id, at(660)).await {
        Err(AppError::Domain(msg)) => {
            let requests = db.approval_requests_for_count(&close.id).await;
            assert_eq!(
                requests.len(),
                1,
                "first verify creates one PENDING request"
            );
            let (req_id, _, req_status, _) = &requests[0];
            assert_eq!(req_status, "PENDING");
            assert_eq!(msg, CashError::AwaitingFinance(req_id.clone()).to_string());
            req_id.clone()
        }
        other => panic!("expected AppError::Domain with AwaitingFinance, got {other:?}"),
    };

    // Second verify while request is pending: looks up request by key, answers same refusal without calling gate again
    let req_id2 = match counts.verify(&manager, &close.id, at(700)).await {
        Err(AppError::Domain(msg)) => {
            assert_eq!(
                msg,
                CashError::AwaitingFinance(req_id1.clone()).to_string(),
                "second verify must return same AwaitingFinance refusal and request id"
            );
            let requests = db.approval_requests_for_count(&close.id).await;
            assert_eq!(requests.len(), 1, "no second request created on re-verify");
            requests[0].0.clone()
        }
        other => panic!("expected AppError::Domain with AwaitingFinance, got {other:?}"),
    };

    assert_eq!(
        req_id1, req_id2,
        "request id must be identical across re-verifies"
    );
    assert_eq!(db.shift_status(&shift_id).await, "OPEN");
    let verified_by = db.count_verified_by(&close.id).await;
    assert!(verified_by.is_none());
}

// ── §8 Row 5: after FINANCE approves ────────────────────────────────────────

#[tokio::test]
async fn verify_count_after_finance_approves_succeeds_and_closes_shift() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let finance_user = db.person("Somchai", &[Role::Finance]).await;
    let (shift_id, _) = open_shift(&db, &branch, &teller, &manager).await;
    let counts = db.counts();

    let lines = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 2),
    ];
    let close = counts
        .submit(
            &teller,
            cmd(&branch, Some(&shift_id), CountKind::Close, lines),
            at(600),
        )
        .await
        .expect("submit close count");

    // First verify: triggers gate, creates PENDING request, refuses AwaitingFinance(id)
    let req_id = match counts.verify(&manager, &close.id, at(660)).await {
        Err(AppError::Domain(msg)) => {
            let requests = db.approval_requests_for_count(&close.id).await;
            assert_eq!(
                requests.len(),
                1,
                "first verify creates one PENDING request"
            );
            let (id, _, req_status, _) = &requests[0];
            assert_eq!(req_status, "PENDING");
            assert_eq!(msg, CashError::AwaitingFinance(id.clone()).to_string());
            id.clone()
        }
        other => panic!("expected AppError::Domain with AwaitingFinance, got {other:?}"),
    };

    // FINANCE decides Approved through ApprovalService::decide
    let approvals = approval_service(&db.pool);
    let finance_actor = finance_of(&finance_user.user_id, &branch);
    let mut tx = db.pool.begin().await.expect("begin");
    approvals
        .decide(
            &mut tx,
            &RequestId::new(&req_id),
            &finance_actor,
            Outcome::Approved,
            "approved difference",
            &unique("decide"),
        )
        .await
        .expect("FINANCE approves COUNT_DIFFERENCE request");
    tx.commit().await.expect("commit");

    // Verify after approval succeeds and closes shift
    let verified = counts
        .verify(&manager, &close.id, at(700))
        .await
        .expect("verify succeeds after FINANCE approves");

    assert_eq!(
        verified.verified_by.as_deref(),
        Some(manager.user_id.as_str())
    );
    assert_eq!(db.shift_status(&shift_id).await, "CLOSED");

    // Cash exceptions row is written on verified count
    let exceptions = db.cash_exceptions_for_count(&close.id).await;
    assert_eq!(exceptions.len(), 1);
    let exc = &exceptions[0];
    assert_eq!(exc["status"], "OPEN");
    assert_eq!(exc["note"], format!("COUNT {}", close.id));
    assert_eq!(exc["cashVariance"]["THB"], -10000);
    assert!(exc["safeVariance"].is_null());
    assert_eq!(exc["at"], at(700).timestamp_millis());

    // Still one request, now APPROVED
    let requests = db.approval_requests_for_count(&close.id).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, req_id);
    assert_eq!(requests[0].2, "APPROVED");
}

// ── §8 Row 6: after FINANCE rejects ────────────────────────────────────────

#[tokio::test]
async fn verify_count_after_finance_rejects_refuses_rejected_by_finance() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let finance_user = db.person("Somchai", &[Role::Finance]).await;
    let (shift_id, _) = open_shift(&db, &branch, &teller, &manager).await;
    let counts = db.counts();

    let lines = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 2),
    ];
    let close = counts
        .submit(
            &teller,
            cmd(&branch, Some(&shift_id), CountKind::Close, lines),
            at(600),
        )
        .await
        .expect("submit close count");

    // First verify: triggers gate, creates PENDING request, refuses AwaitingFinance(id)
    let req_id = match counts.verify(&manager, &close.id, at(660)).await {
        Err(AppError::Domain(msg)) => {
            let requests = db.approval_requests_for_count(&close.id).await;
            assert_eq!(
                requests.len(),
                1,
                "first verify creates one PENDING request"
            );
            let (id, _, req_status, _) = &requests[0];
            assert_eq!(req_status, "PENDING");
            assert_eq!(msg, CashError::AwaitingFinance(id.clone()).to_string());
            id.clone()
        }
        other => panic!("expected AppError::Domain with AwaitingFinance, got {other:?}"),
    };

    // FINANCE decides Rejected through ApprovalService::decide
    let approvals = approval_service(&db.pool);
    let finance_actor = finance_of(&finance_user.user_id, &branch);
    let mut tx = db.pool.begin().await.expect("begin");
    approvals
        .decide(
            &mut tx,
            &RequestId::new(&req_id),
            &finance_actor,
            Outcome::Rejected,
            "rejected difference",
            &unique("decide"),
        )
        .await
        .expect("FINANCE rejects COUNT_DIFFERENCE request");
    tx.commit().await.expect("commit");

    // Verify after rejection is refused with RejectedByFinance(id)
    let res = counts.verify(&manager, &close.id, at(700)).await;

    match res {
        Err(AppError::Domain(msg)) => {
            assert!(
                msg.contains(&req_id),
                "rejection message '{msg}' must contain request id '{req_id}'"
            );
            assert_eq!(
                msg,
                CashError::RejectedByFinance(req_id.clone()).to_string(),
                "refusal reaches caller as AppError::Domain of CashError::RejectedByFinance"
            );
        }
        other => panic!("expected AppError::Domain with RejectedByFinance, got {other:?}"),
    }

    // Shift stays OPEN, count stays unverified, no second request (§8)
    assert_eq!(db.shift_status(&shift_id).await, "OPEN");
    let verified_by = db.count_verified_by(&close.id).await;
    assert!(verified_by.is_none());

    let requests = db.approval_requests_for_count(&close.id).await;
    assert_eq!(requests.len(), 1, "no second request created");
    assert_eq!(requests[0].0, req_id);
    assert_eq!(requests[0].2, "REJECTED");

    let exceptions = db.cash_exceptions_for_count(&close.id).await;
    assert!(
        exceptions.is_empty(),
        "no cash exceptions written after rejection"
    );
}

// ── §8 Row 7: OPEN count (no difference to judge) ───────────────────────────

#[tokio::test]
async fn verify_open_count_verifies_without_approval_request() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let counts = db.counts();

    // OPEN count has no difference to judge (§8, §3)
    let open = counts
        .submit(
            &teller,
            cmd(&branch, None, CountKind::Open, thb_2300(&db).await),
            at(0),
        )
        .await
        .expect("submit OPEN count");

    let verified = counts
        .verify(&manager, &open.id, at(60))
        .await
        .expect("OPEN count verifies cleanly");

    assert_eq!(
        verified.verified_by.as_deref(),
        Some(manager.user_id.as_str())
    );
    assert!(verified.shift_id.is_some());

    let requests = db.approval_requests_for_count(&open.id).await;
    assert!(
        requests.is_empty(),
        "OPEN count must not create approval request"
    );
}

// ── §8 Row 8: verify refused before the gate ────────────────────────────────

#[tokio::test]
async fn verify_refused_before_gate_writes_no_approval_request() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let (shift_id, _) = open_shift(&db, &branch, &teller, &manager).await;
    let counts = db.counts();

    let lines = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 2),
    ];
    let close = counts
        .submit(
            &teller,
            cmd(&branch, Some(&shift_id), CountKind::Close, lines),
            at(600),
        )
        .await
        .expect("submit close count");

    // Case A: Verifier without permission (teller lacks ShiftCloseCertify, refused before gate §8)
    let res_no_perm = counts.verify(&teller, &close.id, at(660)).await;
    match res_no_perm {
        Err(AppError::Domain(msg)) => {
            assert_eq!(msg, CashError::Refused(Refusal::RoleNotAllowed).to_string());
        }
        other => panic!("expected RoleNotAllowed refusal, got {other:?}"),
    }

    let requests_a = db.approval_requests_for_count(&close.id).await;
    assert!(
        requests_a.is_empty(),
        "refusal before gate (RoleNotAllowed) must write no approval request: {requests_a:?}"
    );

    // Case B: Not newest count (submit recount, then try verifying older count)
    let recount = counts
        .submit(
            &teller,
            cmd(
                &branch,
                Some(&shift_id),
                CountKind::Close,
                thb_2300(&db).await,
            ),
            at(680),
        )
        .await
        .expect("submit recount");

    let res_old = counts.verify(&manager, &close.id, at(700)).await;
    match res_old {
        Err(AppError::Domain(msg)) => {
            assert_eq!(msg, CashError::NotNewestCount.to_string());
        }
        other => panic!("expected NotNewestCount refusal, got {other:?}"),
    }

    let requests_b = db.approval_requests_for_count(&close.id).await;
    assert!(
        requests_b.is_empty(),
        "refusal before gate (NotNewestCount) must write no approval request: {requests_b:?}"
    );
    assert!(db.count_verified_by(&recount.id).await.is_none());
}

// ── §9 Edge case: rollback on failure leaves no orphan request ──────────────

#[tokio::test]
async fn verify_failure_due_to_balances_changed_leaves_no_orphan_request() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let (shift_id, _) = open_shift(&db, &branch, &teller, &manager).await;
    let counts = db.counts();

    // Close count with non-zero difference
    let lines = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 2),
    ];
    let close = counts
        .submit(
            &teller,
            cmd(&branch, Some(&shift_id), CountKind::Close, lines),
            at(600),
        )
        .await
        .expect("submit close count");

    // An exchange occurs after count was submitted, moving shift balance
    let mut tx = db.pool.begin().await.expect("begin");
    lg4_exchange_for_teller(&mut tx, &teller.user_id, "THB", "100").await; // was ShiftService::apply_delta_for_teller_in, "apply delta"
    tx.commit().await.expect("commit");

    // Verify should fail with BalancesChanged before gate (§3 step 1)
    let res = counts.verify(&manager, &close.id, at(700)).await;
    match res {
        Err(AppError::Domain(msg)) => {
            assert_eq!(msg, CashError::BalancesChanged.to_string());
        }
        other => panic!("expected BalancesChanged, got {other:?}"),
    }

    // Gate runs inside verify transaction: failed verify leaves no orphan request (§9)
    let requests = db.approval_requests_for_count(&close.id).await;
    assert!(
        requests.is_empty(),
        "failed verify must leave no orphan request in approval_requests: {requests:?}"
    );
    assert_eq!(db.shift_status(&shift_id).await, "OPEN");
}

// ── §7: CashError codes and display ─────────────────────────────────────────

#[test]
fn cash_error_codes_answer_camel_case_identifiers() {
    assert_eq!(
        CashError::AwaitingFinance("req-123".into()).code(),
        "awaitingFinance"
    );
    assert_eq!(
        CashError::RejectedByFinance("req-456".into()).code(),
        "rejectedByFinance"
    );
}

#[test]
fn cash_error_display_contains_request_id() {
    let awaiting = CashError::AwaitingFinance("req-abc-789".into()).to_string();
    assert!(
        awaiting.contains("req-abc-789"),
        "AwaitingFinance Display must contain request id: {awaiting}"
    );

    let rejected = CashError::RejectedByFinance("req-xyz-000".into()).to_string();
    assert!(
        rejected.contains("req-xyz-000"),
        "RejectedByFinance Display must contain request id: {rejected}"
    );
}

/// OR-7.1 #1020 (design §3.3, §4.3): the decider `ApprovalService::decide` is handed, `access::Actor`: the
/// FINANCE user on the count's branch, the B1 mapping of the `PolicyActor::staff(.., Finance, branch)` this
/// file passed before. Moved 1:1 by #1020's ticket author; every assertion is unchanged.
fn finance_of(user_id: &str, branch: &BranchId) -> Actor {
    Actor::new(
        user_id,
        ORG,
        vec![ScopedRole::new(
            Role::Finance,
            Some(branch.clone()),
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            None,
        )],
        None,
    )
}

/// LG-4 (#929, lane controller's 1:1 ruling): an OPEN count takes its counted cash out of the branch's SAFE
/// (design §4.4 row 1), so a branch's SAFE is funded through the ledger first. Before the cutover this file needed
/// no funding because nothing was taken from the safe; THB is not inventory, so the funding moves no position.
static LG4_FUND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
                        LG4_FUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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

/// LG-4 (#929, 1:1): after the cutover an exchange moves the drawer through the ledger (TX-5's `post_in`), no
/// longer through `ShiftService::apply_delta_for_teller_in`; this posts the same `currency amount` into the
/// teller's OPEN shift's DRAWER as an `EXCHANGE`, on the caller's transaction.
async fn lg4_exchange_for_teller(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    teller_user_id: &str,
    currency: &str,
    amount: &str,
) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
    let shift: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'")
            .bind(teller_user_id)
            .fetch_one(&mut **tx)
            .await
            .expect("the teller's OPEN shift");
    xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
        .post_in(
            tx,
            &[PostingRequest {
                from: CashPlace::Customer,
                to: CashPlace::Drawer(shift.clone()),
                currency: xc_kernel::money::CurrencyCode::parse(currency).expect("a currency"),
                quantity: xc_kernel::money::ForeignAmount::from_decimal(
                    amount.parse().expect("an amount"),
                ),
                source: SourceRef {
                    source_type: SourceType::Exchange,
                    source_id: format!("lg4-exchange:{shift}:{currency}:{amount}"),
                    source_key: "complete".to_string(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("the exchange posts to the teller's drawer");
}
