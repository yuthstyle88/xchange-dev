//! Ticket #1290 (CA-7c), design `docs/design/blueprint/cash-shift-transfer.md` §4.9 and §9.1's CA-7c row: the
//! xc-cash services run on the caller's transaction.
//!
//! Written before the implementation by the test-writer claude/07d3765f, on the BE lane controller
//! claude/ad05bf8d's order (rule A). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The CM-1 seam runs a command inside `BranchTx::run` / `run_pair`. So each test plays the seam:
//! - It opens a transaction, sets `statement_timeout`, and takes the branch's
//!   `SELECT … FROM branch_versions … FOR UPDATE` exactly as `xc-kernel/src/branch_tx.rs` does. `receive_in`
//!   runs under `Locks::One` on the destination (PR 1299, G9) and refuses any other branch with
//!   `AppError::Forbidden`.
//! - It then calls the `_in` form on that transaction. The form must finish without waiting on the lock its
//!   own caller holds. A tokio timeout turns a self-wait into a failure instead of a hang.
//! - Its rows read back on that transaction.
//! - Rolling the transaction back leaves none of them.
//! - `verify_in` answers awaiting FINANCE as a value (`CountVerified::AwaitingFinance`), and its approval
//!   request commits with the caller's transaction.
//! - The wrappers keep today's behaviour: they commit, and `verify` still maps awaiting FINANCE to
//!   `Err(CashError::AwaitingFinance(id))`.
//!
//! Earlier steps of a flow (an opened shift, a submitted and approved transfer) are made through today's
//! wrappers and committed. Only the step under test runs on the locked transaction.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips. Each test works
//! in a schema of its own.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use xc_approval::domain::request::Outcome;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::count_service::CountVerified;
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverService, StartHandover};
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::error::CashError;
use xc_cash::domain::handover::HandoverError;
use xc_cash::domain::transfer::cash_transfer::{
    NewTransfer, NewTransferLine, TransferError, TransferKind,
};
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// Long enough for real work on a loaded machine, short enough to fail fast on a self-wait.
const STATEMENT_TIMEOUT: &str = "15s";
const MUST_FINISH: Duration = Duration::from_secs(30);

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 11, 9, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
}

/// Fails the test, instead of hanging it, when a form waits on a lock its caller holds.
async fn within<T>(what: &str, fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(MUST_FINISH, fut)
        .await
        .unwrap_or_else(|_| {
            panic!("{what} did not finish: it waits on the lock its own caller holds")
        })
}

// ------------------------------------------------------------ database

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
        let schema = unique("ca7c");
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
        // more than one connection: a form that opens its own transaction must be able to try, so the
        // test can see it wait or commit apart
        let pool = PgPoolOptions::new()
            .max_connections(6)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(dir.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("every migration applies in version order");
        Db { pool }
    }

    /// A branch with USD 500.00 in its safe.
    async fn branch(&self, label: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&id)
            .bind(ORG)
            .execute(&self.pool)
            .await
            .expect("insert a branch");
        sqlx::query(
            "INSERT INTO ops_state (branch_id, enabled, phase, safe) \
             VALUES ($1, true, 'OPEN', jsonb_build_object('USD', 50000::bigint))",
        )
        .bind(&id)
        .execute(&self.pool)
        .await
        .expect("the branch's safe");
        // LG-4 (#929, 1:1): the same USD 500.00 is the safe's figure in the ledger, which now holds it.
        lg4_fund_safe(&self.pool, &id, "USD", "500.00").await;
        lg4_fund_safe(&self.pool, &id, "THB", "1000000.00").await;
        BranchId::new(id)
    }

    /// A user holding `role` on `branch` (`None` = organisation-wide), and their `Actor`.
    async fn person(&self, role: Role, branch: Option<&BranchId>) -> Actor {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique(role.as_str()))
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
        .bind(branch.map(|b| b.as_str().to_owned()))
        .execute(&self.pool)
        .await
        .expect("give the user a role");
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Actor::new(
            &id,
            ORG,
            vec![ScopedRole::new(role, branch.cloned(), from, None)],
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
        .unwrap_or_else(|e| panic!("the {currency} {face} face value: {e}"))
    }

    async fn transfer_limit_usd(&self, by: &Actor, amount: &str) {
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

    async fn cash_limit(&self, branch: &BranchId, limit_thb: &str) {
        sqlx::query(
            "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
             VALUES ($1::uuid, $2, $3::numeric, 'test', now())",
        )
        .bind(ORG)
        .bind(branch.as_str())
        .bind(limit_thb)
        .execute(&self.pool)
        .await
        .expect("a cash limit");
    }

    /// A count of rows, through the pool (only committed rows).
    async fn committed(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn committed_text(&self, sql: &str) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// The caller's transaction, as the seam opens it: `statement_timeout` set, and `branches` locked in
    /// `branch_versions` exactly as `BranchTx::run` / `run_pair` lock them (in id order).
    async fn locked(&self, branches: &[&BranchId]) -> Transaction<'static, Postgres> {
        let mut tx = self
            .pool
            .begin()
            .await
            .expect("begin the caller's transaction");
        sqlx::query(AssertSqlSafe(format!(
            "SET LOCAL statement_timeout = '{STATEMENT_TIMEOUT}'"
        )))
        .execute(&mut *tx)
        .await
        .expect("statement_timeout");
        let mut ids: Vec<&str> = branches.iter().map(|b| b.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            // The version row is committed first, as it is for any branch that has had a write: a row
            // created only inside this transaction would be invisible to a second connection, which could
            // then never wait on it.
            sqlx::query(
                "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 0) ON CONFLICT (branch_id) DO NOTHING",
            )
            .bind(id)
            .execute(&self.pool)
            .await
            .expect("the branch's version row, committed");
            sqlx::query("SELECT version FROM branch_versions WHERE branch_id = $1 FOR UPDATE")
                .bind(id)
                .execute(&mut *tx)
                .await
                .expect("the branch lock the seam holds");
        }
        tx
    }

    fn counts(&self) -> CashCountService {
        CashCountService::pg(self.pool.clone())
    }

    fn handovers(&self) -> HandoverService {
        HandoverService::pg(self.pool.clone())
    }

    fn transfers(&self) -> TransferFlow {
        TransferFlow::pg(self.pool.clone())
    }
}

