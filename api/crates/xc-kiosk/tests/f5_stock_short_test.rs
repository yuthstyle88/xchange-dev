//! Ticket #6094 (F5, rehearsal finding on #564): a teller can record that a kiosk request was
//! cancelled because the drawer could not cover it. The staff code `STOCK_SHORT` ("เงินสดในลิ้นชัก
//! ไม่พอ", note optional, offered after `CUSTOMER_LEFT`) is carried by `CancelReason`, by the queue's
//! `ticketCancel` mapping (`KioskQueueEnded::reason_for`) and by the widened
//! `kiosk_requests_cancel_reason_check` (`docs/design/blueprint/kiosk-cancel.md` §2 and §3.7, as
//! amended for #6094; migration approved by the owner, #6094 issuecomment-6627165045).
//!
//! Written before the implementation by the ticket author (claude/01TxaRJG). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! The database tests need `TEST_DATABASE_URL` naming an existing database whose name ends in
//! `_test`; the others are pure.

mod kiosk_service_common;

use kiosk_service_common::*;
use sqlx::AssertSqlSafe;
use xc_kernel::kiosk_queue_events::KioskTicketEnd;
use xc_kiosk::application::kiosk::queue_ended::KioskQueueEnded;
use xc_kiosk::domain::kiosk::value_objects::cancel_reason::{CancelReason, CancelReasonError};

const CHECK_VIOLATION: &str = "23514";

// ------------------------------------------------------------ CancelReason

#[test]
fn f5_stock_short_parses_with_no_note_and_prints_back() {
    let reason = CancelReason::parse("STOCK_SHORT", None);
    assert_eq!(reason, Ok(CancelReason::StockShort { note: None }));
    let reason = reason.unwrap();
    assert_eq!(reason.as_str(), "STOCK_SHORT");
    assert_eq!(reason.note(), None);
}

#[test]
fn f5_stock_short_takes_an_optional_note_trimmed_and_blank_is_none() {
    assert_eq!(
        CancelReason::parse("STOCK_SHORT", Some("  USD ขาด 7  ")),
        Ok(CancelReason::StockShort {
            note: Some("USD ขาด 7".to_owned())
        })
    );
    assert_eq!(
        CancelReason::parse("STOCK_SHORT", Some("   ")),
        Ok(CancelReason::StockShort { note: None })
    );
    assert_eq!(
        CancelReason::StockShort {
            note: Some("USD ขาด 7".to_owned())
        }
        .note(),
        Some("USD ขาด 7")
    );
}

#[test]
fn f5_stock_short_note_stops_at_two_hundred_characters() {
    let at_limit = "ข".repeat(200);
    assert_eq!(
        CancelReason::parse("STOCK_SHORT", Some(&at_limit)),
        Ok(CancelReason::StockShort {
            note: Some(at_limit.clone())
        })
    );
    let over = "ข".repeat(201);
    assert_eq!(
        CancelReason::parse("STOCK_SHORT", Some(&over)),
        Err(CancelReasonError::NoteTooLong)
    );
}

#[test]
fn f5_stock_short_is_not_a_no_show_and_other_spellings_stay_unknown() {
    assert!(!CancelReason::StockShort { note: None }.is_no_show());
    assert!(CancelReason::CustomerLeft { note: None }.is_no_show());
    for code in ["stock_short", "STOCK_SHORT ", "STOCK", "STOCK_INSUFFICIENT"] {
        assert_eq!(
            CancelReason::parse(code, None),
            Err(CancelReasonError::ReasonUnknown),
            "{code:?}"
        );
    }
}

// ------------------------------------------------------------ reason_for (§3.7)

fn cancelled(reason: &str) -> KioskTicketEnd {
    KioskTicketEnd::Cancelled {
        reason: reason.to_owned(),
    }
}

