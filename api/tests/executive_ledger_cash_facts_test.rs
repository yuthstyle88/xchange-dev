//! Ticket #1076 (EX-7): Ledger Cash Facts
//! (`docs/design/blueprint/executive-finance.md` §3, D-2, D-3; `ledger-inventory.md` §4.3, §4.4; #1076 §8–§9).
//!
//! Written before the implementation by the rule-A test-writer gemini/studio-12, on the BE lane controller
//! claude/ad05bf8d's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! One test per row of the ticket's §9 table.
//! - `available_cash` = DRAWER + SAFE from the ledger (`cash_accounts`, `ledger_entries`), equals the sum of
//!   `CashPostings::expected_in` over the branch's places;
//! - `in_transit` = transfer accounts from the ledger (the sender carries a transfer until received);
//! - `on_hand_by_currency` = `inventory_positions.quantity` per foreign currency, and THB from DRAWER + SAFE;
//! - `realized_margin` = Σ `transaction_lines.realized_margin_thb` of COMPLETED LIVE lines −
//!   Σ `reversals.realized_margin_reversed_thb`, by their business dates;
//!   Answers `Err(AwaitsLedger)` only for a period with no completed line at all.
//! - Money amounts are decimal THB strings, rounded half away from zero, never banker's rounding.
//!
//! Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips. Each test works in a schema of its own.

mod common;

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{NaiveDate, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money};
use xchange::application::executive::metrics::MetricUnavailable;
use xchange::application::executive::overview::{BranchHead, ExecutiveOverviewService};
use xchange::application::executive::ports::{
    BrokerNotConnected, CashFacts, ExchangeFacts, ExecutiveFilter,
};
use xchange::infrastructure::executive::alert_facts::PgAlertFacts;
use xchange::infrastructure::executive::booking_facts::BookingFacts;
use xchange::infrastructure::executive::ledger_cash_facts::LedgerCashFacts;
use xchange::infrastructure::executive::wide_transaction_facts::WideTransactionFacts;

