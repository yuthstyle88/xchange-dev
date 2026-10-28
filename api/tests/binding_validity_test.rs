//! Up-front tests for BQ-4 (validity by type at confirm and complete; design
//! `docs/design/blueprint/binding-quote.md` D-3, O-4 A, §4.2 Q8–Q11, Q17, Q18). Written by the sub-controller
//! claude/644b3958 on `feat/binding-quote` before the implementation. **Do not edit, delete or weaken.**
//!
//! `check_valid_in` (`exchange/create.rs`, called by confirm and complete) judges a quote by its type:
//! - `INDICATIVE`: `now < expires_at` **and** its publication is the branch's current one (today's rule);
//! - `BINDING`: `now < expires_at`, whatever is published since — even when the branch has no current
//!   publication at all (the rule must not short-circuit on `None`).
//!
//! A binding quote is made here from a desk quote by SQL (`type = 'BINDING'`, a 600 s lifetime), so these tests
//! need BQ-0's migration (the second CHECK on `quotes.type` dropped) but not BQ-3's issuer. Its hold is made
//! through B05's `PgStockReservations::reserve_in`, as BQ-3 will make it. Every settle stays under any branch
//! limit: the confirm-time T2 approval of a binding quote is an open design point, not pinned here.
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`), as every `tx5_*` test does.

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use pos_error::AppResult;
use quote_common::*;
use tx4_common::*;
use tx5_common::*;
use xc_cash::domain::reservation::port::{ReserveLine, StockReservations};
use xc_cash::infrastructure::stock_reservations::PgStockReservations;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::quote::quote::Quote;
use xc_exchange::domain::quote::quote_type::QuoteType;
use xc_exchange::domain::quote::repository::QuoteRepository;
use xc_exchange::domain::transaction::transaction::TransactionId;
use xc_exchange::infrastructure::quote::repository::PgQuoteRepository;
use xc_kernel::money::Side;
use xchange::application::workflows::exchange::complete::{CompleteExchange, CompletedExchange};
use xchange::application::workflows::exchange::confirm::ConfirmOutcome;
use xchange::bootstrap::{AppState, exchange_effects};

/// The binding quote's lifetime in these tests: longer than the indicative 300 s.
const LOCK_SECONDS: i64 = 600;

/// USD republished at MAIN after the quote, at other rates.
const USD_NEW: Line = Line::manual("USD", "1", "33.00", "33.50");

/// `q` turned into a BINDING quote ending `now + LOCK_SECONDS`, read back through the repository.
async fn binding(desk: &Desk, q: &Quote, now: DateTime<Utc>) -> Quote {
    desk.db
        .exec(&format!(
            "UPDATE quotes SET type = 'BINDING', expires_at = '{}' WHERE id = '{}'",
            (now + Duration::seconds(LOCK_SECONDS)).to_rfc3339(),
            q.id
        ))
        .await;
    let mut tx = desk.db.pool.begin().await.unwrap();
    let stored = PgQuoteRepository::new(desk.db.pool.clone())
        .find_in(&mut tx, q.id)
        .await
        .unwrap_or_else(|e| panic!("find_in: {e}"))
        .expect("the quote is stored");
    tx.commit().await.unwrap();
    assert_eq!(stored.quote_type, QuoteType::Binding);
    stored
}

/// MAIN's USD position at `quantity`.
async fn usd_position(desk: &Desk, q: &Quote, quantity: &str) {
    desk.db
        .exec(&format!(
            "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb) \
             VALUES ('{org}', 'MAIN', 'USD', {quantity}, {quantity} * 32) \
             ON CONFLICT (organization_id, branch_id, currency) DO UPDATE SET quantity = EXCLUDED.quantity",
            org = q.organization_id
        ))
        .await;
}

/// The quote's one SELL line held (B05's store, as BQ-3 holds it), committed.
async fn hold(desk: &Desk, q: &Quote, now: DateTime<Utc>) {
    usd_position(desk, q, "1000").await;
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
}

/// `status|released_reason|transaction_id` of the quote line's reservation rows.
async fn holds(desk: &Desk, q: &Quote) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT status || '|' || COALESCE(released_reason, '-') || '|' || \
                COALESCE(transaction_id::text, '-') \
         FROM stock_reservations WHERE quote_line_id = $1 ORDER BY created_at, id",
    )
    .bind(q.lines[0].id)
    .fetch_all(&desk.db.pool)
    .await
    .expect("read stock_reservations")
}

async fn quote_status(desk: &Desk, q: &Quote) -> String {
    desk.db
        .text(&format!("SELECT status FROM quotes WHERE id = '{}'", q.id))
        .await
        .expect("a quote row")
}

/// `CompleteExchange` with the ledger double and every registered exchange effect (stock included).
fn service(c: &Counter, ledger: &Ledger) -> CompleteExchange {
    let state = AppState::new(Arc::new(common::settings(&[])), c.desk.db.pool.clone());
    CompleteExchange::new(
        c.desk.db.pool.clone(),
        c.files.clone(),
        Arc::new(ledger.clone()),
        Vec::new(),
        Arc::new(exchange_effects::effects(&state)),
    )
}

/// As production: committed on `Ok`, rolled back on `Err`.
async fn complete(
    c: &Counter,
    ledger: &Ledger,
    id: TransactionId,
    now: DateTime<Utc>,
) -> AppResult<CompletedExchange> {
    let service = service(c, ledger);
    let mut tx = c.desk.db.pool.begin().await.unwrap();
    let got = service.run_in(&mut tx, id, &teller(), now).await;
    if got.is_ok() {
        tx.commit().await.unwrap();
    } else {
        tx.rollback().await.unwrap();
    }
    got
}

#[tokio::test]
async fn q8_a_binding_quote_confirms_and_completes_within_its_lifetime_and_its_hold_is_consumed() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let q = c.desk.quote("MAIN", Side::Sell, "100", None, now).await;
    let q = binding(&c.desk, &q, now).await;
    hold(&c.desk, &q, now).await;
    let t = c.live_draft(&q, PaymentMethod::Cash, true, now).await;

    assert_eq!(
        c.desk.confirm(t.id, later(now, 1)).await.expect("confirm"),
        ConfirmOutcome::Confirmed
    );
    assert_eq!(c.desk.quote_row(&q).await, format!("ACCEPTED|{}", t.id.0));

    // Past the indicative 300 s, inside the lock's 600 s.
    let ledger = Ledger::new("3210.00", None);
    complete(&c, &ledger, t.id, later(now, 400))
        .await
        .unwrap_or_else(|e| panic!("complete within the lock: {e}"));

    assert_eq!(c.status(t.id).await, "COMPLETED");
    assert_eq!(holds(&c.desk, &q).await, [format!("CONSUMED|-|{}", t.id.0)]);
    assert_eq!(ledger.calls().len(), 1);
    c.finish().await;
}

#[tokio::test]
async fn q9_a_binding_quote_completed_after_its_expiry_is_quote_expired_and_nothing_is_posted() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let q = c.desk.quote("MAIN", Side::Sell, "100", None, now).await;
    let q = binding(&c.desk, &q, now).await;
    hold(&c.desk, &q, now).await;
    let t = c.live_of(&q, PaymentMethod::Cash, true, now).await;
    let ledger = Ledger::new("3210.00", None);

    let past = q.expires_at + Duration::seconds(1);
    assert_eq!(
        code(complete(&c, &ledger, t.id, past).await),
        "quoteExpired"
    );

    assert_eq!(c.status(t.id).await, "CONFIRMED");
    assert_eq!(c.written(t.id).await, nothing_written());
    assert!(ledger.calls().is_empty(), "nothing posted");
    assert_eq!(
        holds(&c.desk, &q).await,
        ["RESERVED|-|-"],
        "nothing consumed"
    );
    c.finish().await;
}

#[tokio::test]
async fn q10_a_binding_quotes_exchange_cancelled_after_confirm_cancels_the_quote_and_releases_its_hold()
 {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let q = desk.quote("MAIN", Side::Sell, "100", None, now).await;
    let q = binding(&desk, &q, now).await;
    hold(&desk, &q, now).await;
    let t = desk.draft(&q, now).await;
    assert_eq!(
        desk.confirm(t.id, later(now, 1)).await.expect("confirm"),
        ConfirmOutcome::Confirmed
    );

    desk.cancel(t.id, "customer left", later(now, 5))
        .await
        .unwrap_or_else(|e| panic!("cancel: {e}"));

    assert_eq!(quote_status(&desk, &q).await, "CANCELLED");
    assert_eq!(
        holds(&desk, &q).await,
        ["RELEASED|CANCELLED|-"],
        "released at once, not at expires_at"
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q11_an_indicative_quote_confirms_and_completes_as_today_with_no_hold() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let q = c.desk.quote("MAIN", Side::Sell, "100", None, now).await;
    assert_eq!(q.quote_type, QuoteType::Indicative);
    let t = c.live_of(&q, PaymentMethod::Cash, true, now).await;
    let ledger = Ledger::new("3210.00", None);

    complete(&c, &ledger, t.id, later(now, 5))
        .await
        .unwrap_or_else(|e| panic!("complete: {e}"));

    assert_eq!(c.status(t.id).await, "COMPLETED");
    assert_eq!(holds(&c.desk, &q).await, Vec::<String>::new());
    c.finish().await;
}

#[tokio::test]
async fn q11_an_indicative_quote_past_its_expiry_is_quote_expired_at_confirm() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let q = desk.quote("MAIN", Side::Buy, "100", None, now).await;
    let t = desk.draft(&q, now).await;

    let past = q.expires_at + Duration::seconds(1);
    assert_eq!(code(desk.confirm(t.id, past).await), "quoteExpired");
    assert_eq!(desk.quote_row(&q).await, "ACTIVE|");
    desk.db.finish().await;
}

#[tokio::test]
async fn q17_a_new_publication_does_not_end_a_binding_quote_at_confirm_or_at_complete() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let q = c.desk.quote("MAIN", Side::Sell, "100", None, now).await;
    let q = binding(&c.desk, &q, now).await;
    let t = c.live_draft(&q, PaymentMethod::Cash, true, now).await;

    c.desk.db.publish("MAIN", &[USD_NEW]).await;
    assert_eq!(
        c.desk.confirm(t.id, later(now, 1)).await.expect(
            "O-4 A: a binding quote stays valid until expires_at, whatever is published since"
        ),
        ConfirmOutcome::Confirmed
    );

    c.desk.db.publish("MAIN", &[USD_NEW]).await;
    let ledger = Ledger::new("3210.00", None);
    complete(&c, &ledger, t.id, later(now, 5))
        .await
        .unwrap_or_else(|e| panic!("complete after a second publication: {e}"));

    assert_eq!(c.status(t.id).await, "COMPLETED");
    let stored = c.desk.stored(t.id).await;
    assert_eq!(
        stored.lines[0].rate_snapshot, q.lines[0].final_rate,
        "the exchange keeps the quote's own rate"
    );
    assert_eq!(stored.lines[0].settle_thb, q.lines[0].settle_thb);
    c.finish().await;
}

#[tokio::test]
async fn q17_a_binding_quote_is_valid_when_its_branch_has_no_current_publication() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let q = desk.quote("MAIN", Side::Sell, "100", None, now).await;
    let q = binding(&desk, &q, now).await;
    let t = desk.draft(&q, now).await;
    desk.db
        .exec("UPDATE branches SET current_publication_id = NULL WHERE id = 'MAIN'")
        .await;

    assert_eq!(
        desk.confirm(t.id, later(now, 1))
            .await
            .expect("a binding quote is judged by expires_at alone"),
        ConfirmOutcome::Confirmed
    );
    desk.db.finish().await;
}

#[tokio::test]
async fn q18_a_new_publication_still_ends_an_indicative_quote_at_confirm() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let q = desk.quote("MAIN", Side::Sell, "100", None, now).await;
    let t = desk.draft(&q, now).await;

    desk.db.publish("MAIN", &[USD_NEW]).await;

    assert_eq!(
        code(desk.confirm(t.id, later(now, 1)).await),
        "quoteExpired"
    );
    assert_eq!(desk.quote_row(&q).await, "ACTIVE|");
    desk.db.finish().await;
}

#[tokio::test]
async fn q18_a_new_publication_after_confirm_still_ends_an_indicative_quote_at_complete() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let q = c.desk.quote("MAIN", Side::Sell, "100", None, now).await;
    let t = c.live_of(&q, PaymentMethod::Cash, true, now).await;

    c.desk.db.publish("MAIN", &[USD_NEW]).await;
    let ledger = Ledger::new("3210.00", None);

    assert_eq!(
        code(complete(&c, &ledger, t.id, later(now, 5)).await),
        "quoteExpired"
    );
    assert_eq!(c.status(t.id).await, "CONFIRMED");
    assert!(ledger.calls().is_empty());
    c.finish().await;
}

#[tokio::test]
async fn q18_an_indicative_quote_is_quote_expired_when_its_branch_has_no_current_publication() {
    let desk = Desk::start().await;
    let now = whole_second_now();
    let q = desk.quote("MAIN", Side::Sell, "100", None, now).await;
    let t = desk.draft(&q, now).await;
    desk.db
        .exec("UPDATE branches SET current_publication_id = NULL WHERE id = 'MAIN'")
        .await;

    assert_eq!(
        code(desk.confirm(t.id, later(now, 1)).await),
        "quoteExpired"
    );
    desk.db.finish().await;
}
