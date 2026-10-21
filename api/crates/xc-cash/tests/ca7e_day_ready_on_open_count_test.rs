//! CA-7e: the branch day opens on the first verified OPEN count. Design `docs/design/blueprint/cash-shift-transfer.md`
//! §4.2 ("CLOSED --first OPEN count verified--> READY", and the paragraph on the phase as read) and §9.1's CA-7e row;
//! the BE lane controller's ruling Q1 on #626 (2026-10-15).
//!
//! Written before the implementation by the ticket author (claude/c4d72c). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! The end of a day is derived, not stored (`BranchDay::read_phase`: a stored CUT_OFF with no OPEN shift reads as
//! CLOSED). So the rule applies to the phase **as read before the count's shift is inserted**:
//! - READY is written when the day reads CLOSED: stored CLOSED, no `ops_state` row, or stored CUT_OFF with no OPEN
//!   shift;
//! - READY, OPEN, and CUT_OFF with another teller's shift still open are left as stored.
//!
//! Only `phase` is written, on the verify's transaction. A CLOSE count and a hand-over accept do not apply the rule.
//!
//! These tests touch only the `ops_state` columns that outlive CA-10 (`phase`, `entity`, `target`,
//! `cutoff`, `service_opened_at`), never `enabled` or `custodian_id`, so CA-10's M3 does not break them.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips. Each test works in a
//! schema of its own.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, TimeZone, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::count_service::CountVerified;
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverService, StartHandover};
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

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
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
        let schema = unique("ca7e");
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

// ------------------------------------------------------------ the day moves to READY

/// §4.2: a branch whose day is stored CLOSED is READY once the first OPEN count is verified.
#[tokio::test]
async fn a_verified_open_count_moves_a_closed_day_to_ready() {
    let db = Db::fresh().await;
    let branch = db.branch(Some("CLOSED")).await;
    let teller = db.person(Role::Teller, &branch).await;
    let manager = db.person(Role::BranchManager, &branch).await;

    open_shift(&db, &branch, &teller, &manager, 0).await;

    assert_eq!(db.phase(&branch).await.as_deref(), Some("READY"));
}

/// §4.2: a branch that never had a day row reads as CLOSED, so its first verified OPEN count leaves a row at READY.
#[tokio::test]
async fn a_branch_with_no_day_row_gets_one_at_ready() {
    let db = Db::fresh().await;
    let branch = db.branch(None).await;
    let teller = db.person(Role::Teller, &branch).await;
    let manager = db.person(Role::BranchManager, &branch).await;
    assert_eq!(db.phase(&branch).await, None, "no day row yet");

    open_shift(&db, &branch, &teller, &manager, 0).await;

    assert_eq!(db.phase(&branch).await.as_deref(), Some("READY"));
}

/// §4.2: yesterday's day is stored CUT_OFF and its last shift is closed, so it reads as CLOSED. Today's first
/// verified OPEN count makes it READY.
#[tokio::test]
async fn yesterdays_cut_off_day_with_no_open_shift_starts_again_at_ready() {
    let db = Db::fresh().await;
    let branch = db.branch(Some("CUT_OFF")).await;
    let teller = db.person(Role::Teller, &branch).await;
    let manager = db.person(Role::BranchManager, &branch).await;
    assert_eq!(db.open_shifts(&branch).await, 0);

    open_shift(&db, &branch, &teller, &manager, 0).await;

    assert_eq!(db.phase(&branch).await.as_deref(), Some("READY"));
}

