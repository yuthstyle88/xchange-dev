//! Ticket #929 (LG-4) — `CashPlace::Variance(branch)` through the port (design
//! `docs/design/blueprint/ledger-inventory.md` §4.4 rows 6 and 7; lane controller's ruling on #929: commit 3 adds the
//! place). Written before the implementation by the ticket author (claude/89bd61d9, rule A). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! This file does not compile on `main` (the place does not exist yet). That is intended: the behaviour cannot be
//! reached without the variant.
//!
//! What this test pins ([required]):
//! - `DRAWER → VARIANCE` of a foreign currency is an inventory `ADJUST` of −quantity at the position's average —
//!   never a `DISPOSE` (no sale happened).
//! - `VARIANCE → DRAWER` of a foreign currency is an `ADJUST` of +quantity at the average — never an `ACQUIRE` (no
//!   purchase happened, so nothing is costed at a new price).
//! - `expected_in(VARIANCE(branch))` reads what was posted there, per branch.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

fn src(kind: SourceType) -> SourceRef {
    SourceRef {
        source_type: kind,
        source_id: unique("src"),
        source_key: "lg4-variance".to_string(),
    }
}

fn req(
    from: CashPlace,
    to: CashPlace,
    currency: CurrencyCode,
    qty: &str,
    source: &SourceRef,
) -> PostingRequest {
    PostingRequest {
        from,
        to,
        currency,
        quantity: ForeignAmount::from_decimal(d(qty)),
        source: source.clone(),
        line: None,
        reverses: None,
    }
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
        let schema = unique("lg4v");
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

    async fn post(&self, reqs: Vec<PostingRequest>) -> Result<(), String> {
        let mut tx = self.pool.begin().await.unwrap();
        LedgerCashPostings::new()
            .post_in(&mut tx, &reqs, None)
            .await
            .map_err(|e| format!("{e:?}"))?;
        tx.commit().await.unwrap();
        Ok(())
    }

    /// A branch whose SAFE holds USD 300 bought at 32.10 (cost 9,630.00), and a shift `SH-<branch>` whose DRAWER
    /// holds USD 100 of it (a real `shifts` row: the port checks the drawer's shift exists).
    async fn branch(&self, label: &str) -> (BranchId, String) {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .unwrap();
        let b = BranchId::new(id.clone());
        let buy = src(SourceType::Exchange);
        self.post(vec![
            req(
                CashPlace::Customer,
                CashPlace::Safe(b.clone()),
                usd(),
                "300.00",
                &buy,
            ),
            req(
                CashPlace::Bank,
                CashPlace::Customer,
                CurrencyCode::THB,
                "9630.00",
                &buy,
            ),
        ])
        .await
        .expect("seed the safe");
        let teller: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ('00000000-0000-0000-0000-000000000001'::uuid, 'Teller', 'ACTIVE', 'test') RETURNING id::text",
        )
        .fetch_one(&self.pool)
        .await
        .expect("a teller");
        let shift = format!("SH-{id}");
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', now(), 'system', $3, $4::uuid)",
        )
        .bind(&shift)
        .bind(&id)
        .bind(unique("open-req"))
        .bind(&teller)
        .execute(&self.pool)
        .await
        .expect("an open shift");
        self.post(vec![req(
            CashPlace::Safe(b.clone()),
            CashPlace::Drawer(shift.clone()),
            usd(),
            "100.00",
            &src(SourceType::Opening),
        )])
        .await
        .expect("seed the drawer");
        (b, shift)
    }

    async fn at(&self, place: CashPlace) -> Decimal {
        let mut tx = self.pool.begin().await.unwrap();
        let b = LedgerCashPostings::new()
            .expected_in(&mut tx, &place)
            .await
            .expect("expected_in");
        tx.rollback().await.unwrap();
        Decimal::new(b.get(usd()).minor(), 2)
    }

    /// (movement type, Σ quantity, Σ cost) of the inventory movements of the postings named by `source`.
    async fn movements(&self, source: &SourceRef) -> Vec<(String, Decimal, Decimal)> {
        sqlx::query_as(
            "SELECT m.movement_type, sum(m.quantity_signed), sum(m.cost_thb_signed) \
               FROM inventory_movements m JOIN ledger_postings p ON p.id = m.posting_id \
              WHERE p.source_id = $1 GROUP BY m.movement_type ORDER BY 1",
        )
        .bind(&source.source_id)
        .fetch_all(&self.pool)
        .await
        .expect("read movements")
    }

    async fn position(&self, branch: &BranchId) -> (Decimal, Decimal) {
        sqlx::query_as(
            "SELECT quantity, cost_thb FROM inventory_positions WHERE branch_id = $1 AND trim(currency) = 'USD'",
        )
        .bind(branch.as_str())
        .fetch_one(&self.pool)
        .await
        .expect("the USD position")
    }
}

