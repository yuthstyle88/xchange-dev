//! Up-front tests (BE-3, #5107): `HandoverService::start_in` refuses a second hand-over while one is already `PENDING`
//! from the same shift (`HandoverError::AlreadyPending`), and `HandoverService::cancel_in` lets the giver cancel a
//! `PENDING` hand-over (`CANCELLED`) while keeping their shift `OPEN` and drawer untouched.
//!
//! Written before the implementation by the test writer gemini/pool-2 (ticket author claude/bc8fd29a);
//! do not edit, delete or weaken.
//!
//! Root cause (§2):
//! - `HandoverService::start_in` (`api/crates/xc-cash/src/application/handover/handover_service.rs:318-`) checks the
//!   counter, the receiver and the drawer, but never whether `from_shift_id` already has a `PENDING` hand-over.
//! - The only ways out of `PENDING` are the receiver's accept and reject (`:740`, `:799`). `handovers.status` allows
//!   only `PENDING`, `ACCEPTED` and `REJECTED` (`api/migrations/20261012100000_cash.sql:9`).
//!
//! What this file pins (§4, H1–H5):
//! - H1: starting a hand-over while one is `PENDING` from the same shift is refused with
//!   `HandoverError::AlreadyPending`, and nothing is written.
//! - H2: starting a hand-over after the first was `REJECTED` or `CANCELLED` is allowed.
//! - H3: the giver cancelling a `PENDING` hand-over moves it to `CANCELLED`, leaves the giver's shift `OPEN` and
//!   drawer unchanged, posts nothing to the ledger, and no longer counts as holding cash in `holds_cash_in`.
//! - H4: someone other than the giver (e.g. the receiver or a manager) cancelling a `PENDING` hand-over is refused,
//!   and the hand-over stays `PENDING`.
//! - H5: cancelling an `ACCEPTED` or `REJECTED` hand-over is refused with `HandoverError::NotPending`.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverService, StartHandover};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::handover::{Handover, HandoverError, HandoverStatus};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 25, 2, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

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
        let schema = unique("hopend");
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
            .expect("every migration applies");
        Db { pool }
    }

    /// A branch with THB 10,000 in its SAFE (ledger) and a count-difference limit of `limit_thb`.
    async fn branch(&self, limit_thb: &str) -> BranchId {
        let id = unique("main");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert a branch");
        sqlx::query(
            "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
             VALUES ($1::uuid, $2, $3::numeric, 'test', now())",
        )
        .bind(ORG)
        .bind(&id)
        .bind(limit_thb)
        .execute(&self.pool)
        .await
        .expect("a count-difference limit");
        let b = BranchId::new(id);
        let mut tx = self.pool.begin().await.unwrap();
        LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[PostingRequest {
                    from: CashPlace::Bank,
                    to: CashPlace::Safe(b.clone()),
                    currency: CurrencyCode::THB,
                    quantity: ForeignAmount::from_decimal(d("10000.00")),
                    source: SourceRef {
                        source_type: SourceType::Opening,
                        source_id: unique("fund"),
                        source_key: "thb".to_string(),
                    },
                    line: None,
                    reverses: None,
                }],
                None,
            )
            .await
            .unwrap_or_else(|e| panic!("fund the safe: {e:?}"));
        tx.commit().await.unwrap();
        b
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
                "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
                 VALUES ($1::uuid, $2, $3::uuid, NULL, '2026-01-01T00:00:00Z')",
            )
            .bind(&id)
            .bind(role.as_str())
            .bind(ORG)
            .execute(&self.pool)
            .await
            .expect("give the user a role");
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

    async fn face(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the {currency} {face} face value: {e}"))
    }

    /// A place's THB balance in the ledger, through the port.
    async fn thb_at(&self, place: CashPlace) -> Decimal {
        let mut tx = self.pool.begin().await.unwrap();
        let b = LedgerCashPostings::new()
            .expected_in(&mut tx, &place)
            .await
            .expect("expected_in");
        tx.rollback().await.unwrap();
        b.get(CurrencyCode::THB).value().normalize()
    }

    async fn int(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn shift_status(&self, id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM shifts WHERE id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("read the shift status")
    }

    async fn handover_status(&self, id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM handovers WHERE id = $1::uuid")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("read the hand-over status")
    }
}

async fn on_int(tx: &mut Transaction<'_, Postgres>, sql: &str) -> i64 {
    sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
        .fetch_one(&mut **tx)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

/// THB 1000 × 2 + 100 × `hundreds`.
async fn thb(db: &Db, hundreds: i32) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.face("THB", "1000").await, 2),
        CountLine::new("THB", db.face("THB", "100").await, hundreds),
    ]
}

/// The floor: a branch with the given limit, teller A on a shift whose drawer holds THB 2,300, teller B to receive,
/// and branch manager Krit.
struct Floor {
    db: Db,
    main: BranchId,
    a: Actor,
    b: Actor,
    manager: Actor,
    a_shift: String,
}