/// §4.2: only `phase` is written. The other kept columns of the day row read back byte for byte.
#[tokio::test]
async fn only_the_phase_is_written() {
    let db = Db::fresh().await;
    let branch = db.branch(Some("CLOSED")).await;
    sqlx::query(
        "UPDATE ops_state SET \
           entity = '{\"company\":\"XC\",\"site\":\"B\",\"confirmedBy\":\"owner\",\"at\":1791262800000}'::jsonb, \
           target = '{\"amounts\":{\"THB\":10000000},\"by\":\"mgr\",\"at\":1791262800000}'::jsonb, \
           cutoff = '{\"at\":1791262800000,\"by\":\"mgr\",\"shiftId\":\"SH-yesterday\",\"lastRef\":null,\"pending\":[]}'::jsonb, \
           service_opened_at = '2026-10-14T02:00:00Z' \
         WHERE branch_id = $1",
    )
    .bind(branch.as_str())
    .execute(&db.pool)
    .await
    .expect("seed the kept columns");
    let before = db.kept_columns(&branch).await;
    let teller = db.person(Role::Teller, &branch).await;
    let manager = db.person(Role::BranchManager, &branch).await;

    open_shift(&db, &branch, &teller, &manager, 0).await;

    assert_eq!(db.phase(&branch).await.as_deref(), Some("READY"));
    assert_eq!(
        db.kept_columns(&branch).await,
        before,
        "entity, target, cutoff and service_opened_at are not written"
    );
}

/// The move is written on the verify's transaction: it reads back on that transaction, and a caller that rolls back
/// leaves the day as stored.
#[tokio::test]
async fn the_day_moves_on_the_callers_transaction_and_rolls_back_with_it() {
    let db = Db::fresh().await;
    let branch = db.branch(Some("CLOSED")).await;
    let teller = db.person(Role::Teller, &branch).await;
    let manager = db.person(Role::BranchManager, &branch).await;
    let counts = db.counts();
    let open = counts
        .submit(
            &teller,
            count_cmd(&branch, None, CountKind::Open, db.thb_2300().await),
            at(0),
        )
        .await
        .expect("the OPEN count, committed");

    let mut tx = db.locked(&branch).await;
    match counts
        .verify_in(&mut tx, &manager, &open.id, at(60))
        .await
        .expect("verify_in on the caller's transaction")
    {
        CountVerified::Verified(_) => {}
        CountVerified::AwaitingFinance(id) => {
            panic!("an OPEN count does not wait for FINANCE (request {id})")
        }
    }
    let on_tx: Option<String> =
        sqlx::query_scalar("SELECT phase FROM ops_state WHERE branch_id = $1")
            .bind(branch.as_str())
            .fetch_optional(&mut *tx)
            .await
            .expect("read the phase on the caller's transaction");
    assert_eq!(
        on_tx.as_deref(),
        Some("READY"),
        "READY on the verify's transaction"
    );
    tx.rollback().await.expect("the caller rolls back");

    assert_eq!(
        db.phase(&branch).await.as_deref(),
        Some("CLOSED"),
        "a rolled-back verify leaves the day as stored"
    );
}

// ------------------------------------------------------------ the day is left as stored

/// §4.2: a second teller's OPEN count on a day that is already READY or OPEN changes nothing.
#[tokio::test]
async fn a_day_already_ready_or_open_is_left_as_stored() {
    for stored in ["READY", "OPEN"] {
        let db = Db::fresh().await;
        let branch = db.branch(Some(stored)).await;
        let teller = db.person(Role::Teller, &branch).await;
        let manager = db.person(Role::BranchManager, &branch).await;

        open_shift(&db, &branch, &teller, &manager, 0).await;

        assert_eq!(
            db.phase(&branch).await.as_deref(),
            Some(stored),
            "{stored} stays {stored}"
        );
    }
}

/// §4.2: a CUT_OFF day with another teller's shift still open reads as CUT_OFF, not CLOSED; a late OPEN count does
/// not reopen it.
#[tokio::test]
async fn a_cut_off_day_with_another_tellers_shift_open_stays_cut_off() {
    let db = Db::fresh().await;
    let branch = db.branch(Some("OPEN")).await;
    let first = db.person(Role::Teller, &branch).await;
    let second = db.person(Role::Teller, &branch).await;
    let manager = db.person(Role::BranchManager, &branch).await;
    open_shift(&db, &branch, &first, &manager, 0).await;
    db.set_phase(&branch, "CUT_OFF").await;
    assert_eq!(
        db.open_shifts(&branch).await,
        1,
        "the first teller's shift is still open"
    );

    open_shift(&db, &branch, &second, &manager, 600).await;

    assert_eq!(db.phase(&branch).await.as_deref(), Some("CUT_OFF"));
}

