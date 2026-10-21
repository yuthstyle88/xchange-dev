//! #4136 W1: the CLOSE verify asks whether the branch day closed. Design `docs/design/blueprint/queue-day-close.md`
//! §2 rows 1, 2, 5 and 9, §3.3: `CashCountService::verify_in`'s CLOSE branch calls
//! `ShiftService::close_day_if_last_in` right after `close_on_verified`, so the lifecycle hook's `on_day_closed`
//! runs on the verify's transaction.
//!
//! Written before the implementation by the ticket author (claude/studio-claude-9). Do not edit, delete or weaken;
//! add tests in a file of your own. The harness is copied from `ca7e_day_ready_on_open_count_test.rs`.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::count_service::CountVerified;
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::HandoverService;
use xc_cash::domain::count::{CountKind, CountLine};
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

/// `entity`, `target`, `cutoff` and `service_opened_at` of a day row, as text.
type KeptColumns = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

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
    Utc.with_ymd_and_hms(2026, 10, 15, 2, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
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
        let schema = unique("dc4136c");
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
            .expect("every migration applies in version order");
        Db { pool }
    }

    /// A branch whose day is stored as `phase`, or with no `ops_state` row at all when `phase` is `None`. Its SAFE
    /// holds THB 1,000,000.00 in the ledger, which an OPEN count draws on (LG-4).
    async fn branch(&self, phase: Option<&str>) -> BranchId {
        let id = unique("b");
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&id)
            .bind(ORG)
            .execute(&self.pool)
            .await
            .expect("insert a branch");
        if let Some(phase) = phase {
            sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, $2)")
                .bind(&id)
                .bind(phase)
                .execute(&self.pool)
                .await
                .expect("the branch's day row");
        }
        fund_safe(&self.pool, &id, "THB", "1000000.00").await;
        BranchId::new(id)
    }

    /// A user holding `role` on `branch`, and their `Actor`.
    async fn person(&self, role: Role, branch: &BranchId) -> Actor {
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
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
             VALUES ($1::uuid, $2, $3::uuid, $4, '2026-01-01T00:00:00Z')",
        )
        .bind(&id)
        .bind(role.as_str())
        .bind(ORG)
        .bind(branch.as_str())
        .execute(&self.pool)
        .await
        .expect("give the user a role");
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Actor::new(
            &id,
            ORG,
            vec![ScopedRole::new(role, Some(branch.clone()), from, None)],
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

    /// THB 1000 × 2 + 100 × 3 = 2,300.
    async fn thb_2300(&self) -> Vec<CountLine> {
        vec![
            CountLine::new("THB", self.denomination("THB", "1000").await, 2),
            CountLine::new("THB", self.denomination("THB", "100").await, 3),
        ]
    }

    async fn set_phase(&self, branch: &BranchId, phase: &str) {
        sqlx::query("UPDATE ops_state SET phase = $2 WHERE branch_id = $1")
            .bind(branch.as_str())
            .bind(phase)
            .execute(&self.pool)
            .await
            .expect("store the day's phase");
    }

    /// The stored phase (committed), or `None` when the branch has no `ops_state` row.
    async fn phase(&self, branch: &BranchId) -> Option<String> {
        sqlx::query_scalar("SELECT phase FROM ops_state WHERE branch_id = $1")
            .bind(branch.as_str())
            .fetch_optional(&self.pool)
            .await
            .expect("read the day's phase")
    }

    /// Every kept column of the branch's day row, as text, the phase excepted.
    async fn kept_columns(&self, branch: &BranchId) -> Vec<Option<String>> {
        let row: KeptColumns = sqlx::query_as(
            "SELECT entity::text, target::text, cutoff::text, service_opened_at::text \
             FROM ops_state WHERE branch_id = $1",
        )
        .bind(branch.as_str())
        .fetch_one(&self.pool)
        .await
        .expect("read the day row");
        vec![row.0, row.1, row.2, row.3]
    }

    async fn open_shifts(&self, branch: &BranchId) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM shifts WHERE branch_id = $1 AND status = 'OPEN'")
            .bind(branch.as_str())
            .fetch_one(&self.pool)
            .await
            .expect("count open shifts")
    }

    /// The caller's transaction, as the seam opens it: the branch locked in `branch_versions`.
    async fn locked(&self, branch: &BranchId) -> Transaction<'static, Postgres> {
        sqlx::query(
            "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 0) ON CONFLICT (branch_id) DO NOTHING",
        )
        .bind(branch.as_str())
        .execute(&self.pool)
        .await
        .expect("the branch's version row, committed");
        let mut tx = self.pool.begin().await.expect("begin");
        sqlx::query("SET LOCAL statement_timeout = '15s'")
            .execute(&mut *tx)
            .await
            .expect("statement_timeout");
        sqlx::query("SELECT version FROM branch_versions WHERE branch_id = $1 FOR UPDATE")
            .bind(branch.as_str())
            .execute(&mut *tx)
            .await
            .expect("the branch lock the seam holds");
        tx
    }

    fn counts(&self) -> CashCountService {
        CashCountService::pg(self.pool.clone())
    }

    fn handovers(&self) -> HandoverService {
        HandoverService::pg(self.pool.clone())
    }
}

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