/// A count of rows on the caller's transaction (its own uncommitted writes included).
async fn on(tx: &mut Transaction<'static, Postgres>, sql: &str) -> i64 {
    sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
        .fetch_one(&mut **tx)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

async fn on_text(tx: &mut Transaction<'static, Postgres>, sql: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(AssertSqlSafe(sql.to_owned()))
        .fetch_one(&mut **tx)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

// ------------------------------------------------------------ scenarios

fn count_cmd(
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

/// A branch, its teller and branch manager, and the teller's shift opened through today's wrappers (2,300 THB).
struct Floor {
    branch: BranchId,
    teller: Actor,
    manager: Actor,
    shift: String,
}

async fn floor(db: &Db) -> Floor {
    let branch = db.branch("b").await;
    let teller = db.person(Role::Teller, Some(&branch)).await;
    let manager = db.person(Role::BranchManager, Some(&branch)).await;
    let counts = db.counts();
    let open = counts
        .submit(
            &teller,
            count_cmd(&branch, None, CountKind::Open, thb_2300(db).await),
            at(0),
        )
        .await
        .expect("the OPEN count");
    let shift = counts
        .verify(&manager, &open.id, at(60))
        .await
        .expect("the OPEN count verifies")
        .shift_id
        .expect("it opens the teller's shift");
    Floor {
        branch,
        teller,
        manager,
        shift,
    }
}

/// A BRANCH transfer of USD 50 × `notes` from a new branch to another, approved, through today's wrappers.
struct Moving {
    from: BranchId,
    to: BranchId,
    creator: Actor,
    receiver: Actor,
    finance: Actor,
    id: String,
    usd50: i64,
}

async fn approved_transfer(db: &Db, notes: i32) -> Moving {
    let from = db.branch("from").await;
    let to = db.branch("to").await;
    let creator = db.person(Role::BranchManager, Some(&from)).await;
    let approver = db.person(Role::BranchManager, Some(&from)).await;
    let receiver = db.person(Role::BranchManager, Some(&to)).await;
    let finance = db.person(Role::Finance, None).await;
    let executive = db.person(Role::Executive, None).await;
    db.transfer_limit_usd(&executive, "1000.00").await;
    let usd50 = db.denomination("USD", "50").await;
    let flow = db.transfers();
    let t = flow
        .create(&creator, branch_move(&from, &to, usd50, notes), at(0))
        .await
        .expect("the creator creates");
    flow.submit(&creator, &t.id, at(10))
        .await
        .expect("the creator submits");
    flow.decide(&approver, &t.id, Outcome::Approved, "ok", at(20))
        .await
        .expect("a branch manager of the source approves within the limit");
    Moving {
        from,
        to,
        creator,
        receiver,
        finance,
        id: t.id,
        usd50,
    }
}

fn branch_move(from: &BranchId, to: &BranchId, usd50: i64, notes: i32) -> NewTransfer {
    NewTransfer {
        kind: TransferKind::Branch,
        from_branch: from.clone(),
        to_branch: Some(to.clone()),
        from_shift: None,
        to_shift: None,
        lines: vec![NewTransferLine {
            currency: CurrencyCode::parse("USD").expect("USD"),
            denomination_id: usd50,
            qty: notes,
        }],
        note: Some("move".to_owned()),
        idempotency_key: unique("t"),
    }
}

fn usd_lines(usd50: i64, notes: i32) -> Vec<NewTransferLine> {
    vec![NewTransferLine {
        currency: CurrencyCode::parse("USD").expect("USD"),
        denomination_id: usd50,
        qty: notes,
    }]
}

fn status_of(transfer_id: &str) -> String {
    format!("SELECT status FROM cash_transfers WHERE id = '{transfer_id}'::uuid")
}

// ------------------------------------------------------------ counts

#[tokio::test]
async fn submit_in_runs_on_the_locked_caller_tx_and_rolls_back_with_it() {
    let db = Db::fresh().await;
    let branch = db.branch("b").await;
    let teller = db.person(Role::Teller, Some(&branch)).await;
    let lines = thb_2300(&db).await;

    let mut tx = db.locked(&[&branch]).await;
    let count = within(
        "submit_in",
        db.counts().submit_in(
            &mut tx,
            &teller,
            count_cmd(&branch, None, CountKind::Open, lines),
            at(0),
        ),
    )
    .await
    .expect("submit_in on the caller's transaction");
    let id = count.id.clone();
    assert_eq!(
        on(
            &mut tx,
            &format!("SELECT count(*) FROM cash_counts WHERE id = '{id}'::uuid")
        )
        .await,
        1
    );
    assert_eq!(
        on(
            &mut tx,
            &format!("SELECT count(*) FROM cash_count_lines WHERE count_id = '{id}'::uuid")
        )
        .await,
        2
    );
    assert_eq!(
        on(
            &mut tx,
            &format!("SELECT count(*) FROM cash_count_totals WHERE count_id = '{id}'::uuid")
        )
        .await,
        1
    );
    tx.rollback().await.expect("the caller rolls back");

    assert_eq!(
        db.committed("SELECT count(*) FROM cash_counts").await,
        0,
        "no count survives"
    );
    assert_eq!(
        db.committed("SELECT count(*) FROM cash_count_lines").await,
        0,
        "no line survives"
    );
    assert_eq!(
        db.committed("SELECT count(*) FROM cash_count_totals").await,
        0,
        "no total survives"
    );
}

/// §9: an idempotent replay inside `_in` answers the existing row, on the same transaction.
#[tokio::test]
async fn submit_in_twice_with_one_key_on_one_tx_is_the_same_count() {
    let db = Db::fresh().await;
    let branch = db.branch("b").await;
    let teller = db.person(Role::Teller, Some(&branch)).await;
    let cmd = count_cmd(&branch, None, CountKind::Open, thb_2300(&db).await);

    let mut tx = db.locked(&[&branch]).await;
    let counts = db.counts();
    let first = within(
        "submit_in",
        counts.submit_in(&mut tx, &teller, cmd.clone(), at(0)),
    )
    .await
    .expect("the first submit_in");
    let again = within(
        "submit_in replay",
        counts.submit_in(&mut tx, &teller, cmd, at(5)),
    )
    .await
    .expect("the replay");
    assert_eq!(again.id, first.id, "the same count");
    assert_eq!(on(&mut tx, "SELECT count(*) FROM cash_counts").await, 1);
    tx.rollback().await.expect("roll back");
}

#[tokio::test]
async fn verify_in_opens_the_shift_on_the_locked_caller_tx_and_rolls_back_with_it() {
    let db = Db::fresh().await;
    let branch = db.branch("b").await;
    let teller = db.person(Role::Teller, Some(&branch)).await;
    let manager = db.person(Role::BranchManager, Some(&branch)).await;
    let counts = db.counts();
    let open = counts
        .submit(
            &teller,
            count_cmd(&branch, None, CountKind::Open, thb_2300(&db).await),
            at(0),
        )
        .await
        .expect("the OPEN count, committed");

    let mut tx = db.locked(&[&branch]).await;
    let verified = within(
        "verify_in",
        counts.verify_in(&mut tx, &manager, &open.id, at(60)),
    )
    .await
    .expect("verify_in on the caller's transaction");
    let shift = match verified {
        CountVerified::Verified(count) => count
            .shift_id
            .expect("the verified OPEN count names its shift"),
        CountVerified::AwaitingFinance(id) => {
            panic!("an OPEN count does not wait for FINANCE (request {id})")
        }
    };
    assert_eq!(
        on_text(
            &mut tx,
            &format!("SELECT status FROM shifts WHERE id = '{shift}'")
        )
        .await
        .as_deref(),
        Some("OPEN"),
        "the shift opens on the caller's transaction"
    );
    tx.rollback().await.expect("the caller rolls back");

    assert_eq!(
        db.committed("SELECT count(*) FROM shifts WHERE status = 'OPEN'")
            .await,
        0,
        "no shift survives"
    );
    assert_eq!(
        db.committed_text(&format!(
            "SELECT verified_by::text FROM cash_counts WHERE id = '{}'::uuid",
            open.id
        ))
        .await,
        None,
        "the count is unverified again"
    );
}

/// A CLOSE count 100 THB short against a limit of 50 THB, submitted through today's wrapper.
async fn short_close_count(db: &Db, f: &Floor) -> String {
    db.cash_limit(&f.branch, "50.00").await;
    let lines = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 2),
    ];
    db.counts()
        .submit(
            &f.teller,
            count_cmd(&f.branch, Some(&f.shift), CountKind::Close, lines),
            at(600),
        )
        .await
        .expect("the short CLOSE count")
        .id
}

/// Done criterion 1: a difference within the branch's limit verifies and writes a `cash_exceptions` row. That row is
/// written on the caller's transaction and rolls back with it.
#[tokio::test]
async fn verify_in_writes_the_count_exception_on_the_caller_tx_and_rolls_back_with_it() {
    let db = Db::fresh().await;
    let f = floor(&db).await;
    db.cash_limit(&f.branch, "500.00").await;
    let lines = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 2),
    ];
    let close = db
        .counts()
        .submit(
            &f.teller,
            count_cmd(&f.branch, Some(&f.shift), CountKind::Close, lines),
            at(600),
        )
        .await
        .expect("a CLOSE count 100 THB short, within the 500 THB limit")
        .id;
    let exceptions =
        format!("SELECT count(*) FROM cash_exceptions WHERE data->>'requestId' = '{close}'");

    let mut tx = db.locked(&[&f.branch]).await;
    let answer = within(
        "verify_in",
        db.counts().verify_in(&mut tx, &f.manager, &close, at(660)),
    )
    .await
    .expect("a difference within the limit verifies");
    assert!(
        matches!(answer, CountVerified::Verified(_)),
        "within the limit it verifies without FINANCE"
    );
    assert_eq!(
        on(&mut tx, &exceptions).await,
        1,
        "the count's exception is written on the caller's transaction"
    );
    let exc_data: (serde_json::Value,) = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT data FROM cash_exceptions WHERE data->>'requestId' = '{close}'"
    )))
    .fetch_one(&mut *tx)
    .await
    .expect("fetch exception on tx");
    let exc = &exc_data.0;
    assert_eq!(exc["status"], "OPEN");
    assert_eq!(exc["note"], format!("COUNT {close}"));
    assert_eq!(exc["cashVariance"]["THB"], -10000);
    assert!(exc["safeVariance"].is_null());
    assert_eq!(exc["at"], at(660).timestamp_millis());
    tx.rollback().await.expect("the caller rolls back");

    assert_eq!(
        db.committed(&exceptions).await,
        0,
        "nothing of the exception survives the rollback"
    );
}

