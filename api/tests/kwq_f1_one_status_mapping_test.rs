//! KWQ-F1 (#4709): one `ticket_wait` and one web-request status mapping (`derive_status`), shared
//! by `my_web_request`, `my_queue_ticket` and `WebRequestCopy`.
//!
//! For a `NO_SHOW` ticket and a `CALLED` ticket, the e-mail copy's status equals the
//! `my_web_request` status.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use actix_web::http::{Method, StatusCode};
use bk6_common::*;
use chrono::Utc;
use serde_json::json;
use sha2::{Digest, Sha256};
use xchange::application::copy_by_email::subject::CopySubject;
use xchange::application::copy_by_email::web_request_copy::WebRequestCopy;

const ALICE: &str = "CUSTOMER@customer-alice";

fn sha(qr: &str) -> [u8; 32] {
    Sha256::digest(qr.as_bytes()).into()
}

async fn seed_request_with_ticket(
    db: &TestDb,
    id: &str,
    token: &str,
    ticket_status: &str,
    seq: i32,
) {
    sqlx::query(
        "INSERT INTO kiosk_requests (branch_id, id, status, created_at, side, currency, rate, \
             foreign_amount, thb, input_amount, payment_method, customer_id, cancel_token_hash) \
         VALUES ($1, $2, 'WAITING', $3, 'BUY', 'USD', 35.5, 100.00, 3550.00, 100.00, 'Cash', $4, \
             encode(sha256(convert_to($5, 'UTF8')), 'hex'))",
    )
    .bind(B01)
    .bind(id)
    .bind(Utc::now())
    .bind(CUSTOMER_ALICE_CUST_ID)
    .bind(token)
    .execute(&db.pool)
    .await
    .expect("seed kiosk_request");

    sqlx::query(
        "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, \
             sequence, display_number, status, kiosk_request_id, customer_id, issued_at, idempotency_key) \
         VALUES (gen_random_uuid(), $1::uuid, $2, current_date, 'WALK_IN', $3, $4, $5, $6, $7, now(), $8)",
    )
    .bind(ORG)
    .bind(B01)
    .bind(seq)
    .bind(format!("A{seq:03}"))
    .bind(ticket_status)
    .bind(id)
    .bind(CUSTOMER_ALICE_CUST_ID)
    .bind(format!("key-{id}"))
    .execute(&db.pool)
    .await
    .expect("seed queue_ticket");
}

#[tokio::test(flavor = "multi_thread")]
async fn kwq_f1_email_copy_status_equals_my_web_request_status_for_no_show_and_called() {
    let db = TestDb::start().await;
    seed_request_with_ticket(&db, "WR-NO-SHOW", "tok-ns", "NO_SHOW", 1).await;
    seed_request_with_ticket(&db, "WR-CALLED", "tok-cl", "CALLED", 2).await;

    let subject = WebRequestCopy::new(db.pool.clone());

    for (id, token, expected) in [
        ("WR-NO-SHOW", "tok-ns", "NO_SHOW"),
        ("WR-CALLED", "tok-cl", "CALLED"),
    ] {
        let view_ans = call(
            &db,
            Some(ALICE),
            Method::GET,
            &format!("/api/v3/views/my_web_request?branch={B01}&id={id}"),
            None,
            None,
        )
        .await;
        assert_eq!(view_ans.status, StatusCode::OK, "{}", view_ans.text);
        let page_status = view_ans.body["data"]["status"]
            .as_str()
            .expect("page status");
        assert_eq!(page_status, expected);

        let copy = subject
            .load_for(CUSTOMER_ALICE_CUST_ID, id, &sha(token))
            .await
            .expect("load_for")
            .expect("found");
        assert_eq!(copy.status, page_status);
        assert_eq!(json!(copy.status), json!(expected));
    }

    db.finish().await;
}
