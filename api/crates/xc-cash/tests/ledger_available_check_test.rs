//! Up-front tests for SR-6 #3550 (B05 D-14, owner's O-3 A; design `docs/design/blueprint/stock-reservation.md`
//! §4.2 S18–S19): `LedgerCashPostings` refuses a disposal that would take a position into **another quote's
//! hold**, not only below zero. Written by the B05 sub-controller claude/6530d490 on `feat/b05-sr6` before the
//! implementation. **Do not edit, delete or weaken.**
//!
//! A completion consumes its own reservation before it posts (SR-2, phase `Consume`), so at `post_in` every
//! active `RESERVED` row of the (organisation, branch, currency) belongs to *other* quotes. The rule is
//! `position.quantity − active_reserved < q` ⇒ `stockReserved` (#3819; was `stockInsufficient`), and nothing is written. An APPROVED
//! `NEGATIVE_STOCK` (N-1) still lets a posting go below zero, but never into another quote's hold.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Each test works in a fresh schema
//! with every migration applied.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Duration, Utc};
use pos_error::AppError;
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Row};
use uuid::Uuid;

use xc_approval::domain::approval_repository::ApprovalRepository;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_cash::application::ledger_approval::negative_stock::NegativeStockSource;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{
    ApprovalRequestId, CashPlace, CashPostings, PostingRequest, TransactionLineId,
};
use xc_cash::domain::reservation::port::{ReserveLine, StockReservations};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_cash::infrastructure::stock_reservations::PgStockReservations;
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

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a valid decimal")
}

fn fa(s: &str) -> ForeignAmount {
    ForeignAmount::from_decimal(d(s))
}

fn usd() -> CurrencyCode {
    CurrencyCode::parse("USD").unwrap()
}

fn thb() -> CurrencyCode {
    CurrencyCode::parse("THB").unwrap()
}

fn migrations_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}

/// A fresh schema, one branch and one OPEN shift, funded with 3,200.00 THB in its drawer and then
/// 100 USD bought into it at 32.00 (position 100 / 3,200.00).
struct Lot {
    pool: PgPool,
    branch: BranchId,
    shift: String,
    postings: LedgerCashPostings,
}

