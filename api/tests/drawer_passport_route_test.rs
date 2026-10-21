//! Row PD-0 (`docs/design/xchange-v3.md` §5.5.5): `POST /api/drawer-passport` in v2's
//! switched-off state — what every v2 server without `XC_PASSPORT_DRAWER=1` answers — so
//! that the desk script of the 10-06 bundle (`desk-passport-drawer.js`, injected by row PG)
//! shows *ยังไม่เปิดการเชื่อมต่อลิ้นชักจริงสำหรับสาขานี้* on the passport step instead of a
//! parse error. Written before the implementation. Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! The answers are recorded from the 10-06 bundle by `api/tools/v2-fixtures/gen_drawer_passport.mjs`
//! into `api/tests/fixtures/drawer_passport/v2_drawer_passport_cases.json`: every row 1-12 of
//! §5.5.5's table has at least one case; switched off, rows 8-12 are never reached and answer
//! row 7's `DISABLED`.
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real
//! sessions made at the real sign-in doors (`rpc_common`).
//!
//! Mutations that must turn this red:
//! - no route, or the route on GET only                     -> every_recorded_answer_…
//! - the branch's shape asked before the role               -> every_recorded_answer_… (row 4, second case)
//! - a staff member's named branch used instead of their own -> every_recorded_answer_… (row 5, MANAGER)
//! - `DISABLED` before `SIMULATED` (row 7 before row 6)     -> every_recorded_answer_… (row 6)
//! - the order id checked before the switch (row 8 first)   -> every_recorded_answer_… (row 8)
//! - a body limit of 2,000 bytes counted wrongly            -> every_recorded_answer_… (row 3)
//! - RATES, MONITOR, … let through                          -> only_the_four_desk_roles_…
//! - a device job or a device line written                  -> nothing_is_written_…
//! - a save answering differently because of `drawerOrderId` -> a_save_carrying_drawer_order_id_…
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use rpc_common::{Answer, App, Call, Cookies, Desk, RPC, one_of_each_role, sign_in};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";
const ROUTE: &str = "/api/drawer-passport";
const ROLE_REFUSED: &str = "บัญชีนี้ไม่มีสิทธิ์เปิดลิ้นชัก";
/// OR-4a's one new sentence (§10 `outOfScope`).
const OUT_OF_SCOPE: &str = "สาขานี้อยู่นอกขอบเขตของบัญชีนี้";
/// The roles v2 lets ask (`passport-drawer.mjs:15` of the 10-06 bundle).
const DESK_ROLES: [&str; 4] = ["OWNER", "FRONT", "BACKEND", "MANAGER"];

fn recorded() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/drawer_passport/v2_drawer_passport_cases.json"
    );
    let text = std::fs::read_to_string(path).expect("read v2_drawer_passport_cases.json");
    serde_json::from_str(&text).expect("v2_drawer_passport_cases.json is JSON")
}

