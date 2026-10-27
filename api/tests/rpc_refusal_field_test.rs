//! RF-2: a `reserveCustomer` refusal names the payload key at fault on both RPC routes
//! (`docs/design/rpc-refusal-field.md` §4 rows R16–R18, decisions RF-D1, RF-D4, RF-D5, RF-D9).
//!
//! Written before the implementation by the sub-controller (RF-0). Do not edit, delete or weaken these
//! tests; add your own in another file.
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real sessions
//! (`rpc_common`). Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use std::collections::BTreeMap;

use rpc_common::{
    Answer, App, Call, Cookies, Desk, RPC, STAFF_RPC, V3_REFUSAL_FIELDS, one_of_each_role, sign_in,
};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";
const BAD_TEXT: &str = "ข้อมูลข้อความไม่ครบหรือยาวเกินกำหนด";
const BAD_DATE_FORMAT: &str = "รูปแบบวันที่ไม่ถูกต้อง";
const BAD_DATE: &str = "วันที่ไม่ถูกต้อง";
const BAD_MODE: &str = "กรุณาเลือกประเภทรายการ";
const INVALID_NATIONALITY: &str = "สัญชาติไม่ถูกต้อง";

struct Room {
    _desk: Desk,
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
        Room {
            _desk: desk,
            app,
            cookies,
        }
    }

    async fn reserve(&self, who: &str, path: &str, payload: Value) -> Answer {
        let call =
            Call::action(ORIGIN, path, "reserveCustomer", payload).cookie(&self.cookies[who]);
        self.app.send(call).await
    }
}

/// `engine_actions.json` #77's payload, which v2 accepts, with `edit` applied.
fn payload(edit: impl FnOnce(&mut serde_json::Map<String, Value>)) -> Value {
    let mut p = json!({
        "branchId": "MAIN", "name": "Customer 1", "passportNo": "P1000001", "nationality": "DEU",
        "birthDate": "1990-01-02", "passportExpiry": "2031-06-11", "mode": "LIVE",
    });
    edit(p.as_object_mut().expect("an object"));
    p
}

fn set(key: &'static str, value: Value) -> impl FnOnce(&mut serde_json::Map<String, Value>) {
    move |p| {
        p.insert(key.to_string(), value);
    }
}

fn drop_key(key: &'static str) -> impl FnOnce(&mut serde_json::Map<String, Value>) {
    move |p| {
        p.remove(key);
    }
}

/// The R1–R10 payloads: (what, payload, sentence, field).
fn cases() -> Vec<(&'static str, Value, &'static str, &'static str)> {
    vec![
        (
            "R1 #68 blank name",
            payload(|p| {
                p.insert("name".into(), json!(""));
                p.insert("nationality".into(), json!("DE"));
            }),
            BAD_TEXT,
            "name",
        ),
        (
            "R2 #69 long name",
            payload(set("name", json!("n".repeat(101)))),
            BAD_TEXT,
            "name",
        ),
        (
            "R3 #70 long passport",
            payload(set("passportNo", json!("P".repeat(31)))),
            BAD_TEXT,
            "passportNo",
        ),
        (
            "R3 blank passport",
            payload(set("passportNo", json!(""))),
            BAD_TEXT,
            "passportNo",
        ),
        (
            "R4 #71 no nationality",
            payload(drop_key("nationality")),
            BAD_TEXT,
            "nationality",
        ),
        (
            "R5 two-letter nationality",
            payload(set("nationality", json!("DE"))),
            INVALID_NATIONALITY,
            "nationality",
        ),
        (
            "R6 #74 birth date format",
            payload(set("birthDate", json!("01/02/1990"))),
            BAD_DATE_FORMAT,
            "birthDate",
        ),
        (
            "R7 #75 no such birth date",
            payload(set("birthDate", json!("1990-02-30"))),
            BAD_DATE,
            "birthDate",
        ),
        (
            "R8 #76 no such expiry",
            payload(set("passportExpiry", json!("2031-13-01"))),
            BAD_DATE,
            "passportExpiry",
        ),
        (
            "R9 #72 mode REAL",
            payload(set("mode", json!("REAL"))),
            BAD_MODE,
            "mode",
        ),
        (
            "R9 #73 no mode",
            payload(drop_key("mode")),
            BAD_MODE,
            "mode",
        ),
        (
            "R10 passport of dashes",
            payload(set("passportNo", json!("---"))),
            BAD_TEXT,
            "passportNo",
        ),
    ]
}

fn assert_names(answer: &Answer, sentence: &str, field: &str, what: &str) {
    assert_eq!(answer.status, 400, "{what}: {:.300}", answer.text);
    assert_eq!(
        answer.text,
        format!(
            "{{\"error\":{},\"field\":{}}}",
            Value::String(sentence.to_string()),
            Value::String(field.to_string())
        ),
        "{what}: error first, then field"
    );
    assert_eq!(answer.header("cache-control"), Some("no-store"), "{what}");
    assert_eq!(
        answer.header("content-type"),
        Some("application/json"),
        "{what}"
    );
}

