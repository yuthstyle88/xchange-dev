//! CA-5 (#621 Part 1): `TransferFlow` over the real tables: create, submit to the approval core, decide within and
//! above the limit, dispatch, receive with an exception, reconcile, and the per-kind CHECKs of the new migration
//! (design `docs/design/blueprint/cash-shift-transfer.md` §3.1, §4.1–§4.6, §9.1 CA-5; lane controller rulings on #621,
//! issuecomment-6093029327 (gaps 1–4) and -6093051936 (gap 5)).
//!
//! Written before the implementation by claude/adc848e6, on the lane controller's order. Do not edit, delete or weaken
//! these tests; add your own in another file.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.
//!
//! [required] by these tests, beside `transfers_test.rs`'s list:
//! - `xc_cash::application::transfer::TransferFlow`:
//!   - `TransferFlow::pg(pool: PgPool) -> Self` wires the transfer repository, `LedgerCashPostings` (LG-4), the kinds of
//!     `TransferKinds::standard()` and the approval core (`TransferApprovalSubject` + `TransferApprovalPolicy`);
//!   - `create(&self, who: &Actor, cmd: NewTransfer, now: DateTime<Utc>) -> AppResult<CashTransfer>` → DRAFT;
//!   - `submit(&self, who: &Actor, id: &str, now) -> AppResult<CashTransfer>`: the creator asks the approval core; the
//!     transfer stays DRAFT and `cash_transfers.approval_request_id` names the PENDING request;
//!   - `decide(&self, who: &Actor, id: &str, outcome: xc_approval::domain::request::Outcome, reason: &str, now) ->
//!     AppResult<CashTransfer>`: ruling (b). It checks `ApprovalsDecide` through `Authorizer` on the source branch and
//!     the limit (BRANCH_MANAGER within it, EXECUTIVE above it), then calls the core. Approved → APPROVED; rejected →
//!     still DRAFT, and `submit` may be called again;
//!   - `dispatch(&self, who, id, now)`, `reconcile(&self, who, id, note: &str, now)`;
//!   - `receive(&self, who, id, counted: Vec<NewTransferLine>, now)`: `qty_received` per (currency, denomination); a
//!     sent line the receiver does not list was received as 0.
//!   - Refusals reach the caller as `AppError::Domain(sentence)`. `TransferError::StockInsufficient` and
//!     `TransferError::IllegalTransition { from, step }` are the sentences this file compares.
//! - The limit: the newest `cash_transfer_policy_versions` row of the organisation (by `created_at`); `limits` is
//!   `{"<CUR>": "<amount>"}`. A transfer is within the limit when, for every currency it moves, Σ `qty_sent` ×
//!   face value ≤ that currency's limit. A currency with no limit, or no policy row at all, is above the limit.
//! - Postings: since LG-4 (#929) the ledger's `Safe(branch)` (before it, ruling 2's interim port read
//!   `ops_state.safe`). BRANCH: dispatch moves `Safe(from)` out, receipt moves `Safe(to)` in by the counted notes.
//! - The receipt exception: `qty_received ≠ qty_sent` writes one `cash_exceptions` row on the **destination** branch
//!   per currency that differs, whose `data` has `transferId`, `currency` and `difference` (received − sent, a
//!   decimal string).
//! - BRANCH_MANAGER holds `ApprovalsDecide` (gap 5, ruling (a): one line in `role_permissions.rs`).
//! - The migration's per-kind CHECKs (ruling 3; columns `to_branch_id / from_shift_id / to_shift_id`): BRANCH set / NULL /
//!   NULL; DRAWER_TO_SAFE NULL / set / NULL; SAFE_TO_DRAWER NULL / NULL / set; EXTERNAL_IN and EXTERNAL_OUT NULL / NULL /
//!   NULL. A violating row is refused by PostgreSQL (`23514`).

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_approval::domain::request::Outcome;
use xc_auth::domain::access::{Actor, Authorizer, Permission, Role, ScopedRole, Target};
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::transfer::cash_transfer::{
    CashTransfer, NewTransfer, NewTransferLine, TransferError, TransferKind, TransferStatus,
    TransferStep,
};
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_testdb::people::{RoleRow, seed_person};

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

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

// ── the database ────────────────────────────────────────────────────────────

struct Db {
    pool: PgPool,
}