const MUST_FINISH: Duration = Duration::from_secs(30);
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const B1: &str = "XC-B01";
const B2: &str = "XC-B02";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn thb() -> CurrencyCode {
    CurrencyCode::parse("THB").unwrap()
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("valid decimal")
}

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("ex7_ledger_cash");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");

        for branch in [B1, B2] {
            sqlx::query(
                "INSERT INTO branches (id, name, organization_id, timezone) \
                 VALUES ($1, $1, $2::uuid, 'Asia/Bangkok')",
            )
            .bind(branch)
            .bind(ORG)
            .execute(&pool)
            .await
            .expect("a branch");
            sqlx::query(
                "INSERT INTO branch_versions (branch_id, version, organization_id, changed_at) \
                 VALUES ($1, 1, $2::uuid, now())",
            )
            .bind(branch)
            .bind(ORG)
            .execute(&pool)
            .await
            .expect("a branch version");
        }
        Room { pool, schema }
    }

    /// Opens a shift for teller at `branch`.
    async fn open_shift(&self, branch: &str) -> String {
        let shift_id = unique("shift");
        let teller_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, 'teller', 'ACTIVE', 'test')",
        )
        .bind(teller_id)
        .bind(ORG)
        .execute(&self.pool)
        .await
        .expect("teller user");

        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id, organization_id) \
             VALUES ($1, $2, 'OPEN', now(), 'owner', $3, $4, $5::uuid)",
        )
        .bind(&shift_id)
        .bind(branch)
        .bind(unique("req"))
        .bind(teller_id)
        .bind(ORG)
        .execute(&self.pool)
        .await
        .expect("open shift");

        shift_id
    }

    /// Posts cash movement through `LedgerCashPostings`.
    async fn post(&self, reqs: Vec<PostingRequest>) {
        let mut tx = self.pool.begin().await.expect("begin tx");
        LedgerCashPostings::new()
            .post_in(&mut tx, &reqs, None)
            .await
            .expect("post_in succeeded");
        tx.commit().await.expect("commit tx");
    }

    /// Creates and dispatches a cash transfer from `from_branch` to `to_branch`.
    async fn dispatch_transfer(
        &self,
        from_branch: &str,
        to_branch: &str,
        amount_thb: &str,
    ) -> String {
        let transfer_id = Uuid::new_v4();
        let transfer_id_str = transfer_id.to_string();

        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, 'dispatcher', 'ACTIVE', 'test')",
        )
        .bind(user_id)
        .bind(ORG)
        .execute(&self.pool)
        .await
        .expect("user");

        sqlx::query(
            "INSERT INTO cash_transfers (id, organization_id, from_branch_id, to_branch_id, kind, status, \
               created_by, created_at, dispatched_by, dispatched_at, idempotency_key) \
             VALUES ($1, $2::uuid, $3, $4, 'BRANCH', 'DISPATCHED', \
               $5, now(), $5, now(), $6)",
        )
        .bind(transfer_id)
        .bind(ORG)
        .bind(from_branch)
        .bind(to_branch)
        .bind(user_id)
        .bind(unique("transfer_key"))
        .execute(&self.pool)
        .await
        .expect("insert cash_transfer DISPATCHED");

        let src = SourceRef {
            source_type: SourceType::Transfer,
            source_id: transfer_id_str.clone(),
            source_key: "dispatch".to_string(),
        };

        self.post(vec![PostingRequest {
            from: CashPlace::Safe(BranchId::new(from_branch)),
            to: CashPlace::InTransit(transfer_id_str.clone()),
            currency: thb(),
            quantity: ForeignAmount::from_decimal(d(amount_thb)),
            source: src,
            line: None,
            reverses: None,
        }])
        .await;

        transfer_id_str
    }

    /// Receives a dispatched cash transfer into `to_branch`'s safe.
    async fn receive_transfer(&self, transfer_id: &str, to_branch: &str, amount_thb: &str) {
        let receiver_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, 'receiver', 'ACTIVE', 'test')",
        )
        .bind(receiver_id)
        .bind(ORG)
        .execute(&self.pool)
        .await
        .expect("user");

        sqlx::query(
            "UPDATE cash_transfers SET status = 'RECEIVED', received_by = $2, received_at = now() \
             WHERE id = $1::uuid",
        )
        .bind(transfer_id)
        .bind(receiver_id)
        .execute(&self.pool)
        .await
        .expect("update cash_transfer RECEIVED");

        let src = SourceRef {
            source_type: SourceType::Transfer,
            source_id: transfer_id.to_string(),
            source_key: "receive".to_string(),
        };

        self.post(vec![PostingRequest {
            from: CashPlace::InTransit(transfer_id.to_string()),
            to: CashPlace::Safe(BranchId::new(to_branch)),
            currency: thb(),
            quantity: ForeignAmount::from_decimal(d(amount_thb)),
            source: src,
            line: None,
            reverses: None,
        }])
        .await;
    }

    /// Inserts a transaction and line with given mode, status, shop_side, margin, and business date.
    #[allow(clippy::too_many_arguments)]
    async fn insert_transaction_with_line(
        &self,
        branch: &str,
        date: NaiveDate,
        mode: &str,
        shop_side: &str,
        currency: &str,
        thb_amount: &str,
        cost_basis_thb: Option<&str>,
        realized_margin_thb: Option<&str>,
    ) -> (Uuid, Uuid) {
        let tx_id = Uuid::new_v4();
        let line_id = Uuid::new_v4();
        let recorded_at = date.and_hms_opt(10, 0, 0).unwrap().and_utc();
        let customer_id = format!("cust-{}", unique("c"));

        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2::uuid, $3, 'Test Customer', 'LIVE', now()) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(&customer_id)
        .bind(ORG)
        .bind(unique("no"))
        .execute(&self.pool)
        .await
        .expect("insert customer");

        let shift_id = if mode == "LIVE" {
            let existing: Option<String> = sqlx::query_scalar(
                "SELECT id FROM shifts WHERE branch_id = $1 AND status = 'OPEN' LIMIT 1",
            )
            .bind(branch)
            .fetch_optional(&self.pool)
            .await
            .expect("query existing shift");

            match existing {
                Some(id) => Some(id),
                None => Some(self.open_shift(branch).await),
            }
        } else {
            None
        };

        sqlx::query(
            "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, customer_id, \
                 customer_name, nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
                 recorded_by, completed_at, status, organization_id) VALUES ($1, $2, $3, $4, $5, $6, 'hash', $7, 'Test Customer', 'THA', 'staff', '', '', $8, $9, \
                 $9, 'staff', $9, 'COMPLETED', $10::uuid)",
        )
        .bind(tx_id)
        .bind(unique("TX"))
        .bind(branch)
        .bind(mode)
        .bind(shift_id)
        .bind(unique("sub"))
        .bind(customer_id)
        .bind(date)
        .bind(recorded_at)
        .bind(ORG)
        .execute(&self.pool)
        .await
        .expect("insert transaction");

        sqlx::query(
            "INSERT INTO transaction_lines ( \
                 id, organization_id, transaction_id, quote_line_id, shop_side, currency, \
                 denomination_id, quantity, unit_size, rate_snapshot, reference_rate, \
                 thb_amount, fee_thb, settle_thb, cost_basis_thb, realized_margin_thb, \
                 line_no, created_at, updated_at \
             ) VALUES ( \
                 $1, $2::uuid, $3, NULL, $4, $5, \
                 NULL, 100.0, 1.0, 1.0, NULL, \
                 $6::numeric, 0.00, $6::numeric, $7::numeric, $8::numeric, \
                 1, $9, $9 \
             )",
        )
        .bind(line_id)
        .bind(ORG)
        .bind(tx_id)
        .bind(shop_side)
        .bind(currency)
        .bind(thb_amount)
        .bind(cost_basis_thb)
        .bind(realized_margin_thb)
        .bind(recorded_at)
        .execute(&self.pool)
        .await
        .expect("insert transaction_line");

        (tx_id, line_id)
    }

    /// Inserts a POSTED reversal for `original_tx_id` on `business_date`.
    async fn insert_posted_reversal(
        &self,
        branch: &str,
        original_tx_id: Uuid,
        business_date: NaiveDate,
        reversed_margin_thb: &str,
    ) {
        let reversal_id = Uuid::new_v4();
        let posted_at = business_date.and_hms_opt(12, 0, 0).unwrap().and_utc();

        let file_key = format!("file-{}", unique("receipt"));
        sqlx::query(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ($1, $2, 'test', 'application/pdf', 'sha', now())",
        )
        .bind(&file_key)
        .bind(branch)
        .execute(&self.pool)
        .await
        .expect("insert file");

        let receipt_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, document_ref, kind, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, $4, 2, $5, $6, 'REVERSAL', $5, $5)",
        )
        .bind(receipt_id)
        .bind(ORG)
        .bind(original_tx_id)
        .bind(format!("REC-{}", unique("no")))
        .bind(posted_at)
        .bind(&file_key)
        .execute(&self.pool)
        .await
        .expect("insert receipt");

        let approval_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, object_version_hash, policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES ($1, $2::uuid, 'TRANSACTION_REVERSAL', $3, $4, 'h', '1', '{}'::jsonb, 'test', $5, 'APPROVED', $6)",
        )
        .bind(approval_id)
        .bind(ORG)
        .bind(branch)
        .bind(original_tx_id.to_string())
        .bind(posted_at)
        .bind(unique("appr_key"))
        .execute(&self.pool)
        .await
        .expect("insert approval");

        let posting_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, source_key, status, posted_at, posted_by) \
             VALUES ($1, $2::uuid, $3, 'REVERSAL', $4, 'reversal', 'POSTED', $5, 'finance')",
        )
        .bind(posting_id)
        .bind(ORG)
        .bind(branch)
        .bind(original_tx_id.to_string())
        .bind(posted_at)
        .execute(&self.pool)
        .await
        .expect("ledger posting");

        sqlx::query(
            "INSERT INTO reversals ( \
                 id, organization_id, branch_id, original_transaction_id, status, reason, \
                 amount_thb, business_date, approval_request_id, posting_id, receipt_id, requested_by, requested_at, \
                 posted_by, posted_at, realized_margin_reversed_thb \
             ) VALUES ( \
                 $1, $2::uuid, $3, $4, 'POSTED', 'Customer return', \
                 100.00, $5, $6, $7, $8, 'staff', $9, \
                 'staff', $9, $10::numeric \
             )",
        )
        .bind(reversal_id)
        .bind(ORG)
        .bind(branch)
        .bind(original_tx_id)
        .bind(business_date)
        .bind(approval_id)
        .bind(posting_id)
        .bind(receipt_id)
        .bind(posted_at)
        .bind(reversed_margin_thb)
        .execute(&self.pool)
        .await
        .expect("insert reversal");
    }

    async fn finish(self) {
        let Room { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop test schema");
        pool.close().await;
    }
}

