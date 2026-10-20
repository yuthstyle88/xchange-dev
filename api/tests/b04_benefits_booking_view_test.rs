//! Ticket #3403 (B04): `GET /api/v3/views/transaction?id=` returns `benefits` (mapped from
//! `quotes.benefit_snapshot.applied` in stored order) and `booking` (`{ id, bookingNo }` from
//! `bookings.transaction_id`, or `null`).

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use quote_common::{ORG_1, TestDb};
use serde_json::{Value, json};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const AS: &str = "x-test-as";
const MANAGER: &str = "staff:MANAGER:MAIN";
const T0: &str = "2026-10-06T05:00:00Z";
const T1: &str = "2026-10-06T05:03:00Z";

struct HeaderSessions;

fn callers_of(spec: &str) -> Callers {
    if spec.is_empty() {
        return test_callers::nobody();
    }
    if spec == "owner" {
        return test_callers::callers(Some(test_callers::owner("owner-1")), None);
    }
    let mut parts = spec.splitn(3, ':');
    assert_eq!(parts.next(), Some("staff"), "spec {spec:?}");
    let role = parts.next().expect("a role");
    let roles = test_callers::v2_staff_roles(role).expect("a known role");
    let branch = parts.next().expect("a branch");
    test_callers::callers(
        None,
        Some(test_callers::staff(
            &format!("employee-{role}"),
            roles,
            BranchId::new(branch),
        )),
    )
}

impl SessionReader for HeaderSessions {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move { callers_of(&spec) })
    }
}

async fn get_transaction(db: &TestDb, who: &str, id: &str) -> Value {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderSessions);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri(&format!("/api/v3/views/transaction?id={id}"))
        .insert_header((header::HOST, "xc.test"))
        .insert_header((AS, who))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let body: Value = serde_json::from_slice(&bytes).expect("valid json response");
    assert_eq!(status, StatusCode::OK, "{who} {id}: {body}");
    body.pointer("/data/transaction")
        .unwrap_or_else(|| panic!("{who} {id}: no data.transaction in {body}"))
        .clone()
}

async fn setup_base(db: &TestDb) -> String {
    db.exec(
        "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
         SELECT 'CU-MAIN', organization_id, 'CU-MAIN', 'Somchai', 'DEMO', now() \
         FROM branches WHERE id = 'MAIN'",
    )
    .await;
    db.exec(
        "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
         VALUES ('SLIP-MAIN', 'MAIN', 'owner', 'application/pdf', 'sha-slip', now())",
    )
    .await;
    let p = db.publish("MAIN", &[quote_common::USD]).await;
    p.id
}

async fn insert_quote(db: &TestDb, pub_id: &str, benefit_snapshot: &Value) -> String {
    let quote_id = Uuid::new_v4().to_string();
    let snap_str = serde_json::to_string(benefit_snapshot).unwrap();
    sqlx::query(
        "INSERT INTO quotes (id, organization_id, branch_id, customer_id, type, channel, status, \
            publication_id, expires_at, benefit_snapshot, created_by, created_at, updated_at) \
         VALUES ($1::uuid, $2::uuid, 'MAIN', 'CU-MAIN', 'INDICATIVE', 'DESK', 'ACTIVE', $3::uuid, \
            $4::timestamptz, $5::jsonb, 'EMP-001', now(), now())",
    )
    .bind(&quote_id)
    .bind(ORG_1)
    .bind(pub_id)
    .bind(T1)
    .bind(snap_str)
    .execute(&db.pool)
    .await
    .expect("insert quote");
    quote_id
}