#[tokio::test]
async fn a_shortfall_to_variance_adjusts_at_the_average_and_disposes_nothing() {
    let db = Db::fresh().await;
    let (main, shift) = db.branch("main").await;
    let short = src(SourceType::Count);
    db.post(vec![req(
        CashPlace::Drawer(shift.clone()),
        CashPlace::Variance(main.clone()),
        usd(),
        "10.00",
        &short,
    )])
    .await
    .expect("a shortfall posts");
    assert_eq!(
        db.movements(&short).await,
        vec![("ADJUST".to_owned(), d("-10"), d("-321.00"))],
        "one ADJUST of −10 at the average 32.10; no DISPOSE"
    );
    assert_eq!(db.at(CashPlace::Variance(main.clone())).await, d("10.00"));
    assert_eq!(db.at(CashPlace::Drawer(shift)).await, d("90.00"));
    assert_eq!(db.position(&main).await, (d("290"), d("9309.00")));
}

#[tokio::test]
async fn an_overage_from_variance_adjusts_at_the_average_and_acquires_nothing() {
    let db = Db::fresh().await;
    let (main, shift) = db.branch("main").await;
    let over = src(SourceType::Count);
    db.post(vec![req(
        CashPlace::Variance(main.clone()),
        CashPlace::Drawer(shift.clone()),
        usd(),
        "5.00",
        &over,
    )])
    .await
    .expect("an overage posts");
    assert_eq!(
        db.movements(&over).await,
        vec![("ADJUST".to_owned(), d("5"), d("160.50"))],
        "one ADJUST of +5 at the average 32.10; no ACQUIRE"
    );
    let variance: Decimal = sqlx::query_scalar(
        "SELECT sum(e.quantity_signed) FROM ledger_entries e JOIN cash_accounts a ON a.id = e.account_id \
         WHERE a.kind = 'VARIANCE' AND a.branch_id = $1 AND trim(a.currency) = 'USD'",
    )
    .bind(main.as_str())
    .fetch_one(&db.pool)
    .await
    .expect("VARIANCE's entries");
    assert_eq!(
        variance,
        d("-5"),
        "VARIANCE gave the 5 (SQL: a place's negative balance is not the port's to show)"
    );
    assert_eq!(db.at(CashPlace::Drawer(shift)).await, d("105.00"));
    assert_eq!(db.position(&main).await, (d("305"), d("9790.50")));
}

#[tokio::test]
async fn each_branch_has_its_own_variance() {
    let db = Db::fresh().await;
    let (a, a_shift) = db.branch("a").await;
    let (b, _) = db.branch("b").await;
    db.post(vec![req(
        CashPlace::Drawer(a_shift),
        CashPlace::Variance(a.clone()),
        usd(),
        "10.00",
        &src(SourceType::Count),
    )])
    .await
    .expect("a shortfall at A");
    assert_eq!(db.at(CashPlace::Variance(a)).await, d("10.00"));
    assert_eq!(
        db.at(CashPlace::Variance(b)).await,
        d("0.00"),
        "B's variance is untouched"
    );
}
