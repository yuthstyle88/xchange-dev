//! LA-3 up-front tests (design `docs/design/blueprint/ledger-inventory.md` §13.3, §13.5 "Decide", §13.9 rows
//! LA-T7 to LA-T10): who decides a `LEDGER_ADJUSTMENT` request, and that its version reads no balance.
//!
//! 🔴 Written by the sub-controller claude/f32b85db before the implementation. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! The requests are opened with `ApprovalService::request` directly (not LA-4's service), so this file needs
//! only LA-3's subject and policy, and LA-1's `ApproveLedgerAdjustment` in FINANCE's seed. Every decision goes
//! through `ApprovalService::decide`, which refuses the proposer itself (AP-1) and then asks the policy.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use chrono::{TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_approval::application::approval_service::{ApprovalError, ApprovalService};
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::ledger_approval::ledger_adjustment::{LEDGER_ADJUSTMENT, register};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const B01: &str = "B01";
const B02: &str = "B02";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn since() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

fn person(id: &str, rows: &[(Role, Option<&str>)]) -> Actor {
    Actor::new(
        id,
        ORG,
        rows.iter()
            .map(|(r, b)| ScopedRole::new(*r, b.map(BranchId::new), since(), None))
            .collect(),
        None,
    )
}

/// The §13.9 fixture's people.
fn fin_a() -> Actor {
    person("fin-a", &[(Role::Finance, Some(B01))])
}
fn fin_b() -> Actor {
    person("fin-b", &[(Role::Finance, Some(B01))])
}
fn fin_org() -> Actor {
    person("fin-org", &[(Role::Finance, None)])
}
fn fin_c() -> Actor {
    person("fin-c", &[(Role::Finance, Some(B02))])
}
fn mgr() -> Actor {
    person("mgr", &[(Role::BranchManager, Some(B01))])
}
fn exec() -> Actor {
    person("exec", &[(Role::Executive, None)])
}

async fn db() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!("TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip.")
    });
    let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
    assert!(
        options.get_database().unwrap_or("").ends_with("_test"),
        "TEST_DATABASE_URL must name a database ending in `_test`"
    );
    let schema = unique("la3");
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
    let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    Migrator::new(dir.as_path())
        .await
        .expect("read api/migrations")
        .run(&pool)
        .await
        .expect("every migration applies");
    // MAIN is where AP-1 files an organisation-wide requester's request (`requester_branch`).
    for b in [B01, B02, "MAIN"] {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1) ON CONFLICT (id) DO NOTHING")
            .bind(b)
            .execute(&pool)
            .await
            .expect("a branch");
    }
    pool
}

fn service(pool: &PgPool) -> ApprovalService {
    let mut registry = ApprovalRegistry::new();
    register(&mut registry, pool.clone());
    ApprovalService::new(
        registry,
        Arc::new(PgApprovalRepository::new(pool.clone())),
        Arc::new(PgAuditLog::new(pool.clone())),
    )
}

/// §13.4's QUANTITY payload for `branch`: SAFE USD −100.
fn payload(branch: &str) -> Value {
    json!({
        "branch": branch,
        "kind": "QUANTITY",
        "place": "SAFE",
        "lines": [ { "currency": "USD", "quantity": "-100" } ],
        "reason": "Bank deposit keyed as 1,100 USD; the slip says 1,000",
        "evidence": { "files": [], "reference": "slip 004512" }
    })
}

/// Opens `who`'s `LEDGER_ADJUSTMENT` request for a new adjustment of `branch`.
async fn open(svc: &ApprovalService, pool: &PgPool, who: &Actor, branch: &str) -> ApprovalRequest {
    let id = format!("ADJ-{}", unique("t"));
    let mut tx = pool.begin().await.unwrap();
    let req = svc
        .request(
            &mut tx,
            LEDGER_ADJUSTMENT,
            &id,
            &payload(branch),
            who,
            &format!("ledger-adjustment:{id}"),
        )
        .await
        .expect("the request opens");
    tx.commit().await.unwrap();
    req
}

async fn decide(
    svc: &ApprovalService,
    pool: &PgPool,
    req: &ApprovalRequest,
    who: &Actor,
    outcome: Outcome,
) -> Result<(), ApprovalError> {
    let mut tx = pool.begin().await.unwrap();
    let out = svc
        .decide(
            &mut tx,
            &req.id,
            who,
            outcome,
            "checked the slip",
            &unique("k"),
        )
        .await;
    if out.is_ok() {
        tx.commit().await.unwrap();
    }
    out
}

