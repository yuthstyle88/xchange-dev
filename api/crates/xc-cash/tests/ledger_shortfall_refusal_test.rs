//! XC-BE-GAPS-A row 1: `LedgerCashPostings` refuses with the shortfall it would have left, and a reversal's state is
//! part of the `NEGATIVE_STOCK` source hash (design `docs/design/blueprint/ledger-inventory.md:418-425` §5.1: "requested
//! by the caller that got `stockInsufficient` … with the shortfall per currency in `proposed_payload`"; `version_hash`
//! = "the source's state"; §4.5 for what "below zero" is).
//! Written before the implementation by the ticket author (claude/491239fc, XC-BE-GAPS-A row 1, 2026-10-27). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Pinned here:
//! - the refusal is still `AppError::Domain` starting with today's sentence (every existing reader matches
//!   `stockInsufficient`), followed by `StockShortfall::MARKER`;
//! - `StockShortfall::from_refusal(<message>)` names one line per drawer/safe the posting takes below zero
//!   (`short_by` = −(balance + change)) and one per position whose final quantity is below zero (`short_by` =
//!   −quantity, place `POSITION`), sorted by currency code, then `DRAWER` < `SAFE` < `POSITION`;
//! - two drawers of one currency in one posting are summed into one `(currency, DRAWER)` line (ticket author's
//!   choice: the request is about the source, and §5.1 asks for the shortfall "per currency");
//! - a move inside one branch that empties the safe names the safe only: the position does not move (§3.2);
//! - `NegativeStockSource::version_hash` of a `reversals.id` changes with the reversal's status and differs from an
//!   unknown id's (a reversal's post is a `NEGATIVE_STOCK` source too, `negative_stock_gate.rs`
//!   `NegativeStockObject::Reversal`).
//! - the reversal arm adds nothing for any other id: an exchange's and a transfer's `version_hash` stay byte-for-byte
//!   the hash the skeleton computed (`SKELETON_HASH_SQL`, copied verbatim from `ledger_postings.rs` `source_hash` at
//!   `1c6e58eee`), so a request already decided at one of those hashes still matches (lane controller, 2026-10-27).
//!   This one passes on the skeleton too; it guards the change.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips. Each test gets a fresh
//! schema with every migration of `api/migrations` applied (harness copied from `ledger_postings_test.rs`).

use std::path::{Path, PathBuf};
use std::str::FromStr;

use pos_error::AppError;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Transaction};
use uuid::Uuid;

use xc_approval::domain::approval_repository::ApprovalRepository;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_cash::application::ledger_approval::negative_stock::NegativeStockSource;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::domain::shortfall::{ShortPlace, Shortfall, StockShortfall};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};

const ORG: &str = "00000000-0000-0000-0000-000000000001";
const SENTENCE: &str =
    "stockInsufficient: the posting takes a drawer, a safe or a position below zero";

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a valid decimal")
}

fn fa(s: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(d(s))
}

fn cur(s: &str) -> CurrencyCode {
    CurrencyCode::parse(s).expect("a currency")
}

fn short(code: &str, place: ShortPlace, by: &str) -> Shortfall {
    Shortfall {
        currency: cur(code),
        place,
        short_by: d(by),
    }
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

async fn db() -> (PgPool, BranchId) {
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
    let schema = unique("gapsa1");
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
        .expect("every migration applies in version order on a fresh database");

    let branch = unique("B");
    sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
        .bind(&branch)
        .execute(&pool)
        .await
        .expect("insert a branch");
    sqlx::query("INSERT INTO ops_state (branch_id, phase) VALUES ($1, 'OPEN')")
        .bind(&branch)
        .execute(&pool)
        .await
        .expect("insert ops_state");
    (pool, BranchId::new(branch))
}

async fn seed_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, organization_id, name, status, created_by) \
         VALUES ($1, $2::uuid, 'Test Teller', 'ACTIVE', 'test')",
    )
    .bind(id)
    .bind(ORG)
    .execute(pool)
    .await
    .expect("insert user");
    id
}

/// Closes the branch's open shift, if any, and opens a new one (one open shift per branch).
async fn open_shift(pool: &PgPool, branch: &BranchId) -> String {
    sqlx::query(
        "UPDATE shifts SET status = 'CLOSED', closed_at = now() WHERE branch_id = $1 AND status = 'OPEN'",
    )
    .bind(branch.as_str())
    .execute(pool)
    .await
    .expect("close the open shift");
    let teller_id = seed_user(pool).await;
    let shift_id = unique("shift");
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ($1, $2, 'OPEN', now(), 'system', $3, $4)",
    )
    .bind(&shift_id)
    .bind(branch.as_str())
    .bind(unique("open-req"))
    .bind(teller_id)
    .execute(pool)
    .await
    .expect("insert shift");
    shift_id
}

