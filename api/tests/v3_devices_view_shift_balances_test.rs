//! Up-front tests for LG-6.r3 (#1042): the devices view reads shift_balances
//! (design `docs/design/blueprint/ledger-inventory.md` §4.2, §4.2.1; ticket #1042 §8–§9).
//!
//! Written before the implementation by the test-writer (gemini/pool-2, rule A).
//!
//! Every row of #1042 §9 is covered:
//! - Row 1: an open shift -> THB/USD = its expected from ShiftBalances (derived from ledger postings)
//! - Row 2: no open shift, last shift counted -> THB/USD = counted from ShiftBalances (from cash_counts / cash_count_totals)
//! - Row 3: no open shift, last shift not counted -> 0.00 both
//!
//! Needs `TEST_DATABASE_URL` naming an existing database ending in `_test`.

mod common;
mod compat_common;
mod publication_seed;
mod rates_re4_common;
mod rates_re4e_publish;
mod rpc_common;
mod snapshot_common;

use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Utc};
use rpc_common::{App, Call, Cookies, Desk, one_of_each_role, sign_in};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};

const ORIGIN: &str = "http://desk.test";
const MAIN: &str = "MAIN";

static NEXT: AtomicU64 = AtomicU64::new(100);
fn next_id() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

struct TestRoom {
    desk: Desk,
    app: App,
    cookies: Cookies,
    org: Uuid,
}

impl TestRoom {
    async fn open() -> Self {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let t0: DateTime<Utc> = snapshot_common::time("2026-10-06T05:00:00.000Z");
        desk.clock.set(t0);

        let org: Uuid =
            sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = 'MAIN'")
                .fetch_one(&desk.db.pool)
                .await
                .expect("read MAIN organization_id");

        Self {
            desk,
            app,
            cookies,
            org,
        }
    }

    async fn balances(&self, branch: &str) -> (String, String) {
        let path = format!("/api/v3/views/devices?branch={branch}");
        let mut call = Call::post(ORIGIN, &path, Vec::<u8>::new())
            .without("x-xchange-request")
            .without("content-type")
            .without("origin")
            .cookie(&self.cookies["OWNER"]);
        call.method = Method::GET;
        let answer = self.app.send(call).await;
        assert_eq!(answer.status, 200, "GET {path}: {}", answer.text);
        let body = answer.json();
        let thb = body["data"]["balanceThb"]
            .as_str()
            .unwrap_or_else(|| panic!("balanceThb in {body}"))
            .to_string();
        let usd = body["data"]["balanceUsd"]
            .as_str()
            .unwrap_or_else(|| panic!("balanceUsd in {body}"))
            .to_string();
        (thb, usd)
    }

    async fn seed_user(&self) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             VALUES ($1, $2, 'Test Teller', 'ACTIVE', 'test')",
        )
        .bind(id)
        .bind(self.org)
        .execute(&self.desk.db.pool)
        .await
        .expect("insert user");
        id
    }

    async fn create_shift(&self, branch: &str, status: &str) -> String {
        // If opening, close any existing open shift on this branch first
        if status == "OPEN" {
            sqlx::query(
                "UPDATE shifts SET status = 'CLOSED', closed_at = now() WHERE branch_id = $1 AND status = 'OPEN'",
            )
            .bind(branch)
            .execute(&self.desk.db.pool)
            .await
            .expect("close open shifts");
        }
        let teller_id = self.seed_user().await;
        let shift_id = format!("shift-{}", next_id());
        let open_req_id = format!("open-req-{}", next_id());
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ($1, $2, $3, now(), 'system', $4, $5)",
        )
        .bind(&shift_id)
        .bind(branch)
        .bind(status)
        .bind(open_req_id)
        .bind(teller_id)
        .execute(&self.desk.db.pool)
        .await
        .expect("insert shift");
        shift_id
    }

    async fn post_opening_to_drawer(
        &self,
        branch: &str,
        shift_id: &str,
        currency: &str,
        amount: &str,
    ) {
        self.post_movement_to_drawer(
            branch,
            shift_id,
            currency,
            amount,
            SourceType::Opening,
            "opening",
        )
        .await;
    }

    async fn post_transfer_to_drawer(
        &self,
        branch: &str,
        shift_id: &str,
        currency: &str,
        amount: &str,
    ) {
        self.post_movement_to_drawer(
            branch,
            shift_id,
            currency,
            amount,
            SourceType::Transfer,
            "transfer",
        )
        .await;
    }

    async fn post_movement_to_drawer(
        &self,
        branch: &str,
        shift_id: &str,
        currency: &str,
        amount: &str,
        source_type: SourceType,
        source_key: &str,
    ) {
        let mut tx = self.desk.db.pool.begin().await.expect("begin");
        let req = PostingRequest {
            from: CashPlace::Safe(BranchId::new(branch.to_owned())),
            to: CashPlace::Drawer(shift_id.to_owned()),
            currency: CurrencyCode::parse(currency).expect("valid currency"),
            quantity: ForeignAmount::from_decimal(
                Decimal::from_str(amount).expect("valid decimal"),
            ),
            source: SourceRef {
                source_type,
                source_id: format!("post-{}", next_id()),
                source_key: source_key.to_string(),
            },
            line: None,
            reverses: None,
        };
        LedgerCashPostings::new()
            .post_in(&mut tx, &[req], None)
            .await
            .expect("post to drawer");
        tx.commit().await.expect("commit");
    }

    async fn seed_verified_count(&self, branch: &str, shift_id: &str, totals: &[(&str, &str)]) {
        let counter = self.seed_user().await;
        let verifier = self.seed_user().await;
        let count_id: Uuid = sqlx::query_scalar(
            "INSERT INTO cash_counts (organization_id, branch_id, shift_id, kind, counted_by, counted_at, \
             verified_by, verified_at, idempotency_key) \
             VALUES ($1, $2, $3, 'CLOSE', $4, now(), $5, now(), $6) \
             RETURNING id",
        )
        .bind(self.org)
        .bind(branch)
        .bind(shift_id)
        .bind(counter)
        .bind(verifier)
        .bind(format!("count-{}", next_id()))
        .fetch_one(&self.desk.db.pool)
        .await
        .expect("insert cash_counts");

        for (curr, counted) in totals {
            sqlx::query(
                "INSERT INTO cash_count_totals (count_id, organization_id, currency, counted, expected, difference) \
                 VALUES ($1, $2, $3, $4, $4, 0)",
            )
            .bind(count_id)
            .bind(self.org)
            .bind(*curr)
            .bind(Decimal::from_str(counted).expect("valid decimal"))
            .execute(&self.desk.db.pool)
            .await
            .expect("insert cash_count_totals");
        }
    }
}

