//! LA-4 up-front test (design `docs/design/blueprint/ledger-inventory.md` §13.5 "Post" step 2, §13.9 row LA-T29):
//! `LedgerAdjustmentService::post_in` posts a COST adjustment through `CashPostings::revalue_in`, never `post_in`.
//!
//! 🔴 Written by the sub-controller claude/f32b85db before the implementation. Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! The approved request is a row written directly; the port is a test double that records its calls. The rest of
//! the service is pinned through the commands' tests (`api/tests/ledger_adjustment_{create,post}_test.rs`).
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use serde_json::json;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::adjustment::adjustment_service::LedgerAdjustmentService;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{
    ApprovalRequestId, CashPlace, CashPostings, PostingId, PostingRequest, PostingResult,
    RevalueRequest,
};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, MoneyDelta};
use xc_shift::domain::shift::value_objects::balances::Balances;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const B01: &str = "B01";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

/// Records every call; `revalue_in` answers one fixed posting id.
#[derive(Default)]
struct Recorder {
    post_in: Mutex<Vec<Vec<PostingRequest>>>,
    revalue_in: Mutex<Vec<Vec<RevalueRequest>>>,
    id: Mutex<Option<Uuid>>,
}

impl CashPostings for Recorder {
    fn post_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        req: &'a [PostingRequest],
        _allow_negative: Option<ApprovalRequestId>,
    ) -> BoxFuture<'a, AppResult<PostingResult>> {
        self.post_in.lock().unwrap().push(req.to_vec());
        Box::pin(async move {
            Err(AppError::Internal(
                "post_in must not be called for a COST adjustment".into(),
            ))
        })
    }

    fn expected_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _place: &'a CashPlace,
    ) -> BoxFuture<'a, AppResult<Balances>> {
        Box::pin(async move { Err(AppError::Internal("not used".into())) })
    }

    fn revalue_in<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        reqs: &'a [RevalueRequest],
    ) -> BoxFuture<'a, AppResult<PostingId>> {
        self.revalue_in.lock().unwrap().push(reqs.to_vec());
        let id = *self.id.lock().unwrap().get_or_insert_with(Uuid::new_v4);
        Box::pin(async move { Ok(PostingId(id)) })
    }
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
    let schema = unique("la4");
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
    sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
        .bind(B01)
        .execute(&pool)
        .await
        .expect("a branch");
    pool
}

fn fin_a() -> Actor {
    Actor::new(
        "fin-a",
        ORG,
        vec![ScopedRole::new(
            Role::Finance,
            Some(BranchId::new(B01)),
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            None,
        )],
        None,
    )
}

#[tokio::test]
async fn la_t29_a_cost_adjustment_posts_through_revalue_in_in_line_order() {
    let pool = db().await;
    let id = format!("ADJ-{}", Uuid::new_v4());
    let payload = json!({
        "branch": B01,
        "kind": "COST",
        "lines": [
            { "currency": "USD", "costDeltaThb": "450.00" },
            { "currency": "EUR", "costDeltaThb": "-12.50" }
        ],
        "reason": "Opening cost was the BUY rate; the invoice says otherwise",
        "evidence": { "files": [], "reference": "invoice 77" }
    });
    sqlx::query(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
           proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
         VALUES (gen_random_uuid(), 'LEDGER_ADJUSTMENT', $1, $2, 'h', '1', $3, 'fin-b', now(), 'APPROVED', $4, \
           $5::uuid)",
    )
    .bind(B01)
    .bind(&id)
    .bind(&payload)
    .bind(format!("ledger-adjustment:{id}"))
    .bind(ORG)
    .execute(&pool)
    .await
    .expect("an approved COST adjustment");

    let recorder = Arc::new(Recorder::default());
    let approvals = Arc::new(ApprovalService::new(
        ApprovalRegistry::new(),
        Arc::new(PgApprovalRepository::new(pool.clone())),
        Arc::new(PgAuditLog::new(pool.clone())),
    ));
    let service = LedgerAdjustmentService::new(approvals, recorder.clone());

    let mut tx = pool.begin().await.unwrap();
    let posted = service
        .post_in(&mut tx, &BranchId::new(B01), &id, &fin_a())
        .await
        .expect("no AppError")
        .expect("posted");
    tx.rollback().await.unwrap();

    assert!(
        recorder.post_in.lock().unwrap().is_empty(),
        "post_in is not called for COST"
    );
    let calls = recorder.revalue_in.lock().unwrap().clone();
    let source = SourceRef {
        source_type: SourceType::Adjustment,
        source_id: id.clone(),
        source_key: "post".to_owned(),
    };
    assert_eq!(
        calls,
        vec![vec![
            RevalueRequest {
                branch: BranchId::new(B01),
                currency: CurrencyCode::parse("USD").unwrap(),
                cost_delta_thb: MoneyDelta::from_minor(45_000),
                source: source.clone(),
            },
            RevalueRequest {
                branch: BranchId::new(B01),
                currency: CurrencyCode::parse("EUR").unwrap(),
                cost_delta_thb: MoneyDelta::from_minor(-1_250),
                source,
            },
        ]],
        "one revalue_in call with one request per line, in line order, under (ADJUSTMENT, id, post)"
    );
    assert_eq!(posted.id, id);
    assert_eq!(
        posted.posting_id,
        PostingId(recorder.id.lock().unwrap().expect("the double's id"))
    );
    let currencies: Vec<&str> = posted.lines.iter().map(|l| l.currency.as_str()).collect();
    assert_eq!(currencies, ["USD", "EUR"], "the answer keeps line order");
}