// ── §9 Behaviour Tests ──────────────────────────────────────────────────────

/// Row 1: a branch after an exchange and a drawer→safe move ->
/// `available_cash` equals Σ `expected_in(Drawer)` + `expected_in(Safe)`
#[tokio::test]
async fn available_cash_after_exchange_and_drawer_to_safe_move_equals_sum_expected_in() {
    let room = Room::start().await;
    let b1 = BranchId::new(B1);
    let shift = room.open_shift(B1).await;

    // Seed Drawer with 50,000 THB from Bank (Opening float)
    let src1 = SourceRef {
        source_type: SourceType::Opening,
        source_id: unique("seed"),
        source_key: "float".to_string(),
    };
    room.post(vec![PostingRequest {
        from: CashPlace::Bank,
        to: CashPlace::Drawer(shift.clone()),
        currency: thb(),
        quantity: ForeignAmount::from_decimal(d("50000.00")),
        source: src1,
        line: None,
        reverses: None,
    }])
    .await;

    // Exchange: Customer brings 100 USD, receives 3,500 THB from Drawer
    let src_ex = SourceRef {
        source_type: SourceType::Exchange,
        source_id: unique("exchange"),
        source_key: "complete".to_string(),
    };
    room.post(vec![
        PostingRequest {
            from: CashPlace::Customer,
            to: CashPlace::Drawer(shift.clone()),
            currency: usd(),
            quantity: ForeignAmount::from_decimal(d("100.00")),
            source: src_ex.clone(),
            line: None,
            reverses: None,
        },
        PostingRequest {
            from: CashPlace::Drawer(shift.clone()),
            to: CashPlace::Customer,
            currency: thb(),
            quantity: ForeignAmount::from_decimal(d("3500.00")),
            source: src_ex,
            line: None,
            reverses: None,
        },
    ])
    .await;

    // Move 20,000 THB from Drawer to Safe
    let src2 = SourceRef {
        source_type: SourceType::Adjustment,
        source_id: unique("move"),
        source_key: "drawer_to_safe".to_string(),
    };
    room.post(vec![PostingRequest {
        from: CashPlace::Drawer(shift.clone()),
        to: CashPlace::Safe(b1.clone()),
        currency: thb(),
        quantity: ForeignAmount::from_decimal(d("20000.00")),
        source: src2,
        line: None,
        reverses: None,
    }])
    .await;

    // Read expected balances through CashPostings::expected_in
    let postings = LedgerCashPostings::new();
    let mut tx = room.pool.begin().await.unwrap();
    let drawer_bal = postings
        .expected_in(&mut tx, &CashPlace::Drawer(shift))
        .await
        .expect("expected_in drawer");
    let safe_bal = postings
        .expected_in(&mut tx, &CashPlace::Safe(b1.clone()))
        .await
        .expect("expected_in safe");
    tx.rollback().await.unwrap();

    let expected_drawer_thb = Decimal::new(drawer_bal.get(thb()).try_minor().unwrap(), 2);
    let expected_safe_thb = Decimal::new(safe_bal.get(thb()).try_minor().unwrap(), 2);
    let expected_total = expected_drawer_thb + expected_safe_thb;
    assert_eq!(expected_drawer_thb, d("26500.00"));
    assert_eq!(expected_safe_thb, d("20000.00"));
    assert_eq!(expected_total, d("46500.00"));

    // Compare with LedgerCashFacts::available_cash
    let facts = LedgerCashFacts::new(room.pool.clone());
    let available = facts.available_cash(&b1).await.expect("available_cash");
    assert_eq!(
        available, expected_total,
        "available_cash must equal Σ expected_in(Drawer) + expected_in(Safe)"
    );

    room.finish().await;
}