async fn floor(limit_thb: &str) -> Floor {
    let db = Db::fresh().await;
    let main = db.branch(limit_thb).await;
    let a = db.person("Ploy", &[Role::Teller]).await;
    let b = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let counts = CashCountService::pg(db.pool.clone());
    let open = counts
        .submit(
            &a,
            SubmitCount {
                branch: main.clone(),
                shift_id: None,
                kind: CountKind::Open,
                lines: thb(&db, 3).await,
                note: None,
                idempotency_key: unique("open"),
            },
            at(0),
        )
        .await
        .expect("A submits the OPEN count");
    let a_shift = counts
        .verify(&manager, &open.id, at(60))
        .await
        .expect("the BRANCH_MANAGER verifies the OPEN count")
        .shift_id
        .expect("A's shift");
    assert_eq!(
        db.thb_at(CashPlace::Drawer(a_shift.clone())).await,
        d("2300"),
        "the fixture: A's drawer expects THB 2,300"
    );
    Floor {
        db,
        main,
        a,
        b,
        manager,
        a_shift,
    }
}

async fn start_cmd(f: &Floor, hundreds: i32) -> StartHandover {
    StartHandover {
        branch: f.main.clone(),
        from_shift_id: f.a_shift.clone(),
        receiver_id: f.b.user_id.clone(),
        lines: thb(&f.db, hundreds).await,
        note: None,
        idempotency_key: unique("handover"),
    }
}

/// A starts a hand-over to B counting THB 2,000 + 100 × `hundreds`.
async fn start(f: &Floor, hundreds: i32) -> Handover {
    HandoverService::pg(f.db.pool.clone())
        .start(&f.a, start_cmd(f, hundreds).await, at(600))
        .await
        .expect("A starts the hand-over")
}

// ── H1: start a hand-over while one is PENDING from the same shift ───────────

#[tokio::test]
async fn h1_start_while_one_is_pending_from_the_same_shift_is_refused_already_pending() {
    let f = floor("500").await;
    let h1 = start(&f, 3).await;
    assert_eq!(h1.status, HandoverStatus::Pending);
    let handovers_before = f.db.int("SELECT count(*) FROM handovers").await;
    let counts_before = f.db.int("SELECT count(*) FROM cash_counts").await;

    let mut tx = f.db.pool.begin().await.unwrap();
    let res = HandoverService::pg(f.db.pool.clone())
        .start_in(&mut tx, &f.a, start_cmd(&f, 3).await, at(650))
        .await;

    match res {
        Err(AppError::Domain(msg)) => {
            assert_eq!(
                msg,
                HandoverError::AlreadyPending.to_string(),
                "starting a second hand-over while one is PENDING must be refused AlreadyPending"
            );
        }
        other => panic!(
            "starting a second hand-over while one is PENDING must be refused AlreadyPending; got {other:?}"
        ),
    }

    assert_eq!(
        on_int(&mut tx, "SELECT count(*) FROM handovers").await,
        handovers_before,
        "no second hand-over row is written"
    );
    assert_eq!(
        on_int(&mut tx, "SELECT count(*) FROM cash_counts").await,
        counts_before,
        "no second HANDOVER_OUT count is written"
    );
    tx.rollback().await.unwrap();
    assert_eq!(f.db.handover_status(&h1.id).await, "PENDING");
}

// ── H2: start after the first was REJECTED or CANCELLED ──────────────────────

#[tokio::test]
async fn h2_start_after_first_was_rejected_or_cancelled_is_allowed() {
    let f = floor("500").await;
    let h1 = start(&f, 3).await;

    // 1. After REJECTED
    HandoverService::pg(f.db.pool.clone())
        .reject(&f.b, &h1.id, at(650))
        .await
        .expect("B rejects the first hand-over");
    assert_eq!(f.db.handover_status(&h1.id).await, "REJECTED");

    let mut tx = f.db.pool.begin().await.unwrap();
    let h2 = HandoverService::pg(f.db.pool.clone())
        .start_in(&mut tx, &f.a, start_cmd(&f, 3).await, at(700))
        .await
        .expect("starting a hand-over after the first was REJECTED is allowed");
    tx.commit().await.unwrap();
    assert_eq!(h2.status, HandoverStatus::Pending);
    assert_ne!(h2.id, h1.id);

    // 2. After CANCELLED
    let mut tx = f.db.pool.begin().await.unwrap();
    let cancelled = HandoverService::pg(f.db.pool.clone())
        .cancel_in(&mut tx, &f.a, &h2.id, at(750))
        .await
        .expect("A cancels the second hand-over");
    tx.commit().await.unwrap();
    assert_eq!(cancelled.status, HandoverStatus::Cancelled);
    assert_eq!(f.db.handover_status(&h2.id).await, "CANCELLED");

    let mut tx = f.db.pool.begin().await.unwrap();
    let h3 = HandoverService::pg(f.db.pool.clone())
        .start_in(&mut tx, &f.a, start_cmd(&f, 3).await, at(800))
        .await
        .expect("starting a hand-over after the previous was CANCELLED is allowed");
    tx.commit().await.unwrap();
    assert_eq!(h3.status, HandoverStatus::Pending);
    assert_ne!(h3.id, h2.id);
}