// R16
#[actix_web::test]
async fn owner_route_names_the_field() {
    let room = Room::open().await;
    for (what, payload, sentence, field) in cases() {
        let answer = room.reserve("OWNER", RPC, payload).await;
        assert_names(&answer, sentence, field, what);
    }
    // a refusal with no field stays `{"error": …}` byte for byte
    let answer = room
        .reserve("OWNER", RPC, payload(set("branchId", json!("NOPE"))))
        .await;
    assert_eq!(answer.status, 400, "{:.300}", answer.text);
    assert!(
        !answer
            .json()
            .as_object()
            .expect("an object")
            .contains_key("field"),
        "an unknown branch names no field: {}",
        answer.text
    );
    // an accepted payload is still accepted
    let answer = room.reserve("OWNER", RPC, payload(|_| {})).await;
    assert_eq!(answer.status, 200, "{:.300}", answer.text);
}

// R17
#[actix_web::test]
async fn staff_route_names_the_field() {
    let room = Room::open().await;
    let wanted = ["R3 #70 long passport", "R6 #74 birth date format"];
    for (what, payload, sentence, field) in cases().into_iter().filter(|c| wanted.contains(&c.0)) {
        let answer = room.reserve("FRONT", STAFF_RPC, payload).await;
        assert_names(&answer, sentence, field, what);
    }
}

// R18 (the harness half; the replay half is `rpc_test.rs::engine_actions_json_is_answered_as_v2_answered_it`)
#[test]
fn field_outside_the_list_fails_the_harness() {
    fn answer(text: &str) -> Answer {
        let mut headers = BTreeMap::new();
        headers.insert("cache-control".to_string(), "no-store".to_string());
        headers.insert("content-type".to_string(), "application/json".to_string());
        Answer {
            status: 400,
            headers,
            cookies: Vec::new(),
            text: text.to_string(),
        }
    }
    fn fails(f: impl FnOnce() + std::panic::UnwindSafe) -> bool {
        std::panic::catch_unwind(f).is_err()
    }

    assert!(V3_REFUSAL_FIELDS.contains(&(RPC, "reserveCustomer")));
    assert!(V3_REFUSAL_FIELDS.contains(&(STAFF_RPC, "reserveCustomer")));
    assert_eq!(V3_REFUSAL_FIELDS.len(), 2, "one row, both routes (RF-D9)");

    let named = format!(
        "{{\"error\":{},\"field\":\"passportNo\"}}",
        Value::String(BAD_TEXT.into())
    );
    // listed, the expected key: held
    let ok = answer(&named);
    ok.assert_refused_at(
        RPC,
        "reserveCustomer",
        400,
        BAD_TEXT,
        "passportNo",
        "listed",
    );
    let ok = answer(&named);
    ok.assert_refused_at(
        STAFF_RPC,
        "reserveCustomer",
        400,
        BAD_TEXT,
        "passportNo",
        "listed staff",
    );

    // an action not in the list
    let a = answer(&named);
    assert!(fails(move || a.assert_refused_at(
        RPC,
        "adminCreateBranch",
        400,
        BAD_TEXT,
        "passportNo",
        "x"
    )));
    // another key than the case expects
    let a = answer(&named);
    assert!(fails(move || a.assert_refused_at(
        RPC,
        "reserveCustomer",
        400,
        BAD_TEXT,
        "name",
        "x"
    )));
    // a key outside RF-D5's list
    let other = format!(
        "{{\"error\":{},\"field\":\"branchId\"}}",
        Value::String(BAD_TEXT.into())
    );
    let a = answer(&other);
    assert!(fails(move || a.assert_refused_at(
        RPC,
        "reserveCustomer",
        400,
        BAD_TEXT,
        "branchId",
        "x"
    )));
    // no field at all
    let bare = format!("{{\"error\":{}}}", Value::String(BAD_TEXT.into()));
    let a = answer(&bare);
    assert!(fails(move || a.assert_refused_at(
        RPC,
        "reserveCustomer",
        400,
        BAD_TEXT,
        "passportNo",
        "x"
    )));
    // field before error
    let swapped = format!(
        "{{\"field\":\"passportNo\",\"error\":{}}}",
        Value::String(BAD_TEXT.into())
    );
    let a = answer(&swapped);
    assert!(fails(move || a.assert_refused_at(
        RPC,
        "reserveCustomer",
        400,
        BAD_TEXT,
        "passportNo",
        "x"
    )));
    // another sentence or status than v2's
    let a = answer(&named);
    assert!(fails(move || a.assert_refused_at(
        RPC,
        "reserveCustomer",
        400,
        BAD_MODE,
        "passportNo",
        "x"
    )));
    let a = answer(&named);
    assert!(fails(move || a.assert_refused_at(
        RPC,
        "reserveCustomer",
        401,
        BAD_TEXT,
        "passportNo",
        "x"
    )));

    // and a refusal of any other action keeps the byte-for-byte check
    let a = answer(&named);
    assert!(fails(move || a.assert_refused(400, BAD_TEXT, "x")));
}