/// #1042 §9 Row 1: an open shift -> THB/USD = its expected from ShiftBalances (derived from ledger postings)
#[actix_web::test]
async fn test_open_shift_balance_is_expected_from_shift_balances() {
    let room = TestRoom::open().await;
    let shift_id = room.create_shift(MAIN, "OPEN").await;

    // Opening drawer balances: 50,000.00 THB and 1,500.00 USD
    room.post_opening_to_drawer(MAIN, &shift_id, "THB", "50000.00")
        .await;
    room.post_opening_to_drawer(MAIN, &shift_id, "USD", "1500.00")
        .await;

    // Non-opening movement (Transfer into drawer) so expected != opening (kills mutant M1)
    room.post_transfer_to_drawer(MAIN, &shift_id, "THB", "5000.00")
        .await;
    room.post_transfer_to_drawer(MAIN, &shift_id, "USD", "200.00")
        .await;

    // Expected: 50,000 + 5,000 = 55,000.00 THB; 1,500 + 200 = 1,700.00 USD.
    // Ensure shift_balances SQL table has no rows for this shift, representing the post-cutover
    // state where nothing writes shift_balances.
    let (thb, usd) = room.balances(MAIN).await;
    assert_eq!(
        thb, "55000.00",
        "open shift balance THB must equal expected from ledger postings"
    );
    assert_eq!(
        usd, "1700.00",
        "open shift balance USD must equal expected from ledger postings"
    );
}

/// #1042 §9 Row 2: no open shift, last shift counted -> THB/USD = counted from ShiftBalances (from cash_counts / cash_count_totals)
#[actix_web::test]
async fn test_no_open_shift_last_shift_counted_is_counted_from_shift_balances() {
    let room = TestRoom::open().await;
    let shift_id = room.create_shift(MAIN, "CLOSED").await;

    // Seed verified close count in cash_counts / cash_count_totals: 25,000.00 THB and 800.00 USD
    room.seed_verified_count(MAIN, &shift_id, &[("THB", "25000.00"), ("USD", "800.00")])
        .await;

    // Ensure shift_balances SQL table has no counted rows for this shift.
    let (thb, usd) = room.balances(MAIN).await;
    assert_eq!(
        thb, "25000.00",
        "closed shift balance THB must equal counted from verified cash_counts"
    );
    assert_eq!(
        usd, "800.00",
        "closed shift balance USD must equal counted from verified cash_counts"
    );
}

/// #1042 §9 Row 3: no open shift, last shift not counted -> 0.00 both
#[actix_web::test]
async fn test_no_open_shift_last_shift_not_counted_is_zero() {
    let room = TestRoom::open().await;
    let shift_id = room.create_shift(MAIN, "CLOSED").await;

    // Post to the closed, uncounted shift's drawer so expected is non-zero (kills mutant M3)
    room.post_opening_to_drawer(MAIN, &shift_id, "THB", "30000.00")
        .await;
    room.post_opening_to_drawer(MAIN, &shift_id, "USD", "1000.00")
        .await;

    // The shift has NO count in cash_counts / cash_count_totals.
    // Insert stale/legacy counted figures into shift_balances table to prove that the view
    // does not read from shift_balances table (which would show the stale 1234.00 / 56.00).
    sqlx::query(
        "INSERT INTO shift_balances (shift_id, code, opening, expected, counted, difference) \
         VALUES ($1, 'THB', 1000.00, 1000.00, 1234.00, 234.00), \
                ($1, 'USD', 50.00, 50.00, 56.00, 6.00)",
    )
    .bind(&shift_id)
    .execute(&room.desk.db.pool)
    .await
    .expect("insert stale shift_balances");

    let (thb, usd) = room.balances(MAIN).await;
    assert_eq!(
        thb, "0.00",
        "uncounted closed shift balance THB must be 0.00, ignoring non-zero expected and stale shift_balances"
    );
    assert_eq!(
        usd, "0.00",
        "uncounted closed shift balance USD must be 0.00, ignoring non-zero expected and stale shift_balances"
    );
}