impl Db {
    async fn fresh() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set. These tests need a real PostgreSQL and do not skip."
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("ca5");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .expect("connect");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("schema");
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
            .expect("every migration applies in version order on a fresh database");
        Db { pool }
    }

    async fn branch(&self, label: &str, safe_usd_minor: i64) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert ops_state");
        // LG-4 (#929, 1:1): the same USD amount is the safe's figure in the ledger, which now holds it.
        if safe_usd_minor > 0 {
            lg4_fund_safe(
                &self.pool,
                &id,
                "USD",
                &rust_decimal::Decimal::new(safe_usd_minor, 2).to_string(),
            )
            .await;
        }
        BranchId::new(id)
    }

    /// A user holding `roles`, each on a branch (`None` = organisation-wide), and their `Actor`.
    async fn person(&self, name: &str, roles: &[(Role, Option<&BranchId>)]) -> Actor {
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let role_rows: Vec<RoleRow> = roles
            .iter()
            .map(|(role, branch)| RoleRow {
                role_code: role.as_str(),
                branch_id: branch.map(|b| b.as_str()),
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
                .map(|(r, b)| ScopedRole::new(*r, b.cloned(), from, None))
                .collect(),
            None,
        )
    }

    async fn usd(&self, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = 'USD' AND face_value = $1::numeric",
        )
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the USD {face} face value (A7): {e}"))
    }

    async fn limit_usd(&self, by: &Actor, amount: &str) {
        sqlx::query(
            "INSERT INTO cash_transfer_policy_versions (version, organization_id, limits, set_by) \
             VALUES ($1, $2::uuid, jsonb_build_object('USD', $3::text), $4::uuid)",
        )
        .bind(unique("v"))
        .bind(ORG)
        .bind(amount)
        .bind(&by.user_id)
        .execute(&self.pool)
        .await
        .expect("a transfer policy version");
    }

    /// The branch's SAFE USD, in minor units (0 when absent). LG-4 (#929, 1:1): read through the port,
    /// `CashPostings::expected_in(Safe(branch))`, instead of `ops_state.safe`.
    async fn safe_usd(&self, branch: &BranchId) -> i64 {
        use xc_cash::domain::posting::{CashPlace, CashPostings};
        let mut tx = self.pool.begin().await.expect("begin");
        let b = xc_cash::infrastructure::ledger_postings::LedgerCashPostings::new()
            .expected_in(&mut tx, &CashPlace::Safe(branch.clone()))
            .await
            .expect("read the safe");
        tx.rollback().await.expect("rollback");
        b.get(usd()).try_minor().unwrap()
    }

    async fn status(&self, id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM cash_transfers WHERE id = $1::uuid")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("read the transfer")
    }

    async fn scalar(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    fn flow(&self) -> TransferFlow {
        TransferFlow::pg(self.pool.clone())
    }
}

/// The cast of a BRANCH transfer from B1 to B2.
struct Cast {
    b1: BranchId,
    b2: BranchId,
    creator: Actor,
    manager_b1: Actor,
    exec: Actor,
    receiver_b2: Actor,
    finance: Actor,
}

async fn cast(db: &Db) -> Cast {
    let b1 = db.branch("b1", 50_000).await; // USD 500.00 in B1's safe
    let b2 = db.branch("b2", 0).await;
    let creator = db.person("Fon", &[(Role::Finance, Some(&b1))]).await;
    let manager_b1 = db.person("Krit", &[(Role::BranchManager, Some(&b1))]).await;
    let exec = db.person("Exec", &[(Role::Executive, None)]).await;
    let receiver_b2 = db.person("Nida", &[(Role::BranchManager, Some(&b2))]).await;
    let finance = db.person("Pim", &[(Role::Finance, None)]).await;
    db.limit_usd(&exec, "300.00").await;
    Cast {
        b1,
        b2,
        creator,
        manager_b1,
        exec,
        receiver_b2,
        finance,
    }
}

async fn usd_50s(db: &Db, c: &Cast, notes: i32) -> NewTransfer {
    NewTransfer {
        kind: TransferKind::Branch,
        from_branch: c.b1.clone(),
        to_branch: Some(c.b2.clone()),
        from_shift: None,
        to_shift: None,
        lines: vec![NewTransferLine {
            currency: usd(),
            denomination_id: db.usd("50").await,
            qty: notes,
        }],
        note: None,
        idempotency_key: unique("t"),
    }
}

