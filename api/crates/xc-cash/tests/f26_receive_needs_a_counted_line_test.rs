//! F26 (#564 issuecomment-6657891388, customer simulation on xc-v3 at 7caf1e7, 2026-11-05): a receive saved with no
//! line (`lines: []`, transfer `#95782d60`) answered 200 and marked the transfer RECEIVED, moved nothing into the
//! drawer, and left USD 600 / THB 3,000 in IN_TRANSIT. `receive_on` (`transfer_flow.rs`) reads a missing line as 0
//! (`with_counted_lines`), so "nothing was counted" and "everything was lost" are the same request.
//!
//! Written before the implementation by the ticket's author (claude/33691c06, sub-controller of the xchange lane
//! controller claude/022a7453). Do not edit, delete or weaken these tests; add your own in another file.
//!
//! [required] by these tests: a receive with no counted line is refused `AppError::Validation` before anything is
//! read under lock or written, the way a count with no line is (`count_service.rs`: "a count needs at least one
//! line; count an empty drawer as 0"); the transfer stays DISPATCHED and nothing posts. A receive that states a
//! line as 0 is still a real count (RECEIVED, short), unchanged.
//!
//! The harness is a trimmed copy of `transfer_reconcile_at_sender_test.rs`'s (that file is not edited). Needs
//! `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_approval::domain::request::Outcome;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::transfer::cash_transfer::{
    CashTransfer, NewTransfer, NewTransferLine, TransferKind, TransferStatus,
};
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xc_testdb::people::{RoleRow, seed_person};

const ORG: &str = "00000000-0000-0000-0000-000000000001";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 3, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
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
        let schema = unique("f26");
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
            .expect("every migration applies in version order on a fresh database");
        Db { pool }
    }

    async fn branch(&self, label: &str, safe_usd_minor: i64) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(&id)
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert ops_state");
        // LG-4 (#929, 1:1): the same USD amount is the safe's figure in the ledger, which now holds it.
        if safe_usd_minor > 0 {
            lg4_fund_safe(
                &self.pool,
                &id,
                "USD",
                &rust_decimal::Decimal::new(safe_usd_minor, 2).to_string(),
            )
            .await;
        }
        BranchId::new(id)
    }

    /// A user holding `roles`, each on a branch (`None` = organisation-wide), and their `Actor`.
    async fn person(&self, name: &str, roles: &[(Role, Option<&BranchId>)]) -> Actor {
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let role_rows: Vec<RoleRow> = roles
            .iter()
            .map(|(role, branch)| RoleRow {
                role_code: role.as_str(),
                branch_id: branch.map(|b| b.as_str()),
                valid_from: from,
                valid_to: None,
            })
            .collect();
        let mut conn = self.pool.acquire().await.expect("acquire connection");
        let id = seed_person(&mut conn, ORG, name, &role_rows)
            .await
            .expect("seed person");
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

    async fn usd(&self, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = 'USD' AND face_value = $1::numeric",
        )
        .bind(face)
        .fetch_one(&self.pool)
        .await
        .unwrap_or_else(|e| panic!("the USD {face} face value (A7): {e}"))
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

    async fn status(&self, id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM cash_transfers WHERE id = $1::uuid")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .expect("read the transfer")
    }

    fn flow(&self) -> TransferFlow {
        TransferFlow::pg(self.pool.clone())
    }
}

static LG4_FUND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
                        LG4_FUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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

/// A BRANCH transfer of 4 × USD 50 from B1 to B2, created, approved and dispatched, so it sits in IN_TRANSIT.
struct Dispatched {
    receiver: Actor,
    fifty: i64,
    t: CashTransfer,
}

async fn dispatched(db: &Db) -> Dispatched {
    let b1 = db.branch("b1", 50_000).await;
    let b2 = db.branch("b2", 0).await;
    let creator = db.person("Fon", &[(Role::Finance, Some(&b1))]).await;
    let manager_b1 = db.person("Krit", &[(Role::BranchManager, Some(&b1))]).await;
    let exec = db.person("Exec", &[(Role::Executive, None)]).await;
    let receiver = db.person("Nida", &[(Role::BranchManager, Some(&b2))]).await;
    db.limit_usd(&exec, "300.00").await;
    let flow = db.flow();
    let fifty = db.usd("50").await;
    let t = flow
        .create(
            &creator,
            NewTransfer {
                kind: TransferKind::Branch,
                from_branch: b1.clone(),
                to_branch: Some(b2.clone()),
                from_shift: None,
                to_shift: None,
                lines: vec![NewTransferLine {
                    currency: usd(),
                    denomination_id: fifty,
                    qty: 4,
                }],
                note: None,
                idempotency_key: unique("t"),
            },
            at(0),
        )
        .await
        .expect("FINANCE of B1 creates");
    flow.submit(&creator, &t.id, at(10))
        .await
        .expect("the creator submits");
    flow.decide(&manager_b1, &t.id, Outcome::Approved, "ok", at(20))
        .await
        .expect("approve");
    let t = flow
        .dispatch(&creator, &t.id, at(30))
        .await
        .expect("dispatch");
    Dispatched { receiver, fifty, t }
}

/// Ledger postings written for `transfer_id` (any kind).
async fn postings_of(db: &Db, transfer_id: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM ledger_postings WHERE source_id = $1")
        .bind(transfer_id)
        .fetch_one(&db.pool)
        .await
        .expect("count the transfer's postings")
}

#[tokio::test]
async fn f26_a_receive_with_no_counted_line_is_refused_and_the_transfer_stays_dispatched() {
    let db = Db::fresh().await;
    let d = dispatched(&db).await;
    let before = postings_of(&db, &d.t.id).await;

    let refused = db
        .flow()
        .receive(&d.receiver, &d.t.id, vec![], at(40))
        .await
        .expect_err("a receive that counted nothing is not a count");

    assert!(matches!(&refused, AppError::Validation(_)), "{refused:?}");
    assert_eq!(db.status(&d.t.id).await, "DISPATCHED");
    assert_eq!(
        postings_of(&db, &d.t.id).await,
        before,
        "nothing leaves IN_TRANSIT on a refused receive"
    );
}

#[tokio::test]
async fn f26_a_receive_that_states_a_line_as_zero_is_still_a_short_count() {
    let db = Db::fresh().await;
    let d = dispatched(&db).await;

    let t = db
        .flow()
        .receive(
            &d.receiver,
            &d.t.id,
            vec![NewTransferLine {
                currency: usd(),
                denomination_id: d.fifty,
                qty: 0,
            }],
            at(40),
        )
        .await
        .expect("a stated zero is a count");

    assert_eq!(t.status, TransferStatus::Received);
    assert_eq!(db.status(&d.t.id).await, "RECEIVED");
}