/// Row 2: a BRANCH transfer B1 -> B2 dispatched, not received ->
/// B1's `in_transit` carries it; B2's does not
#[tokio::test]
async fn transfer_dispatched_not_received_carried_by_sender_in_transit_only() {
    let room = Room::start().await;
    let b1 = BranchId::new(B1);
    let b2 = BranchId::new(B2);

    // Seed B1 Safe with 100,000 THB
    let src = SourceRef {
        source_type: SourceType::Opening,
        source_id: unique("seed"),
        source_key: "safe".to_string(),
    };
    room.post(vec![PostingRequest {
        from: CashPlace::Bank,
        to: CashPlace::Safe(b1.clone()),
        currency: thb(),
        quantity: ForeignAmount::from_decimal(d("100000.00")),
        source: src,
        line: None,
        reverses: None,
    }])
    .await;

    // Dispatch 30,000 THB from B1 to B2
    let _transfer_id = room.dispatch_transfer(B1, B2, "30000.00").await;

    let facts = LedgerCashFacts::new(room.pool.clone());
    let b1_transit = facts.in_transit(&b1).await.expect("b1 in_transit");
    let b2_transit = facts.in_transit(&b2).await.expect("b2 in_transit");

    assert_eq!(
        b1_transit,
        d("30000.00"),
        "sender B1 carries transfer in transit"
    );
    assert_eq!(
        b2_transit,
        Decimal::ZERO,
        "recipient B2 does not carry transfer in transit before receipt"
    );

    room.finish().await;
}

