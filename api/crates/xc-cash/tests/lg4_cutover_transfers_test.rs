//! Ticket #929 (LG-4) — the cutover, as the transfer flow sees it: `LedgerCashPostings` behind every holder of the
//! port, `TransferFlow::receive` on `BranchTx::run_pair`, and a reconciled short transfer's shortfall on the
//! sender's `VARIANCE` (design `docs/design/blueprint/ledger-inventory.md` §4.4, L-5, §8; lane controller's rulings
//! on #929: commits 1 and 3).
//! Written before the implementation by the ticket author (claude/89bd61d9, rule A). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! What this test pins ([required] unless marked):
//! - **Commit 1, the cutover.**
//!   - `InterimCashPostings` is gone (ticket §4 criterion 2): no `interim_postings.rs`, and no source under
//!     `api/src` or `api/crates/*/src` names it (test comments may, as history).
//!   - `CashCountService` holds the port as `postings: Arc<dyn CashPostings>` (ticket §7), and the three places that
//!     build the postings build `LedgerCashPostings::new()`: `CashCountService::pg` (`count_service.rs`),
//!     `TransferFlow::pg` (`transfer_flow.rs`) and the TX-5 completion command (`commands/complete.rs`).
//!   - A BRANCH transfer's receipt moves **both** branches' `branch_versions` (L-5, `run_pair`), and two opposite
//!     receipts at once both complete, each moving both versions.
//!   - The transfer moves cash and cost through the ledger (§4.4): dispatch takes the notes out of B1's SAFE; the
//!     receipt puts what was counted into B2's SAFE, with a `TRANSFER_OUT` on B1 at B1's average and a
//!     `TRANSFER_IN` lot on B2 at that same unit cost.
//! - **Commit 3, a short transfer reconciled.** The shortfall left `IN_TRANSIT` goes to the **sender's** `VARIANCE`
//!   account (§4.4 row 6), with an inventory `ADJUST` on the sender at its average and no `DISPOSE` or `ACQUIRE`.
//!   This file reads `VARIANCE` with SQL, so it compiles before `CashPlace::Variance` exists.
//!
//! Seeding: each branch's safe is seeded in both stores (`ops_state.safe`, which main's port reads, and the
//! ledger, which the cutover reads), so on `main` each test fails at the assertion it is about. B1's USD is funded
//! by a BUY-shaped posting (USD in, THB out) so its position has a cost: 500 USD for THB 16,050.00, average 32.10.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.

use std::path::{Path, PathBuf};
use std::str::FromStr;

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

// ── commit 1: the sources ─────────────────────────────────────────────────────

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let p = e.expect("an entry").path();
        if p.is_dir() {
            rust_sources(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn interim_cash_postings_is_gone_and_the_port_is_built_as_the_ledger() {
    let api = api_dir();
    assert!(
        !api.join("crates/xc-cash/src/infrastructure/interim_postings.rs")
            .exists(),
        "interim_postings.rs is deleted (ticket §3 item 3)"
    );
    let mut files = Vec::new();
    rust_sources(&api.join("src"), &mut files);
    for c in std::fs::read_dir(api.join("crates")).expect("read api/crates") {
        let src = c.expect("a crate").path().join("src");
        if src.is_dir() {
            rust_sources(&src, &mut files);
        }
    }
    let naming: Vec<String> = files
        .iter()
        .filter(|f| {
            std::fs::read_to_string(f)
                .unwrap_or_default()
                .contains("InterimCashPostings")
        })
        .map(|f| f.display().to_string())
        .collect();
    assert!(
        naming.is_empty(),
        "no source names InterimCashPostings: {naming:?}"
    );

    let read =
        |rel: &str| std::fs::read_to_string(api.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    let flat = |s: String| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let count = flat(read(
        "crates/xc-cash/src/application/count/count_service.rs",
    ));
    assert!(
        count.contains("postings:Arc<dynCashPostings>"),
        "CashCountService holds `postings: Arc<dyn CashPostings>` (ticket §7)"
    );
    for rel in [
        "crates/xc-cash/src/application/count/count_service.rs",
        "crates/xc-cash/src/application/transfer/transfer_flow.rs",
        "src/presentation/http/v3/commands/complete.rs",
    ] {
        assert!(
            flat(read(rel)).contains("LedgerCashPostings::new()"),
            "{rel} builds LedgerCashPostings"
        );
    }
}

// ── commit 1: run_pair (L-5) ─────────────────────────────────────────────────

#[tokio::test]
async fn a_branch_transfer_received_moves_both_branches_versions() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    let b1 = db.branch("b1", "500.00").await;
    let b2 = db.branch("b2", "0").await;
    let t = dispatched(&db, &c, &b1, &b2, 4).await;
    let (v1, v2) = (db.version(&b1).await, db.version(&b2).await);
    let who = receiver(&db, &b2).await;
    let t = receive(&db, &who, &t, 4).await;
    assert_eq!(t.status, TransferStatus::Received);
    assert_eq!(
        db.version(&b2).await,
        v2 + 1,
        "the receiver's version moves"
    );
    assert_eq!(
        db.version(&b1).await,
        v1 + 1,
        "the sender's version moves too: its figures changed (L-5, run_pair)"
    );
}

#[tokio::test]
async fn two_opposite_receipts_at_once_both_complete_and_each_moves_both_versions() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    let a = db.branch("a", "500.00").await;
    let b = db.branch("b", "500.00").await;
    for round in 0..3 {
        let ab = dispatched(&db, &c, &a, &b, 2).await;
        let ba = dispatched(&db, &c, &b, &a, 2).await;
        let (va, vb) = (db.version(&a).await, db.version(&b).await);
        let at_b = receiver(&db, &b).await;
        let at_a = receiver(&db, &a).await;
        let fifty = db.fifty().await;
        let (flow1, flow2) = (db.flow(), db.flow());
        let line = |q| NewTransferLine {
            currency: usd(),
            denomination_id: fifty,
            qty: q,
        };
        let (r1, r2) = tokio::join!(
            flow1.receive(&at_b, &ab.id, vec![line(2)], at(20)),
            flow2.receive(&at_a, &ba.id, vec![line(2)], at(20)),
        );
        let (r1, r2) = (
            r1.unwrap_or_else(|e| panic!("round {round}: A→B: {e:?}")),
            r2.unwrap_or_else(|e| panic!("round {round}: B→A: {e:?}")),
        );
        assert_eq!(
            (r1.status, r2.status),
            (TransferStatus::Received, TransferStatus::Received)
        );
        assert_eq!(
            (db.version(&a).await, db.version(&b).await),
            (va + 2, vb + 2),
            "round {round}: each receipt moves both versions"
        );
    }
}

// ── commit 1: cash and cost through the ledger (§4.4) ────────────────────────

#[tokio::test]
async fn a_branch_transfer_moves_cash_and_cost_through_the_ledger() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    let b1 = db.branch("b1", "500.00").await;
    let b2 = db.branch("b2", "0").await;
    let t = dispatched(&db, &c, &b1, &b2, 4).await;
    assert_eq!(
        db.usd_at(CashPlace::Safe(b1.clone())).await,
        d("300.00"),
        "dispatch takes USD 200 out of B1's SAFE (the ledger)"
    );
    let who = receiver(&db, &b2).await;
    receive(&db, &who, &t, 4).await;
    assert_eq!(db.usd_at(CashPlace::Safe(b2.clone())).await, d("200.00"));
    assert_eq!(
        db.scalar_dec(&format!(
            "SELECT sum(cost_thb_signed) FROM inventory_movements WHERE branch_id = '{}' \
             AND movement_type = 'TRANSFER_OUT'",
            b1.as_str()
        ))
        .await,
        d("-6420.00"),
        "TRANSFER_OUT on the sender at its average 32.10"
    );
    assert_eq!(
        db.scalar_dec(&format!(
            "SELECT max(l.unit_cost_thb) FROM inventory_lots l JOIN inventory_movements m ON m.lot_id = l.id \
             WHERE m.branch_id = '{}' AND m.movement_type = 'TRANSFER_IN'",
            b2.as_str()
        ))
        .await
        .normalize(),
        d("32.10"),
        "a TRANSFER_IN lot on the receiver at that same unit cost"
    );
}