async fn insert_exchange(db: &TestDb, quote_id: Option<&str>) -> String {
    let id = Uuid::new_v4().to_string();
    let quote_sql = quote_id.map_or("NULL".to_owned(), |q| format!("'{q}'"));
    db.exec(&format!(
        "INSERT INTO transactions (id, document_ref, branch_id, mode, submission_id, payload_hash, \
            customer_id, customer_name, nationality, staff_id, booking_no, notes, value_date, \
            paid_at, recorded_at, recorded_by, status, quote_id, organization_id) \
         VALUES ('{id}', NULL, 'MAIN', 'DEMO', 'sub-{id}', 'hash', 'CU-MAIN', 'Somchai', 'THA', \
            'EMP-001', 'WALK-IN', '', '2026-10-06', '{T0}', '{T0}', 'owner', 'DRAFT', {quote_sql}, \
            '{ORG_1}')"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, \
            quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, \
            created_at, updated_at) \
         VALUES (gen_random_uuid(), '{ORG_1}', '{id}', 'BUY', 'USD', 100, 1, 35.25, 3525.00, \
            0.00, 3525.00, 1, now(), now())"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, \
            method, reference, account, slip_file_key, amount_thb, bank_fee_thb, paid_at, \
            created_at, updated_at) \
         VALUES (gen_random_uuid(), '{ORG_1}', '{id}', 1, 'Bank Transfer', 'TRX-9', 'KBank 1', \
            'SLIP-MAIN', 3525.00, 0.00, '{T0}', now(), now())"
    ))
    .await;
    id
}

#[tokio::test]
async fn quote_with_one_applied_rate_improvement_returns_benefits_item_as_stored() {
    let db = TestDb::start().await;
    let pub_id = setup_base(&db).await;
    let rule_id = "11111111-1111-1111-1111-111111111111";
    let version_id = "22222222-2222-2222-2222-222222222222";
    let snap = json!({
        "engineVersion": 1,
        "applied": [{
            "ruleId": rule_id,
            "versionId": version_id,
            "benefitType": "RATE_IMPROVEMENT",
            "benefitClass": "CAMPAIGN",
            "couponId": "CPN-100",
            "reservationId": "33333333-3333-3333-3333-333333333333",
            "amountThb": "120.00",
            "rateBefore": "35.10000000",
            "rateAfter": "35.25000000",
            "feeBefore": "10.00",
            "feeAfter": "10.00",
            "floorCapped": false
        }],
        "skipped": []
    });
    let quote_id = insert_quote(&db, &pub_id, &snap).await;
    let tx_id = insert_exchange(&db, Some(&quote_id)).await;

    let t = get_transaction(&db, MANAGER, &tx_id).await;
    assert_eq!(
        t["benefits"],
        json!([{
            "ruleId": rule_id,
            "versionId": version_id,
            "benefitType": "RATE_IMPROVEMENT",
            "benefitClass": "CAMPAIGN",
            "couponId": "CPN-100",
            "amountThb": "120.00",
            "rateBefore": "35.10000000",
            "rateAfter": "35.25000000",
            "feeBefore": "10.00",
            "feeAfter": "10.00",
            "floorCapped": false
        }])
    );
    db.finish().await;
}