/// Row 3: received -> moves to B2's on hand; B1's in transit is empty
#[tokio::test]
async fn transfer_received_moves_to_recipient_on_hand_and_sender_in_transit_empty() {
    let room = Room::start().await;
    let b1 = BranchId::new(B1);
    let b2 = BranchId::new(B2);

    // Seed B1 Safe with 100,000 THB
    let src = SourceRef {
        source_type: SourceType::Opening,
        source_id: unique("seed"),
        source_key: "safe".to_string(),
    };
    room.post(vec![PostingRequest {
        from: CashPlace::Bank,
        to: CashPlace::Safe(b1.clone()),
        currency: thb(),
        quantity: ForeignAmount::from_decimal(d("100000.00")),
        source: src,
        line: None,
        reverses: None,
    }])
    .await;

    // Dispatch and then receive 30,000 THB from B1 to B2
    let transfer_id = room.dispatch_transfer(B1, B2, "30000.00").await;
    room.receive_transfer(&transfer_id, B2, "30000.00").await;

    let facts = LedgerCashFacts::new(room.pool.clone());
    let b1_transit = facts.in_transit(&b1).await.expect("b1 in_transit");
    let b2_available = facts.available_cash(&b2).await.expect("b2 available_cash");

    assert_eq!(
        b1_transit,
        Decimal::ZERO,
        "sender B1's in_transit is empty after receipt"
    );
    assert_eq!(
        b2_available,
        d("30000.00"),
        "received transfer moves to recipient B2's available cash (Safe)"
    );

    room.finish().await;
}

