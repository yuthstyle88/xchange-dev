//! Up-front tests for SR-3 (B05: release on cancel; design `docs/design/blueprint/stock-reservation.md` D-6, §4.2
//! S7 end to end). Written by the B05 sub-controller claude/6530d490 on `feat/b05-stock` before the
//! implementation. **Do not edit, delete or weaken.**
//!
//! A quote is cancelled in two places today, and both must release its holds in the same transaction:
//! `CancelTransaction::run_in` (`exchange/cancel.rs`) and a REJECTED approval (`exchange/approval.rs`,
//! `on_decided`). Stock comes back at once, not at `expires_at`.
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`), as every `tx4_*` test does.

mod quote_common;
mod tx4_common;

use chrono::{DateTime, Utc};
use quote_common::*;
use rust_decimal::Decimal;
use tx4_common::*;
use xc_approval::domain::request::Outcome;
use xc_cash::domain::reservation::port::{ReserveLine, StockReservations};
use xc_cash::infrastructure::stock_reservations::PgStockReservations;
use xc_exchange::domain::quote::quote::Quote;
use xc_exchange::domain::transaction::transaction::{ExchangeTransaction, TransactionStatus};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Side};
use xchange::application::workflows::exchange::confirm::ConfirmOutcome;

const MANAGER: &str = "employee-manager";

/// A DRAFT selling 100 USD at MAIN whose quote line holds 100 of MAIN's 1000 USD.
async fn reserved_draft(desk: &Desk, now: DateTime<Utc>) -> (Quote, ExchangeTransaction) {
    let q = desk.quote("MAIN", Side::Sell, "100", None, now).await;
    desk.db
        .exec(&format!(
            "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb) \
             VALUES ('{}', 'MAIN', 'USD', 1000, 32000) \
             ON CONFLICT (organization_id, branch_id, currency) DO UPDATE SET quantity = 1000",
            q.organization_id
        ))
        .await;
    let mut tx = desk.db.pool.begin().await.unwrap();
    PgStockReservations::new()
        .reserve_in(
            &mut tx,
            &ReserveLine {
                quote_line_id: q.lines[0].id,
                denomination_id: None,
            },
            now,
        )
        .await
        .unwrap_or_else(|e| panic!("reserve_in: {e}"));
    tx.commit().await.unwrap();
    let t = desk.draft(&q, now).await;
    (q, t)
}

/// `status|released_reason` of the quote line's reservation rows.
async fn rows(desk: &Desk, q: &Quote) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT status || '|' || COALESCE(released_reason, '-') FROM stock_reservations \
          WHERE quote_line_id = $1 ORDER BY created_at, id",
    )
    .bind(q.lines[0].id)
    .fetch_all(&desk.db.pool)
    .await
    .expect("read stock_reservations")
}

async fn reserved(desk: &Desk, q: &Quote, now: DateTime<Utc>) -> Decimal {
    let mut conn = desk.db.pool.acquire().await.unwrap();
    PgStockReservations::new()
        .availability_in(
            &mut conn,
            q.organization_id,
            &BranchId::new("MAIN"),
            CurrencyCode::parse("USD").unwrap(),
            now,
        )
        .await
        .unwrap_or_else(|e| panic!("availability_in: {e}"))
        .reserved
}

#[tokio::test]
async fn s7_cancelling_the_exchange_releases_its_hold_at_once() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (q, t) = reserved_draft(&desk, now).await;
    assert_eq!(reserved(&desk, &q, now).await, Decimal::from(100));

    desk.cancel(t.id, "customer left", later(now, 5))
        .await
        .unwrap_or_else(|e| panic!("cancel: {e}"));

    assert_eq!(rows(&desk, &q).await, ["RELEASED|CANCELLED"]);
    assert_eq!(
        reserved(&desk, &q, later(now, 6)).await,
        Decimal::ZERO,
        "available before the quote's expires_at"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn a_refused_cancel_releases_nothing() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (q, t) = reserved_draft(&desk, now).await;

    let got = desk.cancel(t.id, "   ", later(now, 5)).await;

    assert!(got.is_err(), "a cancel needs a reason");
    assert_eq!(rows(&desk, &q).await, ["RESERVED|-"]);
    desk.db.finish().await;
}

#[tokio::test]
async fn a_rejected_approval_releases_the_quotes_hold() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let (q, t) = reserved_draft(&desk, now).await;
    desk.limit("MAIN", "1000.00").await;
    let request = match desk.confirm(t.id, later(now, 10)).await {
        Ok(ConfirmOutcome::PendingApproval(request)) => request,
        other => panic!("100 USD sold above 1000.00 THB waits for approval: {other:?}"),
    };

    desk.decide(
        &request,
        &staff_as(MANAGER, "MANAGER", "MAIN"),
        Outcome::Rejected,
        "not today",
    )
    .await
    .expect("the branch's manager rejects");

    assert_eq!(desk.stored(t.id).await.status, TransactionStatus::Cancelled);
    assert_eq!(rows(&desk, &q).await, ["RELEASED|CANCELLED"]);
    assert_eq!(reserved(&desk, &q, later(now, 11)).await, Decimal::ZERO);
    desk.db.finish().await;
}
