//! Ticket #5315 R4, wired: the receipt `CompleteExchange` stores prints the staff member's `users.name`, and keeps
//! the stored `staff_id` when it names no user. Written before the implementation by the up-front test writer
//! (claude/e2926a49, on the xchange BE lane controller claude/553782's order, 2026-10-28). Do not edit, delete or
//! weaken; add tests in a file of your own. The pure rows (R1–R6) are in
//! `crates/xc-exchange/tests/receipt_rounding_test.rs`.
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips. Each test
//! works in a schema of its own.

mod quote_common;
mod tx4_common;
mod tx5_common;

use quote_common::*;
use tx4_common::*;
use tx5_common::*;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_kernel::money::Side;

/// The stored receipt's text: `receipts.document_ref` is the `files` key (`FileService::store_receipt_in`).
fn receipt_text(c: &Counter, key: &str) -> String {
    std::fs::read_to_string(c.scratch.join("uploads").join(key))
        .unwrap_or_else(|e| panic!("receipt {key} on disk: {e}"))
}

fn staff_row(html: &str) -> &str {
    let open = "<dt>Staff</dt><dd>";
    let at = html.find(open).expect("a Staff row") + open.len();
    &html[at..at + html[at..].find("</dd>").expect("closed")]
}

#[tokio::test]
async fn the_stored_receipt_names_the_staff_member_by_users_name() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    // A user of the organisation named on the exchange, as the v3 desk stores `staff_id` (the user id).
    let user_id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         SELECT id, 'Malee S.', 'ACTIVE', 'test' FROM organizations WHERE code = 'XC' \
         RETURNING id::text",
    )
    .fetch_one(&c.desk.db.pool)
    .await
    .expect("seed the staff user");
    sqlx::query("UPDATE transactions SET staff_id = $1 WHERE id = $2")
        .bind(&user_id)
        .bind(t.id.0)
        .execute(&c.desk.db.pool)
        .await
        .expect("name the user on the exchange");

    let done = c
        .complete(&Ledger::new("3000.00", None), t.id, later(now, 5))
        .await
        .expect("completes");
    let html = receipt_text(&c, &done.receipt.document_ref);
    assert_eq!(
        staff_row(&html),
        "Malee S.",
        "R4: the display name, not {user_id}"
    );
    c.finish().await;
}

#[tokio::test]
async fn a_staff_id_that_names_no_user_stays_on_the_receipt() {
    let c = Counter::start().await;
    let now = whole_second_now();
    let t = c.live(Side::Buy, PaymentMethod::Cash, true, now).await;
    assert_eq!(
        t.staff_id, TELLER,
        "tx5_common stores a staff id that is not a users.id"
    );

    let done = c
        .complete(&Ledger::new("3000.00", None), t.id, later(now, 5))
        .await
        .expect("completes");
    let html = receipt_text(&c, &done.receipt.document_ref);
    assert_eq!(
        staff_row(&html),
        TELLER,
        "R4: no name keeps the id, never empty"
    );
    c.finish().await;
}