fn src(source_type: SourceType, id: &str) -> SourceRef {
    SourceRef {
        source_type,
        source_id: id.to_string(),
        source_key: "gapsa1".to_string(),
    }
}

fn leg(
    from: CashPlace,
    to: CashPlace,
    code: &str,
    qty: &str,
    source: &SourceRef,
) -> PostingRequest {
    PostingRequest {
        from,
        to,
        currency: cur(code),
        quantity: fa(qty),
        source: source.clone(),
        line: None,
        reverses: None,
    }
}

/// `BANK → to` of `qty` (an `OPENING`, one foreign leg): the place and the branch's position hold it.
async fn fund(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    postings: &LedgerCashPostings,
    to: CashPlace,
    code: &str,
    qty: &str,
) {
    let s = src(SourceType::Opening, &unique("fund"));
    postings
        .post_in(tx, &[leg(CashPlace::Bank, to, code, qty, &s)], None)
        .await
        .expect("fund through the ledger");
}

/// The posting is refused `stockInsufficient` with today's sentence, then the marker; answers the message.
async fn refusal_of(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    postings: &LedgerCashPostings,
    legs: &[PostingRequest],
    what: &str,
) -> String {
    match postings.post_in(tx, legs, None).await {
        Err(AppError::Domain(m)) => {
            assert!(
                m.starts_with(&format!("{SENTENCE}{}", StockShortfall::MARKER)),
                "{what}: the §4.5 sentence, then the shortfall marker; got {m:?}"
            );
            m
        }
        other => panic!("{what}: expected stockInsufficient, got {other:?}"),
    }
}