async fn status(pool: &PgPool, req: &ApprovalRequest) -> String {
    sqlx::query_scalar("SELECT status FROM approval_requests WHERE id = $1::uuid")
        .bind(req.id.to_string())
        .fetch_one(pool)
        .await
        .expect("the request row")
}

fn refused_out_of_scope(out: Result<(), ApprovalError>, what: &str) {
    let e = out.expect_err(&format!("{what} is refused"));
    assert_eq!(e.code(), "outOfScope", "{what}: {e:?}");
}

// ── LA-T7 ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn la_t7_the_creator_never_decides() {
    let pool = db().await;
    let svc = service(&pool);
    let req = open(&svc, &pool, &fin_a(), B01).await;
    refused_out_of_scope(
        decide(&svc, &pool, &req, &fin_a(), Outcome::Approved).await,
        "the proposer approving",
    );
    refused_out_of_scope(
        decide(&svc, &pool, &req, &fin_a(), Outcome::Rejected).await,
        "the proposer rejecting",
    );
    assert_eq!(status(&pool, &req).await, "PENDING");
}

// ── LA-T8 ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn la_t8_a_second_finance_of_the_branch_or_the_organisation_approves() {
    let pool = db().await;
    let svc = service(&pool);
    for (who, label) in [(fin_b(), "fin-b"), (fin_org(), "fin-org")] {
        let req = open(&svc, &pool, &fin_a(), B01).await;
        decide(&svc, &pool, &req, &who, Outcome::Approved)
            .await
            .unwrap_or_else(|e| panic!("{label} approves: {e:?}"));
        assert_eq!(status(&pool, &req).await, "APPROVED", "{label}");
    }
}

#[tokio::test]
async fn la_t8_another_branchs_finance_a_manager_and_the_executive_do_not_decide() {
    let pool = db().await;
    let svc = service(&pool);
    let req = open(&svc, &pool, &fin_a(), B01).await;
    for (who, label) in [
        (fin_c(), "FINANCE of B02"),
        (mgr(), "the branch manager of B01"),
        (exec(), "the executive (owner OA-3)"),
    ] {
        for outcome in [Outcome::Approved, Outcome::Rejected] {
            refused_out_of_scope(decide(&svc, &pool, &req, &who, outcome).await, label);
        }
    }
    assert_eq!(status(&pool, &req).await, "PENDING");
}

// ── LA-T9 ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn la_t9_the_policy_reads_the_payloads_branch_not_the_requesters() {
    let pool = db().await;
    let svc = service(&pool);
    let req = open(&svc, &pool, &fin_org(), B01).await;
    assert_eq!(
        req.branch_id.as_deref(),
        Some("MAIN"),
        "AP-1 files an organisation-wide requester's request at MAIN (approval_service.rs requester_branch)"
    );
    decide(&svc, &pool, &req, &fin_b(), Outcome::Approved)
        .await
        .expect("FINANCE of B01 approves B01's adjustment, whatever branch_id says");
    assert_eq!(status(&pool, &req).await, "APPROVED");

    let other = open(&svc, &pool, &fin_org(), B01).await;
    refused_out_of_scope(
        decide(&svc, &pool, &other, &fin_c(), Outcome::Approved).await,
        "FINANCE of B02 on B01's adjustment filed at MAIN",
    );
}

// ── LA-T10 ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn la_t10_an_exchange_posting_on_the_branch_does_not_make_the_request_stale() {
    let pool = db().await;
    let svc = service(&pool);
    let req = open(&svc, &pool, &fin_a(), B01).await;

    let usd = CurrencyCode::parse("USD").unwrap();
    let buy = SourceRef {
        source_type: SourceType::Exchange,
        source_id: unique("ex"),
        source_key: "la-t10".to_owned(),
    };
    let mut tx = pool.begin().await.unwrap();
    LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[
                PostingRequest {
                    from: CashPlace::Customer,
                    to: CashPlace::Safe(BranchId::new(B01)),
                    currency: usd,
                    quantity: ForeignAmount::from_decimal(Decimal::from(500)),
                    source: buy.clone(),
                    line: None,
                    reverses: None,
                },
                PostingRequest {
                    from: CashPlace::Bank,
                    to: CashPlace::Customer,
                    currency: CurrencyCode::THB,
                    quantity: ForeignAmount::from_decimal(Decimal::from(16_250)),
                    source: buy,
                    line: None,
                    reverses: None,
                },
            ],
            None,
        )
        .await
        .expect("an exchange posts on B01");
    tx.commit().await.unwrap();

    decide(&svc, &pool, &req, &fin_b(), Outcome::Approved)
        .await
        .expect("approved, not staleVersion: the version reads no balance");
    assert_eq!(status(&pool, &req).await, "APPROVED");
}