impl Lot {
    async fn start() -> Lot {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!("TEST_DATABASE_URL is not set. Point it at a database ending in `_test`.")
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        assert!(
            options.get_database().unwrap_or("").ends_with("_test"),
            "TEST_DATABASE_URL must name a database ending in `_test`"
        );
        let schema = unique("b05_sr6");
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
            .max_connections(6)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        Migrator::new(migrations_dir().as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply migrations");

        let branch = BranchId::new(unique("B"));
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1)")
            .bind(branch.as_str())
            .execute(&pool)
            .await
            .expect("insert a branch");
        let teller = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2::uuid, 'Test Teller', 'ACTIVE', 'test')",
        )
        .bind(teller)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("insert user");
        let shift = unique("shift");
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', now(), 'system', $3, $4)",
        )
        .bind(&shift)
        .bind(branch.as_str())
        .bind(unique("open-req"))
        .bind(teller)
        .execute(&pool)
        .await
        .expect("insert shift");

        let lot = Lot {
            pool,
            branch,
            shift,
            postings: LedgerCashPostings::new(),
        };
        lot.fund_and_buy_100_usd().await;
        lot
    }

    fn src(&self, kind: SourceType, id: &str, key: &str) -> SourceRef {
        SourceRef {
            source_type: kind,
            source_id: id.to_string(),
            source_key: key.to_string(),
        }
    }

    async fn fund_and_buy_100_usd(&self) {
        let mut tx = self.pool.begin().await.unwrap();
        let fund = self.src(SourceType::Transfer, &unique("fund"), "external-in");
        let open = self.src(SourceType::Opening, &unique("open"), "opening");
        let buy = self.src(SourceType::Exchange, &unique("seed-buy"), "complete");
        let legs: Vec<(Vec<PostingRequest>, &str)> = vec![
            (
                vec![
                    PostingRequest {
                        from: CashPlace::Customer,
                        to: CashPlace::Bank,
                        currency: thb(),
                        quantity: fa("3200.00"),
                        source: fund.clone(),
                        line: None,
                        reverses: None,
                    },
                    PostingRequest {
                        from: CashPlace::Bank,
                        to: CashPlace::Safe(self.branch.clone()),
                        currency: thb(),
                        quantity: fa("3200.00"),
                        source: fund,
                        line: None,
                        reverses: None,
                    },
                ],
                "fund the safe",
            ),
            (
                vec![PostingRequest {
                    from: CashPlace::Safe(self.branch.clone()),
                    to: CashPlace::Drawer(self.shift.clone()),
                    currency: thb(),
                    quantity: fa("3200.00"),
                    source: open,
                    line: None,
                    reverses: None,
                }],
                "open the drawer",
            ),
            (
                vec![
                    PostingRequest {
                        from: CashPlace::Customer,
                        to: CashPlace::Drawer(self.shift.clone()),
                        currency: usd(),
                        quantity: fa("100"),
                        source: buy.clone(),
                        line: Some(Uuid::new_v4()),
                        reverses: None,
                    },
                    PostingRequest {
                        from: CashPlace::Drawer(self.shift.clone()),
                        to: CashPlace::Customer,
                        currency: thb(),
                        quantity: fa("3200.00"),
                        source: buy,
                        line: None,
                        reverses: None,
                    },
                ],
                "buy 100 USD",
            ),
        ];
        for (reqs, what) in legs {
            self.postings
                .post_in(&mut tx, &reqs, None)
                .await
                .unwrap_or_else(|e| panic!("{what}: {e}"));
        }
        tx.commit().await.unwrap();
    }

    /// Another quote's SELL line of `quantity` USD, reserved through SR-1's store at `now`, its quote expiring
    /// at `expires_at`.
    async fn hold(&self, quantity: &str, expires_at: chrono::DateTime<Utc>) {
        let (quote_id, pub_id, req_id, rate_line_id, line_id) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let user: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique("user"))
        .fetch_one(&self.pool)
        .await
        .expect("insert user");
        let b = self.branch.as_str();
        sqlx::query(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
             object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
             status, idempotency_key) VALUES ($1, $2::uuid, 'RATE_PUBLICATION', $3, $4, 'test', '1', \
             '{}'::jsonb, $5, now(), 'APPROVED', $6)",
        )
        .bind(req_id)
        .bind(ORG)
        .bind(b)
        .bind(pub_id)
        .bind(&user)
        .bind(format!("k-{req_id}"))
        .execute(&self.pool)
        .await
        .expect("insert approval request");
        sqlx::query(
            "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, \
             draft_id, published_by) VALUES ($1, $2, \
             (SELECT COALESCE(max(version), 0) + 1 FROM rate_publications WHERE branch_id = $2), now(), $3, \
             NULL, $4)",
        )
        .bind(pub_id)
        .bind(b)
        .bind(req_id)
        .bind(&user)
        .execute(&self.pool)
        .await
        .expect("insert rate publication");
        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, \
             unit_size, buy_rate, sell_rate, source_snapshot) \
             VALUES ($1, $2, 'USD', 'ALL', 1.0, 34.0, 35.0, '{}'::jsonb)",
        )
        .bind(rate_line_id)
        .bind(pub_id)
        .execute(&self.pool)
        .await
        .expect("insert published rate line");
        sqlx::query(
            "INSERT INTO quotes (id, organization_id, branch_id, type, channel, status, publication_id, \
             expires_at, created_by, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, 'INDICATIVE', 'DESK', 'ACTIVE', $4, $5, 'test', now(), now())",
        )
        .bind(quote_id)
        .bind(ORG)
        .bind(b)
        .bind(pub_id)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .expect("insert quote");
        sqlx::query(
            "INSERT INTO quote_lines (id, organization_id, quote_id, published_rate_line_id, shop_side, \
             currency, denomination_group, quantity, unit_size, base_rate, final_rate, thb_amount, \
             fee_thb, settle_thb, line_no, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, $4, 'SELL', 'USD', 'ALL', $5::numeric, 1.0, 35.0, 35.0, 0, 0, 0, 1, \
             now(), now())",
        )
        .bind(line_id)
        .bind(ORG)
        .bind(quote_id)
        .bind(rate_line_id)
        .bind(quantity)
        .execute(&self.pool)
        .await
        .expect("insert quote line");
        let mut tx = self.pool.begin().await.unwrap();
        PgStockReservations::new()
            .reserve_in(
                &mut tx,
                &ReserveLine {
                    quote_line_id: line_id,
                    denomination_id: None,
                },
                expires_at - Duration::seconds(300),
            )
            .await
            .unwrap_or_else(|e| panic!("reserve {quantity}: {e}"));
        tx.commit().await.unwrap();
    }

    /// A completed SELL transaction line of `qty` USD (the line a completion posts); answers the line and
    /// its transaction id (the posting's source).
    async fn sell_line(&self, qty: &str) -> (TransactionLineId, String) {
        let cust = unique("CU");
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ($1, $2::uuid, $3, 'Somchai', 'LIVE', now())",
        )
        .bind(&cust)
        .bind(ORG)
        .bind(unique("CUSTNO"))
        .execute(&self.pool)
        .await
        .expect("seed customer");
        let (tx_id, line_id) = (Uuid::new_v4(), Uuid::new_v4());
        sqlx::query(
            "INSERT INTO transactions (id, organization_id, document_ref, branch_id, mode, shift_id, \
             submission_id, payload_hash, customer_id, customer_name, nationality, staff_id, booking_no, \
             notes, value_date, paid_at, recorded_at, recorded_by, status, completed_at) \
             VALUES ($1, $2::uuid, $3, $4, 'LIVE', $5, $6, 'hash', $7, 'Somchai', 'THA', 'system', \
             'WALK-IN', '', current_date, now(), now(), 'system', 'COMPLETED', now())",
        )
        .bind(tx_id)
        .bind(ORG)
        .bind(unique("DOC"))
        .bind(self.branch.as_str())
        .bind(&self.shift)
        .bind(unique("sub"))
        .bind(&cust)
        .execute(&self.pool)
        .await
        .expect("insert transaction");
        sqlx::query(
            "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
             quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, \
             updated_at) VALUES ($1, $2::uuid, $3, 'SELL', 'USD', $4::numeric, 1, 35.0, \
             $4::numeric * 35, 0, $4::numeric * 35, 1, now(), now())",
        )
        .bind(line_id)
        .bind(ORG)
        .bind(tx_id)
        .bind(qty)
        .execute(&self.pool)
        .await
        .expect("insert transaction line");
        (line_id, tx_id.to_string())
    }

    fn sell_legs(&self, line: TransactionLineId, tx_id: &str, qty: &str) -> [PostingRequest; 2] {
        let thb_in = (d(qty) * d("35")).to_string();
        let src = self.src(SourceType::Exchange, tx_id, "complete");
        [
            PostingRequest {
                from: CashPlace::Drawer(self.shift.clone()),
                to: CashPlace::Customer,
                currency: usd(),
                quantity: fa(qty),
                source: src.clone(),
                line: Some(line),
                reverses: None,
            },
            PostingRequest {
                from: CashPlace::Customer,
                to: CashPlace::Drawer(self.shift.clone()),
                currency: thb(),
                quantity: fa(&thb_in),
                source: src,
                line: None,
                reverses: None,
            },
        ]
    }

    /// An APPROVED `NEGATIVE_STOCK` for `tx_id` (N-1), as `ledger_postings_test::approval` makes it.
    async fn approved_negative_stock(&self, tx_id: &str) -> ApprovalRequestId {
        let repo = PgApprovalRepository::new(self.pool.clone());
        let mut tx = self.pool.begin().await.unwrap();
        let hash = NegativeStockSource::version_hash(&self.postings, &mut tx, tx_id)
            .await
            .expect("the source's version hash");
        let req = repo
            .insert_request(
                &mut tx,
                "NEGATIVE_STOCK",
                Some(self.branch.as_str().to_string()),
                tx_id,
                &hash,
                "v1",
                serde_json::json!({}),
                "teller",
                &unique("key"),
            )
            .await
            .expect("insert approval request");
        repo.update_status(&mut tx, &req.id, "APPROVED")
            .await
            .expect("approve");
        tx.commit().await.unwrap();
        ApprovalRequestId::new(req.id.as_str())
    }

    /// Posts in its own transaction, committed on `Ok`.
    async fn post(
        &self,
        reqs: &[PostingRequest],
        allow_negative: Option<ApprovalRequestId>,
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await.unwrap();
        let got = self.postings.post_in(&mut tx, reqs, allow_negative).await;
        match got {
            Ok(_) => {
                tx.commit().await.unwrap();
                Ok(())
            }
            Err(e) => {
                tx.rollback().await.unwrap();
                Err(e)
            }
        }
    }

    async fn usd_position(&self) -> Decimal {
        let row = sqlx::query(
            "SELECT quantity FROM inventory_positions WHERE branch_id = $1 AND currency = 'USD'",
        )
        .bind(self.branch.as_str())
        .fetch_one(&self.pool)
        .await
        .expect("the USD position");
        row.get("quantity")
    }

    async fn postings_count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM ledger_postings")
            .fetch_one(&self.pool)
            .await
            .expect("count postings")
    }
}

