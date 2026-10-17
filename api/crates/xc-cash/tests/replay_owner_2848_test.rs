//! #2848: an idempotency replay answers only the user who wrote the record.
//!
//! Written before the fix by the ticket writer (claude/dell-claude-5). Do not edit, delete or weaken these tests;
//! add your own in another file.
//!
//! `find_count_by_idempotency_key` matches `branch_id` + `idempotency_key` only, and both callers return the hit
//! before the shift-ownership check: `CashCountService::submit_in` (`count_service.rs:814-820`) and
//! `HandoverService::start_in` (`handover_service.rs:165-179`). So another user of the branch who sends the same
//! key gets the first user's count or handover back. These tests require that such a replay is refused with
//! `AppError::Conflict` and writes nothing, and that the same user's retry still answers the first record.
//! (The handover's receiver is refused earlier, with `CashError::SameCounter` — `handover_service.rs` checks the
//! receiver before the lookup — so it is not a case here.)
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.

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

struct Db {
    pool: PgPool,
    schema: String,
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
        let schema = unique("rp2848");
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
        Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply migrations");
        Db { pool, schema }
    }

    async fn drop_schema(self) {
        sqlx::query(AssertSqlSafe(format!(
            "DROP SCHEMA {} CASCADE",
            self.schema
        )))
        .execute(&self.pool)
        .await
        .expect("drop test schema");
    }

    async fn branch(&self, label: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert branch");
        lg4_fund_safe(&self.pool, &id, "THB", "1000000.00").await;
        lg4_fund_safe(&self.pool, &id, "USD", "10000.00").await;
        lg4_fund_safe(&self.pool, &id, "JPY", "100000").await;
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
                "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
                 VALUES ($1::uuid, $2, $3::uuid, NULL, '2026-01-01T00:00:00Z')",
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

    fn handovers(&self) -> HandoverService {
        HandoverService::pg(self.pool.clone())
    }

    async fn rows(&self, table: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"))
    }
}

/// Ploy (A) holds an OPEN shift at `main`; Nok (B) and Dao (C) are tellers of the same organisation, so all three
/// pass `Authorizer::check` for `ShiftOperate` at `main`.
struct Floor {
    db: Db,
    main: BranchId,
    a: Actor,
    b: Actor,
    c: Actor,
    a_shift: String,
    /// The idempotency key of A's OPEN count.
    open_key: String,
    open_count_id: String,
}

async fn floor() -> Floor {
    let db = Db::fresh().await;
    let main = db.branch("main").await;
    let a = db.person("Ploy", &[Role::Teller]).await;
    let b = db.person("Nok", &[Role::Teller]).await;
    let c = db.person("Dao", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    let open_key = unique("open");
    let counts = db.counts();
    let open = counts
        .submit(&a, open_cmd(&db, &main, &open_key).await, at(0))
        .await
        .expect("submit A's OPEN count");
    let a_shift = counts
        .verify(&manager, &open.id, at(60))
        .await
        .expect("verify A's OPEN count")
        .shift_id
        .expect("shift id");

    Floor {
        db,
        main,
        a,
        b,
        c,
        a_shift,
        open_key,
        open_count_id: open.id,
    }
}

async fn thb_2300(db: &Db) -> Vec<CountLine> {
    vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 3),
    ]
}

async fn open_cmd(db: &Db, branch: &BranchId, key: &str) -> SubmitCount {
    SubmitCount {
        branch: branch.clone(),
        shift_id: None,
        kind: CountKind::Open,
        lines: thb_2300(db).await,
        note: None,
        idempotency_key: key.to_owned(),
    }
}

fn handover_cmd(f: &Floor, lines: Vec<CountLine>, key: &str) -> StartHandover {
    StartHandover {
        branch: f.main.clone(),
        from_shift_id: f.a_shift.clone(),
        receiver_id: f.b.user_id.clone(),
        lines,
        note: None,
        idempotency_key: key.to_owned(),
    }
}

fn is_conflict<T: std::fmt::Debug>(result: &Result<T, AppError>) -> bool {
    matches!(result, Err(AppError::Conflict(_)))
}

/// Row 1: B sends A's count key ⇒ `Conflict`, not A's count; nothing is written.
#[tokio::test]
async fn another_users_count_replay_is_a_conflict_not_the_first_users_count() {
    let f = floor().await;
    let before = f.db.rows("cash_counts").await;

    let result =
        f.db.counts()
            .submit(&f.b, open_cmd(&f.db, &f.main, &f.open_key).await, at(120))
            .await;

    assert!(
        is_conflict(&result),
        "B replaying A's key must be AppError::Conflict, got {result:?}"
    );
    if let Ok(count) = &result {
        assert_ne!(count.id, f.open_count_id, "B must not receive A's count");
    }
    assert_eq!(f.db.rows("cash_counts").await, before, "nothing is written");
    f.db.drop_schema().await;
}

/// Row 2: A's own retry of the same key still answers A's first count (pins today's behaviour).
#[tokio::test]
async fn the_same_users_count_replay_still_answers_the_first_count() {
    let f = floor().await;
    let before = f.db.rows("cash_counts").await;

    let again =
        f.db.counts()
            .submit(&f.a, open_cmd(&f.db, &f.main, &f.open_key).await, at(120))
            .await
            .expect("A's retry is answered");

    assert_eq!(again.id, f.open_count_id);
    assert_eq!(f.db.rows("cash_counts").await, before);
    f.db.drop_schema().await;
}

/// Row 3: C (a third teller, not the receiver) sends the key of A's handover ⇒ `Conflict`, not A's handover; nothing is written.
#[tokio::test]
async fn another_users_handover_replay_is_a_conflict_not_the_first_users_handover() {
    let f = floor().await;
    let key = unique("handover");
    let lines = thb_2300(&f.db).await;
    let first =
        f.db.handovers()
            .start(&f.a, handover_cmd(&f, lines.clone(), &key), at(600))
            .await
            .expect("A starts a handover to B");
    let (counts, handovers) = (f.db.rows("cash_counts").await, f.db.rows("handovers").await);

    let result =
        f.db.handovers()
            .start(&f.c, handover_cmd(&f, lines, &key), at(660))
            .await;

    assert!(
        is_conflict(&result),
        "C replaying A's handover key must be AppError::Conflict, got {result:?}"
    );
    if let Ok(h) = &result {
        assert_ne!(h.id, first.id, "C must not receive A's handover");
    }
    assert_eq!(
        f.db.rows("cash_counts").await,
        counts,
        "no count is written"
    );
    assert_eq!(
        f.db.rows("handovers").await,
        handovers,
        "no handover is written"
    );
    f.db.drop_schema().await;
}

/// Row 4: A's own retry of the handover key still answers the first handover (pins today's behaviour).
#[tokio::test]
async fn the_same_users_handover_replay_still_answers_the_first_handover() {
    let f = floor().await;
    let key = unique("handover");
    let lines = thb_2300(&f.db).await;
    let first =
        f.db.handovers()
            .start(&f.a, handover_cmd(&f, lines.clone(), &key), at(600))
            .await
            .expect("A starts a handover to B");
    let handovers = f.db.rows("handovers").await;

    let again =
        f.db.handovers()
            .start(&f.a, handover_cmd(&f, lines, &key), at(660))
            .await
            .expect("A's retry is answered");

    assert_eq!(again.id, first.id);
    assert_eq!(f.db.rows("handovers").await, handovers);
    f.db.drop_schema().await;
}