#[tokio::test]
async fn quote_with_two_applied_benefits_preserves_stored_order() {
    let db = TestDb::start().await;
    let pub_id = setup_base(&db).await;
    let snap = json!({
        "engineVersion": 1,
        "applied": [
            {
                "ruleId": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "versionId": "aaaa0000-0000-0000-0000-000000000001",
                "benefitType": "RATE_IMPROVEMENT",
                "benefitClass": "MEMBER_TIER",
                "amountThb": "50.00",
                "rateBefore": "35.00000000",
                "rateAfter": "35.05000000",
                "feeBefore": "20.00",
                "feeAfter": "20.00",
                "floorCapped": false
            },
            {
                "ruleId": "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "versionId": "bbbb0000-0000-0000-0000-000000000002",
                "benefitType": "FEE_WAIVER",
                "benefitClass": "COUPON",
                "couponId": "CPN-B",
                "amountThb": "20.00",
                "rateBefore": "35.05000000",
                "rateAfter": "35.05000000",
                "feeBefore": "20.00",
                "feeAfter": "0.00",
                "floorCapped": true
            }
        ],
        "skipped": []
    });
    let quote_id = insert_quote(&db, &pub_id, &snap).await;
    let tx_id = insert_exchange(&db, Some(&quote_id)).await;

    let t = get_transaction(&db, MANAGER, &tx_id).await;
    let benefits = t["benefits"].as_array().expect("benefits array");
    assert_eq!(benefits.len(), 2);
    assert_eq!(
        benefits[0]["ruleId"],
        json!("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
    );
    assert_eq!(benefits[0]["couponId"], Value::Null);
    assert_eq!(
        benefits[1]["ruleId"],
        json!("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb")
    );
    assert_eq!(benefits[1]["couponId"], json!("CPN-B"));
    assert_eq!(benefits[1]["floorCapped"], json!(true));
    db.finish().await;
}

#[tokio::test]
async fn quote_with_empty_benefit_snapshot_or_no_quote_returns_empty_benefits() {
    let db = TestDb::start().await;
    let pub_id = setup_base(&db).await;

    let empty_quote = insert_quote(&db, &pub_id, &json!({})).await;
    let tx_with_empty_snap = insert_exchange(&db, Some(&empty_quote)).await;
    let tx_no_quote = insert_exchange(&db, None).await;

    let t1 = get_transaction(&db, MANAGER, &tx_with_empty_snap).await;
    assert_eq!(t1["benefits"], json!([]));

    let t2 = get_transaction(&db, MANAGER, &tx_no_quote).await;
    assert_eq!(t2["benefits"], json!([]));
    db.finish().await;
}

#[tokio::test]
async fn malformed_applied_element_missing_amount_thb_is_skipped_and_rest_are_shown() {
    let db = TestDb::start().await;
    let pub_id = setup_base(&db).await;
    let snap = json!({
        "engineVersion": 1,
        "applied": [
            {
                "ruleId": "bad-rule-missing-amount",
                "versionId": "v-bad",
                "benefitType": "RATE_IMPROVEMENT",
                "benefitClass": "CAMPAIGN",
                "rateBefore": "35.10000000",
                "rateAfter": "35.25000000",
                "feeBefore": "10.00",
                "feeAfter": "10.00",
                "floorCapped": false
            },
            {
                "ruleId": "good-rule-after-bad",
                "versionId": "v-good",
                "benefitType": "RATE_IMPROVEMENT",
                "benefitClass": "CAMPAIGN",
                "amountThb": "80.00",
                "rateBefore": "35.10000000",
                "rateAfter": "35.18000000",
                "feeBefore": "10.00",
                "feeAfter": "10.00",
                "floorCapped": false
            }
        ]
    });
    let quote_id = insert_quote(&db, &pub_id, &snap).await;
    let tx_id = insert_exchange(&db, Some(&quote_id)).await;

    let t = get_transaction(&db, MANAGER, &tx_id).await;
    let benefits = t["benefits"].as_array().expect("benefits array");
    assert_eq!(
        benefits.len(),
        1,
        "malformed element skipped, valid element kept"
    );
    assert_eq!(benefits[0]["ruleId"], json!("good-rule-after-bad"));
    assert_eq!(benefits[0]["amountThb"], json!("80.00"));
    db.finish().await;
}

#[tokio::test]
async fn booking_completed_by_transaction_is_returned_and_absent_booking_is_null() {
    let db = TestDb::start().await;
    let _pub_id = setup_base(&db).await;
    let tx_with_booking = insert_exchange(&db, None).await;
    let tx_without_booking = insert_exchange(&db, None).await;

    let slot_id = Uuid::new_v4();
    db.exec(&format!(
        "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, \
            end_at, capacity, reserved, status) \
         VALUES ('{slot_id}', '{ORG_1}', 'MAIN', '2026-10-06', '2026-10-06T03:00:00Z', \
            '2026-10-06T04:00:00Z', 5, 1, 'OPEN')"
    ))
    .await;
    db.exec(&format!(
        "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, \
            shop_side, requested_currency, requested_quantity, source_channel, status, \
            qr_token_hash, transaction_id, created_by) \
         VALUES ('BK-0001', '{ORG_1}', 'MAIN', 'B261006-0001', 'CU-MAIN', '{slot_id}', \
            'BUY', 'USD', 100.0000, 'DESK', 'COMPLETED', decode('01020304', 'hex'), \
            '{tx_with_booking}', 'EMP-001')"
    ))
    .await;

    let with_bk = get_transaction(&db, MANAGER, &tx_with_booking).await;
    assert_eq!(
        with_bk["booking"],
        json!({
            "id": "BK-0001",
            "bookingNo": "B261006-0001"
        })
    );

    let without_bk = get_transaction(&db, MANAGER, &tx_without_booking).await;
    assert_eq!(without_bk["booking"], Value::Null);
    db.finish().await;
}