fn is_stock_insufficient(got: &Result<(), AppError>) -> bool {
    matches!(got, Err(AppError::Domain(m)) if m.contains("stockInsufficient"))
}

/// #3819 (claude/a90ab16c's ruling): a refusal caused by another quote's hold answers its own code.
fn is_stock_reserved(got: &Result<(), AppError>) -> bool {
    matches!(got, Err(AppError::Domain(m)) if m.starts_with("stockReserved"))
}

#[tokio::test]
async fn s18_a_sale_into_another_quotes_hold_is_refused_and_writes_nothing() {
    let lot = Lot::start().await;
    lot.hold("70", Utc::now() + Duration::seconds(300)).await;
    let before = lot.postings_count().await;
    let (line, tx_id) = lot.sell_line("40").await;

    let got = lot.post(&lot.sell_legs(line, &tx_id, "40"), None).await;

    assert!(
        is_stock_reserved(&got),
        "100 on hand, 70 held: 40 > available 30 — {got:?}"
    );
    assert_eq!(lot.postings_count().await, before, "nothing is written");
    assert_eq!(lot.usd_position().await, d("100"));
}

#[tokio::test]
async fn s19_a_sale_within_available_posts() {
    let lot = Lot::start().await;
    lot.hold("70", Utc::now() + Duration::seconds(300)).await;
    let (line, tx_id) = lot.sell_line("30").await;

    lot.post(&lot.sell_legs(line, &tx_id, "30"), None)
        .await
        .unwrap_or_else(|e| panic!("30 = available: {e}"));

    assert_eq!(lot.usd_position().await, d("70"));
}

