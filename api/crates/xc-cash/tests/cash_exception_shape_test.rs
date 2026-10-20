//! Up-front tests for CA-7d (#1425): cash_exceptions keep the existing CashException row shape (design §4.1, §9.1 CA-7d).
//!
//! 1:1 mapping of §9 rows to tests:
//! - Row 1: `count_verify_writes_one_row_holding_both_currencies`
//! - Row 2: `readers_after_write`
//! - Row 3: `safe_count_fills_safe_variance`
//! - Row 4: `position_is_dense_and_zero_based`
//! - Row 5: `v2_custody_completion_saves_afterwards`
//! - Row 6: `handover_writes_same_shape_with_count_id`
//! - Row 7: `receipt_writes_by_destination_with_transfer_id`

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

use xc_approval::domain::request::Outcome;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverService, StartHandover};

use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::transfer::cash_transfer::{NewTransfer, NewTransferLine, TransferKind};
use xc_kernel::ids::BranchId;

use xc_kernel::money::{CurrencyCode, MoneyDelta};
use xc_shift::domain::custody::entities::cash_exception::{CashException, ExceptionStatus};
use xc_shift::domain::custody::repositories::custody_pages::CustodyPages;
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xc_shift::domain::custody::value_objects::custody_time::CustodyTime;
use xc_shift::domain::shift::value_objects::stock_delta::StockDelta;
use xc_shift::infrastructure::custody::custody_pages::PgCustodyPages;
use xc_shift::infrastructure::custody::custody_repository::{
    PgCustodyRepository, append_cash_exception_in,
};
use xc_shift::infrastructure::custody::decode::exception_from_row;

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
    Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn code(s: &str) -> CurrencyCode {
    CurrencyCode::parse(s).unwrap()
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

static LG4_FUND: AtomicU64 = AtomicU64::new(0);

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
                currency: CurrencyCode::parse(currency).expect("a currency"),
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
        .expect("fund safe in ledger");
    tx.commit().await.expect("commit fund safe");
}

struct Db {
    pool: PgPool,
}

impl Db {
    async fn fresh() -> Db {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set. These tests need a real PostgreSQL.")
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("ca7d");
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
            .expect("connect to test schema");

        Migrator::new(migrations_dir().as_path())
            .await
            .expect("read migrations")
            .run(&pool)
            .await
            .expect("apply migrations");

        Db { pool }
    }

    async fn branch(&self, label: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
             VALUES ($1::uuid, $2, 1000000, 'test', now())",
        )
        .bind(ORG)
        .bind(&id)
        .execute(&self.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
            .bind(&id)
            .execute(&self.pool)
            .await
            .unwrap();

        lg4_fund_safe(&self.pool, &id, "THB", "1000000.00").await;
        lg4_fund_safe(&self.pool, &id, "USD", "1000000.00").await;

        BranchId::new(id)
    }

    async fn person(&self, name: &str, roles: &[(Role, Option<&BranchId>)]) -> Actor {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .expect("insert user");
        for (role, branch) in roles {
            sqlx::query(
                "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) VALUES ($1::uuid, $2, $3::uuid, $4, '2026-01-01T00:00:00Z')",
            )
            .bind(&id)
            .bind(role.as_str())
            .bind(ORG)
            .bind(branch.map(|b| b.as_str().to_owned()))
            .execute(&self.pool)
            .await
            .expect("user role");
        }
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Actor::new(
            &id,
            ORG,
            roles
                .iter()
                .map(|(r, b)| ScopedRole::new(*r, b.map(|x| (*x).clone()), from, None))
                .collect(),
            None,
        )
    }