/// A SELL of USD 100 out of a drawer holding USD 30 (the position holds the same 30), and of EUR 5 from nothing,
/// sent USD first: two currencies, a partly funded drawer, and positions going below zero.
#[tokio::test]
async fn gapsa1_a_short_sell_names_each_drawer_and_position_sorted_by_currency_then_place() {
    let (pool, branch) = db().await;
    let shift = open_shift(&pool, &branch).await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    fund(
        &mut tx,
        &postings,
        CashPlace::Drawer(shift.clone()),
        "USD",
        "30",
    )
    .await;

    let s = src(SourceType::Exchange, &Uuid::new_v4().to_string());
    let drawer = || CashPlace::Drawer(shift.clone());
    let legs = [
        leg(drawer(), CashPlace::Customer, "USD", "100", &s),
        leg(drawer(), CashPlace::Customer, "EUR", "5", &s),
        leg(CashPlace::Customer, drawer(), "THB", "3500.00", &s),
    ];
    let m = refusal_of(&mut tx, &postings, &legs, "a SELL beyond the drawer").await;
    assert_eq!(
        StockShortfall::from_refusal(&m).lines,
        vec![
            short("EUR", ShortPlace::Drawer, "5"),
            short("EUR", ShortPlace::Position, "5"),
            short("USD", ShortPlace::Drawer, "70"),
            short("USD", ShortPlace::Position, "70"),
        ],
        "§5.1: per currency, drawer −(30 − 100) and position −(30 − 100); THB went up and is not short: {m}"
    );

    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// The branch holds USD 100, all in the safe; `SAFE → DRAWER` of 150 empties the safe past zero. A move inside one
/// branch moves no position (§3.2), so only the safe is named.
#[tokio::test]
async fn gapsa1_a_safe_taken_below_zero_is_named_alone() {
    let (pool, branch) = db().await;
    let shift = open_shift(&pool, &branch).await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");
    fund(
        &mut tx,
        &postings,
        CashPlace::Safe(branch.clone()),
        "USD",
        "100.00",
    )
    .await;

    let s = src(SourceType::Transfer, &unique("move"));
    let legs = [leg(
        CashPlace::Safe(branch.clone()),
        CashPlace::Drawer(shift.clone()),
        "USD",
        "150.00",
        &s,
    )];
    let m = refusal_of(&mut tx, &postings, &legs, "SAFE below zero").await;
    assert_eq!(
        StockShortfall::from_refusal(&m).lines,
        vec![short("USD", ShortPlace::Safe, "50")],
        "the safe ends at −50; the position stays at 100: {m}"
    );

    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// Two drawers (two shifts of the branch) each pay USD out of nothing in one posting: one `(USD, DRAWER)` line with
/// the sum (ticket author's choice), and the branch's position short by the same total.
#[tokio::test]
async fn gapsa1_two_drawers_of_one_currency_are_summed_into_one_line() {
    let (pool, branch) = db().await;
    let first = open_shift(&pool, &branch).await;
    let second = open_shift(&pool, &branch).await;
    let postings = LedgerCashPostings::new();
    let mut tx = pool.begin().await.expect("begin");

    let s = src(SourceType::Exchange, &Uuid::new_v4().to_string());
    let legs = [
        leg(
            CashPlace::Drawer(first.clone()),
            CashPlace::Customer,
            "USD",
            "10",
            &s,
        ),
        leg(
            CashPlace::Drawer(second.clone()),
            CashPlace::Customer,
            "USD",
            "5",
            &s,
        ),
        leg(
            CashPlace::Customer,
            CashPlace::Drawer(first.clone()),
            "THB",
            "500.00",
            &s,
        ),
    ];
    let m = refusal_of(&mut tx, &postings, &legs, "two drawers short").await;
    assert_eq!(
        StockShortfall::from_refusal(&m).lines,
        vec![
            short("USD", ShortPlace::Drawer, "15"),
            short("USD", ShortPlace::Position, "15"),
        ],
        "summed per (currency, place): {m}"
    );

    tx.rollback().await.expect("rollback");
    pool.close().await;
}

/// The hash a `NEGATIVE_STOCK` request on a reversal is decided at (§5.1 `version_hash` = the source's state).
async fn hash_of(pool: &PgPool, postings: &LedgerCashPostings, object_id: &str) -> String {
    let mut tx = pool.begin().await.expect("begin");
    let h = NegativeStockSource::version_hash(postings, &mut tx, object_id)
        .await
        .expect("the source's version hash");
    tx.rollback().await.expect("rollback");
    h
}

/// A COMPLETED exchange to reverse; answers its id.
async fn seed_transaction(pool: &PgPool, branch: &BranchId, shift: &str) -> Uuid {
    let cust_id = unique("CU");
    sqlx::query(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         VALUES ($1, $2::uuid, $3, 'Somchai', 'LIVE', now())",
    )
    .bind(&cust_id)
    .bind(ORG)
    .bind(unique("CUSTNO"))
    .execute(pool)
    .await
    .expect("seed customer");
    let tx_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
         customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, \
         recorded_at, recorded_by, status, completed_at) \
         VALUES ($1, $2::uuid, $3, $4, 'LIVE', $5, $6, 'hash', $7, 'Somchai', 'THA', 'system', 'WALK-IN', \
         '', current_date, now(), now(), 'system', 'COMPLETED', now())",
    )
    .bind(tx_id)
    .bind(ORG)
    .bind(unique("DOC"))
    .bind(branch.as_str())
    .bind(shift)
    .bind(unique("sub"))
    .bind(cust_id)
    .execute(pool)
    .await
    .expect("insert transaction");
    tx_id
}

#[tokio::test]
async fn gapsa1_a_reversals_version_hash_follows_its_status() {
    let (pool, branch) = db().await;
    let shift = open_shift(&pool, &branch).await;
    let postings = LedgerCashPostings::new();
    let original = seed_transaction(&pool, &branch, &shift).await;

    let reversal = Uuid::new_v4();
    let unknown = hash_of(&pool, &postings, &Uuid::new_v4().to_string()).await;
    sqlx::query(
        "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, reason, \
         amount_thb, business_date, requested_by, requested_at) \
         VALUES ($1, $2::uuid, $3, $4, 'REQUESTED', 'wrong amount', 330.00, current_date, 'teller', now())",
    )
    .bind(reversal)
    .bind(ORG)
    .bind(branch.as_str())
    .bind(original)
    .execute(&pool)
    .await
    .expect("insert a REQUESTED reversal");
    let requested = hash_of(&pool, &postings, &reversal.to_string()).await;
    assert_ne!(
        requested, unknown,
        "§5.1: a reversal is a source with a state; its hash is not an unknown id's"
    );

    // AP-1's request for the reversal, then the reversal moves to APPROVED.
    let repo = PgApprovalRepository::new(pool.clone());
    let mut tx = pool.begin().await.expect("begin");
    let req = repo
        .insert_request(
            &mut tx,
            ORG,
            "REVERSAL",
            Some(branch.as_str().to_string()),
            &reversal.to_string(),
            "h",
            "v1",
            serde_json::json!({}),
            "teller",
            &unique("key"),
        )
        .await
        .expect("insert approval request");
    sqlx::query(
        "UPDATE reversals SET status = 'APPROVED', approval_request_id = $2::uuid, updated_at = now() WHERE id = $1",
    )
    .bind(reversal)
    .bind(req.id.as_str())
    .execute(&mut *tx)
    .await
    .expect("approve the reversal");
    tx.commit().await.expect("commit");

    let approved = hash_of(&pool, &postings, &reversal.to_string()).await;
    assert_ne!(
        approved, requested,
        "the hash changes when the reversal's status changes"
    );
    assert_ne!(approved, unknown);
    assert_eq!(
        approved,
        hash_of(&pool, &postings, &reversal.to_string()).await,
        "the same state hashes the same"
    );
    pool.close().await;
}

