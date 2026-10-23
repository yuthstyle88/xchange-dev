//! Specification tests for `HandoverService::start` / `start_in` denomination and currency lookups (#1935).

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverService, StartHandover};
use xc_cash::domain::count::{CountKind, CountLine};
use xc_cash::domain::error::CashError;
use xc_cash::domain::handover::HandoverStatus;
use xc_kernel::ids::BranchId;

const ORG: &str = "00000000-0000-0000-0000-000000000001";

static LG4_FUND: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
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
        let schema = unique("ho1935");
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

    async fn handover_rows(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM handovers")
            .fetch_one(&self.pool)
            .await
            .expect("count handovers")
    }
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

    let lines = vec![
        CountLine::new("THB", db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", db.denomination("THB", "100").await, 3),
        CountLine::new("USD", db.denomination("USD", "100").await, 3),
        CountLine::new("JPY", db.denomination("JPY", "10000").await, 2),
    ];
    let counts = db.counts();
    let open = counts
        .submit(
            &a,
            SubmitCount {
                branch: main.clone(),
                shift_id: None,
                kind: CountKind::Open,
                lines,
                note: None,
                idempotency_key: unique("open"),
            },
            at(0),
        )
        .await
        .expect("submit OPEN count");
    let a_shift = counts
        .verify(&manager, &open.id, at(60))
        .await
        .expect("verify OPEN count")
        .shift_id
        .expect("shift id");

    Floor {
        db,
        main,
        a,
        b,
        a_shift,
    }
}

fn start_cmd(f: &Floor, lines: Vec<CountLine>) -> StartHandover {
    StartHandover {
        branch: f.main.clone(),
        from_shift_id: f.a_shift.clone(),
        receiver_id: f.b.user_id.clone(),
        lines,
        note: None,
        idempotency_key: unique("handover"),
    }
}