// ── H3: giver cancels a PENDING hand-over ────────────────────────────────────

#[tokio::test]
async fn h3_giver_cancels_a_pending_handover() {
    let f = floor("500").await;
    let h = start(&f, 3).await;
    let entries_before = f.db.int("SELECT count(*) FROM ledger_entries").await;

    let mut tx = f.db.pool.begin().await.unwrap();
    let cancelled = HandoverService::pg(f.db.pool.clone())
        .cancel_in(&mut tx, &f.a, &h.id, at(700))
        .await
        .expect("the giver cancels their PENDING hand-over");
    tx.commit().await.unwrap();

    assert_eq!(cancelled.id, h.id);
    assert_eq!(cancelled.status, HandoverStatus::Cancelled);
    assert_eq!(cancelled.status.as_str(), "CANCELLED");
    assert_eq!(f.db.handover_status(&h.id).await, "CANCELLED");
    assert_eq!(
        f.db.shift_status(&f.a_shift).await,
        "OPEN",
        "giver's shift stays OPEN"
    );
    assert_eq!(
        f.db.thb_at(CashPlace::Drawer(f.a_shift.clone())).await,
        d("2300"),
        "giver's drawer is untouched"
    );
    assert_eq!(
        f.db.int("SELECT count(*) FROM ledger_entries").await,
        entries_before,
        "nothing posts to the ledger"
    );

    let mut conn = f.db.pool.acquire().await.unwrap();
    assert!(
        !HandoverService::holds_cash_in(&mut conn, ORG, &f.b.user_id)
            .await
            .expect("holds_cash_in"),
        "holds_cash_in no longer counts a CANCELLED hand-over for the receiver"
    );
}

// ── H4: someone other than the giver cancels ─────────────────────────────────

#[tokio::test]
async fn h4_someone_other_than_the_giver_cancelling_is_refused() {
    let f = floor("500").await;
    let h = start(&f, 3).await;

    for other in [&f.b, &f.manager] {
        let mut tx = f.db.pool.begin().await.unwrap();
        let res = HandoverService::pg(f.db.pool.clone())
            .cancel_in(&mut tx, other, &h.id, at(700))
            .await;
        assert!(
            res.is_err(),
            "someone other than the giver ({}) must be refused when cancelling; got {res:?}",
            other.user_id
        );
        tx.rollback().await.unwrap();
    }

    assert_eq!(
        f.db.handover_status(&h.id).await,
        "PENDING",
        "the hand-over stays PENDING"
    );
}

// ── H5: cancel an ACCEPTED or REJECTED hand-over ─────────────────────────────

#[tokio::test]
async fn h5_cancelling_an_accepted_or_rejected_handover_is_refused_not_pending() {
    // 1. ACCEPTED hand-over
    let f1 = floor("500").await;
    let h1 = start(&f1, 3).await;
    HandoverService::pg(f1.db.pool.clone())
        .accept(&f1.b, &h1.id, at(650))
        .await
        .expect("B accepts the hand-over");
    assert_eq!(f1.db.handover_status(&h1.id).await, "ACCEPTED");

    let mut tx = f1.db.pool.begin().await.unwrap();
    let res = HandoverService::pg(f1.db.pool.clone())
        .cancel_in(&mut tx, &f1.a, &h1.id, at(700))
        .await;
    match res {
        Err(AppError::Domain(msg)) => {
            assert_eq!(msg, HandoverError::NotPending.to_string());
        }
        other => {
            panic!("cancelling an ACCEPTED hand-over must be refused NotPending; got {other:?}")
        }
    }
    tx.rollback().await.unwrap();
    assert_eq!(f1.db.handover_status(&h1.id).await, "ACCEPTED");

    // 2. REJECTED hand-over
    let f2 = floor("500").await;
    let h2 = start(&f2, 3).await;
    HandoverService::pg(f2.db.pool.clone())
        .reject(&f2.b, &h2.id, at(650))
        .await
        .expect("B rejects the hand-over");
    assert_eq!(f2.db.handover_status(&h2.id).await, "REJECTED");

    let mut tx = f2.db.pool.begin().await.unwrap();
    let res = HandoverService::pg(f2.db.pool.clone())
        .cancel_in(&mut tx, &f2.a, &h2.id, at(700))
        .await;
    match res {
        Err(AppError::Domain(msg)) => {
            assert_eq!(msg, HandoverError::NotPending.to_string());
        }
        other => {
            panic!("cancelling a REJECTED hand-over must be refused NotPending; got {other:?}")
        }
    }
    tx.rollback().await.unwrap();
    assert_eq!(f2.db.handover_status(&h2.id).await, "REJECTED");
}
