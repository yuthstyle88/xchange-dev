//! Up-front tests: cancelling an exchange must not cancel, or release the stock of, a quote that a
//! **different** exchange has accepted. Written before the fix by the xchange BE lane controller's
//! helper (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The defect (origin/main 3bac87e0a): create (`exchange/create.rs`) does not tie a quote to one
//! draft, so two drafts A and B can name the same quote. Confirming A accepts the quote for A.
//! Cancelling B then runs `quotes.cancel_in(quote)` and `release_for_quote_in(quote)`
//! unconditionally (`exchange/cancel.rs`), and the quote repository cancels ACTIVE *or* ACCEPTED
//! and clears `accepted_by_transaction_id` — the quote A holds becomes CANCELLED and its stock is
//! released while A still goes on to completion.
//!
//! Expected: cancelling B leaves a quote ACCEPTED by A as it is (status, holder, reservations);
//! B itself is CANCELLED. Cancelling the holder, or an exchange whose quote is still ACTIVE,
//! cancels the quote and releases its stock as today.
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`), as every `tx4_*` test does. The
//! drafts are stored through `tx4_common::Desk::draft`, which is how two drafts on one quote are
//! made here without depending on create's own guard.

mod quote_common;
mod tx4_common;

use chrono::{DateTime, Utc};
use quote_common::*;
use rust_decimal::Decimal;
use tx4_common::*;
use xc_cash::domain::reservation::port::{ReserveLine, StockReservations};
use xc_cash::infrastructure::stock_reservations::PgStockReservations;
use xc_exchange::domain::quote::quote::Quote;
use xc_exchange::domain::transaction::transaction::TransactionStatus;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Side};
use xchange::application::workflows::exchange::confirm::ConfirmOutcome;

/// A desk quote selling 100 USD at MAIN whose line holds 100 of MAIN's 1000 USD.
async fn reserved_quote(desk: &Desk, now: DateTime<Utc>) -> Quote {
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
    q
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

/// C1 — the defect: B's cancel must not touch the quote A accepted.
#[tokio::test]
async fn c1_cancelling_a_second_draft_leaves_the_quote_another_exchange_accepted() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let q = reserved_quote(&desk, now).await;
    let a = desk.draft(&q, now).await;
    let b = desk.draft(&q, now).await;

    assert_eq!(
        desk.confirm(a.id, later(now, 10))
            .await
            .expect("A confirms"),
        ConfirmOutcome::Confirmed
    );
    assert_eq!(desk.quote_row(&q).await, format!("ACCEPTED|{}", a.id.0));

    desk.cancel(b.id, "duplicate draft", later(now, 20))
        .await
        .unwrap_or_else(|e| panic!("B is cancelled: {e}"));

    assert_eq!(
        desk.stored(b.id).await.status,
        TransactionStatus::Cancelled,
        "B itself is cancelled"
    );
    assert_eq!(
        desk.quote_row(&q).await,
        format!("ACCEPTED|{}", a.id.0),
        "the quote A accepted stays ACCEPTED by A when B is cancelled"
    );
    assert_eq!(
        rows(&desk, &q).await,
        ["RESERVED|-"],
        "A's stock stays held when B is cancelled"
    );
    assert_eq!(
        reserved(&desk, &q, later(now, 21)).await,
        Decimal::from(100)
    );
    assert_eq!(
        desk.stored(a.id).await.status,
        TransactionStatus::Confirmed,
        "A is untouched and can still be completed"
    );
    desk.db.finish().await;
}

/// C2 — guard (passes today): a lone draft's cancel cancels its ACTIVE quote and releases it.
#[tokio::test]
async fn c2_cancelling_the_only_draft_cancels_its_active_quote_and_releases_it() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let q = reserved_quote(&desk, now).await;
    let t = desk.draft(&q, now).await;
    assert_eq!(desk.quote_row(&q).await, "ACTIVE|");

    desk.cancel(t.id, "customer left", later(now, 5))
        .await
        .unwrap_or_else(|e| panic!("cancel: {e}"));

    assert_eq!(desk.stored(t.id).await.status, TransactionStatus::Cancelled);
    assert_eq!(desk.quote_row(&q).await, "CANCELLED|");
    assert_eq!(rows(&desk, &q).await, ["RELEASED|CANCELLED"]);
    assert_eq!(reserved(&desk, &q, later(now, 6)).await, Decimal::ZERO);
    desk.db.finish().await;
}

/// C3 — guard (passes today): with two drafts on the quote, cancelling the holder A still cancels
/// the quote and releases its stock — the fix must not over-reach.
#[tokio::test]
async fn c3_cancelling_the_exchange_that_holds_the_quote_still_cancels_it() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let q = reserved_quote(&desk, now).await;
    let a = desk.draft(&q, now).await;
    let _b = desk.draft(&q, now).await;
    desk.confirm(a.id, later(now, 10))
        .await
        .expect("A confirms");

    desk.cancel(a.id, "drawer short", later(now, 20))
        .await
        .unwrap_or_else(|e| panic!("A is cancelled: {e}"));

    assert_eq!(desk.stored(a.id).await.status, TransactionStatus::Cancelled);
    assert_eq!(desk.quote_row(&q).await, "CANCELLED|");
    assert_eq!(rows(&desk, &q).await, ["RELEASED|CANCELLED"]);
    desk.db.finish().await;
}
