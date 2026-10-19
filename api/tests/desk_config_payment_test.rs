//! Written before the implementation (row PM-D, design `docs/design/xchange-v3.md` §5.5.2).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `adminDeskConfig` — on `/api/rpc` and `/api/staff/rpc` — carries `paymentMethods`: the full
//! rows of the desk's branch, `account` included, as the last key (`server.mjs:7047` of the
//! 10-06 bundle). The desk page (`desk-payments.js:5-21`) offers only what is on for the
//! direction and disables its save button while the key is missing (`:14-16`).
//!
//! The recorded replays (`snapshot_test.rs`, `rpc_test.rs`, `rpc_documents_test.rs`, re-recorded
//! from the 10-06 bundle on this branch) hold every recorded `adminDeskConfig` answer with v2's
//! defaults; this file holds what they cannot show: rows the owner saved, per branch, on both
//! routes. The rows are saved at `POST /api/payment-methods` (row PM-API).
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use compat_common::Person;
use rpc_common::{App, Call, Cookies, Desk, RPC, STAFF_RPC, one_of_each_role, sign_in};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";
const AIRPORT: &str = "AIRPORT";

/// v2's defaults (`payment-settings.mjs:3` of the 10-06 bundle).
fn defaults() -> Value {
    json!([
        { "id": "Cash", "enabled": true, "buy": true, "sell": true, "account": "", "instruction": "" },
        { "id": "Credit Card", "enabled": true, "buy": false, "sell": true, "account": "", "instruction": "" },
        { "id": "Alipay", "enabled": true, "buy": false, "sell": true, "account": "", "instruction": "" },
        { "id": "PromptPay", "enabled": true, "buy": false, "sell": true, "account": "", "instruction": "" },
        { "id": "Bank Transfer", "enabled": true, "buy": true, "sell": true, "account": "",
          "instruction": "If transfer, call agent · หากโอนเงิน กรุณาติดต่อพนักงาน" },
    ])
}