/// A BRANCH transfer of `notes` × USD 50, created and submitted.
async fn submitted(db: &Db, c: &Cast, notes: i32) -> CashTransfer {
    let flow = db.flow();
    let t = flow
        .create(&c.creator, usd_50s(db, c, notes).await, at(0))
        .await
        .expect("FINANCE of B1 creates");
    assert_eq!(t.status, TransferStatus::Draft);
    let t = flow
        .submit(&c.creator, &t.id, at(10))
        .await
        .expect("the creator submits");
    assert_eq!(
        t.status,
        TransferStatus::Draft,
        "submitting asks the core; the transfer stays DRAFT (C4)"
    );
    t
}

fn refused_with<T: std::fmt::Debug>(result: Result<T, AppError>, expected: TransferError) {
    match result {
        Err(AppError::Domain(sentence)) => assert_eq!(sentence, expected.to_string()),
        other => panic!(
            "expected AppError::Domain({:?}), got {other:?}",
            expected.to_string()
        ),
    }
}

// ── the permission (gap 5) ──────────────────────────────────────────────────

#[test]
fn a_branch_manager_holds_approvals_decide_on_its_own_branch() {
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let bm = Actor::new(
        "bm",
        ORG,
        vec![ScopedRole::new(
            Role::BranchManager,
            Some(BranchId::new("B1")),
            from,
            None,
        )],
        None,
    );
    let on = |b: &str| Target::new(ORG, Some(BranchId::new(b)), None);
    assert!(Authorizer::check(Some(&bm), Permission::ApprovalsDecide, &on("B1"), at(0)).is_ok());
    assert!(
        Authorizer::check(Some(&bm), Permission::ApprovalsDecide, &on("B2"), at(0)).is_err(),
        "only its own branch"
    );
}

// ── submit and decide (C6, A5) ──────────────────────────────────────────────

#[tokio::test]
async fn submitting_creates_one_pending_approval_request_and_names_it_on_the_transfer() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    let t = submitted(&db, &c, 4).await;
    let sql = format!(
        "SELECT count(*) FROM cash_transfers t JOIN approval_requests r ON r.id = t.approval_request_id \
         WHERE t.id = '{}'::uuid AND r.status = 'PENDING'",
        t.id
    );
    assert_eq!(db.scalar(&sql).await, 1);
}

#[tokio::test]
async fn within_the_limit_a_branch_manager_of_the_source_approves() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    let t = submitted(&db, &c, 4).await; // USD 200 ≤ 300
    let t = db
        .flow()
        .decide(&c.manager_b1, &t.id, Outcome::Approved, "ok", at(20))
        .await
        .expect("within the limit");
    assert_eq!(t.status, TransferStatus::Approved);
    assert_eq!(db.status(&t.id).await, "APPROVED");
}

#[tokio::test]
async fn above_the_limit_only_an_executive_approves() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    let t = submitted(&db, &c, 8).await; // USD 400 > 300
    assert!(
        db.flow()
            .decide(&c.manager_b1, &t.id, Outcome::Approved, "ok", at(20))
            .await
            .is_err()
    );
    assert_eq!(
        db.status(&t.id).await,
        "DRAFT",
        "a refused decision writes nothing"
    );
    let t = db
        .flow()
        .decide(&c.exec, &t.id, Outcome::Approved, "ok", at(30))
        .await
        .expect("EXECUTIVE above the limit");
    assert_eq!(t.status, TransferStatus::Approved);
}

#[tokio::test]
async fn with_no_policy_version_every_transfer_is_above_the_limit() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    sqlx::query("DELETE FROM cash_transfer_policy_versions")
        .execute(&db.pool)
        .await
        .unwrap();
    let t = submitted(&db, &c, 1).await; // USD 50
    assert!(
        db.flow()
            .decide(&c.manager_b1, &t.id, Outcome::Approved, "ok", at(20))
            .await
            .is_err()
    );
    assert!(
        db.flow()
            .decide(&c.exec, &t.id, Outcome::Approved, "ok", at(30))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn the_creator_never_approves_their_own_transfer_and_a_manager_of_another_branch_cannot() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    let manager_creator = db
        .person("Somchai", &[(Role::BranchManager, Some(&c.b1))])
        .await;
    let flow = db.flow();
    let t = flow
        .create(&manager_creator, usd_50s(&db, &c, 2).await, at(0))
        .await
        .expect("BRANCH_MANAGER of B1 creates");
    flow.submit(&manager_creator, &t.id, at(10))
        .await
        .expect("submit");
    assert!(
        flow.decide(&manager_creator, &t.id, Outcome::Approved, "mine", at(20))
            .await
            .is_err(),
        "never the creator"
    );
    assert!(
        flow.decide(&c.receiver_b2, &t.id, Outcome::Approved, "ok", at(25))
            .await
            .is_err(),
        "not the source branch"
    );
    assert_eq!(db.status(&t.id).await, "DRAFT");
}