#[tokio::test]
async fn verify_in_answers_awaiting_finance_as_a_value_and_the_request_commits_with_the_caller() {
    let db = Db::fresh().await;
    let f = floor(&db).await;
    let close = short_close_count(&db, &f).await;

    let mut tx = db.locked(&[&f.branch]).await;
    let answer = within(
        "verify_in",
        db.counts().verify_in(&mut tx, &f.manager, &close, at(660)),
    )
    .await
    .expect("awaiting FINANCE is a value, not an error");
    let request = match answer {
        CountVerified::AwaitingFinance(id) => id,
        CountVerified::Verified(_) => panic!("a difference over the limit waits for FINANCE"),
    };
    tx.commit().await.expect("the caller commits");

    assert_eq!(
        db.committed_text(&format!(
            "SELECT status FROM approval_requests WHERE id = '{request}'::uuid"
        ))
        .await
        .as_deref(),
        Some("PENDING"),
        "the approval request commits with the caller's transaction"
    );
    assert_eq!(
        db.committed_text(&format!(
            "SELECT type FROM approval_requests WHERE id = '{request}'::uuid"
        ))
        .await
        .as_deref(),
        Some("COUNT_DIFFERENCE")
    );
    assert_eq!(
        db.committed_text(&format!(
            "SELECT verified_by::text FROM cash_counts WHERE id = '{close}'::uuid"
        ))
        .await,
        None,
        "the count stays unverified"
    );
    let difference: Option<Decimal> = sqlx::query_scalar(
        "SELECT difference FROM cash_count_totals WHERE count_id = $1::uuid AND currency = 'THB'",
    )
    .bind(&close)
    .fetch_one(&db.pool)
    .await
    .expect("the stored difference");
    assert_eq!(difference, Some(d("-100")), "the difference stays written");
}

#[tokio::test]
async fn verify_in_awaiting_finance_rolls_back_with_the_caller() {
    let db = Db::fresh().await;
    let f = floor(&db).await;
    let close = short_close_count(&db, &f).await;

    let mut tx = db.locked(&[&f.branch]).await;
    let answer = within(
        "verify_in",
        db.counts().verify_in(&mut tx, &f.manager, &close, at(660)),
    )
    .await
    .expect("awaiting FINANCE is a value");
    assert!(
        matches!(answer, CountVerified::AwaitingFinance(_)),
        "{answer:?}"
    );
    tx.rollback().await.expect("the caller rolls back");

    assert_eq!(
        db.committed("SELECT count(*) FROM approval_requests WHERE type = 'COUNT_DIFFERENCE'")
            .await,
        0,
        "no approval request survives the caller's rollback"
    );
}

/// §9: a second `verify_in` while the FINANCE request is pending answers awaiting again and writes no second
/// request.
#[tokio::test]
async fn verify_in_again_while_finance_is_pending_writes_no_second_request() {
    let db = Db::fresh().await;
    let f = floor(&db).await;
    let close = short_close_count(&db, &f).await;
    let counts = db.counts();

    let mut tx = db.locked(&[&f.branch]).await;
    let first = within(
        "verify_in",
        counts.verify_in(&mut tx, &f.manager, &close, at(660)),
    )
    .await
    .expect("the first verify_in");
    tx.commit().await.expect("commit");
    let mut tx = db.locked(&[&f.branch]).await;
    let again = within(
        "verify_in again",
        counts.verify_in(&mut tx, &f.manager, &close, at(700)),
    )
    .await
    .expect("the second verify_in");
    tx.commit().await.expect("commit");
    match (first, again) {
        (CountVerified::AwaitingFinance(a), CountVerified::AwaitingFinance(b)) => {
            assert_eq!(a, b, "the same pending request")
        }
        other => panic!("both wait for FINANCE: {other:?}"),
    }
    assert_eq!(
        db.committed("SELECT count(*) FROM approval_requests WHERE type = 'COUNT_DIFFERENCE'")
            .await,
        1,
        "no second request"
    );
}

