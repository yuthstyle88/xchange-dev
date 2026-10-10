//! CA-4 (#620) races and idempotency tests:
//! - a retried start with the same key → same result, one count, one handover;
//! - reject after accept → refused and status stays ACCEPTED;
//! - accept after reject → refused and status stays REJECTED;
//! - retried start after accept → returns the accepted handover without duplicating rows;
//! - accept and reject in two **overlapping** transactions: the one that waits for the row lock is
//!   refused `NotPending` and changes nothing (added by the test author, claude/14cc24bd, on the
//!   lane controller's ruling on #620; the two sequential tests above pass on unlocked code too).

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverService, StartHandover};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::handover::{HandoverError, HandoverStatus};
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

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

struct Db {
    pool: PgPool,
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
        let schema = unique("ca4_race");
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
        let migrator = Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations");
        migrator.run(&pool).await.expect("apply migrations");
        Db { pool }
    }

    async fn branch(&self, label: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert branch");
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
        .expect("insert user");
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
            .expect("give role");
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
        .unwrap_or_else(|e| panic!("the {currency} {face} face value: {e}"))
    }

    fn counts(&self) -> CashCountService {
        CashCountService::pg(self.pool.clone())
    }

    fn handovers(&self) -> HandoverService {
        HandoverService::pg(self.pool.clone())
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }
}

async fn thb_2300(db: &Db) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 3),
    ]
}

async fn opened(db: &Db, branch: &BranchId, teller: &Actor, manager: &Actor) -> String {
    let counts = db.counts();
    let count = counts
        .submit(
            teller,
            SubmitCount {
                branch: branch.clone(),
                shift_id: None,
                kind: CountKind::Open,
                lines: thb_2300(db).await,
                note: None,
                idempotency_key: unique("open"),
            },
            at(0),
        )
        .await
        .expect("submit OPEN count");
    counts
        .verify(manager, &count.id, at(60))
        .await
        .expect("verify OPEN count")
        .shift_id
        .expect("names shift")
}

struct Floor {
    db: Db,
    main: BranchId,
    a: Actor,
    b: Actor,
    a_shift: String,
}

async fn floor() -> Floor {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let a = db.person("Ploy", &[Role::Teller]).await;
    let b = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;
    let a_shift = opened(&db, &main, &a, &manager).await;
    Floor {
        db,
        main,
        a,
        b,
        a_shift,
    }
}

#[tokio::test]
async fn retried_start_with_same_key_returns_same_result_one_count_one_handover() {
    let f = floor().await;
    let hs = f.db.handovers();
    let key = unique("handover_idem");

    let cmd1 = StartHandover {
        branch: f.main.clone(),
        from_shift_id: f.a_shift.clone(),
        receiver_id: f.b.user_id.clone(),
        lines: thb_2300(&f.db).await,
        note: Some("attempt 1".into()),
        idempotency_key: key.clone(),
    };
    let h1 = hs
        .start(&f.a, cmd1, at(600))
        .await
        .expect("first start succeeds");

    let cmd2 = StartHandover {
        branch: f.main.clone(),
        from_shift_id: f.a_shift.clone(),
        receiver_id: f.b.user_id.clone(),
        lines: thb_2300(&f.db).await,
        note: Some("attempt 2".into()),
        idempotency_key: key.clone(),
    };
    let h2 = hs
        .start(&f.a, cmd2, at(601))
        .await
        .expect("retried start returns same handover");

    assert_eq!(h1.id, h2.id);
    assert_eq!(h1.count_id, h2.count_id);
    assert_eq!(h1.status, HandoverStatus::Pending);
    assert_eq!(h2.status, HandoverStatus::Pending);

    let count_rows =
        f.db.count(&format!(
            "SELECT count(*) FROM cash_counts WHERE idempotency_key = '{key}'"
        ))
        .await;
    assert_eq!(count_rows, 1, "exactly one cash_counts row written");

    let handover_rows =
        f.db.count(&format!(
            "SELECT count(*) FROM handovers WHERE count_id = '{}'::uuid",
            h1.count_id
        ))
        .await;
    assert_eq!(handover_rows, 1, "exactly one handovers row written");
}