/// Row 4: a SELL with margin 120.00 and a BUY -> `realized_margin` = 120.00
#[tokio::test]
async fn realized_margin_sell_with_margin_and_buy() {
    let room = Room::start().await;
    let b1 = BranchId::new(B1);
    let date = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();

    // SELL with margin 120.00 (sales 3,200.00, cost 3,080.00)
    room.insert_transaction_with_line(
        B1,
        date,
        "LIVE",
        "SELL",
        "USD",
        "3200.00",
        Some("3080.00"),
        Some("120.00"),
    )
    .await;

    // BUY with no margin (NULL)
    room.insert_transaction_with_line(B1, date, "LIVE", "BUY", "USD", "3100.00", None, None)
        .await;

    let filter = ExecutiveFilter {
        from: Some(date),
        to: Some(date),
        currency: None,
        channel: None,
    };

    let facts = LedgerCashFacts::new(room.pool.clone());
    let margin = facts
        .realized_margin(&[b1], &filter)
        .await
        .expect("realized_margin");

    assert_eq!(
        margin,
        Ok(Money::from_minor(12000).unwrap()),
        "realized margin must equal 120.00 THB"
    );

    room.finish().await;
}

/// Row 5: that SELL reversed in the period -> `realized_margin` = 0.00
#[tokio::test]
async fn realized_margin_sell_reversed_in_period_is_zero() {
    let room = Room::start().await;
    let b1 = BranchId::new(B1);
    let date = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();

    // SELL with margin 120.00
    let (tx_id, _) = room
        .insert_transaction_with_line(
            B1,
            date,
            "LIVE",
            "SELL",
            "USD",
            "3200.00",
            Some("3080.00"),
            Some("120.00"),
        )
        .await;

    // BUY with no margin
    room.insert_transaction_with_line(B1, date, "LIVE", "BUY", "USD", "3100.00", None, None)
        .await;

    // Reverse the SELL in the period
    // Production stores minus the margin (reversal.md:83,156; reversal_post.rs:384). Fixture corrected by the
    // ticket writer claude/b61d81bb (was +120.00, which hid the sign bug).
    room.insert_posted_reversal(B1, tx_id, date, "-120.00")
        .await;

    let filter = ExecutiveFilter {
        from: Some(date),
        to: Some(date),
        currency: None,
        channel: None,
    };

    let facts = LedgerCashFacts::new(room.pool.clone());
    let margin = facts
        .realized_margin(&[b1], &filter)
        .await
        .expect("realized_margin");

    assert_eq!(
        margin,
        Ok(Money::from_minor(0).unwrap()),
        "reversed SELL in period leaves net margin 0.00 THB"
    );

    room.finish().await;
}

/// Row 6: a DEMO exchange -> not counted
#[tokio::test]
async fn demo_exchange_not_counted_in_realized_margin() {
    let room = Room::start().await;
    let b1 = BranchId::new(B1);
    let date = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();

    // DEMO SELL with margin 500.00
    room.insert_transaction_with_line(
        B1,
        date,
        "DEMO",
        "SELL",
        "USD",
        "3500.00",
        Some("3000.00"),
        Some("500.00"),
    )
    .await;

    // LIVE SELL with margin 100.00
    room.insert_transaction_with_line(
        B1,
        date,
        "LIVE",
        "SELL",
        "USD",
        "1850.00",
        Some("1750.00"),
        Some("100.00"),
    )
    .await;

    let filter = ExecutiveFilter {
        from: Some(date),
        to: Some(date),
        currency: None,
        channel: None,
    };

    let facts = LedgerCashFacts::new(room.pool.clone());
    let margin = facts
        .realized_margin(std::slice::from_ref(&b1), &filter)
        .await
        .expect("realized_margin");

    assert_eq!(
        margin,
        Ok(Money::from_minor(10000).unwrap()),
        "DEMO exchange is not counted: margin must be 100.00 THB"
    );

    // Also: when ONLY a DEMO transaction exists in the period, answers AwaitsLedger
    let other_date = NaiveDate::from_ymd_opt(2026, 10, 13).unwrap();
    room.insert_transaction_with_line(
        B1,
        other_date,
        "DEMO",
        "SELL",
        "USD",
        "2000.00",
        Some("1800.00"),
        Some("200.00"),
    )
    .await;

    let demo_only_filter = ExecutiveFilter {
        from: Some(other_date),
        to: Some(other_date),
        currency: None,
        channel: None,
    };

    let demo_only_margin = facts
        .realized_margin(&[b1], &demo_only_filter)
        .await
        .expect("demo only realized_margin");

    assert_eq!(
        demo_only_margin,
        Err(MetricUnavailable::AwaitsLedger),
        "period with only DEMO completed lines answers AwaitsLedger"
    );

    room.finish().await;
}