/// `teller` submits an OPEN count of THB 2,300 and `verifier` verifies it, through today's wrappers. Answers the
/// opened shift's id.
async fn open_shift(
    db: &Db,
    branch: &BranchId,
    teller: &Actor,
    verifier: &Actor,
    t: i64,
) -> String {
    let counts = db.counts();
    let open = counts
        .submit(
            teller,
            count_cmd(branch, None, CountKind::Open, db.thb_2300().await),
            at(t),
        )
        .await
        .expect("the OPEN count");
    counts
        .verify(verifier, &open.id, at(t + 60))
        .await
        .expect("the OPEN count verifies")
        .shift_id
        .expect("it opens the teller's shift")
}

// ------------------------------------------------------------ #4136

/// Records every `on_day_closed` (the branch, and the OPEN shifts it reads on the verify's transaction); answers an
/// error when `fail` is set.
#[derive(Default)]
struct Recorder {
    calls: std::sync::Mutex<Vec<(String, i64)>>,
    fail: bool,
}

impl Recorder {
    fn calls(&self) -> Vec<(String, i64)> {
        self.calls.lock().expect("recorder lock").clone()
    }
}

impl xc_kernel::shift_hook::ShiftLifecycleHook for Recorder {
    fn on_day_closed<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        _now: DateTime<Utc>,
    ) -> futures::future::BoxFuture<'a, pos_error::AppResult<()>> {
        Box::pin(async move {
            let open: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM shifts WHERE branch_id = $1 AND status = 'OPEN'",
            )
            .bind(branch.as_str())
            .fetch_one(&mut **tx)
            .await?;
            self.calls
                .lock()
                .expect("recorder lock")
                .push((branch.as_str().to_owned(), open));
            if self.fail {
                return Err(pos_error::AppError::Internal("the day hook failed".into()));
            }
            Ok(())
        })
    }
}

fn counts_with(db: &Db, hook: std::sync::Arc<Recorder>) -> CashCountService {
    db.counts().with_shift_lifecycle_hook(hook)
}

async fn submit_close(
    _db: &Db,
    counts: &CashCountService,
    branch: &BranchId,
    teller: &Actor,
    shift: &str,
    lines: Vec<CountLine>,
    t: i64,
) -> String {
    counts
        .submit(
            teller,
            count_cmd(branch, Some(shift), CountKind::Close, lines),
            at(t),
        )
        .await
        .expect("the CLOSE count")
        .id
}

async fn shift_status(db: &Db, shift: &str) -> String {
    sqlx::query_scalar("SELECT status FROM shifts WHERE id = $1")
        .bind(shift)
        .fetch_one(&db.pool)
        .await
        .expect("read the shift")
}

/// Row 1: after cut-off, the verify of the last shift's CLOSE count calls `on_day_closed` once, for this branch,
/// on the verify's transaction (it reads 0 OPEN there). An OPEN count's verify never calls it.
#[tokio::test]
async fn w1_row1_the_last_close_verify_after_cut_off_calls_the_hook_once() {
    let db = Db::fresh().await;
    let branch = db.branch(Some("CLOSED")).await;
    let teller = db.person(Role::Teller, &branch).await;
    let manager = db.person(Role::BranchManager, &branch).await;
    let hook = std::sync::Arc::new(Recorder::default());
    let counts = counts_with(&db, hook.clone());
    let open = counts
        .submit(
            &teller,
            count_cmd(&branch, None, CountKind::Open, db.thb_2300().await),
            at(0),
        )
        .await
        .expect("the OPEN count");
    let shift = counts
        .verify(&manager, &open.id, at(60))
        .await
        .expect("the OPEN count verifies")
        .shift_id
        .expect("a shift");
    assert!(
        hook.calls().is_empty(),
        "an OPEN count's verify does not close a day"
    );
    db.set_phase(&branch, "CUT_OFF").await;

    let close = submit_close(
        &db,
        &counts,
        &branch,
        &teller,
        &shift,
        db.thb_2300().await,
        600,
    )
    .await;
    counts
        .verify(&manager, &close, at(660))
        .await
        .expect("the CLOSE count verifies");

    assert_eq!(hook.calls(), vec![(branch.as_str().to_owned(), 0)]);
    assert_eq!(shift_status(&db, &shift).await, "CLOSED");
}