#[tokio::test]
async fn reject_after_accept_refused_and_status_stays_accepted() {
    let f = floor().await;
    let hs = f.db.handovers();

    let cmd = StartHandover {
        branch: f.main.clone(),
        from_shift_id: f.a_shift.clone(),
        receiver_id: f.b.user_id.clone(),
        lines: thb_2300(&f.db).await,
        note: None,
        idempotency_key: unique("handover"),
    };
    let h = hs.start(&f.a, cmd, at(600)).await.expect("start succeeds");

    let accepted = hs
        .accept(&f.b, &h.id, at(700))
        .await
        .expect("accept succeeds");
    assert_eq!(accepted.status, HandoverStatus::Accepted);
    assert!(accepted.to_shift_id.is_some());

    // Receiver attempts reject after accept
    let reject_result = hs.reject(&f.b, &h.id, at(800)).await;
    match reject_result {
        Err(AppError::Domain(msg)) => {
            assert_eq!(msg, HandoverError::NotPending.to_string());
        }
        other => panic!("expected AppError::Domain(NotPending), got {other:?}"),
    }

    // Handover in DB remains ACCEPTED
    let (status, to_shift): (String, Option<String>) =
        sqlx::query_as("SELECT status, to_shift_id FROM handovers WHERE id = $1::uuid")
            .bind(&h.id)
            .fetch_one(&f.db.pool)
            .await
            .expect("read handover");
    assert_eq!(status, "ACCEPTED");
    assert_eq!(to_shift, accepted.to_shift_id);
}

#[tokio::test]
async fn accept_after_reject_refused_and_status_stays_rejected() {
    let f = floor().await;
    let hs = f.db.handovers();

    let cmd = StartHandover {
        branch: f.main.clone(),
        from_shift_id: f.a_shift.clone(),
        receiver_id: f.b.user_id.clone(),
        lines: thb_2300(&f.db).await,
        note: None,
        idempotency_key: unique("handover"),
    };
    let h = hs.start(&f.a, cmd, at(600)).await.expect("start succeeds");

    let rejected = hs
        .reject(&f.b, &h.id, at(700))
        .await
        .expect("reject succeeds");
    assert_eq!(rejected.status, HandoverStatus::Rejected);

    // Receiver attempts accept after reject
    let accept_result = hs.accept(&f.b, &h.id, at(800)).await;
    match accept_result {
        Err(AppError::Domain(msg)) => {
            assert_eq!(msg, HandoverError::NotPending.to_string());
        }
        other => panic!("expected AppError::Domain(NotPending), got {other:?}"),
    }

    let status: String = sqlx::query_scalar("SELECT status FROM handovers WHERE id = $1::uuid")
        .bind(&h.id)
        .fetch_one(&f.db.pool)
        .await
        .expect("read handover");
    assert_eq!(status, "REJECTED");
}

#[tokio::test]
async fn retried_start_after_accept_returns_accepted_handover() {
    let f = floor().await;
    let hs = f.db.handovers();
    let key = unique("handover_accepted_retry");

    let cmd1 = StartHandover {
        branch: f.main.clone(),
        from_shift_id: f.a_shift.clone(),
        receiver_id: f.b.user_id.clone(),
        lines: thb_2300(&f.db).await,
        note: None,
        idempotency_key: key.clone(),
    };
    let h1 = hs.start(&f.a, cmd1, at(600)).await.expect("start succeeds");
    let accepted = hs
        .accept(&f.b, &h1.id, at(700))
        .await
        .expect("accept succeeds");
    assert_eq!(accepted.status, HandoverStatus::Accepted);

    // Caller retries start with same idempotency key after accept
    let cmd2 = StartHandover {
        branch: f.main.clone(),
        from_shift_id: f.a_shift.clone(),
        receiver_id: f.b.user_id.clone(),
        lines: thb_2300(&f.db).await,
        note: None,
        idempotency_key: key.clone(),
    };
    let h2 = hs
        .start(&f.a, cmd2, at(800))
        .await
        .expect("retry start succeeds");
    assert_eq!(h2.id, accepted.id);
    assert_eq!(h2.status, HandoverStatus::Accepted);
    assert_eq!(h2.to_shift_id, accepted.to_shift_id);
}

