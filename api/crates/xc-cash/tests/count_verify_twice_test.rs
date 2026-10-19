//! Tests for #2675:
//! A cash count can be verified twice: verifying an OPEN count again opens a second shift
//! whose opening has no cash in the ledger (verify_in never checks verified_at).
//!
//! Tests every row of #2675 §9 (behaviour table):
//! 1. OPEN count, first verify (existing behaviour, keep): Verified, shift S1 opens, opening posted from SAFE.
//! 2. The same OPEN count verified again after S1 is closed: Err(Conflict), no new shift row, count unchanged.
//! 3. The same OPEN count verified again while S1 is still open: Err(Conflict), takes precedence over ShiftAlreadyOpen.
//! 4. A CLOSE count verified again: Err(Conflict), no second close posting or close_on_verified.
//! 5. A count never verified, not the newest: NotNewestCount, unchanged.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::domain::count::{CashCount, CountKind, CountLine};
use xc_cash::domain::error::CashError;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
static NEXT: AtomicU64 = AtomicU64::new(0);
static LG4_FUND: AtomicU64 = AtomicU64::new(0);

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
        let schema = unique("verify_twice_test");
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

    async fn count_shifts_for_teller(&self, teller_id: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM shifts WHERE teller_id = $1::uuid")
            .bind(teller_id)
            .fetch_one(&self.pool)
            .await
            .expect("count shifts for teller")
    }

    async fn count_details(
        &self,
        id: &str,
    ) -> (Option<String>, Option<String>, Option<DateTime<Utc>>) {
        sqlx::query_as(
            "SELECT shift_id, verified_by::text, verified_at FROM cash_counts WHERE id = $1::uuid",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .expect("read count details")
    }

    async fn balance(&self, shift: &str, currency: &str) -> (Decimal, Decimal, Option<Decimal>) {
        use xc_shift::domain::shift::repositories::shift_balances::ShiftBalances;
        let rows = xc_cash::infrastructure::ledger::shift_balances::PgLedgerShiftBalances::new(
            self.pool.clone(),
        )
        .shift_balances(&shift.to_owned())
        .await
        .unwrap_or_else(|e| panic!("the {currency} balance of {shift}: {e}"));
        let row = rows
            .into_iter()
            .find(|b| b.currency.as_str() == currency)
            .unwrap_or_else(|| panic!("the {currency} balance of {shift}: row not found"));
        let to_dec = |minor: i128| Decimal::from_i128_with_scale(minor, 2).normalize();
        (
            to_dec(row.opening.minor_i128()),
            to_dec(row.expected.minor()),
            row.counted.map(|m| to_dec(m.minor_i128())),
        )
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

// ── Behaviour table row 1: OPEN count, first verify ─────────────────────────

#[tokio::test]
async fn open_count_first_verify_opens_shift_and_posts_opening() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let lines = thb_2300(&db).await;
    let counts = db.counts();

    let count = counts
        .submit(&teller, cmd(&branch, None, CountKind::Open, lines), at(0))
        .await
        .expect("submit OPEN count");

    let verified = counts
        .verify(&manager, &count.id, at(60))
        .await
        .expect("the BRANCH_MANAGER verifies the OPEN count");

    assert_eq!(verified.verified_by, Some(manager.user_id.clone()));
    let shift = verified.shift_id.expect("the shift it opened");
    assert_eq!(db.shift_status(&shift).await, "OPEN");

    let (opening, expected, _) = db.balance(&shift, "THB").await;
    assert_eq!(opening, d("2300"), "opening balance in drawer");
    assert_eq!(expected, d("2300"), "expected balance in drawer");
}

// ── Behaviour table row 2: The same OPEN count verified again after S1 is closed

#[tokio::test]
async fn open_count_verified_again_after_shift_closed_is_refused_conflict() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let (shift_id, open_count) = open_shift(&db, &branch, &teller, &manager).await;
    let counts = db.counts();

    // Close S1
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
        .expect("submit CLOSE count");
    counts
        .verify(&manager, &close.id, at(660))
        .await
        .expect("verify CLOSE count closes shift");
    assert_eq!(db.shift_status(&shift_id).await, "CLOSED");

    let shifts_before = db.count_shifts_for_teller(&teller.user_id).await;
    let (orig_shift, orig_by, orig_at) = db.count_details(&open_count.id).await;

    // Verify the OPEN count again after S1 is closed
    let res = counts.verify(&manager, &open_count.id, at(700)).await;

    match res {
        Err(AppError::Conflict(msg)) => {
            assert_eq!(msg, "การนับนี้ได้รับการรับรองแล้ว");
        }
        other => panic!("expected Err(AppError::Conflict), got {other:?}"),
    }

    // No new shift row created
    assert_eq!(
        db.count_shifts_for_teller(&teller.user_id).await,
        shifts_before
    );

    // Count's shift_id, verified_by, verified_at are unchanged
    let (after_shift, after_by, after_at) = db.count_details(&open_count.id).await;
    assert_eq!(orig_shift, after_shift);
    assert_eq!(orig_by, after_by);
    assert_eq!(orig_at, after_at);
}