    async fn denom_id(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    fn counts(&self) -> CashCountService {
        CashCountService::pg(self.pool.clone())
    }

    fn handovers(&self) -> HandoverService {
        HandoverService::pg(self.pool.clone())
    }

    fn flow(&self) -> TransferFlow {
        TransferFlow::pg(self.pool.clone())
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
}

/// Helper: opens a shift for teller at branch with initial amounts (e.g. 1000 THB, 500 USD).
async fn open_shift_with(
    db: &Db,
    branch: &BranchId,
    teller: &Actor,
    manager: &Actor,
    thb_qty: i32,
    usd_qty: i32,
    now: DateTime<Utc>,
) -> String {
    let counts = db.counts();
    let thb_denom = db.denom_id("THB", "1000").await;
    let usd_denom = db.denom_id("USD", "100").await;
    let mut lines = Vec::new();
    if thb_qty > 0 {
        lines.push(CountLine::new("THB", thb_denom, thb_qty));
    }
    if usd_qty > 0 {
        lines.push(CountLine::new("USD", usd_denom, usd_qty));
    }

    let cmd = SubmitCount {
        branch: branch.clone(),
        shift_id: None,
        kind: CountKind::Open,
        lines,
        note: None,
        idempotency_key: unique("open_shift"),
    };
    let open_count = counts.submit(teller, cmd, now).await.unwrap();
    let verified = counts.verify(manager, &open_count.id, now).await.unwrap();
    verified.shift_id.expect("open verified gives shift id")
}

// ─────────────────────────────────────────────────────────────────────────────
// Row 1: Count verify: a verified count with a THB and a USD difference writes
// one row. exception_from_row decodes it as OPEN, with request_id = the count id,
// cash_variance holding both currencies, and at = the command time.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn count_verify_writes_one_row_holding_both_currencies() {
    let db = Db::fresh().await;
    let branch = db.branch("b1").await;
    let teller = db.person("teller", &[(Role::Teller, Some(&branch))]).await;
    let manager = db
        .person("manager", &[(Role::BranchManager, Some(&branch))])
        .await;

    let now_open = at(100);
    let shift_id = open_shift_with(&db, &branch, &teller, &manager, 1, 5, now_open).await; // 1000 THB, 500 USD

    let usd_denom = db.denom_id("USD", "100").await;

    // Close count: count 0 THB (diff -1000 THB) and 6 USD (count 600, diff +100 USD)
    let now_close = at(200);
    let counts = db.counts();
    let close_cmd = SubmitCount {
        branch: branch.clone(),
        shift_id: Some(shift_id),
        kind: CountKind::Close,
        lines: vec![CountLine::new("USD", usd_denom, 6)],
        note: None,
        idempotency_key: unique("close_diff"),
    };
    let count = counts.submit(&teller, close_cmd, now_close).await.unwrap();
    counts.verify(&manager, &count.id, now_close).await.unwrap();

    // Query cash_exceptions
    let rows = sqlx::query("SELECT id, data FROM cash_exceptions WHERE branch_id = $1")
        .bind(branch.as_str())
        .fetch_all(&db.pool)
        .await
        .unwrap();

    assert_eq!(
        rows.len(),
        1,
        "a verified count with differences across multiple currencies writes exactly one row"
    );

    let exc = exception_from_row(&rows[0]).expect("exception_from_row decodes the row");
    assert_eq!(exc.status, ExceptionStatus::Open);
    assert_eq!(exc.request_id, count.id);
    assert_eq!(exc.note, format!("COUNT {}", count.id));
    assert_eq!(exc.at.epoch_millis(), now_close.timestamp_millis());
    assert!(exc.safe_variance.is_none());
    assert!(exc.target_difference.is_none());
    assert!(exc.reviewed_by.is_none());
    assert!(exc.reviewed_at.is_none());

    let cv = exc
        .cash_variance
        .expect("cash_variance must be present for a drawer count");
    assert_eq!(
        cv.get(CurrencyCode::THB).minor(),
        -100000,
        "THB difference is -1000.00 (-100000 minor)"
    );
    assert_eq!(
        cv.get(code("USD")).minor(),
        10000,
        "USD difference is +100.00 (10000 minor)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Row 2: Readers after a write: the branch's custody state still loads, and
// exceptions_between around that time returns the row.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn readers_after_write() {
    let db = Db::fresh().await;
    let branch = db.branch("b1").await;
    let teller = db.person("teller", &[(Role::Teller, Some(&branch))]).await;
    let manager = db
        .person("manager", &[(Role::BranchManager, Some(&branch))])
        .await;

    let now_open = at(100);
    let shift_id = open_shift_with(&db, &branch, &teller, &manager, 1, 0, now_open).await;

    let now_close = at(300);
    let counts = db.counts();
    let close_cmd = SubmitCount {
        branch: branch.clone(),
        shift_id: Some(shift_id),
        kind: CountKind::Close,
        lines: vec![], // 0 counted vs 1000 THB expected -> diff -1000 THB
        note: None,
        idempotency_key: unique("close_reader"),
    };
    let count = counts.submit(&teller, close_cmd, now_close).await.unwrap();
    counts.verify(&manager, &count.id, now_close).await.unwrap();

    // Reader 1: CustodyRepository::load_state loads without error
    let repo = PgCustodyRepository::new(db.pool.clone());
    let state = repo
        .load_state(&branch)
        .await
        .expect("branch custody state still loads after exception write");
    assert_eq!(state.exceptions.len(), 1);
    assert_eq!(state.exceptions[0].request_id, count.id);
    assert_eq!(state.exceptions[0].status, ExceptionStatus::Open);

    // Reader 2: CustodyPages::exceptions_between around that time returns the row
    let pages = PgCustodyPages::new(db.pool.clone());
    let excs = pages
        .exceptions_between(
            &branch,
            now_close.timestamp_millis() - 5000,
            now_close.timestamp_millis() + 5000,
        )
        .await
        .expect("exceptions_between loads rows");
    assert_eq!(excs.len(), 1);
    assert_eq!(excs[0].request_id, count.id);
    assert_eq!(excs[0].note, format!("COUNT {}", count.id));
}

// ─────────────────────────────────────────────────────────────────────────────
// Row 3: SAFE count: it fills safe_variance.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn safe_count_fills_safe_variance() {
    let db = Db::fresh().await;
    let branch = db.branch("b1").await;
    let count_id = unique("safe_count");
    let now = at(400);

    let exc = CashException {
        id: format!("EX-{}", Uuid::new_v4()),
        request_id: count_id.clone(),
        status: ExceptionStatus::Open,
        target_difference: None,
        cash_variance: None,
        safe_variance: Some(StockDelta::of(&[(
            CurrencyCode::THB,
            MoneyDelta::from_minor(-50000),
        )])),
        note: format!("COUNT {count_id}"),
        at: CustodyTime::from_epoch_millis(now.timestamp_millis()),
        reviewed_by: None,
        reviewed_at: None,
    };

    let mut tx = db.pool.begin().await.unwrap();
    append_cash_exception_in(&mut tx, &branch, &exc)
        .await
        .expect("append_cash_exception_in succeeds");
    tx.commit().await.unwrap();

    let row = sqlx::query("SELECT id, data FROM cash_exceptions WHERE branch_id = $1")
        .bind(branch.as_str())
        .fetch_one(&db.pool)
        .await
        .unwrap();

    let decoded = exception_from_row(&row).expect("decodes safe exception");
    assert_eq!(decoded.status, ExceptionStatus::Open);
    assert_eq!(decoded.request_id, count_id);
    assert_eq!(decoded.note, format!("COUNT {count_id}"));
    assert!(
        decoded.cash_variance.is_none(),
        "cash_variance must be None for safe count"
    );
    let sv = decoded.safe_variance.expect("safe_variance must be filled");
    assert_eq!(sv.get(CurrencyCode::THB).minor(), -50000);
}

// ─────────────────────────────────────────────────────────────────────────────
// Row 4: Position: on an empty branch the row takes position 0; with n rows
// (positions 0…n−1) it takes position n.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn position_is_dense_and_zero_based() {
    let db = Db::fresh().await;
    let branch = db.branch("b1").await;
    let now = at(500);

