//! Up-front tests for the executive realized-margin sign bug (ticket: "A01 shows a reversed SELL's margin twice").
//! Written by the ticket writer claude/b61d81bb on the xchange BE controller claude/1656900d's order.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! A posted reversal stores `reversals.realized_margin_reversed_thb` as **minus** the original lines' margin
//! (`docs/design/blueprint/reversal.md:83,156`; written by
//! `api/src/application/workflows/exchange/reversal_post.rs:384` as `-margin_minor`; pinned by
//! `api/tests/rv3_reversal_post_test.rs:742` `== -margin`). These tests seed the column the way production writes
//! it. The helpers below are copied from `executive_ledger_cash_facts_test.rs` (that file says not to edit it).
//!
//! Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips. Each test works in a schema of its own.
#![allow(dead_code, unused_imports)]

mod common;

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

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

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
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

fn period(date: NaiveDate) -> ExecutiveFilter {
    ExecutiveFilter {
        from: Some(date),
        to: Some(date),
        currency: None,
        channel: None,
    }
}

/// R1: a SELL with margin 120.00 reversed in the same period, the reversal stored as production stores it
/// (-120.00) -> realized margin 0.00 (the reversal cancels the margin; it does not add it again).
#[tokio::test]
async fn a_sell_reversed_in_the_period_leaves_no_margin() {
    let room = Room::start().await;
    let date = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
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
    room.insert_posted_reversal(B1, tx_id, date, "-120.00")
        .await;

    let margin = LedgerCashFacts::new(room.pool.clone())
        .realized_margin(&[BranchId::new(B1)], &period(date))
        .await
        .expect("realized_margin");
    assert_eq!(
        margin,
        Ok(Money::from_minor(0).unwrap()),
        "120.00 - 120.00 reversed = 0.00"
    );
    room.finish().await;
}

/// R2: two SELLs (margin 120.00 and 50.00), the first reversed in the same period -> 50.00.
#[tokio::test]
async fn only_the_unreversed_sells_margin_remains() {
    let room = Room::start().await;
    let date = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    let (first, _) = room
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
    room.insert_transaction_with_line(
        B1,
        date,
        "LIVE",
        "SELL",
        "USD",
        "1000.00",
        Some("950.00"),
        Some("50.00"),
    )
    .await;
    room.insert_posted_reversal(B1, first, date, "-120.00")
        .await;

    let margin = LedgerCashFacts::new(room.pool.clone())
        .realized_margin(&[BranchId::new(B1)], &period(date))
        .await
        .expect("realized_margin");
    assert_eq!(
        margin,
        Ok(Money::from_minor(5000).unwrap()),
        "170.00 - 120.00 reversed = 50.00"
    );
    room.finish().await;
}

/// R3: no reversal -> the lines' margin as it is (unchanged behaviour).
#[tokio::test]
async fn without_a_reversal_the_margin_is_the_lines_sum() {
    let room = Room::start().await;
    let date = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
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

    let margin = LedgerCashFacts::new(room.pool.clone())
        .realized_margin(&[BranchId::new(B1)], &period(date))
        .await
        .expect("realized_margin");
    assert_eq!(margin, Ok(Money::from_minor(12000).unwrap()));
    room.finish().await;
}