/// The body a case sent: its JSON, padded with a last key `pad` when the case says so.
fn body_of(case: &Value) -> String {
    let text = case["body"].to_string();
    match case["padTo"].as_u64() {
        None => text,
        Some(n) => {
            let head = format!("{},\"pad\":\"", &text[..text.len() - 1]);
            let tail = "\"}";
            let body = format!(
                "{head}{}{tail}",
                "x".repeat(n as usize - head.len() - tail.len())
            );
            assert_eq!(body.len() as u64, n, "the padded body");
            body
        }
    }
}

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

    async fn ask(&self, who: Option<&str>, headers: &Value, body: String) -> Answer {
        let mut call = Call::post(ORIGIN, ROUTE, body);
        if let Some(who) = who {
            call = call.cookie(&self.cookies[who]);
        }
        for (name, value) in headers.as_object().expect("the case's headers") {
            call = match value.as_str() {
                Some(value) => call.header(name, value),
                None => call.without(name),
            };
        }
        self.app.send(call).await
    }

    /// How many jobs and lines the device tables hold, all branches.
    async fn written(&self) -> (i64, i64) {
        let mut counts = Vec::new();
        for sql in [
            "SELECT count(*) FROM device_jobs",
            "SELECT count(*) FROM device_events",
        ] {
            counts.push(
                sqlx::query_scalar::<_, i64>(sql)
                    .fetch_one(&self.desk.db.pool)
                    .await
                    .unwrap_or_else(|e| panic!("{sql}: {e}")),
            );
        }
        (counts[0], counts[1])
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// OR-4b (D-4): v2 let an employee through who names a branch other than their own (`MAIN`).
fn names_another_branch(case: &Value) -> bool {
    let who = case["who"].as_str().unwrap_or("OWNER");
    let named = case["body"]["branchId"].as_str().unwrap_or("");
    who != "OWNER" && !named.is_empty() && named != "MAIN" && case["status"] == json!(200)
}

fn label(case: &Value) -> String {
    format!(
        "row {} ({}) as {}",
        case["row"],
        case["label"].as_str().unwrap_or(""),
        case["who"].as_str().unwrap_or("nobody")
    )
}

/// Every case of the recording, in its order, answered as v2 answered it: the status, the
/// JSON body (`==` on values), and `Cache-Control: no-store` where v2 sent it.
#[actix_web::test]
async fn every_recorded_answer_of_the_switched_off_drawer() {
    let file = recorded();
    let cases = file["cases"].as_array().expect("the recorded cases");
    assert_eq!(
        cases.len(),
        20,
        "v2_drawer_passport_cases.json was cut short"
    );
    for row in 1..=12 {
        assert!(
            cases.iter().any(|c| c["row"] == json!(row)),
            "row {row} has a case"
        );
    }

    let room = Room::open().await;
    for case in cases {
        let what = label(case);
        let answer = room
            .ask(case["who"].as_str(), &case["headers"], body_of(case))
            .await;
        // OR-4b (#767, D-4, §02 "Server ตรวจ … branch scope"): an employee v2 let through naming a
        // branch other than their own (row 5) is refused as out of scope
        let (status, expected) = if names_another_branch(case) {
            (403, json!({ "error": OUT_OF_SCOPE }))
        } else {
            (
                case["status"].as_u64().expect("a status") as u16,
                case["answer"].clone(),
            )
        };
        assert_eq!(
            answer.status, status,
            "{what}: status: {:.300}",
            answer.text
        );
        let body: Value = serde_json::from_str(&answer.text)
            .unwrap_or_else(|e| panic!("{what}: the body is JSON ({e}): {:.300}", answer.text));
        assert_eq!(body, expected, "{what}: body");
        if let Some(cache) = case["cacheControl"].as_str() {
            assert_eq!(answer.header("cache-control"), Some(cache), "{what}");
        }
    }
    room.finish().await;
}

/// Row 4 for every role an account can hold: the four desk roles reach row 7 (`DISABLED`),
/// every other staff role is refused, whatever the branch it names.
#[actix_web::test]
async fn only_the_four_desk_roles_reach_the_switch() {
    let room = Room::open().await;
    let order = "5e5e0001-0000-4000-8000-0000000000aa";
    for person in one_of_each_role() {
        let who = person.key.as_str();
        for branch in ["MAIN", "no such branch"] {
            let body =
                json!({ "branchId": branch, "orderId": order, "kiosk": false, "mode": "LIVE" });
            let answer = room.ask(Some(who), &json!({}), body.to_string()).await;
            let what = format!("{who} naming {branch:?}");
            let (status, expected) = if !DESK_ROLES.contains(&who) {
                (400, json!({ "error": ROLE_REFUSED }))
            } else if who == "OWNER" && branch != "MAIN" {
                (400, json!({ "error": "สาขาไม่ถูกต้อง" }))
            } else if branch != "MAIN" {
                // OR-4b (#767, D-4): an employee naming another branch is out of scope
                (403, json!({ "error": OUT_OF_SCOPE }))
            } else {
                (
                    200,
                    json!({ "state": "DISABLED", "physicalResultConfirmed": false }),
                )
            };
            assert_eq!(answer.status, status, "{what}: {:.300}", answer.text);
            let body: Value = serde_json::from_str(&answer.text)
                .unwrap_or_else(|e| panic!("{what}: JSON ({e}): {:.300}", answer.text));
            assert_eq!(body, expected, "{what}");
        }
    }
    room.finish().await;
}

/// Switched off, nothing reaches the device layer: no job is queued and no line written,
/// however often an order is asked for and by whom.
#[actix_web::test]
async fn nothing_is_written_and_nothing_is_queued() {
    let room = Room::open().await;
    let file = recorded();
    for case in file["cases"].as_array().expect("cases") {
        room.ask(case["who"].as_str(), &case["headers"], body_of(case))
            .await;
    }
    for who in DESK_ROLES {
        for _ in 0..2 {
            let body = json!({ "branchId": "MAIN", "orderId": file["order"], "kiosk": true, "mode": "LIVE" });
            let answer = room.ask(Some(who), &json!({}), body.to_string()).await;
            assert_eq!(answer.status, 200, "{who}: {:.300}", answer.text);
        }
    }
    assert_eq!(
        room.written().await,
        (0, 0),
        "device_jobs / device_events stay empty"
    );
    room.finish().await;
}

// ── the save ignores `drawerOrderId` ─────────────────────────────────────────

/// The first bytes of a PNG: enough for the upload route.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, b'I', b'H', b'D', b'R',
];

async fn ok(app: &App, call: Call, what: &str) -> Value {
    let answer = app.send(call).await;
    assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
    answer.json()
}