/// Row 7: `on_hand_by_currency` for USD -> equals `inventory_positions.quantity`
#[tokio::test]
async fn on_hand_by_currency_usd_equals_inventory_positions_quantity() {
    let room = Room::start().await;
    let b1 = BranchId::new(B1);

    // Insert inventory_positions for USD with quantity 1234.5600
    sqlx::query(
        "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb, version) \
         VALUES ($1::uuid, $2, 'USD', 1234.5600, 40000.00, 1)",
    )
    .bind(ORG)
    .bind(B1)
    .execute(&room.pool)
    .await
    .expect("insert inventory_positions");

    let facts = LedgerCashFacts::new(room.pool.clone());
    let on_hand = facts
        .on_hand_by_currency(&b1)
        .await
        .expect("on_hand_by_currency");

    let usd_entry = on_hand.iter().find(|(c, _)| c.as_str() == "USD");
    assert!(
        usd_entry.is_some(),
        "USD must be present in on_hand_by_currency: {on_hand:?}"
    );
    assert_eq!(
        usd_entry.unwrap().1,
        d("1234.5600"),
        "on_hand_by_currency for USD must equal inventory_positions.quantity"
    );

    room.finish().await;
}

/// Row 8: A01 after the swap -> the overview's other figures are unchanged (EX-3's tests, unedited)
#[tokio::test]
async fn a01_overview_after_swap_other_figures_unchanged() {
    let room = Room::start().await;
    let b1 = BranchId::new(B1);
    let date = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();

    // Insert COMPLETED LIVE SELL transaction
    room.insert_transaction_with_line(
        B1,
        date,
        "LIVE",
        "SELL",
        "USD",
        "3200.00",
        Some("3080.00"),
        Some("120.00"),
    )
    .await;

    let head = BranchHead {
        id: b1.clone(),
        name: "Branch 1".to_string(),
        as_of: Some(Utc::now()),
    };

    let filter = ExecutiveFilter {
        from: Some(date),
        to: Some(date),
        currency: None,
        channel: None,
    };

    // Wire ExecutiveOverviewService with LedgerCashFacts
    let cash = Arc::new(LedgerCashFacts::new(room.pool.clone()));
    let service = ExecutiveOverviewService::new(
        Arc::new(WideTransactionFacts::new(room.pool.clone())),
        cash.clone(),
        Arc::new(PgAlertFacts::new(room.pool.clone(), cash)),
        Arc::new(BrokerNotConnected),
    );

    let scope = [b1];
    let overview = service
        .overview(&scope, &[head], &filter, &scope)
        .await
        .expect("overview after swap");

    // Other figures remain unchanged and intact
    assert_eq!(overview.branches.len(), 1);
    assert_eq!(overview.branches[0].head.id, scope[0]);
    assert_eq!(overview.branches[0].volume_thb, d("3200.00"));
    assert!(!overview.definitions.is_empty());

    room.finish().await;
}

