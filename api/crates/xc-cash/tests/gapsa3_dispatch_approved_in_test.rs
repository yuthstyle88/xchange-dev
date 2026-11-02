//! XC-BE-GAPS-A row 3 (owner ruling R1, 2026-10-27: "ledger-inventory N-1 wins"): a transfer dispatch may take a
//! drawer or the safe below zero, but only through an APPROVED `NEGATIVE_STOCK` request keyed on the transfer.
//!
//! Written before the implementation by the ticket author (claude/491239fc, XC-BE-GAPS-A row 3, 2026-10-27). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Pinned here, at the crate level (`TransferFlow::dispatch_approved_in`):
//! - `docs/design/blueprint/ledger-inventory.md:34` (N-1): a move below zero is refused `stockInsufficient` unless
//!   a BRANCH_MANAGER approves it through AP-1. The posting port is the one check: `dispatch_on`'s own Drawer/Safe
//!   pre-check (which answered `TransferError::StockInsufficient`, "stock insufficient", with no code prefix) goes,
//!   and `post_in(tx, &reqs, allow_negative)` refuses with the port's `stockInsufficient: …` message.
//! - A refused dispatch writes nothing on the caller's transaction: the transfer is still APPROVED on that same
//!   transaction (so the status change comes after the posting, or is undone with it) and no ledger posting names it.
//! - §5.1 (`ledger-inventory.md:418-425`): the request's `object_id` is the transfer's id, the posting's `source_id`,
//!   and its `version_hash` is the transfer's lines and status at the time it was requested (APPROVED). An
//!   APPROVED request made at that state lets the dispatch take the safe below zero by the shortfall.
//! - An APPROVED `NEGATIVE_STOCK` request for another transfer does not.
//!
//! The scenario is `services_on_caller_tx_test.rs`'s: a BRANCH move of USD 50 notes out of a safe holding USD
//! 500.00, approved through today's wrappers; only the dispatch runs on a caller's transaction that holds the
//! source branch's lock, as the CM-1 seam does.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips. Each test works in a
//! schema of its own.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use pos_error::AppError;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Postgres, Transaction};
use xc_approval::domain::approval_repository::ApprovalRepository;
use xc_approval::domain::request::Outcome;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_cash::application::ledger_approval::negative_stock::NegativeStockSource;
use xc_cash::application::transfer::TransferFlow;
use xc_cash::domain::posting::ApprovalRequestId;
use xc_cash::domain::transfer::cash_transfer::{NewTransfer, NewTransferLine, TransferKind};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const STATEMENT_TIMEOUT: &str = "15s";
const MUST_FINISH: Duration = Duration::from_secs(30);

static FUND: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 27, 9, 0, 0).unwrap() + chrono::Duration::seconds(seconds)
}

async fn within<T>(what: &str, fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(MUST_FINISH, fut)
        .await
        .unwrap_or_else(|_| {
            panic!("{what} did not finish: it waits on the lock its own caller holds")
        })
}

// ------------------------------------------------------------ database

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
        let schema = unique("gapsa3");
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
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        Migrator::new(dir.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("every migration applies in version order");
        Db { pool }
    }

    /// A branch with USD 500.00 in its safe, through the ledger.
    async fn branch(&self, label: &str) -> BranchId {
        let id = unique(label);
        sqlx::query("INSERT INTO branches (id, name, organization_id) VALUES ($1, $1, $2::uuid)")
            .bind(&id)
            .bind(ORG)
            .execute(&self.pool)
            .await
            .expect("insert a branch");
        sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
            .bind(&id)
            .execute(&self.pool)
            .await
            .expect("insert ops_state");
        fund_safe(&self.pool, &id, "USD", "500.00").await;
        BranchId::new(id)
    }

    async fn person(&self, role: Role, branch: Option<&BranchId>) -> Actor {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique(role.as_str()))
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
             VALUES ($1::uuid, $2, $3::uuid, $4, '2026-01-01T00:00:00Z')",
        )
        .bind(&id)
        .bind(role.as_str())
        .bind(ORG)
        .bind(branch.map(|b| b.as_str().to_owned()))
        .execute(&self.pool)
        .await
        .expect("give the user a role");
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        Actor::new(
            &id,
            ORG,
            vec![ScopedRole::new(role, branch.cloned(), from, None)],
            None,
        )
    }

    async fn usd50(&self) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = 'USD' AND face_value = 50::numeric",
        )
        .fetch_one(&self.pool)
        .await
        .expect("the USD 50 denomination")
    }

    async fn transfer_limit_usd(&self, by: &Actor, amount: &str) {
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

    async fn committed(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn committed_text(&self, sql: &str) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    /// The caller's transaction as the seam opens it: `statement_timeout`, and `branch` locked in `branch_versions`.
    async fn locked(&self, branch: &BranchId) -> Transaction<'static, Postgres> {
        sqlx::query(
            "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 0) ON CONFLICT (branch_id) DO NOTHING",
        )
        .bind(branch.as_str())
        .execute(&self.pool)
        .await
        .expect("the branch's version row, committed");
        let mut tx = self.pool.begin().await.expect("begin the caller's tx");
        sqlx::query(AssertSqlSafe(format!(
            "SET LOCAL statement_timeout = '{STATEMENT_TIMEOUT}'"
        )))
        .execute(&mut *tx)
        .await
        .expect("statement_timeout");
        sqlx::query("SELECT version FROM branch_versions WHERE branch_id = $1 FOR UPDATE")
            .bind(branch.as_str())
            .execute(&mut *tx)
            .await
            .expect("the branch lock the seam holds");
        tx
    }

    fn transfers(&self) -> TransferFlow {
        TransferFlow::pg(self.pool.clone())
    }
}

