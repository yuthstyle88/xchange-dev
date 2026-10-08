//! Body limit and branchId default tests for row PD-0 (`POST /api/drawer-passport`).
//!
//! Pins:
//! - UTF-16 unit counting vs byte counting (a Thai-padded body of > 2,000 bytes but <= 2,000 units is accepted)
//! - Exact boundary at 2,000 UTF-16 units (accepted, answers DISABLED)
//! - 2,001 UTF-16 units is rejected with 400 `{"error":"Request too large"}`
//! - An owner sending `"branchId": ""` defaults to `"MAIN"` and answers DISABLED (v2 `p.branchId || 'MAIN'`)
//!
//! Needs `TEST_DATABASE_URL`.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";
const ROUTE: &str = "/api/drawer-passport";

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
        Room { desk, app, cookies }
    }

    async fn ask(&self, who: Option<&str>, body: String) -> Answer {
        let mut call = Call::post(ORIGIN, ROUTE, body);
        if let Some(who) = who {
            call = call.cookie(&self.cookies[who]);
        }
        self.app.send(call).await
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// A body of more than 2,000 bytes but at most 2,000 UTF-16 units (900 Thai characters)
/// is accepted and not rejected by byte size limits.
#[actix_web::test]
async fn body_with_more_than_2000_bytes_but_under_2000_utf16_units_is_accepted() {
    let room = Room::open().await;
    let order = "5e5e0001-0000-4000-8000-000000000001";
    let thai_pad = "ก".repeat(900);
    let body = json!({
        "branchId": "MAIN",
        "orderId": order,
        "kiosk": false,
        "mode": "LIVE",
        "pad": thai_pad,
    })
    .to_string();

    assert!(
        body.len() > 2000,
        "body byte length {} should be > 2000",
        body.len()
    );
    assert!(
        body.encode_utf16().count() <= 2000,
        "body UTF-16 count {} should be <= 2000",
        body.encode_utf16().count()
    );

    let answer = room.ask(Some("FRONT"), body).await;
    assert_eq!(answer.status, 200, "status: {:.300}", answer.text);
    let body_res: Value = serde_json::from_str(&answer.text).expect("valid json response");
    assert_eq!(
        body_res,
        json!({ "state": "DISABLED", "physicalResultConfirmed": false })
    );
    assert_eq!(answer.header("cache-control"), Some("no-store"));

    room.finish().await;
}

/// A body of exactly 2,000 UTF-16 units containing Thai characters is read and answered 200 DISABLED.
#[actix_web::test]
async fn body_of_exactly_2000_utf16_units_containing_thai_characters_is_accepted() {
    let room = Room::open().await;
    let head = r#"{"branchId":"MAIN","orderId":"5e5e0001-0000-4000-8000-000000000001","kiosk":false,"mode":"LIVE","pad":""#;
    let tail = r#""}"#;
    let base_units = head.encode_utf16().count() + tail.encode_utf16().count();
    assert!(base_units < 2000);
    let thai_pad = "ก".repeat(2000 - base_units);
    let body = format!("{head}{thai_pad}{tail}");

    assert_eq!(body.encode_utf16().count(), 2000);
    assert!(
        body.len() > 2000,
        "byte size {} must exceed 2000",
        body.len()
    );

    let answer = room.ask(Some("FRONT"), body).await;
    assert_eq!(answer.status, 200, "status: {:.300}", answer.text);
    let body_res: Value = serde_json::from_str(&answer.text).expect("valid json response");
    assert_eq!(
        body_res,
        json!({ "state": "DISABLED", "physicalResultConfirmed": false })
    );
    assert_eq!(answer.header("cache-control"), Some("no-store"));

    room.finish().await;
}

/// A body of 2,001 UTF-16 units is rejected with 400 Request too large.
#[actix_web::test]
async fn body_of_2001_utf16_units_is_rejected_as_request_too_large() {
    let room = Room::open().await;
    let head = r#"{"branchId":"MAIN","orderId":"5e5e0001-0000-4000-8000-000000000001","kiosk":false,"mode":"LIVE","pad":""#;
    let tail = r#""}"#;
    let base_units = head.encode_utf16().count() + tail.encode_utf16().count();
    assert!(base_units < 2001);
    let thai_pad = "ก".repeat(2001 - base_units);
    let body = format!("{head}{thai_pad}{tail}");

    assert_eq!(body.encode_utf16().count(), 2001);

    let answer = room.ask(Some("FRONT"), body).await;
    assert_eq!(answer.status, 400, "status: {:.300}", answer.text);
    let body_res: Value = serde_json::from_str(&answer.text).expect("valid json response");
    assert_eq!(body_res, json!({ "error": "Request too large" }));
    assert_eq!(answer.header("cache-control"), Some("no-store"));

    room.finish().await;
}

/// An owner supplying `"branchId": ""` defaults to `"MAIN"` (v2 `p.branchId || 'MAIN'`)
/// and reaches the switched-off answer 200 DISABLED, rather than 400 "สาขาไม่ถูกต้อง".
#[actix_web::test]
async fn owner_with_empty_branch_id_reads_as_main() {
    let room = Room::open().await;
    let order = "5e5e0001-0000-4000-8000-000000000001";
    let body = json!({
        "branchId": "",
        "orderId": order,
        "kiosk": false,
        "mode": "LIVE",
    })
    .to_string();

    let answer = room.ask(Some("OWNER"), body).await;
    assert_eq!(answer.status, 200, "status: {:.300}", answer.text);
    let body_res: Value = serde_json::from_str(&answer.text).expect("valid json response");
    assert_eq!(
        body_res,
        json!({ "state": "DISABLED", "physicalResultConfirmed": false })
    );
    assert_eq!(answer.header("cache-control"), Some("no-store"));

    room.finish().await;
}