#[test]
fn f5_a_ticket_cancel_with_stock_short_keeps_that_code_with_no_note() {
    assert_eq!(
        KioskQueueEnded::reason_for(&cancelled("STOCK_SHORT")),
        CancelReason::StockShort { note: None }
    );
}

#[test]
fn f5_other_spellings_still_map_to_other_and_a_skip_is_still_customer_left() {
    assert_eq!(
        KioskQueueEnded::reason_for(&cancelled("stock_short")),
        CancelReason::Other {
            note: "stock_short".to_owned()
        }
    );
    assert_eq!(
        KioskQueueEnded::reason_for(&cancelled(" STOCK_SHORT ")),
        CancelReason::Other {
            note: "STOCK_SHORT".to_owned()
        }
    );
    assert_eq!(
        KioskQueueEnded::reason_for(&KioskTicketEnd::Skipped),
        CancelReason::CustomerLeft { note: None }
    );
}

// ------------------------------------------------------------ the CHECK (migration)

/// The SQLSTATE `sql` fails with, or `"ok"`.
async fn outcome(db: &TestDb, sql: &str) -> String {
    match sqlx::query(AssertSqlSafe(sql.to_owned()))
        .execute(&db.pool)
        .await
    {
        Ok(_) => "ok".to_owned(),
        Err(e) => match e.as_database_error().and_then(|d| d.code()) {
            Some(code) => code.into_owned(),
            None => panic!("{sql}: not a database error: {e}"),
        },
    }
}

async fn waiting_request(db: &TestDb, id: &str) {
    let sql = format!(
        "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, \
             rate, foreign_amount, thb, input_amount, payment_method) \
         VALUES ('MAIN', '{id}', 'WAITING', now(), NULL, 'BUY', 'USD', 35.0000, 100.00, \
             3500.00, 100.00, 'Cash')"
    );
    assert_eq!(outcome(db, &sql).await, "ok", "{sql}");
}

async fn cancel(db: &TestDb, id: &str, reason: &str, note: &str) -> String {
    outcome(
        db,
        &format!(
            "UPDATE kiosk_requests SET status = 'CANCELLED', cancelled_at = now(), \
                 cancelled_by = 'teller-1', cancel_reason = '{reason}', cancel_note = {note} \
             WHERE branch_id = 'MAIN' AND id = '{id}'"
        ),
    )
    .await
}

#[tokio::test]
async fn f5_the_reason_check_accepts_stock_short_with_or_without_a_note() {
    let db = TestDb::start().await;
    waiting_request(&db, "F5-1").await;
    assert_eq!(cancel(&db, "F5-1", "STOCK_SHORT", "NULL").await, "ok");
    waiting_request(&db, "F5-2").await;
    assert_eq!(
        cancel(&db, "F5-2", "STOCK_SHORT", "'USD ขาด 7'").await,
        "ok"
    );
    db.finish().await;
}

#[tokio::test]
async fn f5_the_reason_check_keeps_its_name_and_the_old_codes_and_refuses_other_spellings() {
    let db = TestDb::start().await;
    let def: String = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(oid) FROM pg_constraint \
         WHERE conrelid = 'kiosk_requests'::regclass AND conname = 'kiosk_requests_cancel_reason_check'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("kiosk_requests_cancel_reason_check exists under that name");
    for code in [
        "CUSTOMER_LEFT",
        "CUSTOMER_CANCELLED",
        "CUSTOMER_CHANGED_MIND",
        "DUPLICATE",
        "OTHER",
        "DAY_CLOSED",
        "STOCK_SHORT",
    ] {
        assert!(def.contains(&format!("'{code}'")), "{code} in {def}");
    }
    waiting_request(&db, "F5-BAD").await;
    for code in ["stock_short", "STOCK", "STOCK_SHORT "] {
        assert_eq!(
            cancel(&db, "F5-BAD", code, "NULL").await,
            CHECK_VIOLATION,
            "{code:?}"
        );
    }
    db.finish().await;
}