async fn on(tx: &mut Transaction<'static, Postgres>, sql: &str) -> i64 {
    sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
        .fetch_one(&mut **tx)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

async fn on_text(tx: &mut Transaction<'static, Postgres>, sql: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(AssertSqlSafe(sql.to_owned()))
        .fetch_one(&mut **tx)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

async fn fund_safe(pool: &PgPool, branch: &str, currency: &str, amount: &str) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
    let mut tx = pool.begin().await.expect("begin");
    LedgerCashPostings::new()
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
                        "gapsa3-fund:{branch}:{currency}:{}",
                        FUND.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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

/// The safe's USD in cents, from the signed ledger sum (a balance below zero reads as itself).
fn safe_usd(branch: &BranchId) -> String {
    format!(
        "SELECT coalesce((sum(e.quantity_signed) * 100)::bigint, 0) FROM ledger_entries e \
         JOIN cash_accounts a ON a.id = e.account_id \
         WHERE a.kind = 'SAFE' AND a.branch_id = '{}' AND trim(a.currency) = 'USD'",
        branch.as_str()
    )
}

fn status_of(id: &str) -> String {
    format!("SELECT status FROM cash_transfers WHERE id = '{id}'::uuid")
}

fn postings_of(id: &str) -> String {
    format!("SELECT count(*) FROM ledger_postings WHERE source_id = '{id}'")
}

// ------------------------------------------------------------ scenario

struct Moving {
    from: BranchId,
    creator: Actor,
    id: String,
}

/// A BRANCH move of `notes` × USD 50 out of a safe holding USD 500.00, created, submitted and approved (committed).
async fn approved_transfer(db: &Db, notes: i32) -> Moving {
    let from = db.branch("from").await;
    let to = db.branch("to").await;
    let creator = db.person(Role::BranchManager, Some(&from)).await;
    let approver = db.person(Role::BranchManager, Some(&from)).await;
    let executive = db.person(Role::Executive, None).await;
    db.transfer_limit_usd(&executive, "1000.00").await;
    let usd50 = db.usd50().await;
    let flow = db.transfers();
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
                    currency: CurrencyCode::parse("USD").expect("USD"),
                    denomination_id: usd50,
                    qty: notes,
                }],
                note: Some("move".to_owned()),
                idempotency_key: unique("t"),
            },
            at(0),
        )
        .await
        .expect("the creator creates");
    flow.submit(&creator, &t.id, at(10))
        .await
        .expect("the creator submits");
    flow.decide(&approver, &t.id, Outcome::Approved, "ok", at(20))
        .await
        .expect("a branch manager of the source approves within the limit");
    Moving {
        from,
        creator,
        id: t.id,
    }
}

/// A `NEGATIVE_STOCK` request for `object_id` on `branch`, decided `APPROVED`, at the object's current version hash
/// (the transfer's lines and status, §5.1), committed.
async fn approved_negative_stock(db: &Db, branch: &BranchId, object_id: &str) -> ApprovalRequestId {
    let repo = PgApprovalRepository::new(db.pool.clone());
    let postings = LedgerCashPostings::new();
    let mut tx = db.pool.begin().await.expect("begin");
    let hash = NegativeStockSource::version_hash(&postings, &mut tx, object_id)
        .await
        .expect("the source's version hash");
    let req = repo
        .insert_request(
            &mut tx,
            ORG,
            "NEGATIVE_STOCK",
            Some(branch.as_str().to_string()),
            object_id,
            &hash,
            "1",
            serde_json::json!({ "shortfall": [] }),
            "dispatcher",
            &unique("key"),
        )
        .await
        .expect("insert the approval request");
    repo.update_status(&mut tx, &req.id, "APPROVED")
        .await
        .expect("approve it");
    tx.commit().await.expect("commit");
    ApprovalRequestId::new(req.id.as_str())
}