    // Empty branch -> row 0 takes position 0
    let exc0 = CashException {
        id: format!("EX-{}", Uuid::new_v4()),
        request_id: unique("req0"),
        status: ExceptionStatus::Open,
        target_difference: None,
        cash_variance: Some(StockDelta::of(&[(
            code("USD"),
            MoneyDelta::from_minor(1000),
        )])),
        safe_variance: None,
        note: "COUNT 0".to_string(),
        at: CustodyTime::from_epoch_millis(now.timestamp_millis()),
        reviewed_by: None,
        reviewed_at: None,
    };
    let mut tx = db.pool.begin().await.unwrap();
    append_cash_exception_in(&mut tx, &branch, &exc0)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let pos0: i32 = sqlx::query_scalar("SELECT position FROM cash_exceptions WHERE id = $1")
        .bind(&exc0.id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(pos0, 0, "empty branch takes position 0");

    // With 1 row (position 0) -> row 1 takes position 1
    let exc1 = CashException {
        id: format!("EX-{}", Uuid::new_v4()),
        request_id: unique("req1"),
        status: ExceptionStatus::Open,
        target_difference: None,
        cash_variance: Some(StockDelta::of(&[(
            code("USD"),
            MoneyDelta::from_minor(2000),
        )])),
        safe_variance: None,
        note: "COUNT 1".to_string(),
        at: CustodyTime::from_epoch_millis(now.timestamp_millis()),
        reviewed_by: None,
        reviewed_at: None,
    };
    let mut tx = db.pool.begin().await.unwrap();
    append_cash_exception_in(&mut tx, &branch, &exc1)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let pos1: i32 = sqlx::query_scalar("SELECT position FROM cash_exceptions WHERE id = $1")
        .bind(&exc1.id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(pos1, 1, "branch with 1 row takes position 1");

    // With 2 rows (positions 0, 1) -> row 2 takes position 2
    let exc2 = CashException {
        id: format!("EX-{}", Uuid::new_v4()),
        request_id: unique("req2"),
        status: ExceptionStatus::Open,
        target_difference: None,
        cash_variance: Some(StockDelta::of(&[(
            code("USD"),
            MoneyDelta::from_minor(3000),
        )])),
        safe_variance: None,
        note: "COUNT 2".to_string(),
        at: CustodyTime::from_epoch_millis(now.timestamp_millis()),
        reviewed_by: None,
        reviewed_at: None,
    };
    let mut tx = db.pool.begin().await.unwrap();
    append_cash_exception_in(&mut tx, &branch, &exc2)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let pos2: i32 = sqlx::query_scalar("SELECT position FROM cash_exceptions WHERE id = $1")
        .bind(&exc2.id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(pos2, 2, "branch with 2 rows takes position 2");
}

// ─────────────────────────────────────────────────────────────────────────────
// Row 5: v2 save afterwards: a v2 custody completion with a difference still saves,
// with no UNIQUE (branch_id, position) violation.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn v2_custody_completion_saves_afterwards() {
    let db = Db::fresh().await;
    let branch = db.branch("b1").await;
    let now = at(600);

    // First write an exception via append_cash_exception_in (takes position 0)
    let exc0 = CashException {
        id: format!("EX-{}", Uuid::new_v4()),
        request_id: unique("req0"),
        status: ExceptionStatus::Open,
        target_difference: None,
        cash_variance: Some(StockDelta::of(&[(
            CurrencyCode::THB,
            MoneyDelta::from_minor(-10000),
        )])),
        safe_variance: None,
        note: "COUNT 0".to_string(),
        at: CustodyTime::from_epoch_millis(now.timestamp_millis()),
        reviewed_by: None,
        reviewed_at: None,
    };
    let mut tx = db.pool.begin().await.unwrap();
    append_cash_exception_in(&mut tx, &branch, &exc0)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Now v2 custody loads the state
    let repo = PgCustodyRepository::new(db.pool.clone());
    let before = repo.load_state(&branch).await.unwrap();
    assert_eq!(before.exceptions.len(), 1);

    // v2 custody completion adds an exception (which will be at index 1)
    let mut after = before.clone();
    let exc1 = CashException {
        id: format!("EX-{}", Uuid::new_v4()),
        request_id: unique("v2_req"),
        status: ExceptionStatus::Open,
        target_difference: None,
        cash_variance: Some(StockDelta::of(&[(
            code("USD"),
            MoneyDelta::from_minor(-5000),
        )])),
        safe_variance: None,
        note: "v2 completion".to_string(),
        at: CustodyTime::from_epoch_millis(now.timestamp_millis()),
        reviewed_by: None,
        reviewed_at: None,
    };
    after.exceptions.push(exc1);

    // v2 save writes position 1; must NOT collide with UNIQUE(branch_id, position)
    let mut tx = db.pool.begin().await.unwrap();
    repo.store_state_in(&mut tx, &branch, &before, &after)
        .await
        .expect("v2 custody save succeeds with no UNIQUE violation");
    tx.commit().await.unwrap();

    let positions: Vec<i32> = sqlx::query_scalar(
        "SELECT position FROM cash_exceptions WHERE branch_id = $1 ORDER BY position ASC",
    )
    .bind(branch.as_str())
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(positions, vec![0, 1]);
}

// ─────────────────────────────────────────────────────────────────────────────
// Row 6: Hand-over: it writes the same shape, with the hand-over's count id.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn handover_writes_same_shape_with_count_id() {
    let db = Db::fresh().await;
    let branch = db.branch("b1").await;
    let teller_a = db
        .person("teller_a", &[(Role::Teller, Some(&branch))])
        .await;
    let teller_b = db
        .person("teller_b", &[(Role::Teller, Some(&branch))])
        .await;
    let manager = db
        .person("manager", &[(Role::BranchManager, Some(&branch))])
        .await;

    let now = at(700);
    let shift_a = open_shift_with(&db, &branch, &teller_a, &manager, 1, 0, now).await; // 1000 THB

    let handovers = db.handovers();

    // Start handover with counted 0 THB (diff -1000 THB)
    let start_cmd = StartHandover {
        branch: branch.clone(),
        from_shift_id: shift_a,
        receiver_id: teller_b.user_id.clone(),
        lines: vec![], // counted 0 THB vs 1000 expected
        note: None,
        idempotency_key: unique("handover_diff"),
    };
    let h = handovers
        .start(&teller_a, start_cmd, at(710))
        .await
        .unwrap();

    // Teller B accepts the handover
    let accept_time = at(720);
    handovers
        .accept(&teller_b, &h.id, accept_time)
        .await
        .unwrap();

    // Query cash_exceptions
    let rows = sqlx::query("SELECT id, data FROM cash_exceptions WHERE branch_id = $1")
        .bind(branch.as_str())
        .fetch_all(&db.pool)
        .await
        .unwrap();

    assert_eq!(
        rows.len(),
        1,
        "exactly one exception row from handover difference"
    );
    let exc = exception_from_row(&rows[0]).expect("decodes handover exception");
    assert_eq!(exc.status, ExceptionStatus::Open);
    assert_eq!(
        exc.request_id, h.count_id,
        "request_id is the hand-over count id"
    );
    assert_eq!(exc.note, format!("HANDOVER {}", h.id));
    assert_eq!(exc.at.epoch_millis(), accept_time.timestamp_millis());
    assert!(exc.safe_variance.is_none());
    assert!(exc.target_difference.is_none());
    assert!(exc.reviewed_by.is_none());
    assert!(exc.reviewed_at.is_none());

    let cv = exc.cash_variance.expect("cash_variance must be present");
    assert_eq!(cv.get(CurrencyCode::THB).minor(), -100000);
}

// ─────────────────────────────────────────────────────────────────────────────
// Row 7: Receipt: a receipt with fewer notes writes safe_variance or cash_variance
// by its destination, with request_id = the transfer id.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test]
async fn receipt_writes_by_destination_with_transfer_id() {
    let db = Db::fresh().await;
    let b1 = db.branch("b1").await;
    let b2 = db.branch("b2").await;

    let creator_b1 = db
        .person("creator_b1", &[(Role::BranchManager, Some(&b1))])
        .await;
    let manager_b1 = db
        .person("manager_b1", &[(Role::BranchManager, Some(&b1))])
        .await;
    let receiver_b2 = db
        .person("receiver_b2", &[(Role::BranchManager, Some(&b2))])
        .await;

    let flow = db.flow();
    let fifty_denom = db.denom_id("USD", "50").await;
    db.limit_usd(&manager_b1, "1000000.00").await;

    // Subcase 7A: BRANCH transfer -> destination is SAFE (to_shift is None) -> fills safe_variance
    let t_branch = flow
        .create(
            &creator_b1,
            NewTransfer {
                kind: TransferKind::Branch,
                from_branch: b1.clone(),
                to_branch: Some(b2.clone()),
                from_shift: None,
                to_shift: None,
                lines: vec![NewTransferLine {
                    currency: code("USD"),
                    denomination_id: fifty_denom,
                    qty: 4, // 4 * 50 = 200 USD sent
                }],
                note: None,
                idempotency_key: unique("t_branch"),
            },
            at(800),
        )
        .await
        .unwrap();

    flow.submit(&creator_b1, &t_branch.id, at(810))
        .await
        .unwrap();
    flow.decide(&manager_b1, &t_branch.id, Outcome::Approved, "ok", at(820))
        .await
        .unwrap();
    flow.dispatch(&creator_b1, &t_branch.id, at(830))
        .await
        .unwrap();

    // Destination B2 receives 3 notes (short 1 note = -50 USD = -5000 minor)
    let recv_time_safe = at(840);
    flow.receive(
        &receiver_b2,
        &t_branch.id,
        vec![NewTransferLine {
            currency: code("USD"),
            denomination_id: fifty_denom,
            qty: 3,
        }],
        recv_time_safe,
    )
    .await
    .unwrap();

    let rows_b2 = sqlx::query("SELECT id, data FROM cash_exceptions WHERE branch_id = $1")
        .bind(b2.as_str())
        .fetch_all(&db.pool)
        .await
        .unwrap();

    assert_eq!(rows_b2.len(), 1, "exactly one exception on branch B2");
    let exc_safe = exception_from_row(&rows_b2[0]).expect("decodes transfer exception");
    assert_eq!(exc_safe.status, ExceptionStatus::Open);
    assert_eq!(
        exc_safe.request_id, t_branch.id,
        "request_id is the transfer id"
    );
    assert_eq!(exc_safe.note, format!("TRANSFER {}", t_branch.id));
    assert_eq!(
        exc_safe.at.epoch_millis(),
        recv_time_safe.timestamp_millis()
    );
    assert!(
        exc_safe.cash_variance.is_none(),
        "cash_variance must be None when destination is Safe"
    );
    let sv = exc_safe
        .safe_variance
        .expect("safe_variance must be filled when destination is Safe");
    assert_eq!(sv.get(code("USD")).minor(), -5000);

    // Subcase 7B: SAFE_TO_DRAWER transfer -> destination is Drawer (to_shift is Some) -> fills cash_variance
    let teller_b1 = db.person("teller_b1", &[(Role::Teller, Some(&b1))]).await;
    let shift_id = open_shift_with(&db, &b1, &teller_b1, &manager_b1, 0, 1, at(850)).await;

    let t_drawer = flow
        .create(
            &creator_b1,
            NewTransfer {
                kind: TransferKind::SafeToDrawer,
                from_branch: b1.clone(),
                to_branch: None,
                from_shift: None,
                to_shift: Some(shift_id),
                lines: vec![NewTransferLine {
                    currency: code("USD"),
                    denomination_id: fifty_denom,
                    qty: 4, // 4 * 50 = 200 USD sent from safe to drawer
                }],
                note: None,
                idempotency_key: unique("t_safe_to_drawer"),
            },
            at(860),
        )
        .await
        .unwrap();

    flow.submit(&creator_b1, &t_drawer.id, at(870))
        .await
        .unwrap();
    flow.decide(&manager_b1, &t_drawer.id, Outcome::Approved, "ok", at(880))
        .await
        .unwrap();
    flow.dispatch(&creator_b1, &t_drawer.id, at(890))
        .await
        .unwrap();

    // Drawer receives 3 notes (short 1 note = -50 USD = -5000 minor)
    let recv_time_drawer = at(900);
    flow.receive(
        &teller_b1,
        &t_drawer.id,
        vec![NewTransferLine {
            currency: code("USD"),
            denomination_id: fifty_denom,
            qty: 3,
        }],
        recv_time_drawer,
    )
    .await
    .unwrap();

    let rows_b1 = sqlx::query("SELECT id, data FROM cash_exceptions WHERE branch_id = $1")
        .bind(b1.as_str())
        .fetch_all(&db.pool)
        .await
        .unwrap();

    assert_eq!(rows_b1.len(), 1, "exactly one exception on branch B1");
    let exc_drawer = exception_from_row(&rows_b1[0]).expect("decodes drawer transfer exception");
    assert_eq!(exc_drawer.status, ExceptionStatus::Open);
    assert_eq!(
        exc_drawer.request_id, t_drawer.id,
        "request_id is the transfer id"
    );
    assert_eq!(exc_drawer.note, format!("TRANSFER {}", t_drawer.id));
    assert_eq!(
        exc_drawer.at.epoch_millis(),
        recv_time_drawer.timestamp_millis()
    );
    assert!(
        exc_drawer.safe_variance.is_none(),
        "safe_variance must be None when destination is Drawer"
    );
    let cv = exc_drawer
        .cash_variance
        .expect("cash_variance must be filled when destination is Drawer");
    assert_eq!(cv.get(code("USD")).minor(), -5000);
}
