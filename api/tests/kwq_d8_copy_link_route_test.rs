//! KWQ-D8 — `POST /api/public/copy-link` (`copyLinkView`, design
//! `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D22, §4.4 B24, B25), through the route table
//! on a real PostgreSQL: a good link answers the details, the live status and the QR; every
//! failure answers one `410 linkExpired`; one address is limited as the kiosk quote is
//! (`QuoteLimit`: 20 a minute ⇒ the 21st is `429 tooManyRequests`). Public: no session needed.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Harness: `bk6_common` (its PII test key is what the server signs with). Needs `TEST_DATABASE_URL`.

#[path = "bk6_common/mod.rs"]
mod bk6_common;

use std::sync::Arc;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, test as atest, web};
use bk6_common::*;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use xc_kernel::pii::PiiKeys;
use xchange::application::copy_by_email::link::CopyLink;
use xchange::application::copy_by_email::subject::CopySubjectKind;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::SessionReader;

const ROUTE: &str = "/api/public/copy-link";

fn sha(qr: &str) -> [u8; 32] {
    Sha256::digest(qr.as_bytes()).into()
}

fn signer() -> CopyLink {
    CopyLink::from_pii(
        &PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").unwrap(),
    )
}

async fn alices_booking(db: &TestDb) {
    let slot = Uuid::new_v4();
    let start = Utc::now() + Duration::hours(3);
    seed_slot(&db.pool, slot, B01, bangkok_today(), start, start + Duration::minutes(30), 5, 1, "OPEN").await;
    seed_booking(&db.pool, "BK-ALICE", "B261022-0001", CUSTOMER_ALICE_CUST_ID, B01, slot, "APP", "CONFIRMED", &sha("alice-qr")).await;
}

async fn view(db: &TestDb, link: &str) -> Answer {
    call(db, None, Method::POST, ROUTE, Some(json!({ "link": link })), None).await
}

fn expired(a: &Answer, what: &str) {
    assert_eq!(a.status, StatusCode::GONE, "{what}: {}", a.text);
    assert_eq!(a.body["error"]["code"], json!("linkExpired"), "{what}: {}", a.text);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_good_link_answers_the_details_status_and_qr_and_nothing_private() {
    let db = TestDb::start().await;
    alices_booking(&db).await;
    let link = signer().sign(CopySubjectKind::Booking, "BK-ALICE", "alice-qr", Utc::now() + Duration::hours(23));
    let a = view(&db, &link).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["subject"], json!("booking"));
    assert_eq!(a.body["id"], json!("BK-ALICE"));
    assert_eq!(a.body["branch"], json!("Branch 1"));
    assert_eq!(a.body["status"], json!("CONFIRMED"));
    assert_eq!(a.body["qr"], json!("alice-qr"));
    assert!(a.body["expiresAt"].is_string(), "{}", a.text);
    let rows = a.body["rows"].as_array().expect("rows");
    assert!(
        rows.iter().any(|r| r["field"] == "bookingNo" && r["value"] == "B261022-0001"),
        "{}",
        a.text
    );
    for secret in ["CUST-ALICE", "CN-001", "+66"] {
        assert!(!a.text.contains(secret), "{secret}: {}", a.text);
    }
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_failure_is_one_link_expired() {
    let db = TestDb::start().await;
    alices_booking(&db).await;
    let s = signer();
    let good = s.sign(CopySubjectKind::Booking, "BK-ALICE", "alice-qr", Utc::now() + Duration::hours(23));

    expired(&view(&db, "garbage").await, "malformed");
    expired(&view(&db, "").await, "empty");
    expired(
        &view(&db, &s.sign(CopySubjectKind::Booking, "BK-ALICE", "alice-qr", Utc::now() - Duration::minutes(1))).await,
        "past its day",
    );
    expired(
        &view(&db, &CopyLink::new([1; 32]).sign(CopySubjectKind::Booking, "BK-ALICE", "alice-qr", Utc::now() + Duration::hours(1))).await,
        "another key",
    );
    expired(
        &view(&db, &s.sign(CopySubjectKind::Booking, "BK-NOPE", "alice-qr", Utc::now() + Duration::hours(1))).await,
        "no such booking",
    );
    sqlx::query("UPDATE bookings SET qr_token_hash = $2, qr_version = 2 WHERE id = 'BK-ALICE'")
        .bind(&sha("alice-qr-2")[..])
        .execute(&db.pool)
        .await
        .unwrap();
    expired(&view(&db, &good).await, "the QR was reissued");

    let missing = call(&db, None, Method::POST, ROUTE, Some(json!({})), None).await;
    expired(&missing, "no link field");
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn one_address_gets_twenty_a_minute() {
    let db = TestDb::start().await;
    let state = AppState::new(Arc::new(test_common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let address = format!("198.51.100.{}", 1 + (Uuid::new_v4().as_bytes()[0] % 250));
    let mut statuses = Vec::new();
    for _ in 0..21 {
        let req = atest::TestRequest::post()
            .uri(ROUTE)
            .insert_header((header::HOST, HOST))
            .insert_header(("x-real-ip", address.as_str()))
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .set_payload(json!({ "link": "garbage" }).to_string())
            .to_request();
        let resp = atest::call_service(&app, req).await;
        statuses.push(resp.status());
        let _: Value = serde_json::from_slice(&atest::read_body(resp).await).unwrap_or(Value::Null);
    }
    assert!(statuses[..20].iter().all(|s| *s == StatusCode::GONE), "{statuses:?}");
    assert_eq!(statuses[20], StatusCode::TOO_MANY_REQUESTS, "{statuses:?}");
    db.finish().await;
}