fn assert_port_refusal(result: Result<impl std::fmt::Debug, AppError>, what: &str) {
    match result {
        Err(AppError::Domain(m)) if m.starts_with("stockInsufficient") => {}
        other => panic!(
            "{what}: N-1 — the posting port refuses `stockInsufficient: …` (the code prefix the HTTP layer reads); \
             got {other:?}"
        ),
    }
}

// ------------------------------------------------------------ tests

/// `None` = no approval: a dispatch beyond the safe is refused by the posting port (`stockInsufficient: …`), and
/// nothing is written on the caller's transaction — the transfer is still APPROVED there and no posting names it.
#[tokio::test]
async fn gapsa3_dispatch_approved_in_without_approval_is_refused_by_the_port_and_writes_nothing() {
    let db = Db::fresh().await;
    let m = approved_transfer(&db, 12).await; // USD 600 > 500

    let mut tx = db.locked(&m.from).await;
    let result = within(
        "dispatch_approved_in",
        db.transfers()
            .dispatch_approved_in(&mut tx, &m.from, &m.creator, &m.id, at(30), None),
    )
    .await;
    assert_port_refusal(result, "no approval");
    assert_eq!(
        on_text(&mut tx, &status_of(&m.id)).await.as_deref(),
        Some("APPROVED"),
        "a refused dispatch leaves the transfer APPROVED on the caller's own transaction"
    );
    assert_eq!(
        on(&mut tx, &postings_of(&m.id)).await,
        0,
        "no posting names it"
    );
    assert_eq!(
        on(&mut tx, &safe_usd(&m.from)).await,
        50_000,
        "the safe did not move"
    );
    tx.rollback().await.expect("roll back");
}

/// `dispatch_in` is `dispatch_approved_in(.., None)`: the same refusal from the port.
#[tokio::test]
async fn gapsa3_dispatch_in_beyond_the_safe_answers_the_ports_refusal() {
    let db = Db::fresh().await;
    let m = approved_transfer(&db, 12).await;

    let mut tx = db.locked(&m.from).await;
    let result = within(
        "dispatch_in",
        db.transfers()
            .dispatch_in(&mut tx, &m.from, &m.creator, &m.id, at(30)),
    )
    .await;
    assert_port_refusal(result, "dispatch_in");
    tx.rollback().await.expect("roll back");
}

/// Restated 2026-11-02 by claude/b091f5e0, citing `ledger-inventory.md` §5.1.2 (owner: "ต้องเติมเงินก่อน ห้ามติดลบ"):
/// even an APPROVED `NEGATIVE_STOCK` request keyed on this transfer no longer lets the dispatch take the safe below
/// zero. It used to (N-1, R1 of 2026-10-27). The dispatch is refused by the port and writes nothing.
#[tokio::test]
async fn gapsa3_dispatch_approved_in_with_an_approved_request_is_still_refused() {
    let db = Db::fresh().await;
    let m = approved_transfer(&db, 12).await;
    let request = approved_negative_stock(&db, &m.from, &m.id).await;

    let mut tx = db.locked(&m.from).await;
    let result = within(
        "dispatch_approved_in",
        db.transfers().dispatch_approved_in(
            &mut tx,
            &m.from,
            &m.creator,
            &m.id,
            at(30),
            Some(request),
        ),
    )
    .await;
    assert_port_refusal(result, "§5.1.2: an approved request for this transfer");
    assert_eq!(
        on_text(&mut tx, &status_of(&m.id)).await.as_deref(),
        Some("APPROVED")
    );
    assert_eq!(
        on(&mut tx, &safe_usd(&m.from)).await,
        50_000,
        "the safe did not move"
    );
    tx.rollback().await.expect("roll back");
}

/// An APPROVED `NEGATIVE_STOCK` request for another transfer does not let this one go below zero.
#[tokio::test]
async fn gapsa3_dispatch_approved_in_with_another_transfers_request_is_refused() {
    let db = Db::fresh().await;
    let m = approved_transfer(&db, 12).await;
    let other = approved_transfer(&db, 12).await;
    let request = approved_negative_stock(&db, &other.from, &other.id).await;

    let mut tx = db.locked(&m.from).await;
    let result = within(
        "dispatch_approved_in",
        db.transfers().dispatch_approved_in(
            &mut tx,
            &m.from,
            &m.creator,
            &m.id,
            at(30),
            Some(request),
        ),
    )
    .await;
    assert_port_refusal(result, "another transfer's request");
    assert_eq!(
        on_text(&mut tx, &status_of(&m.id)).await.as_deref(),
        Some("APPROVED")
    );
    assert_eq!(on(&mut tx, &safe_usd(&m.from)).await, 50_000);
    tx.rollback().await.expect("roll back");
}
