//! Ticket #929 (LG-4) — a two-currency hand-over is one `OPENING` posting with an entry pair per currency (design
//! `docs/design/blueprint/ledger-inventory.md` §4.4 row 2; lane controller's option A, issuecomment-6115055843, and
//! its release note on #929: the up-front tests pin the hand-over with one currency only, so the implementer adds
//! this one).
//!
//! What this test pins: a hand-over counting THB and USD, once accepted, is **one** `OPENING` posting named by the
//! hand-over, whose entries are one pair (`DRAWER(from)` −, `DRAWER(to)` +) per currency; the new drawer holds what
//! was counted in each currency, and exactly that left the giver's drawer.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::count::{CashCountService, SubmitCount};
use xc_cash::application::handover::{HandoverService, StartHandover};
use xc_cash::domain::count::{CountKind, CountLine};
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
    Utc.with_ymd_and_hms(2026, 10, 12, 1, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
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
        let schema = unique("lg4h");
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

    async fn post(&self, reqs: Vec<PostingRequest>) {
        let mut tx = self.pool.begin().await.unwrap();
        LedgerCashPostings::new()
            .post_in(&mut tx, &reqs, None)
            .await
            .unwrap_or_else(|e| panic!("seed posting: {e:?}"));
        tx.commit().await.unwrap();
    }

    fn request(
        from: CashPlace,
        to: CashPlace,
        currency: &str,
        qty: &str,
        source: &SourceRef,
    ) -> PostingRequest {
        PostingRequest {
            from,
            to,
            currency: CurrencyCode::parse(currency).unwrap(),
            quantity: ForeignAmount::from_decimal(d(qty)),
            source: source.clone(),
            line: None,
            reverses: None,
        }
    }

    /// A branch whose SAFE holds THB 10,000 and USD 100 bought at 32.10.
    async fn branch(&self) -> BranchId {
        let id = unique("main");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .unwrap();
        let b = BranchId::new(id);
        let src = |key: &str, kind: SourceType| SourceRef {
            source_type: kind,
            source_id: unique("seed"),
            source_key: key.to_string(),
        };
        let thb = src("thb", SourceType::Opening);
        self.post(vec![Self::request(
            CashPlace::Bank,
            CashPlace::Safe(b.clone()),
            "THB",
            "10000.00",
            &thb,
        )])
        .await;
        let usd = src("usd", SourceType::Opening);
        self.post(vec![
            Self::request(
                CashPlace::Customer,
                CashPlace::Safe(b.clone()),
                "USD",
                "100.00",
                &usd,
            ),
            Self::request(CashPlace::Bank, CashPlace::Customer, "THB", "3210.00", &usd),
        ])
        .await;
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

    async fn at(&self, place: CashPlace, currency: &str) -> Decimal {
        let mut tx = self.pool.begin().await.unwrap();
        let b = LedgerCashPostings::new()
            .expected_in(&mut tx, &place)
            .await
            .expect("expected_in");
        tx.rollback().await.unwrap();
        Decimal::new(
            b.get(CurrencyCode::parse(currency).unwrap())
                .try_minor()
                .unwrap(),
            2,
        )
    }
}

#[tokio::test]
async fn a_two_currency_handover_is_one_opening_posting_with_an_entry_pair_per_currency() {
    let db = Db::fresh().await;
    let main = db.branch().await;
    let a = db.person("Ploy", &[Role::Teller]).await;
    let b = db.person("Nok", &[Role::Teller]).await;
    let manager = db.person("Krit", &[Role::BranchManager]).await;

    // A opens with THB 2,300 and USD 100.
    let counts = CashCountService::pg(db.pool.clone());
    let open = counts
        .submit(
            &a,
            SubmitCount {
                branch: main.clone(),
                shift_id: None,
                kind: CountKind::Open,
                lines: vec![
                    CountLine::new("THB", db.face("THB", "1000").await, 2),
                    CountLine::new("THB", db.face("THB", "100").await, 3),
                    CountLine::new("USD", db.face("USD", "100").await, 1),
                ],
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
        .expect("the BRANCH_MANAGER verifies")
        .shift_id
        .expect("A's shift");

    // A hands THB 2,200 and USD 100 to B.
    let handovers = HandoverService::pg(db.pool.clone());
    let h = handovers
        .start(
            &a,
            StartHandover {
                branch: main.clone(),
                from_shift_id: a_shift.clone(),
                receiver_id: b.user_id.clone(),
                lines: vec![
                    CountLine::new("THB", db.face("THB", "1000").await, 2),
                    CountLine::new("THB", db.face("THB", "100").await, 2),
                    CountLine::new("USD", db.face("USD", "100").await, 1),
                ],
                note: None,
                idempotency_key: unique("handover"),
            },
            at(600),
        )
        .await
        .expect("A starts the hand-over");
    let h = handovers
        .accept(&b, &h.id, at(700))
        .await
        .expect("B accepts");
    let b_shift: String =
        sqlx::query_scalar("SELECT id FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'")
            .bind(&b.user_id)
            .fetch_one(&db.pool)
            .await
            .expect("B's new shift");

    assert_eq!(
        db.at(CashPlace::Drawer(b_shift.clone()), "THB").await,
        d("2200.00")
    );
    assert_eq!(db.at(CashPlace::Drawer(b_shift), "USD").await, d("100.00"));
    assert_eq!(
        db.at(CashPlace::Drawer(a_shift.clone()), "THB").await,
        d("100.00"),
        "exactly THB 2,200 left A's drawer"
    );
    assert_eq!(
        db.at(CashPlace::Drawer(a_shift), "USD").await,
        d("0.00"),
        "all of A's USD went to B"
    );
    let postings: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ledger_postings WHERE source_type = 'OPENING' AND source_id = $1",
    )
    .bind(&h.id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(postings, 1, "one OPENING posting for the whole hand-over");
    let pairs: Vec<(String, i64)> = sqlx::query_as(
        "SELECT trim(e.currency), count(*) FROM ledger_entries e JOIN ledger_postings p ON p.id = e.posting_id \
         WHERE p.source_type = 'OPENING' AND p.source_id = $1 GROUP BY 1 ORDER BY 1",
    )
    .bind(&h.id)
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        pairs,
        vec![("THB".to_owned(), 2), ("USD".to_owned(), 2)],
        "one entry pair per currency"
    );
}