#[tokio::test]
async fn an_approved_negative_stock_never_reaches_into_another_quotes_hold() {
    let lot = Lot::start().await;
    lot.hold("70", Utc::now() + Duration::seconds(300)).await;
    let (line, tx_id) = lot.sell_line("40").await;
    let approved = lot.approved_negative_stock(&tx_id).await;

    let got = lot
        .post(&lot.sell_legs(line, &tx_id, "40"), Some(approved))
        .await;

    assert!(
        is_stock_reserved(&got),
        "N-1 is not a licence to take held stock — {got:?}"
    );
    assert_eq!(lot.usd_position().await, d("100"));
}

#[tokio::test]
async fn with_no_hold_an_approved_negative_stock_still_goes_below_zero() {
    let lot = Lot::start().await;
    let (line, tx_id) = lot.sell_line("110").await;
    let approved = lot.approved_negative_stock(&tx_id).await;

    lot.post(&lot.sell_legs(line, &tx_id, "110"), Some(approved))
        .await
        .unwrap_or_else(|e| panic!("N-1 unchanged when nothing is held: {e}"));

    assert_eq!(lot.usd_position().await, d("-10"));
}

#[tokio::test]
async fn an_expired_hold_does_not_block_a_sale() {
    let lot = Lot::start().await;
    lot.hold("70", Utc::now() - Duration::seconds(1)).await;
    let (line, tx_id) = lot.sell_line("90").await;

    lot.post(&lot.sell_legs(line, &tx_id, "90"), None)
        .await
        .unwrap_or_else(|e| panic!("an expired hold holds nothing (D-3): {e}"));

    assert_eq!(lot.usd_position().await, d("10"));
}

#[tokio::test]
async fn with_no_hold_a_sale_beyond_on_hand_is_refused_as_today() {
    let lot = Lot::start().await;
    let (line, tx_id) = lot.sell_line("101").await;

    let got = lot.post(&lot.sell_legs(line, &tx_id, "101"), None).await;

    assert!(is_stock_insufficient(&got), "§4.5 unchanged — {got:?}");
    assert_eq!(lot.usd_position().await, d("100"));
}