/// A refused verify (PersonRules: the counter may not verify their own count) leaves the day as stored.
#[tokio::test]
async fn a_refused_verify_leaves_the_day_as_stored() {
    let db = Db::fresh().await;
    let branch = db.branch(Some("CLOSED")).await;
    let teller = db.person(Role::Teller, &branch).await;
    let counts = db.counts();
    let open = counts
        .submit(
            &teller,
            count_cmd(&branch, None, CountKind::Open, db.thb_2300().await),
            at(0),
        )
        .await
        .expect("the OPEN count");

    let refused = counts.verify(&teller, &open.id, at(60)).await;
    assert!(
        refused.is_err(),
        "the counter cannot verify their own count"
    );

    assert_eq!(db.phase(&branch).await.as_deref(), Some("CLOSED"));
    assert_eq!(db.open_shifts(&branch).await, 0);
}

/// The rule belongs to the OPEN count only: verifying a CLOSE count does not write the phase. (The day row is
/// stored CLOSED here on purpose, so that a CLOSE count wrongly applying the rule would turn it READY.)
#[tokio::test]
async fn a_close_count_leaves_the_day_as_stored() {
    let db = Db::fresh().await;
    let branch = db.branch(Some("OPEN")).await;
    let teller = db.person(Role::Teller, &branch).await;
    let manager = db.person(Role::BranchManager, &branch).await;
    let shift = open_shift(&db, &branch, &teller, &manager, 0).await;
    db.set_phase(&branch, "CLOSED").await;
    let counts = db.counts();
    let close = counts
        .submit(
            &teller,
            count_cmd(&branch, Some(&shift), CountKind::Close, db.thb_2300().await),
            at(600),
        )
        .await
        .expect("the CLOSE count");

    counts
        .verify(&manager, &close.id, at(660))
        .await
        .expect("the CLOSE count verifies");

    assert_eq!(db.phase(&branch).await.as_deref(), Some("CLOSED"));
}

/// The rule belongs to the OPEN count only: a hand-over accept opens the receiver's shift without writing the
/// phase. (Stored CLOSED on purpose, as above.)
#[tokio::test]
async fn a_handover_accept_leaves_the_day_as_stored() {
    let db = Db::fresh().await;
    let branch = db.branch(Some("OPEN")).await;
    let giver = db.person(Role::Teller, &branch).await;
    let receiver = db.person(Role::Teller, &branch).await;
    let manager = db.person(Role::BranchManager, &branch).await;
    let shift = open_shift(&db, &branch, &giver, &manager, 0).await;
    db.set_phase(&branch, "CLOSED").await;
    let handovers = db.handovers();
    let h = handovers
        .start(
            &giver,
            StartHandover {
                branch: branch.clone(),
                from_shift_id: shift,
                receiver_id: receiver.user_id.clone(),
                lines: db.thb_2300().await,
                note: None,
                idempotency_key: unique("h"),
            },
            at(120),
        )
        .await
        .expect("the hand-over starts");

    handovers
        .accept(&receiver, &h.id, at(180))
        .await
        .expect("the receiver accepts");

    assert_eq!(db.phase(&branch).await.as_deref(), Some("CLOSED"));
}

// ------------------------------------------------------------ funding

static FUND: AtomicU64 = AtomicU64::new(0);

/// LG-4: an OPEN count takes its counted cash out of the branch's SAFE, so the safe is funded through the ledger
/// first (as `services_on_caller_tx_test.rs` does).
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