// ------------------------------------------------------------ hand-overs

/// A hand-over of the floor's shift to a second teller, started through today's wrapper.
async fn started(db: &Db, f: &Floor) -> (Actor, String, String) {
    let receiver = db.person(Role::Teller, Some(&f.branch)).await;
    let h = db
        .handovers()
        .start(
            &f.teller,
            start_cmd(f, &receiver, thb_2300(db).await),
            at(120),
        )
        .await
        .expect("the hand-over starts");
    (receiver, h.id, h.count_id)
}

fn start_cmd(f: &Floor, receiver: &Actor, lines: Vec<CountLine>) -> StartHandover {
    StartHandover {
        branch: f.branch.clone(),
        from_shift_id: f.shift.clone(),
        receiver_id: receiver.user_id.clone(),
        lines,
        note: None,
        idempotency_key: unique("h"),
    }
}

#[tokio::test]
async fn start_in_runs_on_the_locked_caller_tx_and_rolls_back_with_it() {
    let db = Db::fresh().await;
    let f = floor(&db).await;
    let receiver = db.person(Role::Teller, Some(&f.branch)).await;
    let lines = thb_2300(&db).await;
    let counts_before = db.committed("SELECT count(*) FROM cash_counts").await;

    let mut tx = db.locked(&[&f.branch]).await;
    let h = within(
        "start_in",
        db.handovers()
            .start_in(&mut tx, &f.teller, start_cmd(&f, &receiver, lines), at(120)),
    )
    .await
    .expect("start_in on the caller's transaction");
    assert_eq!(
        on_text(
            &mut tx,
            &format!("SELECT status FROM handovers WHERE id = '{}'::uuid", h.id)
        )
        .await
        .as_deref(),
        Some("PENDING")
    );
    tx.rollback().await.expect("the caller rolls back");

    assert_eq!(
        db.committed("SELECT count(*) FROM handovers").await,
        0,
        "no hand-over survives"
    );
    assert_eq!(
        db.committed("SELECT count(*) FROM cash_counts").await,
        counts_before,
        "no HANDOVER_OUT count survives"
    );
}

#[tokio::test]
async fn accept_in_and_reject_in_run_on_the_locked_caller_tx_and_roll_back_with_it() {
    let db = Db::fresh().await;
    for step in ["accept", "reject"] {
        let f = floor(&db).await;
        let (receiver, id, _) = started(&db, &f).await;
        let mut tx = db.locked(&[&f.branch]).await;
        let handovers = db.handovers();
        let h = if step == "accept" {
            within(
                "accept_in",
                handovers.accept_in(&mut tx, &receiver, &id, at(180)),
            )
            .await
        } else {
            within(
                "reject_in",
                handovers.reject_in(&mut tx, &receiver, &id, at(180)),
            )
            .await
        }
        .unwrap_or_else(|e| panic!("{step}_in on the caller's transaction: {e:?}"));
        let expected = if step == "accept" {
            "ACCEPTED"
        } else {
            "REJECTED"
        };
        assert_eq!(
            h.status.as_str(),
            expected,
            "the returned hand-over is this transaction's"
        );
        assert_eq!(
            on_text(
                &mut tx,
                &format!("SELECT status FROM handovers WHERE id = '{id}'::uuid")
            )
            .await
            .as_deref(),
            Some(expected)
        );
        tx.rollback().await.expect("the caller rolls back");
        assert_eq!(
            db.committed_text(&format!(
                "SELECT status FROM handovers WHERE id = '{id}'::uuid"
            ))
            .await
            .as_deref(),
            Some("PENDING"),
            "{step}: the hand-over is PENDING again"
        );
    }
}

/// §9: `accept_in` / `reject_in` on a hand-over that is no longer PENDING is refused, as today.
#[tokio::test]
async fn accept_in_on_a_decided_handover_is_refused() {
    let db = Db::fresh().await;
    let f = floor(&db).await;
    let (receiver, id, _) = started(&db, &f).await;
    db.handovers()
        .reject(&receiver, &id, at(150))
        .await
        .expect("rejected, committed");

    let mut tx = db.locked(&[&f.branch]).await;
    let handovers = db.handovers();
    match within(
        "accept_in",
        handovers.accept_in(&mut tx, &receiver, &id, at(180)),
    )
    .await
    {
        Err(AppError::Domain(m)) => assert_eq!(m, HandoverError::NotPending.to_string()),
        other => panic!("accept_in on a REJECTED hand-over: expected NotPending, got {other:?}"),
    }
    match within(
        "reject_in",
        handovers.reject_in(&mut tx, &receiver, &id, at(181)),
    )
    .await
    {
        Err(AppError::Domain(m)) => assert_eq!(m, HandoverError::NotPending.to_string()),
        other => panic!("reject_in on a REJECTED hand-over: expected NotPending, got {other:?}"),
    }
    tx.rollback().await.expect("roll back");
}

/// §9: `verify_in` on a HANDOVER_OUT count is refused, as today (`OnlyThroughHandover`).
#[tokio::test]
async fn verify_in_on_a_handover_out_count_is_refused() {
    let db = Db::fresh().await;
    let f = floor(&db).await;
    let (_, _, handover_count) = started(&db, &f).await;

    let mut tx = db.locked(&[&f.branch]).await;
    let result = within(
        "verify_in",
        db.counts()
            .verify_in(&mut tx, &f.manager, &handover_count, at(200)),
    )
    .await;
    match result {
        Err(AppError::Domain(m)) => assert_eq!(m, HandoverError::OnlyThroughHandover.to_string()),
        other => panic!("refused with OnlyThroughHandover, got {other:?}"),
    }
    tx.rollback().await.expect("roll back");
}

// ------------------------------------------------------------ transfers