#[tokio::test]
async fn a_rejected_transfer_stays_draft_and_can_be_submitted_again() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    let t = submitted(&db, &c, 4).await;
    let flow = db.flow();
    let t = flow
        .decide(
            &c.manager_b1,
            &t.id,
            Outcome::Rejected,
            "count again",
            at(20),
        )
        .await
        .expect("reject");
    assert_eq!(
        t.status,
        TransferStatus::Draft,
        "no REJECTED state on the transfer (C4)"
    );
    assert_eq!(db.status(&t.id).await, "DRAFT");
    let t = flow
        .submit(&c.creator, &t.id, at(30))
        .await
        .expect("submitted again");
    let t = flow
        .decide(&c.manager_b1, &t.id, Outcome::Approved, "ok", at(40))
        .await
        .expect("then approved");
    assert_eq!(t.status, TransferStatus::Approved);
}

// ── the five states over the flow ──────────────────────────────────────────

#[tokio::test]
async fn the_flow_refuses_every_jump_the_state_machine_refuses_and_writes_nothing() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    let flow = db.flow();
    let t = submitted(&db, &c, 4).await;
    refused_with(
        flow.dispatch(&c.creator, &t.id, at(20)).await,
        TransferError::IllegalTransition {
            from: TransferStatus::Draft,
            step: TransferStep::Dispatch,
        },
    );
    refused_with(
        flow.reconcile(&c.finance, &t.id, "n", at(21)).await,
        TransferError::IllegalTransition {
            from: TransferStatus::Draft,
            step: TransferStep::Reconcile,
        },
    );
    flow.decide(&c.manager_b1, &t.id, Outcome::Approved, "ok", at(30))
        .await
        .expect("approve");
    refused_with(
        flow.receive(&c.receiver_b2, &t.id, vec![], at(40)).await,
        TransferError::IllegalTransition {
            from: TransferStatus::Approved,
            step: TransferStep::Receive,
        },
    );
    assert_eq!(db.status(&t.id).await, "APPROVED");
    assert_eq!(db.safe_usd(&c.b1).await, 50_000, "nothing moved");
}

#[tokio::test]
async fn a_dispatch_beyond_the_source_safe_is_refused_and_nothing_moves() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    let flow = db.flow();
    let t = submitted(&db, &c, 12).await; // USD 600 > 500 in B1's safe
    flow.decide(&c.exec, &t.id, Outcome::Approved, "ok", at(20))
        .await
        .expect("approve above the limit");
    refused_with(
        flow.dispatch(&c.creator, &t.id, at(30)).await,
        TransferError::StockInsufficient,
    );
    assert_eq!(db.status(&t.id).await, "APPROVED");
    assert_eq!(db.safe_usd(&c.b1).await, 50_000);
}

#[tokio::test]
async fn a_transfer_walks_the_five_states_moves_the_safes_and_a_short_receipt_is_an_exception() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    let flow = db.flow();
    let fifty = db.usd("50").await;
    let t = submitted(&db, &c, 4).await;
    flow.decide(&c.manager_b1, &t.id, Outcome::Approved, "ok", at(20))
        .await
        .expect("approve");

    let t = flow
        .dispatch(&c.creator, &t.id, at(30))
        .await
        .expect("dispatch");
    assert_eq!(t.status, TransferStatus::Dispatched);
    assert_eq!(
        db.safe_usd(&c.b1).await,
        50_000 - 20_000,
        "USD 200 left B1's safe"
    );
    assert_eq!(db.safe_usd(&c.b2).await, 0, "in transit, not yet received");

    assert!(
        flow.receive(&c.manager_b1, &t.id, vec![], at(35))
            .await
            .is_err(),
        "only the destination receives"
    );
    let t = flow
        .receive(
            &c.receiver_b2,
            &t.id,
            vec![NewTransferLine {
                currency: usd(),
                denomination_id: fifty,
                qty: 3,
            }],
            at(40),
        )
        .await
        .expect("received by B2");
    assert_eq!(t.status, TransferStatus::Received);
    assert_eq!(
        db.safe_usd(&c.b2).await,
        15_000,
        "B2's safe gains what was counted (C4)"
    );
    let exceptions: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT branch_id, data FROM cash_exceptions \
         WHERE branch_id = $1 AND data->>'requestId' = $2",
    )
    .bind(c.b2.as_str())
    .bind(&t.id)
    .fetch_all(&db.pool)
    .await
    .expect("read cash exceptions");
    assert_eq!(exceptions.len(), 1, "one exception, never forced to match");
    let exc = &exceptions[0].1;
    assert_eq!(exc["status"], "OPEN");
    assert_eq!(exc["note"], format!("TRANSFER {}", t.id));
    assert_eq!(exc["safeVariance"]["USD"], -5000);
    assert!(exc["cashVariance"].is_null());
    assert_eq!(exc["at"], at(40).timestamp_millis());

    assert!(
        flow.reconcile(&c.receiver_b2, &t.id, "n", at(50))
            .await
            .is_err(),
        "not the receiver"
    );
    assert!(
        flow.reconcile(&c.creator, &t.id, "n", at(51))
            .await
            .is_err(),
        "not the creator"
    );
    let t = flow
        .reconcile(&c.finance, &t.id, "short 1 × 50", at(60))
        .await
        .expect("FINANCE reconciles");
    assert_eq!(t.status, TransferStatus::Reconciled);
    assert_eq!(db.status(&t.id).await, "RECONCILED");
}