/// `source_hash` as it was before XC-BE-GAPS-A (verbatim from `ledger_postings.rs` at `1c6e58eee`).
const SKELETON_HASH_SQL: &str = "SELECT md5( \
           coalesce((SELECT 'transaction:' || t.status || '|' || coalesce(string_agg( \
               concat_ws(':', l.id, l.line_no, l.shop_side, l.currency, l.quantity, l.unit_size, \
                         l.rate_snapshot, l.thb_amount, l.fee_thb, l.settle_thb), \
               ',' ORDER BY l.line_no, l.id), '') \
             FROM transactions t LEFT JOIN transaction_lines l ON l.transaction_id = t.id \
             WHERE t.id::text = $1 GROUP BY t.status), '') \
           || '#' || \
           coalesce((SELECT 'transfer:' || ct.status || '|' || coalesce(string_agg( \
               concat_ws(':', cl.currency, cl.denomination_id, cl.qty_sent, cl.qty_received), \
               ',' ORDER BY cl.currency, cl.denomination_id), '') \
             FROM cash_transfers ct LEFT JOIN cash_transfer_lines cl ON cl.transfer_id = ct.id \
             WHERE ct.id::text = $1 GROUP BY ct.status), ''))";

async fn skeleton_hash(pool: &PgPool, object_id: &str) -> String {
    sqlx::query_scalar(SKELETON_HASH_SQL)
        .bind(object_id)
        .fetch_one(pool)
        .await
        .expect("the skeleton's hash")
}

/// A DRAFT transfer with one USD line; answers its id.
async fn seed_transfer(pool: &PgPool, branch: &BranchId) -> String {
    let user = seed_user(pool).await;
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM denominations WHERE currency_code = 'USD' ORDER BY id LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .expect("read denominations");
    let denomination = match existing {
        Some(id) => id,
        None => sqlx::query_scalar(
            "INSERT INTO denominations (currency_code, face_value, group_code) \
             VALUES ('USD', 37, 'ALL') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .expect("a denomination"),
    };
    let id: String = sqlx::query_scalar(
        "INSERT INTO cash_transfers (organization_id, kind, from_branch_id, status, created_by, idempotency_key) \
         VALUES ($1::uuid, 'EXTERNAL_IN', $2, 'DRAFT', $3, $4) RETURNING id::text",
    )
    .bind(ORG)
    .bind(branch.as_str())
    .bind(user)
    .bind(unique("idem"))
    .fetch_one(pool)
    .await
    .expect("a transfer");
    sqlx::query(
        "INSERT INTO cash_transfer_lines (transfer_id, organization_id, currency, denomination_id, qty_sent) \
         VALUES ($1::uuid, $2::uuid, 'USD', $3, 4)",
    )
    .bind(&id)
    .bind(ORG)
    .bind(denomination)
    .execute(pool)
    .await
    .expect("a transfer line");
    id
}

#[tokio::test]
async fn gapsa1_an_exchanges_and_a_transfers_version_hash_are_unchanged() {
    let (pool, branch) = db().await;
    let shift = open_shift(&pool, &branch).await;
    let postings = LedgerCashPostings::new();

    let exchange = seed_transaction(&pool, &branch, &shift).await;
    sqlx::query(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
         quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
         VALUES ($1, $2::uuid, $3, 'SELL', 'USD', 10, 1, 33.0, 330.00, 0, 330.00, 1, now(), now())",
    )
    .bind(Uuid::new_v4())
    .bind(ORG)
    .bind(exchange)
    .execute(&pool)
    .await
    .expect("insert a transaction line");
    let transfer = seed_transfer(&pool, &branch).await;

    for (what, id) in [
        ("an exchange", exchange.to_string()),
        ("a transfer", transfer),
        ("an unknown id", Uuid::new_v4().to_string()),
    ] {
        assert_eq!(
            hash_of(&pool, &postings, &id).await,
            skeleton_hash(&pool, &id).await,
            "{what}: the reversal arm adds nothing to a hash that is not a reversal's"
        );
    }
    pool.close().await;
}