#[tokio::test]
async fn create_in_and_submit_in_run_on_the_locked_caller_tx_and_roll_back_with_it() {
    let db = Db::fresh().await;
    let from = db.branch("from").await;
    let to = db.branch("to").await;
    let creator = db.person(Role::BranchManager, Some(&from)).await;
    let executive = db.person(Role::Executive, None).await;
    db.transfer_limit_usd(&executive, "1000.00").await;
    let usd50 = db.denomination("USD", "50").await;
    let flow = db.transfers();

    let mut tx = db.locked(&[&from]).await;
    let t = within(
        "create_in",
        flow.create_in(&mut tx, &creator, branch_move(&from, &to, usd50, 2), at(0)),
    )
    .await
    .expect("create_in on the caller's transaction");
    let submitted = within(
        "submit_in",
        flow.submit_in(&mut tx, &creator, &t.id, at(10)),
    )
    .await
    .expect("submit_in on the same transaction reads create_in's row");
    assert_eq!(submitted.id, t.id);
    assert_eq!(
        on(
            &mut tx,
            &format!(
                "SELECT count(*) FROM cash_transfer_lines WHERE transfer_id = '{}'::uuid",
                t.id
            )
        )
        .await,
        1
    );
    assert_eq!(
        on(
            &mut tx,
            &format!(
                "SELECT count(*) FROM approval_requests WHERE object_id = '{}'",
                t.id
            )
        )
        .await,
        1,
        "submit_in records its approval request on the caller's transaction"
    );
    tx.rollback().await.expect("the caller rolls back");

    assert_eq!(
        db.committed("SELECT count(*) FROM cash_transfers").await,
        0,
        "no transfer survives"
    );
    assert_eq!(
        db.committed("SELECT count(*) FROM cash_transfer_lines")
            .await,
        0,
        "no line survives"
    );
    assert_eq!(
        db.committed("SELECT count(*) FROM approval_requests").await,
        0,
        "no approval request survives"
    );
}

/// §9: an idempotent replay of `create_in` on one transaction answers the same transfer.
#[tokio::test]
async fn create_in_twice_with_one_key_on_one_tx_is_the_same_transfer() {
    let db = Db::fresh().await;
    let from = db.branch("from").await;
    let to = db.branch("to").await;
    let creator = db.person(Role::BranchManager, Some(&from)).await;
    let usd50 = db.denomination("USD", "50").await;
    let cmd = branch_move(&from, &to, usd50, 2);
    let flow = db.transfers();

    let mut tx = db.locked(&[&from]).await;
    let first = within(
        "create_in",
        flow.create_in(&mut tx, &creator, cmd.clone(), at(0)),
    )
    .await
    .expect("the first create_in");
    let again = within(
        "create_in replay",
        flow.create_in(&mut tx, &creator, cmd, at(5)),
    )
    .await
    .expect("the replay");
    assert_eq!(again.id, first.id);
    assert_eq!(on(&mut tx, "SELECT count(*) FROM cash_transfers").await, 1);
    tx.rollback().await.expect("roll back");
}

/// LG-4 (#929, 1:1): the branch's SAFE USD in minor units, as the ledger holds it: the sum
/// `CashPostings::expected_in(Safe(branch))` takes, as SQL so it runs on a transaction or on committed state as before.
fn safe_usd(branch: &BranchId) -> String {
    format!(
        "SELECT coalesce((sum(e.quantity_signed) * 100)::bigint, 0) FROM ledger_entries e \
         JOIN cash_accounts a ON a.id = e.account_id \
         WHERE a.kind = 'SAFE' AND a.branch_id = '{}' AND trim(a.currency) = 'USD'",
        branch.as_str()
    )
}

#[tokio::test]
async fn dispatch_in_runs_on_the_source_locked_caller_tx_and_rolls_back_with_it() {
    let db = Db::fresh().await;
    let m = approved_transfer(&db, 2).await;

    let mut tx = db.locked(&[&m.from]).await;
    let t = within(
        "dispatch_in",
        db.transfers()
            .dispatch_in(&mut tx, &m.from, &m.creator, &m.id, at(30)),
    )
    .await
    .expect("dispatch_in on a caller holding the source branch's lock");
    assert_eq!(
        t.status.as_str(),
        "DISPATCHED",
        "the returned transfer is this transaction's"
    );
    assert_eq!(
        on(&mut tx, &safe_usd(&m.from)).await,
        50_000 - 10_000,
        "USD 100 left the safe on the caller's tx"
    );
    tx.rollback().await.expect("the caller rolls back");

    assert_eq!(
        db.committed_text(&status_of(&m.id)).await.as_deref(),
        Some("APPROVED")
    );
    assert_eq!(
        db.committed(&safe_usd(&m.from)).await,
        50_000,
        "nothing posted survives"
    );
}

/// §9: `dispatch_in` beyond the expected balance is refused `StockInsufficient` and posts nothing.
#[tokio::test]
async fn dispatch_in_beyond_the_safe_is_refused_and_posts_nothing() {
    let db = Db::fresh().await;
    let m = approved_transfer(&db, 12).await; // USD 600 > 500

    let mut tx = db.locked(&[&m.from]).await;
    let result = within(
        "dispatch_in",
        db.transfers()
            .dispatch_in(&mut tx, &m.from, &m.creator, &m.id, at(30)),
    )
    .await;
    match result {
        Err(AppError::Domain(m)) => assert_eq!(m, TransferError::StockInsufficient.to_string()),
        other => panic!("expected StockInsufficient, got {other:?}"),
    }
    assert_eq!(
        on(&mut tx, &safe_usd(&m.from)).await,
        50_000,
        "nothing posted"
    );
    tx.rollback().await.expect("roll back");
}

#[tokio::test]
async fn receive_in_runs_on_the_destination_locked_caller_tx_and_rolls_back_with_it() {
    let db = Db::fresh().await;
    let m = approved_transfer(&db, 2).await;
    db.transfers()
        .dispatch(&m.creator, &m.id, at(30))
        .await
        .expect("dispatched, committed");

    let mut tx = db.locked(&[&m.to]).await;
    let t = within(
        "receive_in",
        db.transfers().receive_in(
            &mut tx,
            &m.to,
            &m.receiver,
            &m.id,
            usd_lines(m.usd50, 2),
            at(40),
        ),
    )
    .await
    .expect("receive_in on a caller holding the destination's lock (Locks::One)");
    assert_eq!(t.status.as_str(), "RECEIVED");
    assert_eq!(
        on(&mut tx, &safe_usd(&m.to)).await,
        50_000 + 10_000,
        "USD 100 arrived on the caller's tx"
    );
    tx.rollback().await.expect("the caller rolls back");

    assert_eq!(
        db.committed_text(&status_of(&m.id)).await.as_deref(),
        Some("DISPATCHED")
    );
    assert_eq!(
        db.committed(&safe_usd(&m.to)).await,
        50_000,
        "nothing received survives"
    );
}

/// G9: receive_in holds only the destination's lock. Another request holding the source's lock the whole
/// time does not stop it; a receive_in that also took the source's lock would wait here until the timeout.
#[tokio::test]
async fn receive_in_holds_only_the_destination_lock() {
    let db = Db::fresh().await;
    let m = approved_transfer(&db, 2).await;
    db.transfers()
        .dispatch(&m.creator, &m.id, at(30))
        .await
        .expect("dispatched, committed");

    let mut source_holder = db.locked(&[&m.from]).await;
    let mut tx = db.locked(&[&m.to]).await;
    let t = within(
        "receive_in",
        db.transfers().receive_in(
            &mut tx,
            &m.to,
            &m.receiver,
            &m.id,
            usd_lines(m.usd50, 2),
            at(40),
        ),
    )
    .await
    .expect("receive_in finishes while another request holds the source's lock");
    assert_eq!(t.status.as_str(), "RECEIVED");
    tx.commit().await.expect("the caller commits");
    assert_eq!(
        on(&mut source_holder, "SELECT 1::bigint").await,
        1,
        "the source's holder was never disturbed"
    );
    source_holder.rollback().await.expect("release the source");
    assert_eq!(
        db.committed_text(&status_of(&m.id)).await.as_deref(),
        Some("RECEIVED")
    );
}

