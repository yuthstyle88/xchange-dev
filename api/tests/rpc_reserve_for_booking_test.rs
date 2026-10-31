//! C1: `reserveCustomer`'s optional `bookingId` reaches `CustomerService::reserve_for_booking` on both RPC routes
//! (`docs/design/walkin-queue.md` §12.3, ruled C1 by the owner, relayed on #564 issuecomment-6543033385).
//!
//! Written before the implementation by the ticket author (claude/01TxaRJG). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! Everything goes through the server's whole route table with real sessions (`rpc_common`).
//! Needs `TEST_DATABASE_URL`; nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use rpc_common::{Answer, App, Call, Cookies, Desk, RPC, STAFF_RPC, one_of_each_role, sign_in};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";
const BOOKING_NOT_USABLE: &str = "ใช้การจองนี้ไม่ได้ กรุณาตรวจคิวจองอีกครั้ง";
const BOOKING_NOT_THIS_CUSTOMER: &str = "พาสปอร์ตนี้ไม่ใช่ของลูกค้าที่จองคิวนี้ กรุณาตรวจพาสปอร์ตอีกครั้ง";

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let room = Room { desk, app, cookies };
        // An app booker registered by phone (no identity), checked in and being served at MAIN.
        room.exec(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT 'CU-APP-1', organization_id, 'CU-APP-1', 'Phone Booker', 'LIVE', now() \
             FROM branches WHERE id = 'MAIN'",
        )
        .await;
        room.exec(
            "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, \
                 capacity, reserved, status) \
             SELECT gen_random_uuid(), organization_id, id, current_date, now() + interval '1 hour', \
                 now() + interval '90 minutes', 4, 1, 'OPEN' FROM branches WHERE id = 'MAIN'",
        )
        .await;
        room.exec(
            "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, shop_side, \
                 requested_currency, requested_quantity, source_channel, status, qr_token_hash, checked_in_at, \
                 created_by) \
             SELECT 'BK-1', s.organization_id, s.branch_id, 'B-BK-1', 'CU-APP-1', s.id, 'BUY', 'USD', 100, \
                 'APP', 'IN_SERVICE', convert_to('qr-BK-1', 'UTF8'), now(), 'test' \
             FROM booking_slots s WHERE s.branch_id = 'MAIN' LIMIT 1",
        )
        .await;
        room
    }

    async fn exec(&self, sql: &str) {
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
            .execute(&self.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    async fn reserve(&self, who: &str, path: &str, payload: Value) -> Answer {
        let call =
            Call::action(ORIGIN, path, "reserveCustomer", payload).cookie(&self.cookies[who]);
        self.app.send(call).await
    }
}

fn payload(passport_no: &str, booking_id: Option<Value>) -> Value {
    let mut p = json!({
        "branchId": "MAIN", "name": "Booked Customer", "passportNo": passport_no, "nationality": "DEU",
        "birthDate": "1990-01-02", "passportExpiry": "2031-06-11", "mode": "LIVE",
    });
    if let Some(id) = booking_id {
        p.as_object_mut()
            .expect("an object")
            .insert("bookingId".to_string(), id);
    }
    p
}

fn accepted(answer: &Answer, what: &str) -> Value {
    assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
    answer.json()
}

fn refused(answer: &Answer, sentence: &str, field: Option<&str>, what: &str) {
    assert_eq!(answer.status, 400, "{what}: {:.300}", answer.text);
    let body = answer.json();
    assert_eq!(body["error"], json!(sentence), "{what}: {}", answer.text);
    match field {
        Some(f) => assert_eq!(body["field"], json!(f), "{what}: {}", answer.text),
        None => assert!(
            !body.as_object().expect("an object").contains_key("field"),
            "{what} names no field: {}",
            answer.text
        ),
    }
}

// C1 on the owner route: a bookingId answers the booking's customer.
#[actix_web::test]
async fn owner_route_answers_the_booker() {
    let room = Room::open().await;
    let answer = room
        .reserve("OWNER", RPC, payload("P3000001", Some(json!("BK-1"))))
        .await;
    let body = accepted(&answer, "BK-1");
    assert_eq!(body["customerId"], json!("CU-APP-1"), "{}", answer.text);
    assert_eq!(body["existing"], json!(true), "{}", answer.text);
}

// C1 on the staff route: an unknown booking is refused, naming no field.
#[actix_web::test]
async fn staff_route_refuses_an_unknown_booking() {
    let room = Room::open().await;
    let answer = room
        .reserve(
            "FRONT",
            STAFF_RPC,
            payload("P3000001", Some(json!("BK-NONE"))),
        )
        .await;
    refused(&answer, BOOKING_NOT_USABLE, None, "BK-NONE");
}

// C1: the passport refusal names `passportNo` on the wire.
#[actix_web::test]
async fn another_customers_passport_names_passport_no() {
    let room = Room::open().await;
    let made = room.reserve("OWNER", RPC, payload("P3000002", None)).await;
    let other = accepted(&made, "no bookingId makes a customer");
    assert_ne!(other["customerId"], json!("CU-APP-1"));
    let answer = room
        .reserve("OWNER", RPC, payload("P3000002", Some(json!("BK-1"))))
        .await;
    refused(
        &answer,
        BOOKING_NOT_THIS_CUSTOMER,
        Some("passportNo"),
        "P3000002 is another customer's",
    );
}

// No bookingId, a null one or an empty one: today's reserve, a new customer.
#[actix_web::test]
async fn no_booking_id_is_todays_reserve() {
    let room = Room::open().await;
    for (n, id) in [None, Some(Value::Null), Some(json!(""))]
        .into_iter()
        .enumerate()
    {
        let what = format!("bookingId {id:?}");
        let answer = room
            .reserve("OWNER", RPC, payload(&format!("P400000{n}"), id))
            .await;
        let body = accepted(&answer, &what);
        assert_ne!(
            body["customerId"],
            json!("CU-APP-1"),
            "{what}: {}",
            answer.text
        );
        assert_eq!(body["existing"], json!(false), "{what}: {}", answer.text);
    }
}