/// Row 2: a CLOSE verify after cut-off while another teller's shift is OPEN does not call the hook.
#[tokio::test]
async fn w1_row2_a_close_verify_with_another_shift_open_does_not_call_the_hook() {
    let db = Db::fresh().await;
    let branch = db.branch(Some("CLOSED")).await;
    let teller_a = db.person(Role::Teller, &branch).await;
    let teller_b = db.person(Role::Teller, &branch).await;
    let manager = db.person(Role::BranchManager, &branch).await;
    let shift_a = open_shift(&db, &branch, &teller_a, &manager, 0).await;
    open_shift(&db, &branch, &teller_b, &manager, 100).await;
    db.set_phase(&branch, "CUT_OFF").await;
    let hook = std::sync::Arc::new(Recorder::default());
    let counts = counts_with(&db, hook.clone());

    let close = submit_close(
        &db,
        &counts,
        &branch,
        &teller_a,
        &shift_a,
        db.thb_2300().await,
        600,
    )
    .await;
    counts
        .verify(&manager, &close, at(660))
        .await
        .expect("the CLOSE count verifies");

    assert!(hook.calls().is_empty(), "teller B's shift is still OPEN");
}

/// Row 5: the hook's error is the verify's error; the transaction rolls back, so the shift is still OPEN and the
/// count is not verified.
#[tokio::test]
async fn w1_row5_the_hooks_error_rolls_the_close_back() {
    let db = Db::fresh().await;
    let branch = db.branch(Some("CLOSED")).await;
    let teller = db.person(Role::Teller, &branch).await;
    let manager = db.person(Role::BranchManager, &branch).await;
    let shift = open_shift(&db, &branch, &teller, &manager, 0).await;
    db.set_phase(&branch, "CUT_OFF").await;
    let hook = std::sync::Arc::new(Recorder {
        calls: std::sync::Mutex::default(),
        fail: true,
    });
    let counts = counts_with(&db, hook.clone());
    let close = submit_close(
        &db,
        &counts,
        &branch,
        &teller,
        &shift,
        db.thb_2300().await,
        600,
    )
    .await;

    let mut tx = db.locked(&branch).await;
    let answer = counts.verify_in(&mut tx, &manager, &close, at(660)).await;
    assert!(answer.is_err(), "the hook's error is answered");
    tx.rollback().await.expect("the caller rolls back");

    assert_eq!(hook.calls().len(), 1, "the hook was asked once");
    assert_eq!(shift_status(&db, &shift).await, "OPEN");
    let verified: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT verified_at FROM cash_counts WHERE id = $1::uuid")
            .bind(&close)
            .fetch_one(&db.pool)
            .await
            .expect("read the count");
    assert_eq!(verified, None, "the count is not verified");
}

/// Row 9: a CLOSE count whose difference waits for FINANCE answers `AwaitingFinance` before the shift closes, so
/// the hook is not called and the shift is still OPEN.
#[tokio::test]
async fn w1_row9_a_close_awaiting_finance_does_not_call_the_hook() {
    let db = Db::fresh().await;
    let branch = db.branch(Some("CLOSED")).await;
    let teller = db.person(Role::Teller, &branch).await;
    let manager = db.person(Role::BranchManager, &branch).await;
    let shift = open_shift(&db, &branch, &teller, &manager, 0).await;
    db.set_phase(&branch, "CUT_OFF").await;
    sqlx::query(
        "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
         VALUES ($1::uuid, $2, 50.00, 'test', now())",
    )
    .bind(ORG)
    .bind(branch.as_str())
    .execute(&db.pool)
    .await
    .expect("a 50 THB limit");
    let hook = std::sync::Arc::new(Recorder::default());
    let counts = counts_with(&db, hook.clone());
    let short = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 2),
    ];
    let close = submit_close(&db, &counts, &branch, &teller, &shift, short, 600).await;

    let mut tx = db.locked(&branch).await;
    match counts
        .verify_in(&mut tx, &manager, &close, at(660))
        .await
        .expect("verify_in")
    {
        CountVerified::AwaitingFinance(_) => {}
        CountVerified::Verified(_) => {
            panic!("a 100 THB difference over a 50 THB limit waits for FINANCE")
        }
    }
    tx.commit().await.expect("commit the request");

    assert!(
        hook.calls().is_empty(),
        "not called while the close waits for FINANCE"
    );
    assert_eq!(shift_status(&db, &shift).await, "OPEN");
}

/// LG-4: an OPEN count takes its counted cash out of the branch's SAFE, so the safe is funded through the ledger
/// first (as `services_on_caller_tx_test.rs` does).
static FUND: AtomicU64 = AtomicU64::new(0);

async fn fund_safe(pool: &PgPool, branch: &str, currency: &str, amount: &str) {
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
                        "ca7e-fund:{branch}:{currency}:{}",
                        FUND.fetch_add(1, Ordering::Relaxed)
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