/// §9 row 1: a valid multi-line, multi-currency start writes the handover and count totals.
#[tokio::test]
async fn a_valid_multi_line_multi_currency_start_writes_the_handover_and_count() {
    let f = floor().await;
    let lines = vec![
        CountLine::new("THB", f.db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", f.db.denomination("THB", "100").await, 3),
        CountLine::new("USD", f.db.denomination("USD", "100").await, 2),
        CountLine::new("USD", f.db.denomination("USD", "50").await, 2),
        CountLine::new("JPY", f.db.denomination("JPY", "10000").await, 2),
    ];
    let h =
        f.db.handovers()
            .start(&f.a, start_cmd(&f, lines), at(600))
            .await
            .expect("multi-line, multi-currency handover start succeeds");

    assert_eq!(h.status, HandoverStatus::Pending);
    let totals: Vec<(String, Decimal, Option<Decimal>)> = sqlx::query_as(
        "SELECT trim(currency), counted, expected FROM cash_count_totals \
         WHERE count_id = $1::uuid ORDER BY currency",
    )
    .bind(&h.count_id)
    .fetch_all(&f.db.pool)
    .await
    .expect("read count totals");
    assert_eq!(
        totals,
        vec![
            ("JPY".to_string(), d("20000"), Some(d("20000"))),
            ("THB".to_string(), d("2300"), Some(d("2300"))),
            ("USD".to_string(), d("300"), Some(d("300"))),
        ]
    );
    f.db.drop_schema().await;
}

/// §9 row 2: a line with an unknown denomination id answers `NotFound { entity: "denomination", id }`
/// naming the first unknown id in line order.
#[tokio::test]
async fn a_line_with_an_unknown_denomination_id_is_not_found() {
    let f = floor().await;
    let thb_1000 = f.db.denomination("THB", "1000").await;
    let lines = vec![
        CountLine::new("THB", thb_1000, 2),
        CountLine::new("THB", 999_999_001, 1),
        CountLine::new("THB", 999_999_002, 1),
    ];
    let res =
        f.db.handovers()
            .start(&f.a, start_cmd(&f, lines), at(600))
            .await;
    match res {
        Err(AppError::NotFound { entity, id }) => {
            assert_eq!(entity, "denomination");
            assert_eq!(id, "999999001");
        }
        other => panic!("expected AppError::NotFound for 999999001, got {other:?}"),
    }
    assert_eq!(f.db.handover_rows().await, 0);
    f.db.drop_schema().await;
}

/// §9 row 3: a line whose currency differs from its denomination's answers `Validation` with the same text.
#[tokio::test]
async fn a_line_whose_currency_differs_from_its_denominations_is_refused() {
    let f = floor().await;
    let thb_1000 = f.db.denomination("THB", "1000").await;
    let lines = vec![CountLine::new("USD", thb_1000, 2)];
    let res =
        f.db.handovers()
            .start(&f.a, start_cmd(&f, lines), at(600))
            .await;
    match res {
        Err(AppError::Validation(msg)) => {
            assert_eq!(
                msg,
                "line currency 'USD' does not match denomination currency 'THB'"
            );
        }
        other => panic!("expected AppError::Validation for currency mismatch, got {other:?}"),
    }
    assert_eq!(f.db.handover_rows().await, 0);
    f.db.drop_schema().await;
}

/// §9 row 4: a negative quantity answers `Validation` before any denomination lookup.
#[tokio::test]
async fn a_negative_quantity_is_refused_before_any_lookup() {
    let f = floor().await;
    // Denomination 999_999_999 does not exist, yet the negative qty check runs before looking it up.
    let lines = vec![CountLine::new("THB", 999_999_999, -3)];
    let res =
        f.db.handovers()
            .start(&f.a, start_cmd(&f, lines), at(600))
            .await;
    match res {
        Err(AppError::Validation(msg)) => {
            assert_eq!(msg, "line qty cannot be negative: -3");
        }
        other => panic!("expected AppError::Validation for negative qty, got {other:?}"),
    }
    assert_eq!(f.db.handover_rows().await, 0);
    f.db.drop_schema().await;
}

/// §9 row 5: a count with more decimals than the currency allows answers `TooManyDecimals`.
#[tokio::test]
async fn a_count_with_more_decimals_than_the_currency_allows_is_refused() {
    let f = floor().await;
    // JPY has minor_unit = 0, so a counted total with 1 decimal place (0.5 JPY) exceeds max_scale = 0.
    let jpy_half: i64 = sqlx::query_scalar(
        "INSERT INTO denominations (currency_code, face_value, group_code) \
         VALUES ('JPY', 0.5, 'TEST') RETURNING id",
    )
    .fetch_one(&f.db.pool)
    .await
    .expect("insert 0.5 JPY denomination");

    let lines = vec![
        CountLine::new("THB", f.db.denomination("THB", "1000").await, 2),
        CountLine::new("JPY", jpy_half, 1),
    ];
    let res =
        f.db.handovers()
            .start(&f.a, start_cmd(&f, lines), at(600))
            .await;
    match res {
        Err(AppError::Domain(msg)) => {
            assert_eq!(msg, CashError::TooManyDecimals.to_string());
        }
        other => panic!("expected AppError::Domain(TooManyDecimals), got {other:?}"),
    }
    assert_eq!(f.db.handover_rows().await, 0);
    f.db.drop_schema().await;
}

/// Proves `start_in` batches denomination and currency lookups into a single query each (`id = ANY($1)` /
/// `code = ANY($1)`) instead of N per-line and M per-currency queries inside the loops.
#[tokio::test]
async fn start_in_batches_denomination_and_currency_lookups_into_one_query_each() {
    let f = floor().await;
    let lines = vec![
        CountLine::new("THB", f.db.denomination("THB", "1000").await, 2),
        CountLine::new("THB", f.db.denomination("THB", "100").await, 3),
        CountLine::new("USD", f.db.denomination("USD", "100").await, 2),
        CountLine::new("USD", f.db.denomination("USD", "50").await, 2),
        CountLine::new("JPY", f.db.denomination("JPY", "10000").await, 2),
    ];

    // Drop PK indexes (and incoming FKs) on denominations and currencies in this isolated schema
    // so every SELECT against denominations/currencies in start_in performs exactly 1 seq_scan
    // and insert_count performs 0 FK check scans on them.
    sqlx::query("ALTER TABLE denominations DROP CONSTRAINT denominations_pkey CASCADE")
        .execute(&f.db.pool)
        .await
        .expect("drop denominations_pkey");
    sqlx::query("ALTER TABLE currencies DROP CONSTRAINT currencies_pkey CASCADE")
        .execute(&f.db.pool)
        .await
        .expect("drop currencies_pkey");

    let mut tx = f.db.pool.begin().await.expect("begin tx");
    let denom_before: i64 = sqlx::query_scalar(
        "SELECT COALESCE(seq_scan, 0) FROM pg_stat_xact_user_tables \
         WHERE schemaname = current_schema() AND relname = 'denominations'",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("read denominations seq_scan before");
    let currency_before: i64 = sqlx::query_scalar(
        "SELECT COALESCE(seq_scan, 0) FROM pg_stat_xact_user_tables \
         WHERE schemaname = current_schema() AND relname = 'currencies'",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("read currencies seq_scan before");

    let h =
        f.db.handovers()
            .start_in(&mut tx, &f.a, start_cmd(&f, lines), at(600))
            .await
            .expect("start_in succeeds");
    assert_eq!(h.status, HandoverStatus::Pending);

    let denom_after: i64 = sqlx::query_scalar(
        "SELECT COALESCE(seq_scan, 0) FROM pg_stat_xact_user_tables \
         WHERE schemaname = current_schema() AND relname = 'denominations'",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("read denominations seq_scan after");
    let currency_after: i64 = sqlx::query_scalar(
        "SELECT COALESCE(seq_scan, 0) FROM pg_stat_xact_user_tables \
         WHERE schemaname = current_schema() AND relname = 'currencies'",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("read currencies seq_scan after");
    tx.commit().await.expect("commit tx");

    let denom_scans = denom_after - denom_before;
    let currency_scans = currency_after - currency_before;
    assert_eq!(
        denom_scans, 1,
        "expected 1 batched query on denominations for 5 lines, got {denom_scans}"
    );
    assert!(
        currency_scans <= 2,
        "expected the batched register lookup plus the ledger read's register join (at most 2 scans), not one per currency; got {currency_scans}"
    );
    f.db.drop_schema().await;
}