/// A01's branch table reads every branch's cash in one call (`CashFacts::branch_cash`, BE controller
/// claude/7ca80c; up-front test by claude/d7aa64). For each requested branch it answers exactly what
/// `available_cash`, `in_transit` and `shift_status` answer for that branch alone; every requested branch is
/// a key.
#[tokio::test]
async fn branch_cash_answers_each_branchs_own_figures_like_the_single_reads() {
    let room = Room::start().await;
    let b1 = BranchId::new(B1);
    let b2 = BranchId::new(B2);

    room.post(vec![PostingRequest {
        from: CashPlace::Bank,
        to: CashPlace::Safe(b1.clone()),
        currency: thb(),
        quantity: ForeignAmount::from_decimal(d("100000.00")),
        source: SourceRef {
            source_type: SourceType::Opening,
            source_id: unique("seed"),
            source_key: "safe".to_string(),
        },
        line: None,
        reverses: None,
    }])
    .await;
    let _transfer = room.dispatch_transfer(B1, B2, "30000.00").await;
    let _open = room.open_shift(B2).await;

    let today = Utc::now().date_naive();
    let filter = ExecutiveFilter {
        from: Some(today),
        to: Some(today),
        currency: None,
        channel: None,
    };
    let facts = LedgerCashFacts::new(room.pool.clone());
    let branches = vec![b1.clone(), b2.clone()];
    let all = facts
        .branch_cash(&branches, &filter)
        .await
        .expect("the batched read");

    assert_eq!(all.len(), 2, "every requested branch is a key");
    for b in &branches {
        let got = &all[b];
        assert_eq!(
            got.available,
            facts.available_cash(b).await.expect("available_cash"),
            "{b}: available"
        );
        assert_eq!(
            got.in_transit,
            facts.in_transit(b).await.expect("in_transit"),
            "{b}: in transit"
        );
        assert_eq!(
            got.shifts,
            facts.shift_status(b, &filter).await.expect("shift_status"),
            "{b}: shifts"
        );
    }
    assert_eq!(
        all[&b1].in_transit,
        d("30000.00"),
        "the sender carries the transfer"
    );
    assert_eq!(all[&b2].shifts.open, 1, "B2's open shift is counted");

    room.finish().await;
}

/// A01's branch table reads every branch's unfinished LIVE transactions in one call
/// (`ExchangeFacts::unfinished_by_branch`, BE controller claude/7ca80c; up-front test by claude/d7aa64).
/// For each requested branch it answers exactly what `unfinished` answers for that branch alone, through both
/// implementations the overview can be wired with (`BookingFacts` delegates to `WideTransactionFacts`).
#[tokio::test]
async fn unfinished_by_branch_answers_each_branchs_own_count_like_the_single_read() {
    let room = Room::start().await;
    let b1 = BranchId::new(B1);
    let b2 = BranchId::new(B2);
    let today = Utc::now().date_naive();

    // B1: two unfinished LIVE (CONFIRMED, DRAFT), one COMPLETED, one unfinished DEMO (not counted).
    // B2: one unfinished LIVE (PENDING_APPROVAL).
    let mut set_status = Vec::new();
    for (branch, mode, status) in [
        (B1, "LIVE", "CONFIRMED"),
        (B1, "LIVE", "DRAFT"),
        (B1, "LIVE", "COMPLETED"),
        (B1, "DEMO", "CONFIRMED"),
        (B2, "LIVE", "PENDING_APPROVAL"),
    ] {
        let (tx, _) = room
            .insert_transaction_with_line(branch, today, mode, "BUY", "USD", "3200.00", None, None)
            .await;
        set_status.push((tx, status));
    }
    for (tx, status) in set_status {
        if status != "COMPLETED" {
            sqlx::query(
                "UPDATE transactions SET status = $2, completed_at = NULL, document_ref = NULL WHERE id = $1",
            )
            .bind(tx)
            .bind(status)
            .execute(&room.pool)
            .await
            .expect("move the exchange to an unfinished status");
        }
    }

    let branches = vec![b1.clone(), b2.clone()];
    let wide = WideTransactionFacts::new(room.pool.clone());
    let booking = BookingFacts::new(room.pool.clone());
    for (label, facts) in [
        ("wide", &wide as &dyn ExchangeFacts),
        ("booking", &booking as &dyn ExchangeFacts),
    ] {
        let all = facts
            .unfinished_by_branch(&branches)
            .await
            .expect("the batched read");
        assert_eq!(all.len(), 2, "{label}: every requested branch is a key");
        for b in &branches {
            assert_eq!(
                all[b],
                facts.unfinished(b).await.expect("unfinished"),
                "{label} {b}: equals the single read"
            );
        }
        assert_eq!(
            all[&b1], 2,
            "{label}: B1 counts its two unfinished LIVE exchanges only"
        );
        assert_eq!(all[&b2], 1, "{label}: B2 counts its own");
    }

    room.finish().await;
}
