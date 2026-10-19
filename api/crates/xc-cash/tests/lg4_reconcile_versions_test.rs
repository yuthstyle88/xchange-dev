//! Ticket #929 (LG-4) — a reconciled BRANCH transfer moves the **sender's** version too (L-5): the reconcile posts the
//! remainder IN_TRANSIT → the sender's VARIANCE and adjusts the sender's position, so the sender's figures change.
//! Added by the implementer (claude/dell-claude-2) on claude/07d3765f's FAIL at `eda170c9`
//! (PR 1431 issuecomment-6116839590). The harness below is copied from `lg4_cutover_transfers_test.rs` (provided,
//! unchanged); only `a_reconciled_transfer_moves_the_senders_version` is new.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.
#![allow(dead_code, unused_imports)]

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_approval::domain::request::Outcome;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::domain::transfer::cash_transfer::{
    CashTransfer, NewTransfer, NewTransferLine, TransferKind, TransferStatus,
};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};

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

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn api_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

// ── the database ────────────────────────────────────────────────────────────

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
        let schema = unique("lg4t");
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
            .max_connections(6)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        Migrator::new(api_dir().join("migrations").as_path())
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

    /// A branch whose safe holds USD `usd` (in both stores), bought at 32.10, and THB 100,000.00 in the ledger.
    async fn branch(&self, label: &str, amount: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .unwrap();
        let b = BranchId::new(id);
        let src = |key: &str| SourceRef {
            source_type: SourceType::Opening,
            source_id: unique("seed"),
            source_key: key.to_string(),
        };
        self.post(vec![PostingRequest {
            from: CashPlace::Bank,
            to: CashPlace::Safe(b.clone()),
            currency: CurrencyCode::THB,
            quantity: ForeignAmount::from_decimal(d("100000.00")),
            source: src("thb"),
            line: None,
            reverses: None,
        }])
        .await;
        if d(amount) > Decimal::ZERO {
            let s = src("usd");
            self.post(vec![
                PostingRequest {
                    from: CashPlace::Customer,
                    to: CashPlace::Safe(b.clone()),
                    currency: usd(),
                    quantity: ForeignAmount::from_decimal(d(amount)),
                    source: s.clone(),
                    line: None,
                    reverses: None,
                },
                PostingRequest {
                    from: CashPlace::Bank,
                    to: CashPlace::Customer,
                    currency: CurrencyCode::THB,
                    quantity: ForeignAmount::from_decimal(d(amount) * d("32.10")),
                    source: s,
                    line: None,
                    reverses: None,
                },
            ])
            .await;
        }
        b
    }

    async fn person(&self, name: &str, roles: &[(Role, Option<&BranchId>)]) -> Actor {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        for (role, branch) in roles {
            sqlx::query(
                "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id) VALUES ($1::uuid, $2, $3::uuid, $4)",
            )
            .bind(&id)
            .bind(role.as_str())
            .bind(ORG)
            .bind(branch.map(|b| b.as_str().to_owned()))
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
                .map(|(r, b)| ScopedRole::new(*r, b.cloned(), from, None))
                .collect(),
            None,
        )
    }

    async fn fifty(&self) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = 'USD' AND face_value = 50::numeric",
        )
        .fetch_one(&self.pool)
        .await
        .expect("the USD 50 face value")
    }

    async fn version(&self, branch: &BranchId) -> i64 {
        sqlx::query_scalar(
            "SELECT coalesce((SELECT version FROM branch_versions WHERE branch_id = $1), 0)",
        )
        .bind(branch.as_str())
        .fetch_one(&self.pool)
        .await
        .expect("read the branch version")
    }

    /// A place's USD through the port.
    async fn usd_at(&self, place: CashPlace) -> Decimal {
        let mut tx = self.pool.begin().await.unwrap();
        let b = LedgerCashPostings::new()
            .expected_in(&mut tx, &place)
            .await
            .expect("expected_in");
        tx.rollback().await.unwrap();
        Decimal::new(b.get(usd()).try_minor().unwrap(), 2)
    }

    async fn scalar_dec(&self, sql: &str) -> Decimal {
        sqlx::query_scalar::<_, Option<Decimal>>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .unwrap_or_default()
    }

    async fn scalar(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    fn flow(&self) -> TransferFlow {
        TransferFlow::pg(self.pool.clone())
    }
}

/// The people of a BRANCH transfer between two branches.
struct Cast {
    finance: Actor,
    exec: Actor,
}

async fn cast(db: &Db) -> Cast {
    Cast {
        finance: db.person("Pim", &[(Role::Finance, None)]).await,
        exec: db.person("Exec", &[(Role::Executive, None)]).await,
    }
}

/// A BRANCH transfer of `notes` × USD 50 from `from` to `to`, created, approved by the EXECUTIVE and dispatched.
async fn dispatched(db: &Db, c: &Cast, from: &BranchId, to: &BranchId, notes: i32) -> CashTransfer {
    let flow = db.flow();
    let creator = db
        .person(&unique("creator"), &[(Role::Finance, Some(from))])
        .await;
    let t = flow
        .create(
            &creator,
            NewTransfer {
                kind: TransferKind::Branch,
                from_branch: from.clone(),
                to_branch: Some(to.clone()),
                from_shift: None,
                to_shift: None,
                lines: vec![NewTransferLine {
                    currency: usd(),
                    denomination_id: db.fifty().await,
                    qty: notes,
                }],
                note: None,
                idempotency_key: unique("t"),
            },
            at(0),
        )
        .await
        .expect("create");
    flow.submit(&creator, &t.id, at(5)).await.expect("submit");
    flow.decide(&c.exec, &t.id, Outcome::Approved, "ok", at(10))
        .await
        .expect("the EXECUTIVE approves");
    flow.dispatch(&creator, &t.id, at(15))
        .await
        .expect("dispatch")
}

async fn receiver(db: &Db, at_branch: &BranchId) -> Actor {
    db.person(
        &unique("receiver"),
        &[(Role::BranchManager, Some(at_branch))],
    )
    .await
}

async fn receive(db: &Db, who: &Actor, t: &CashTransfer, notes: i32) -> CashTransfer {
    db.flow()
        .receive(
            who,
            &t.id,
            vec![NewTransferLine {
                currency: usd(),
                denomination_id: db.fifty().await,
                qty: notes,
            }],
            at(20),
        )
        .await
        .expect("receive")
}

#[tokio::test]
async fn a_reconciled_transfer_moves_the_senders_version() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    let b1 = db.branch("b1", "500.00").await;
    let b2 = db.branch("b2", "0").await;
    let t = dispatched(&db, &c, &b1, &b2, 4).await;
    let who = receiver(&db, &b2).await;
    receive(&db, &who, &t, 3).await; // USD 150 received of 200 sent: 50 left to reconcile
    let (v1, v2) = (db.version(&b1).await, db.version(&b2).await);
    db.flow()
        .reconcile(&c.finance, &t.id, "fifty short", at(30))
        .await
        .expect("FINANCE reconciles");
    assert_eq!(
        db.version(&b1).await,
        v1 + 1,
        "the sender's version moves: its IN_TRANSIT, VARIANCE and position changed (L-5)"
    );
    assert_eq!(
        db.version(&b2).await,
        v2 + 1,
        "the receiver's version moves"
    );
}