// ── two overlapping transactions (test author, on the lane controller's ruling) ──────────────

/// A PENDING hand-over of A's shift to B.
async fn pending(f: &Floor) -> String {
    f.db.handovers()
        .start(
            &f.a,
            StartHandover {
                branch: f.main.clone(),
                from_shift_id: f.a_shift.clone(),
                receiver_id: f.b.user_id.clone(),
                lines: thb_2300(&f.db).await,
                note: None,
                idempotency_key: unique("race"),
            },
            at(600),
        )
        .await
        .expect("A starts the hand-over")
        .id
}

async fn status_of(f: &Floor, id: &str) -> String {
    sqlx::query_scalar("SELECT status FROM handovers WHERE id = $1::uuid")
        .bind(id)
        .fetch_one(&f.db.pool)
        .await
        .expect("the hand-over row")
}

/// T1 stands in for a concurrent decision: it holds the hand-over row (`FOR UPDATE`) while the
/// service call starts; the test then writes the decision in T1 and commits it. The service call, which overlaps T1,
/// must wait for the row and then see the decision.
async fn hold_row(f: &Floor, id: &str) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut t1 = f.db.pool.begin().await.expect("begin T1");
    sqlx::query("SELECT 1 FROM handovers WHERE id = $1::uuid FOR UPDATE")
        .bind(id)
        .execute(&mut *t1)
        .await
        .expect("T1 holds the row");
    t1
}

#[tokio::test]
async fn a_reject_overlapping_an_accept_that_holds_the_row_is_refused_and_overwrites_nothing() {
    let f = floor().await;
    let id = pending(&f).await;
    let mut t1 = hold_row(&f, &id).await;

    let (svc, b, rid) = (f.db.handovers(), f.b.clone(), id.clone());
    let reject = tokio::spawn(async move { svc.reject(&b, &rid, at(700)).await });
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    sqlx::query("UPDATE handovers SET status = 'ACCEPTED' WHERE id = $1::uuid")
        .bind(&id)
        .execute(&mut *t1)
        .await
        .expect("T1 accepts");
    t1.commit().await.expect("T1 commits");

    match reject.await.expect("the reject task") {
        Err(AppError::Domain(msg)) => assert_eq!(msg, HandoverError::NotPending.to_string()),
        other => panic!("the overlapping reject must be refused NotPending, got {other:?}"),
    }
    assert_eq!(
        status_of(&f, &id).await,
        "ACCEPTED",
        "the accept is not overwritten"
    );
}

#[tokio::test]
async fn an_accept_overlapping_a_reject_that_holds_the_row_is_refused_and_moves_no_shift() {
    let f = floor().await;
    let id = pending(&f).await;
    let mut t1 = hold_row(&f, &id).await;

    let (svc, b, aid) = (f.db.handovers(), f.b.clone(), id.clone());
    let accept = tokio::spawn(async move { svc.accept(&b, &aid, at(700)).await });
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    sqlx::query("UPDATE handovers SET status = 'REJECTED' WHERE id = $1::uuid")
        .bind(&id)
        .execute(&mut *t1)
        .await
        .expect("T1 rejects");
    t1.commit().await.expect("T1 commits");

    match accept.await.expect("the accept task") {
        Err(AppError::Domain(msg)) => assert_eq!(msg, HandoverError::NotPending.to_string()),
        other => panic!("the overlapping accept must be refused NotPending, got {other:?}"),
    }
    assert_eq!(
        status_of(&f, &id).await,
        "REJECTED",
        "the reject is not overwritten"
    );
    assert_eq!(
        f.db.count(&format!(
            "SELECT count(*) FROM shifts WHERE id = '{}' AND status = 'OPEN'",
            f.a_shift
        ))
        .await,
        1,
        "A keeps the drawer: A's shift is still open"
    );
    assert_eq!(
        f.db.count(&format!(
            "SELECT count(*) FROM shifts WHERE teller_id = '{}'::uuid AND status = 'OPEN'",
            f.b.user_id
        ))
        .await,
        0,
        "B holds no shift"
    );
}