/// A DEMO exchange of `engine_actions.json` #125 in `MAIN`, as `rpc_save_test.rs` sends it.
fn demo_save(customer: &str, passport: &str, submission: &str) -> Value {
    json!({
        "branchId": "MAIN",
        "name": "Customer 1", "passportNo": "P1000001", "nationality": "DEU",
        "birthDate": "1990-01-02", "passportExpiry": "2031-06-11", "mode": "DEMO",
        "submissionId": submission, "customerId": customer,
        "type": "BUY", "currency": "USD", "amount": 100, "rate": 1, "fee": 0, "bankFee": 0,
        "method": "Cash", "ref": "", "account": "Cash Drawer", "staff": "desk",
        "booking": "WALK-IN", "notes": "", "valueDate": "2026-10-06",
        "paidAt": "2026-10-06T07:04:00.000Z",
        "paymentConfirmed": true, "identityConfirmed": true, "countConfirmed": true,
        "passport": { "uploadKey": passport },
    })
}

/// v2's desk script adds `drawerOrderId` to every `saveTransaction`
/// (`desk-passport-drawer.js:17`); §5.5.5 [required]: such a save is answered exactly as the
/// same save without it — accepted the same, refused with the same sentence.
#[actix_web::test]
async fn a_save_carrying_drawer_order_id_is_answered_as_without_it() {
    let room = Room::open().await;
    // OR-4b (#767, design §4.2 C2): ExchangeTransact is TELLER's; FRONT (TELLER under B1) reserves,
    // uploads and saves
    let teller = &room.cookies["FRONT"];
    let customer = ok(
        &room.app,
        Call::action(
            ORIGIN,
            RPC,
            "reserveCustomer",
            json!({
                "branchId": "MAIN", "name": "Customer 1", "passportNo": "P1000001",
                "nationality": "DEU", "birthDate": "1990-01-02",
                "passportExpiry": "2031-06-11", "mode": "DEMO",
            }),
        )
        .cookie(teller),
        "the customer",
    )
    .await["customerId"]
        .as_str()
        .expect("the customer's number")
        .to_string();
    let passport = ok(
        &room.app,
        Call::post(ORIGIN, "/api/upload?branch=MAIN", PNG)
            .header("content-type", "image/png")
            .cookie(teller),
        "the passport",
    )
    .await["uploadKey"]
        .as_str()
        .expect("the passport's key")
        .to_string();

    // Re-expected for TX-6 by the test-writer, decision A2 (owner, 2026-10-11; design quote-transaction.md §4.6.1, #674): a DEMO save is quoted like a LIVE one, so
    // with nothing published it is refused; MAIN then publishes the dollar at the rate the save names
    // (1), and the drawerOrderId comparison below is unchanged.
    room.app
        .send(
            Call::action(
                ORIGIN,
                RPC,
                "saveTransaction",
                demo_save(&customer, &passport, "00000000-0000-4000-8000-0000000d0009"),
            )
            .cookie(teller),
        )
        .await
        .assert_refused(
            400,
            "rateUnavailable: no published line",
            "a DEMO save with nothing published (decision A2)",
        );
    snapshot_common::publish_rate_per_one(&room.desk.db.pool, "MAIN", "USD", "1", "1").await;

    // accepted: the same keys, the same mode, numbers one apart
    let plain = ok(
        &room.app,
        Call::action(
            ORIGIN,
            RPC,
            "saveTransaction",
            demo_save(&customer, &passport, "00000000-0000-4000-8000-0000000d0001"),
        )
        .cookie(teller),
        "the save without drawerOrderId",
    )
    .await;
    let mut payload = demo_save(&customer, &passport, "00000000-0000-4000-8000-0000000d0002");
    payload["drawerOrderId"] = json!("5e5e0001-0000-4000-8000-0000000000bb");
    let with = ok(
        &room.app,
        Call::action(ORIGIN, RPC, "saveTransaction", payload).cookie(teller),
        "the save with drawerOrderId",
    )
    .await;
    let keys = |v: &Value| -> Vec<String> {
        v.as_object()
            .expect("an object")
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(keys(&with), keys(&plain), "the keys of the answer");
    for key in ["saved", "mode", "passportUrl", "slipUrl"] {
        assert_eq!(with[key], plain[key], "{key}");
    }
    assert_ne!(with["documentRef"], plain["documentRef"], "two exchanges");

    // refused: the same answer, byte for byte
    let mut bad = demo_save(&customer, "", "00000000-0000-4000-8000-0000000d0003");
    bad["passport"] = json!({ "uploadKey": 7 });
    let refused_plain = room
        .app
        .send(Call::action(ORIGIN, RPC, "saveTransaction", bad.clone()).cookie(teller))
        .await;
    bad["drawerOrderId"] = json!("5e5e0001-0000-4000-8000-0000000000cc");
    let refused_with = room
        .app
        .send(Call::action(ORIGIN, RPC, "saveTransaction", bad).cookie(teller))
        .await;
    assert_eq!(refused_plain.status, 400, "{:.300}", refused_plain.text);
    assert_eq!(refused_with.status, refused_plain.status);
    assert_eq!(refused_with.text, refused_plain.text);
    room.finish().await;
}