// ── Behaviour table row 3: The same OPEN count verified again while S1 is still open

#[tokio::test]
async fn open_count_verified_again_while_shift_open_is_refused_conflict() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let (shift_id, open_count) = open_shift(&db, &branch, &teller, &manager).await;
    assert_eq!(db.shift_status(&shift_id).await, "OPEN");

    let counts = db.counts();
    let res = counts.verify(&manager, &open_count.id, at(120)).await;

    // Must be Conflict, taking precedence over ShiftAlreadyOpen
    match res {
        Err(AppError::Conflict(msg)) => {
            assert_eq!(msg, "การนับนี้ได้รับการรับรองแล้ว");
        }
        other => panic!("expected Err(AppError::Conflict), got {other:?}"),
    }
}

// ── Behaviour table row 4: A CLOSE count verified again ──────────────────────

#[tokio::test]
async fn close_count_verified_again_is_refused_conflict() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let (shift_id, _) = open_shift(&db, &branch, &teller, &manager).await;
    let counts = db.counts();

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
        .expect("submit CLOSE count");

    counts
        .verify(&manager, &close.id, at(660))
        .await
        .expect("first verify of CLOSE count succeeds");
    assert_eq!(db.shift_status(&shift_id).await, "CLOSED");

    // Second verify of the same CLOSE count
    let res = counts.verify(&manager, &close.id, at(720)).await;

    match res {
        Err(AppError::Conflict(msg)) => {
            assert_eq!(msg, "การนับนี้ได้รับการรับรองแล้ว");
        }
        other => panic!("expected Err(AppError::Conflict), got {other:?}"),
    }
}

// ── Behaviour table row 5: A count never verified, not the newest ───────────

#[tokio::test]
async fn count_never_verified_not_newest_returns_not_newest_count() {
    let db = Db::fresh().await;
    let branch = db.branch("main").await;
    let teller = db.person("Ploy", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let (shift_id, _) = open_shift(&db, &branch, &teller, &manager).await;
    let counts = db.counts();

    let first_close = counts
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
        .expect("submit first CLOSE count");

    let _second_close = counts
        .submit(
            &teller,
            cmd(
                &branch,
                Some(&shift_id),
                CountKind::Close,
                thb_2300(&db).await,
            ),
            at(660),
        )
        .await
        .expect("submit second CLOSE count (recount)");

    // Attempt to verify the first CLOSE count (which is not newest and never verified)
    let res = counts.verify(&manager, &first_close.id, at(700)).await;

    match res {
        Err(AppError::Domain(msg)) => {
            assert_eq!(msg, CashError::NotNewestCount.to_string());
        }
        other => panic!("expected Err(AppError::Domain(NotNewestCount)), got {other:?}"),
    }
}