fn with(rows: &Value, id: &str, change: Value) -> Value {
    let mut rows = rows.clone();
    for row in rows.as_array_mut().expect("five rows") {
        if row["id"] == json!(id) {
            for (k, v) in change.as_object().expect("changes") {
                row[k] = v.clone();
            }
        }
    }
    rows
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        desk.db.seed_branch(AIRPORT, "Airport").await;
        let app = rpc_app!(desk);
        let mut people = one_of_each_role();
        people.push(Person {
            key: "AIRPORT_FRONT".to_string(),
            kind: "staff".to_string(),
            id: "00000000-0000-4000-8000-0000000000a1".to_string(),
            username: "front.airport".to_string(),
            name: "Front of the airport".to_string(),
            role: "FRONT".to_string(),
            branch_id: Some(AIRPORT.to_string()),
            must_change: false,
        });
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        Room { desk, app, cookies }
    }

    async fn save(&self, branch: &str, version: i64, methods: &Value) {
        let answer = self
            .app
            .send(
                Call::action(
                    ORIGIN,
                    "/api/payment-methods",
                    "save",
                    json!({ "branchId": branch, "version": version, "methods": methods }),
                )
                .cookie(&self.cookies["OWNER"]),
            )
            .await;
        assert_eq!(
            answer.status, 200,
            "the owner saves {branch}: {:.300}",
            answer.text
        );
    }

    /// The desk config `who` reads on `route`, as text and as JSON.
    async fn desk_config(&self, who: &str, route: &str, branch: Option<&str>) -> (String, Value) {
        let payload = branch.map_or_else(|| json!({}), |b| json!({ "branchId": b }));
        let answer = self
            .app
            .send(
                Call::action(ORIGIN, route, "adminDeskConfig", payload).cookie(&self.cookies[who]),
            )
            .await;
        assert_eq!(
            answer.status, 200,
            "adminDeskConfig as {who} on {route}: {:.300}",
            answer.text
        );
        let json = answer.json();
        (answer.text, json)
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// With nothing saved, both routes carry v2's defaults, as the last key after `operations`.
#[actix_web::test]
async fn the_desk_config_carries_the_defaults_last_when_nothing_was_saved() {
    let room = Room::open().await;
    for (who, route) in [
        ("OWNER", RPC),
        ("FRONT", STAFF_RPC),
        ("AIRPORT_FRONT", STAFF_RPC),
    ] {
        let (text, json) = room.desk_config(who, route, None).await;
        assert_eq!(json["paymentMethods"], defaults(), "{who} on {route}");
        let operations = text.find("\"operations\":").expect("operations");
        let payment = text.find("\"paymentMethods\":").expect("paymentMethods");
        assert!(
            operations < payment,
            "{who}: paymentMethods comes after operations"
        );
        let tail = &text[payment..];
        assert!(
            tail.starts_with(concat!(
                r#""paymentMethods":[{"id":"Cash","enabled":true,"buy":true,"sell":true,"#,
                r#""account":"","instruction":""},{"id":"Credit Card","#
            )),
            "{who}: v2's keys in v2's order: {tail:.200}"
        );
        assert!(
            tail.ends_with("}]}"),
            "{who}: paymentMethods is the last key: {tail:.200}"
        );
    }
    room.finish().await;
}

/// The desk gets the full rows — `account` included, which the kiosk never sees — exactly as
/// the owner saved them (texts trimmed).
#[actix_web::test]
async fn the_desk_config_carries_the_rows_the_owner_saved_with_the_account() {
    let room = Room::open().await;
    let saved = with(
        &with(
            &defaults(),
            "Cash",
            json!({ "account": "  Drawer 1  ", "instruction": "Pay at the counter" }),
        ),
        "Credit Card",
        json!({ "enabled": false }),
    );
    room.save("MAIN", 0, &saved).await;
    let stored = with(&saved, "Cash", json!({ "account": "Drawer 1" }));

    for (who, route) in [("OWNER", RPC), ("FRONT", STAFF_RPC), ("MANAGER", RPC)] {
        let (_, json) = room.desk_config(who, route, None).await;
        assert_eq!(json["paymentMethods"], stored, "{who} on {route}");
        assert_eq!(json["paymentMethods"][0]["account"], json!("Drawer 1"));
    }
    room.finish().await;
}

/// "มันต้องรองรับหลาย สาขา": each desk reads its own branch's rows; a later save is read at once.
#[actix_web::test]
async fn each_desk_reads_its_own_branch_and_the_latest_save() {
    let room = Room::open().await;
    let main = with(&defaults(), "Alipay", json!({ "enabled": false }));
    let airport = with(&defaults(), "PromptPay", json!({ "enabled": false }));
    room.save("MAIN", 0, &main).await;
    room.save(AIRPORT, 0, &airport).await;

    assert_eq!(
        room.desk_config("AIRPORT_FRONT", STAFF_RPC, None).await.1["paymentMethods"],
        airport
    );
    // OR-7.m G2: rule 4 (#3700, #3717)
    room.app
        .send(
            Call::action(
                ORIGIN,
                STAFF_RPC,
                "adminDeskConfig",
                json!({ "branchId": "MAIN" }),
            )
            .cookie(&room.cookies["AIRPORT_FRONT"]),
        )
        .await
        .assert_refused(403, "สาขานี้อยู่นอกขอบเขตของบัญชีนี้", "AIRPORT_FRONT naming MAIN");
    assert_eq!(
        room.desk_config("FRONT", STAFF_RPC, None).await.1["paymentMethods"],
        main
    );
    assert_eq!(
        room.desk_config("OWNER", RPC, Some(AIRPORT)).await.1["paymentMethods"],
        airport
    );
    assert_eq!(
        room.desk_config("OWNER", RPC, Some("MAIN")).await.1["paymentMethods"],
        main
    );

    let later = with(&main, "Cash", json!({ "sell": false }));
    room.save("MAIN", 1, &later).await;
    assert_eq!(
        room.desk_config("FRONT", STAFF_RPC, None).await.1["paymentMethods"],
        later
    );
    assert_eq!(
        room.desk_config("AIRPORT_FRONT", STAFF_RPC, None).await.1["paymentMethods"],
        airport
    );
    room.finish().await;
}