// ── commit 3: a short transfer reconciled ────────────────────────────────────

#[tokio::test]
async fn a_short_transfer_reconciled_puts_the_shortfall_on_the_senders_variance_at_average() {
    let db = Db::fresh().await;
    let c = cast(&db).await;
    let b1 = db.branch("b1", "500.00").await;
    let b2 = db.branch("b2", "0").await;
    let t = dispatched(&db, &c, &b1, &b2, 4).await;
    let who = receiver(&db, &b2).await;
    receive(&db, &who, &t, 3).await; // USD 150 received of 200 sent
    db.flow()
        .reconcile(&c.finance, &t.id, "fifty short", at(30))
        .await
        .expect("FINANCE reconciles");

    let variance = format!(
        "SELECT sum(e.quantity_signed) FROM ledger_entries e JOIN cash_accounts a ON a.id = e.account_id \
         WHERE a.kind = 'VARIANCE' AND a.branch_id = '{}' AND trim(a.currency) = 'USD'",
        b1.as_str()
    );
    assert_eq!(
        db.scalar_dec(&variance).await,
        d("50"),
        "the USD 50 shortfall is on the sender's VARIANCE"
    );
    let in_transit = format!(
        "SELECT sum(e.quantity_signed) FROM ledger_entries e JOIN cash_accounts a ON a.id = e.account_id \
         WHERE a.kind = 'IN_TRANSIT' AND a.branch_id = '{}' AND trim(a.currency) = 'USD'",
        b1.as_str()
    );
    assert_eq!(
        db.scalar_dec(&in_transit).await,
        d("0"),
        "nothing left in transit"
    );
    let reconcile_posting = format!(
        "SELECT p.id FROM ledger_postings p JOIN ledger_entries e ON e.posting_id = p.id \
         JOIN cash_accounts a ON a.id = e.account_id WHERE a.kind = 'VARIANCE' AND a.branch_id = '{}'",
        b1.as_str()
    );
    assert_eq!(
        db.scalar(&format!(
            "SELECT count(*) FROM inventory_movements WHERE posting_id IN ({reconcile_posting}) \
             AND movement_type IN ('DISPOSE', 'ACQUIRE')"
        ))
        .await,
        0,
        "no DISPOSE or ACQUIRE for the shortfall"
    );
    assert_eq!(
        db.scalar_dec(&format!(
            "SELECT sum(quantity_signed) FROM inventory_movements WHERE posting_id IN ({reconcile_posting}) \
             AND movement_type = 'ADJUST'"
        ))
        .await,
        d("-50"),
        "an ADJUST of −50 on the sender"
    );
    assert_eq!(
        db.scalar_dec(&format!(
            "SELECT sum(cost_thb_signed) FROM inventory_movements WHERE posting_id IN ({reconcile_posting}) \
             AND movement_type = 'ADJUST'"
        ))
        .await,
        d("-1605.00"),
        "at the sender's average 32.10"
    );
    assert_eq!(
        db.scalar_dec(&format!(
            "SELECT quantity FROM inventory_positions WHERE branch_id = '{}' AND trim(currency) = 'USD'",
            b1.as_str()
        ))
        .await
        .normalize(),
        d("300"),
        "B1's position: 500 − 150 received − 50 written off"
    );
}