#[tokio::test]
async fn a_full_receipt_writes_no_exception() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    let flow = db.flow();
    let fifty = db.usd("50").await;
    let t = submitted(&db, &c, 2).await;
    flow.decide(&c.manager_b1, &t.id, Outcome::Approved, "ok", at(20))
        .await
        .unwrap();
    flow.dispatch(&c.creator, &t.id, at(30)).await.unwrap();
    flow.receive(
        &c.receiver_b2,
        &t.id,
        vec![NewTransferLine {
            currency: usd(),
            denomination_id: fifty,
            qty: 2,
        }],
        at(40),
    )
    .await
    .unwrap();
    let sql = format!(
        "SELECT count(*) FROM cash_exceptions WHERE data->>'requestId' = '{}'",
        t.id
    );
    assert_eq!(db.scalar(&sql).await, 0);
}

// ── the per-kind CHECKs (ruling 3) ─────────────────────────────────────────

#[tokio::test]
async fn each_kind_refuses_a_row_whose_ends_break_the_matrix() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    // One wrong end per kind, with both shift columns NULL so no shift row is needed: BRANCH without its
    // destination branch; each single-branch kind naming a destination branch (and the drawer kinds missing their shift).
    let b2 = c.b2.as_str().to_owned();
    let cases: [(&str, Option<&str>); 5] = [
        ("BRANCH", None),
        ("DRAWER_TO_SAFE", Some(b2.as_str())),
        ("SAFE_TO_DRAWER", Some(b2.as_str())),
        ("EXTERNAL_IN", Some(b2.as_str())),
        ("EXTERNAL_OUT", Some(b2.as_str())),
    ];
    for (kind, to_branch) in cases {
        let err = sqlx::query(
            "INSERT INTO cash_transfers (organization_id, kind, from_branch_id, to_branch_id, status, created_by, idempotency_key) \
             VALUES ($1::uuid, $2, $3, $4, 'DRAFT', $5::uuid, $6)",
        )
        .bind(ORG)
        .bind(kind)
        .bind(c.b1.as_str())
        .bind(to_branch)
        .bind(&c.creator.user_id)
        .bind(unique("k"))
        .execute(&db.pool)
        .await
        .expect_err(kind);
        let code = err
            .as_database_error()
            .and_then(|e| e.code())
            .map(|c| c.into_owned());
        assert_eq!(
            code.as_deref(),
            Some("23514"),
            "{kind}: a CHECK refuses it: {err}"
        );
    }
}

#[tokio::test]
async fn the_allowed_end_shapes_are_accepted_for_the_kinds_without_a_shift() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    for (kind, to_branch) in [
        ("BRANCH", Some(c.b2.as_str())),
        ("EXTERNAL_IN", None),
        ("EXTERNAL_OUT", None),
    ] {
        sqlx::query(
            "INSERT INTO cash_transfers (organization_id, kind, from_branch_id, to_branch_id, status, created_by, idempotency_key) \
             VALUES ($1::uuid, $2, $3, $4, 'DRAFT', $5::uuid, $6)",
        )
        .bind(ORG)
        .bind(kind)
        .bind(c.b1.as_str())
        .bind(to_branch)
        .bind(&c.creator.user_id)
        .bind(unique("k"))
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{kind}: {e}"));
    }
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