/// G9: receive_in at a branch that is not the destination is refused with `AppError::Forbidden` (CA-8 maps it to
/// `outOfScope`), and nothing moves.
#[tokio::test]
async fn receive_in_at_a_branch_other_than_the_destination_is_forbidden() {
    let db = Db::fresh().await;
    let m = approved_transfer(&db, 2).await;
    db.transfers()
        .dispatch(&m.creator, &m.id, at(30))
        .await
        .expect("dispatched, committed");
    let safe_at_destination = db.committed(&safe_usd(&m.to)).await;

    let mut tx = db.locked(&[&m.from]).await;
    let refused = within(
        "receive_in",
        db.transfers().receive_in(
            &mut tx,
            &m.from,
            &m.receiver,
            &m.id,
            usd_lines(m.usd50, 2),
            at(40),
        ),
    )
    .await;
    assert!(
        matches!(refused, Err(AppError::Forbidden)),
        "receive_in at the source answers AppError::Forbidden, got {refused:?}"
    );
    assert_eq!(
        on_text(&mut tx, &status_of(&m.id)).await.as_deref(),
        Some("DISPATCHED"),
        "nothing changed on the caller's transaction"
    );
    assert_eq!(on(&mut tx, &safe_usd(&m.to)).await, safe_at_destination);
    tx.commit().await.expect("the caller may still commit");
    assert_eq!(
        db.committed_text(&status_of(&m.id)).await.as_deref(),
        Some("DISPATCHED")
    );
    assert_eq!(db.committed(&safe_usd(&m.to)).await, safe_at_destination);
}

/// §9: a single-branch kind (EXTERNAL_IN: destination = source) receives on a transaction locked once.
#[tokio::test]
async fn receive_in_of_a_single_branch_kind_runs_on_a_once_locked_tx() {
    let db = Db::fresh().await;
    let branch = db.branch("b").await;
    let manager = db.person(Role::BranchManager, Some(&branch)).await;
    let approver = db.person(Role::BranchManager, Some(&branch)).await;
    let finance = db.person(Role::Finance, None).await;
    let executive = db.person(Role::Executive, None).await;
    db.transfer_limit_usd(&executive, "1000.00").await;
    let usd50 = db.denomination("USD", "50").await;
    let flow = db.transfers();
    let cmd = NewTransfer {
        kind: TransferKind::ExternalIn,
        from_branch: branch.clone(),
        to_branch: None,
        from_shift: None,
        to_shift: None,
        lines: usd_lines(usd50, 2),
        note: Some("in".to_owned()),
        idempotency_key: unique("x"),
    };
    let t = flow
        .create(&finance, cmd, at(0))
        .await
        .expect("EXTERNAL_IN created");
    flow.submit(&finance, &t.id, at(10))
        .await
        .expect("submitted");
    flow.decide(&approver, &t.id, Outcome::Approved, "ok", at(20))
        .await
        .expect("approved");
    flow.dispatch(&finance, &t.id, at(30))
        .await
        .expect("dispatched");

    let mut tx = db.locked(&[&branch]).await;
    let received = within(
        "receive_in",
        flow.receive_in(
            &mut tx,
            &branch,
            &manager,
            &t.id,
            usd_lines(usd50, 2),
            at(40),
        ),
    )
    .await
    .expect("receive_in on a once-locked transaction");
    assert_eq!(received.status.as_str(), "RECEIVED");
    tx.rollback().await.expect("roll back");
}

#[tokio::test]
async fn reconcile_in_runs_on_the_destination_locked_caller_tx_and_rolls_back_with_it() {
    let db = Db::fresh().await;
    let m = approved_transfer(&db, 2).await;
    let flow = db.transfers();
    flow.dispatch(&m.creator, &m.id, at(30))
        .await
        .expect("dispatched");
    flow.receive(&m.receiver, &m.id, usd_lines(m.usd50, 2), at(40))
        .await
        .expect("received");

    let mut tx = db.locked(&[&m.to]).await;
    let t = within(
        "reconcile_in",
        flow.reconcile_in(&mut tx, &m.finance, &m.id, "checked", at(50)),
    )
    .await
    .expect("reconcile_in on a caller holding the destination's lock");
    assert_eq!(t.status.as_str(), "RECONCILED");
    assert_eq!(
        on_text(&mut tx, &status_of(&m.id)).await.as_deref(),
        Some("RECONCILED"),
        "read back on the caller's transaction"
    );
    tx.rollback().await.expect("the caller rolls back");
    assert_eq!(
        db.committed_text(&status_of(&m.id)).await.as_deref(),
        Some("RECEIVED")
    );
}

// ------------------------------------------------------------ the wrappers keep today's behaviour

/// Each wrapper commits what it did: a new connection sees it.
#[tokio::test]
async fn the_wrappers_still_commit_their_work() {
    let db = Db::fresh().await;
    let f = floor(&db).await;
    assert_eq!(
        db.committed(&format!(
            "SELECT count(*) FROM shifts WHERE id = '{}' AND status = 'OPEN'",
            f.shift
        ))
        .await,
        1,
        "submit + verify committed the OPEN count and the shift"
    );
    let (receiver, id, _) = started(&db, &f).await;
    assert_eq!(
        db.committed_text(&format!(
            "SELECT status FROM handovers WHERE id = '{id}'::uuid"
        ))
        .await
        .as_deref(),
        Some("PENDING"),
        "start committed"
    );
    db.handovers()
        .accept(&receiver, &id, at(200))
        .await
        .expect("accept");
    assert_eq!(
        db.committed_text(&format!(
            "SELECT status FROM handovers WHERE id = '{id}'::uuid"
        ))
        .await
        .as_deref(),
        Some("ACCEPTED"),
        "accept committed"
    );

    let m = approved_transfer(&db, 2).await;
    let flow = db.transfers();
    flow.dispatch(&m.creator, &m.id, at(30))
        .await
        .expect("dispatch");
    assert_eq!(
        db.committed_text(&status_of(&m.id)).await.as_deref(),
        Some("DISPATCHED")
    );
    flow.receive(&m.receiver, &m.id, usd_lines(m.usd50, 2), at(40))
        .await
        .expect("receive");
    assert_eq!(
        db.committed_text(&status_of(&m.id)).await.as_deref(),
        Some("RECEIVED")
    );
    flow.reconcile(&m.finance, &m.id, "checked", at(50))
        .await
        .expect("reconcile");
    assert_eq!(
        db.committed_text(&status_of(&m.id)).await.as_deref(),
        Some("RECONCILED")
    );
}

/// §8: the wrapper `verify` still answers awaiting FINANCE as `Err(CashError::AwaitingFinance(id))`, after
/// committing the request.
#[tokio::test]
async fn the_verify_wrapper_still_maps_awaiting_finance_to_an_error_after_committing() {
    let db = Db::fresh().await;
    let f = floor(&db).await;
    let close = short_close_count(&db, &f).await;
    match db.counts().verify(&f.manager, &close, at(660)).await {
        Err(AppError::Domain(m)) => {
            let request = db
                .committed_text(
                    "SELECT id::text FROM approval_requests WHERE type = 'COUNT_DIFFERENCE' AND status = 'PENDING'",
                )
                .await
                .expect("the request is committed");
            assert_eq!(m, CashError::AwaitingFinance(request).to_string());
        }
        other => panic!("Err(AwaitingFinance), got {other:?}"),
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

// ------------------------------------------------------------ #2199 count verify organisation scope

const OTHER_ORG: &str = "00000000-0000-0000-0000-0000000000b2";

async fn other_org_person(
    db: &Db,
    role: Role,
    db_branch: Option<&BranchId>,
    actor_branch: Option<&BranchId>,
) -> Actor {
    sqlx::query(
        "INSERT INTO organizations (id, code, timezone, created_at) \
         VALUES ($1::uuid, $2, 'Asia/Bangkok', now()) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(OTHER_ORG)
    .bind("OTHER")
    .execute(&db.pool)
    .await
    .expect("insert the other organisation");
    let id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(OTHER_ORG)
    .bind(unique(role.as_str()))
    .fetch_one(&db.pool)
    .await
    .expect("insert a user in the other organisation");
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) \
         VALUES ($1::uuid, $2, $3::uuid, $4)",
    )
    .bind(&id)
    .bind(role.as_str())
    .bind(OTHER_ORG)
    .bind(db_branch.map(|b| b.as_str().to_owned()))
    .execute(&db.pool)
    .await
    .expect("give the user in the other organisation a role");
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Actor::new(
        &id,
        OTHER_ORG,
        vec![ScopedRole::new(role, actor_branch.cloned(), from, None)],
        None,
    )
}

/// #2199 §9 row 3: an organisation-wide BRANCH_MANAGER in another organisation is refused;
/// `cash_counts.verified_by` stays NULL and no new `shifts` row is written.
#[tokio::test]
async fn verify_in_other_org_organisation_wide_branch_manager_is_refused() {
    use xc_auth::domain::access::Refusal;

    let db = Db::fresh().await;
    let branch = db.branch("b").await;
    let teller = db.person(Role::Teller, Some(&branch)).await;
    let outsider = other_org_person(&db, Role::BranchManager, None, None).await;
    let counts = db.counts();
    let open = counts
        .submit(
            &teller,
            count_cmd(&branch, None, CountKind::Open, thb_2300(&db).await),
            at(0),
        )
        .await
        .expect("the OPEN count, committed");

    let mut tx = db.locked(&[&branch]).await;
    let result = within(
        "verify_in cross-org",
        counts.verify_in(&mut tx, &outsider, &open.id, at(60)),
    )
    .await;
    match result {
        Err(AppError::Domain(m)) => {
            assert_eq!(m, CashError::Refused(Refusal::OutOfScope).to_string());
        }
        other => panic!("expected OutOfScope refusal for cross-org verify_in, got {other:?}"),
    }
    assert_eq!(
        on_text(
            &mut tx,
            &format!(
                "SELECT verified_by::text FROM cash_counts WHERE id = '{}'::uuid",
                open.id
            )
        )
        .await,
        None,
        "cash_counts.verified_by stays NULL on tx"
    );
    assert_eq!(
        on(&mut tx, "SELECT count(*) FROM shifts").await,
        0,
        "no new shifts row on tx"
    );
    tx.commit().await.expect("commit");

    let wrapper_res = counts.verify(&outsider, &open.id, at(60)).await;
    match wrapper_res {
        Err(AppError::Domain(m)) => {
            assert_eq!(m, CashError::Refused(Refusal::OutOfScope).to_string());
        }
        other => panic!("expected OutOfScope refusal for cross-org verify, got {other:?}"),
    }
    assert_eq!(
        db.committed_text(&format!(
            "SELECT verified_by::text FROM cash_counts WHERE id = '{}'::uuid",
            open.id
        ))
        .await,
        None,
        "cash_counts.verified_by stays NULL"
    );
    assert_eq!(
        db.committed("SELECT count(*) FROM shifts").await,
        0,
        "no new shifts row"
    );
}

/// #2199 §9 row 1: a same-org BRANCH_MANAGER scoped to the count's branch verifies the OPEN count and opens the shift
/// (after an other-org manager is refused).
#[tokio::test]
async fn verify_in_same_org_branch_manager_on_count_branch_opens_shift() {
    use xc_auth::domain::access::Refusal;

    let db = Db::fresh().await;
    let branch = db.branch("b").await;
    let teller = db.person(Role::Teller, Some(&branch)).await;
    let manager = db.person(Role::BranchManager, Some(&branch)).await;
    let outsider = other_org_person(&db, Role::BranchManager, None, None).await;
    let counts = db.counts();
    let open = counts
        .submit(
            &teller,
            count_cmd(&branch, None, CountKind::Open, thb_2300(&db).await),
            at(0),
        )
        .await
        .expect("the OPEN count, committed");

    let mut tx = db.locked(&[&branch]).await;
    let cross_org = within(
        "verify_in cross-org",
        counts.verify_in(&mut tx, &outsider, &open.id, at(30)),
    )
    .await;
    assert_eq!(
        cross_org
            .expect_err("other-org manager must be refused")
            .to_string(),
        AppError::Domain(CashError::Refused(Refusal::OutOfScope).to_string()).to_string()
    );

    let verified = within(
        "verify_in same-org branch manager",
        counts.verify_in(&mut tx, &manager, &open.id, at(60)),
    )
    .await
    .expect("same-org branch-scoped manager verifies the OPEN count");
    let count = match verified {
        CountVerified::Verified(c) => c,
        CountVerified::AwaitingFinance(id) => panic!("unexpected AwaitingFinance({id})"),
    };
    assert_eq!(count.verified_by.as_deref(), Some(manager.user_id.as_str()));
    let shift_id = count.shift_id.expect("shift opened");
    assert_eq!(
        on_text(
            &mut tx,
            &format!("SELECT status FROM shifts WHERE id = '{shift_id}'")
        )
        .await
        .as_deref(),
        Some("OPEN")
    );
    tx.rollback().await.expect("roll back");
}

/// #2199 §9 row 2: a same-org organisation-wide BRANCH_MANAGER (`branch = None`) verifies the OPEN count
/// (while an other-org organisation-wide BRANCH_MANAGER is refused).
#[tokio::test]
async fn verify_in_same_org_organisation_wide_branch_manager_verifies_count() {
    use xc_auth::domain::access::Refusal;

    let db = Db::fresh().await;
    let branch = db.branch("b").await;
    let teller = db.person(Role::Teller, Some(&branch)).await;
    let org_wide_manager = db.person(Role::BranchManager, None).await;
    let outsider = other_org_person(&db, Role::BranchManager, None, None).await;
    let counts = db.counts();
    let open = counts
        .submit(
            &teller,
            count_cmd(&branch, None, CountKind::Open, thb_2300(&db).await),
            at(0),
        )
        .await
        .expect("the OPEN count, committed");

    let mut tx = db.locked(&[&branch]).await;
    let cross_org = within(
        "verify_in cross-org",
        counts.verify_in(&mut tx, &outsider, &open.id, at(30)),
    )
    .await;
    assert_eq!(
        cross_org
            .expect_err("other-org manager must be refused")
            .to_string(),
        AppError::Domain(CashError::Refused(Refusal::OutOfScope).to_string()).to_string()
    );

    let verified = within(
        "verify_in same-org org-wide manager",
        counts.verify_in(&mut tx, &org_wide_manager, &open.id, at(60)),
    )
    .await
    .expect("same-org org-wide manager verifies the OPEN count");
    let count = match verified {
        CountVerified::Verified(c) => c,
        CountVerified::AwaitingFinance(id) => panic!("unexpected AwaitingFinance({id})"),
    };
    assert_eq!(
        count.verified_by.as_deref(),
        Some(org_wide_manager.user_id.as_str())
    );
    assert!(count.shift_id.is_some(), "shift opened");
    tx.rollback().await.expect("roll back");
}

/// #2199 §9 row 4: an other-org BRANCH_MANAGER scoped to the count's branch id is refused with OutOfScope.
#[tokio::test]
async fn verify_in_other_org_branch_manager_scoped_to_count_branch_id_is_refused() {
    use xc_auth::domain::access::Refusal;

    let db = Db::fresh().await;
    let branch = db.branch("b").await;
    let teller = db.person(Role::Teller, Some(&branch)).await;
    let outsider_scoped = other_org_person(&db, Role::BranchManager, None, Some(&branch)).await;
    let counts = db.counts();
    let open = counts
        .submit(
            &teller,
            count_cmd(&branch, None, CountKind::Open, thb_2300(&db).await),
            at(0),
        )
        .await
        .expect("the OPEN count, committed");

    let mut tx = db.locked(&[&branch]).await;
    let result = within(
        "verify_in cross-org scoped to count branch",
        counts.verify_in(&mut tx, &outsider_scoped, &open.id, at(60)),
    )
    .await;
    match result {
        Err(AppError::Domain(m)) => {
            assert_eq!(m, CashError::Refused(Refusal::OutOfScope).to_string());
        }
        other => panic!("expected OutOfScope refusal, got {other:?}"),
    }
    assert_eq!(
        on_text(
            &mut tx,
            &format!(
                "SELECT verified_by::text FROM cash_counts WHERE id = '{}'::uuid",
                open.id
            )
        )
        .await,
        None
    );
    assert_eq!(on(&mut tx, "SELECT count(*) FROM shifts").await, 0);
    tx.rollback().await.expect("roll back");
}

/// #2199 §9 row 5: unknown count id returns NotFound (and an other-org manager is refused on an existing count).
#[tokio::test]
async fn verify_in_unknown_count_id_returns_not_found() {
    use xc_auth::domain::access::Refusal;

    let db = Db::fresh().await;
    let branch = db.branch("b").await;
    let teller = db.person(Role::Teller, Some(&branch)).await;
    let manager = db.person(Role::BranchManager, Some(&branch)).await;
    let outsider = other_org_person(&db, Role::BranchManager, None, None).await;
    let counts = db.counts();
    let open = counts
        .submit(
            &teller,
            count_cmd(&branch, None, CountKind::Open, thb_2300(&db).await),
            at(0),
        )
        .await
        .expect("the OPEN count, committed");

    let mut tx = db.locked(&[&branch]).await;
    let unknown_id = "00000000-0000-0000-0000-000000000999";
    let not_found = within(
        "verify_in unknown count",
        counts.verify_in(&mut tx, &manager, unknown_id, at(30)),
    )
    .await;
    assert!(
        matches!(
            not_found,
            Err(AppError::NotFound {
                entity: "cash_count",
                ..
            })
        ),
        "expected NotFound for unknown count id, got {not_found:?}"
    );

    let cross_org = within(
        "verify_in cross-org",
        counts.verify_in(&mut tx, &outsider, &open.id, at(45)),
    )
    .await;
    assert_eq!(
        cross_org
            .expect_err("other-org manager must be refused")
            .to_string(),
        AppError::Domain(CashError::Refused(Refusal::OutOfScope).to_string()).to_string()
    );
    tx.rollback().await.expect("roll back");
}

/// #2199 §9 row 6: a same-org verifier who is the counter is refused by PersonRules::verifier_is_not_counter
/// (after an other-org manager is refused with OutOfScope).
#[tokio::test]
async fn verify_in_same_org_verifier_is_counter_is_refused_by_person_rules() {
    use xc_auth::domain::access::Refusal;

    let db = Db::fresh().await;
    let branch = db.branch("b").await;
    let teller = db.person(Role::Teller, Some(&branch)).await;
    let outsider = other_org_person(&db, Role::BranchManager, None, None).await;
    let counts = db.counts();
    let open = counts
        .submit(
            &teller,
            count_cmd(&branch, None, CountKind::Open, thb_2300(&db).await),
            at(0),
        )
        .await
        .expect("the OPEN count, committed");

    let mut tx = db.locked(&[&branch]).await;
    let cross_org = within(
        "verify_in cross-org",
        counts.verify_in(&mut tx, &outsider, &open.id, at(45)),
    )
    .await;
    assert_eq!(
        cross_org
            .expect_err("other-org manager must be refused")
            .to_string(),
        AppError::Domain(CashError::Refused(Refusal::OutOfScope).to_string()).to_string()
    );

    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let counter_as_manager = Actor::new(
        &teller.user_id,
        ORG,
        vec![ScopedRole::new(
            Role::BranchManager,
            Some(branch.clone()),
            from,
            None,
        )],
        None,
    );
    let self_verify = within(
        "verify_in self",
        counts.verify_in(&mut tx, &counter_as_manager, &open.id, at(60)),
    )
    .await;
    match self_verify {
        Err(AppError::Domain(m)) => assert_eq!(m, CashError::SameCounter.to_string()),
        other => panic!("expected SameCounter refusal, got {other:?}"),
    }
    tx.rollback().await.expect("roll back");
}
