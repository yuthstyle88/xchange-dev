//! Ticket #314 — `snapshot` on `POST /api/devices` carries `model`: the admin
//! snapshot of the caller's branch. Written before the implementation. Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! The kept devices screen (`public/devices.js:8`) starts every redraw with
//! `const m=data.model,bal=balance(m)`, and `balance` (`:5`) reads
//! `m.stock.shifts`; the redraw then reads `m.branches`, `m.transactions` and
//! `m.stock.movements`. Without `model` the page throws before it draws.
//!
//! What v2 answered as `model` (deployed bundle 2026-10-05, `server.mjs`):
//! `snapshot = (branch, actor) => execute("adminSnapshot", { branchId: branch },
//! actor)` (`:7606`), called for every action before it is dispatched (`:7612`)
//! and answered as `model` (`:7617`). The branch is a staff member's own and the
//! owner's `branchId` or `MAIN` (`:7609`). `execute` (`:6997`) is the same read
//! the owner's `/api/rpc` makes (`:7116`) and the staff member's `/api/rpc` makes
//! (`:8090-8099`); the staff view of `/api/staff/rpc` (`:7444-7458`: the employee
//! list, and RATES / SYSTEM losing transactions, customers, stock) is not on this
//! path. So every caller, staff included, is given what `/api/rpc` answers that
//! caller for `adminSnapshot` at the same instant — in v3,
//! `SnapshotBuilder::admin_snapshot` (`api/src/application/compat/snapshot.rs:86`).
//!
//! Everything goes through the server's whole route table with real sessions
//! (`rpc_common`). Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`);
//! nothing here skips.

mod common;
mod compat_common;
mod rpc_common;
mod shift_setup_common;
mod snapshot_common;

use std::collections::BTreeSet;

use rpc_common::{App, Call, Cookies, Desk, MAIN, RPC, one_of_each_role, sign_in};
use serde_json::{Value, json};

/// OR-4a's one new sentence (§10 `outOfScope`).
const OUT_OF_SCOPE: &str = "สาขานี้อยู่นอกขอบเขตของบัญชีนี้";
const ORIGIN: &str = "http://desk.test";
const DEVICES: &str = "/api/devices";

/// The first printer model of the configuration: the snapshot's `printerModel`
/// while nobody has set one (`devices_set_printer_test.rs`).
const NEOCAL: &str = "NEOCAL_NTP_8220BT";

/// Every key a devices snapshot answers once `model` is in it: the five it
/// answers today, unchanged, and `model`.
const SNAPSHOT_KEYS: [&str; 6] = [
    "model",
    "jobs",
    "events",
    "printerOnline",
    "printerModel",
    "hasDrawer",
];

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    /// Role name (`OWNER`, `FRONT`, …) → username.
    usernames: Vec<(String, String)>,
}

impl Room {
    async fn open() -> Room {
        let desk = Desk::open().await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let usernames = people
            .iter()
            .map(|p| (p.key.clone(), p.username.clone()))
            .collect();
        Room {
            desk,
            app,
            cookies,
            usernames,
        }
    }

    /// A call that must be answered 200, as JSON.
    async fn ok(&self, who: &str, path: &str, action: &str, payload: Value) -> Value {
        let answer = self
            .app
            .send(Call::action(ORIGIN, path, action, payload).cookie(&self.cookies[who]))
            .await;
        assert_eq!(
            answer.status, 200,
            "{action} on {path} as {who}: {:.400}",
            answer.text
        );
        answer.json()
    }

    /// The owner opens a shift in `MAIN`: the first thing `balance` reads.
    async fn open_a_shift(&self) {
        shift_setup_common::open_shift_by_count(
            &self.desk.db.pool,
            MAIN,
            json!({ "THB": 5000 }),
            chrono::Utc::now(),
        )
        .await;
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// `snapshot` as the owner: `model` is exactly what `/api/rpc` answers the owner
/// for `adminSnapshot` of the same branch at the same instant, and it holds what
/// the kept page reads.
#[actix_web::test]
async fn an_owner_snapshot_carries_the_admin_snapshot_as_model() {
    let room = Room::open().await;
    room.open_a_shift().await;

    let snapshot = room.ok("OWNER", DEVICES, "snapshot", json!({})).await;
    let admin = room
        .ok("OWNER", RPC, "adminSnapshot", json!({ "branchId": MAIN }))
        .await;
    assert_eq!(
        snapshot.get("model"),
        Some(&admin),
        "`model` is the admin snapshot /api/rpc answers the owner: {snapshot:.400}"
    );

    let model = &snapshot["model"];
    assert_eq!(model["branchId"], json!(MAIN), "the branch asked for");
    assert_eq!(model["actor"], json!("owner"), "printed as the caller");
    assert_eq!(
        model["stock"]["shifts"]
            .as_array()
            .map(|shifts| shifts.len()),
        Some(1),
        "`balance` finds the shift in `stock.shifts`: {model:.400}"
    );
    assert_eq!(model["stock"]["shifts"][0]["status"], json!("OPEN"));
    assert_eq!(model["stock"]["movements"], json!([]));
    assert_eq!(model["transactions"], json!([]));
    assert_eq!(
        model["branches"]
            .as_array()
            .map(|branches| branches.iter().any(|b| b["id"] == json!(MAIN))),
        Some(true),
        "the branch picker lists MAIN: {model:.400}"
    );
    room.finish().await;
}

/// `snapshot` as each of the nine staff roles: `model` is what `/api/rpc` answers
/// that employee for `adminSnapshot` — their own branch whatever they name, and
/// the same shifts, movements, transactions and customers the owner is shown,
/// not the staff view of `/api/staff/rpc`.
#[actix_web::test]
async fn every_staff_role_is_given_the_admin_snapshot_of_its_own_branch() {
    let room = Room::open().await;
    room.open_a_shift().await;
    let owners = room
        .ok("OWNER", RPC, "adminSnapshot", json!({ "branchId": MAIN }))
        .await;

    for (role, username) in room.usernames.iter().filter(|(role, _)| role != "OWNER") {
        // OR-4b (#767, D-4, §02 "Server ตรวจ … branch scope"): naming another branch is refused;
        // the employee's own branch is answered
        let outside = room
            .app
            .send(
                Call::action(ORIGIN, DEVICES, "snapshot", json!({ "branchId": "B404" }))
                    .cookie(&room.cookies[role.as_str()]),
            )
            .await;
        outside.assert_refused(403, OUT_OF_SCOPE, &format!("{role} names B404"));
        let snapshot = room
            .ok(role, DEVICES, "snapshot", json!({ "branchId": MAIN }))
            .await;
        let admin = room.ok(role, RPC, "adminSnapshot", json!({})).await;
        assert_eq!(
            snapshot.get("model"),
            Some(&admin),
            "{role}: `model` is the admin snapshot /api/rpc answers this employee: {snapshot:.400}"
        );

        let model = &snapshot["model"];
        assert_eq!(model["branchId"], json!(MAIN), "{role}: its own branch");
        assert_eq!(
            model["actor"],
            json!(username),
            "{role}: printed as the caller"
        );
        assert_eq!(
            model["stock"], owners["stock"],
            "{role}: the owner's shifts"
        );
        assert_eq!(model["transactions"], owners["transactions"], "{role}");
        assert_eq!(model["customers"], owners["customers"], "{role}");
        assert_eq!(model["rates"], owners["rates"], "{role}");
        assert_eq!(model.get("employee"), None, "{role}: not the staff view");
        assert_eq!(model.get("employees"), None, "{role}: not the staff view");
    }
    room.finish().await;
}

/// The keys beside `model` are the ones the snapshot answers today, with the
/// same values: `jobs`, `events`, `printerOnline`, `printerModel`, `hasDrawer`.
#[actix_web::test]
async fn the_device_keys_beside_model_are_unchanged() {
    let room = Room::open().await;
    let job = room
        .ok(
            "FRONT",
            DEVICES,
            "queue",
            json!({ "kind": "DRAWER_REQUEST", "ref": "",
                    "requestId": "aaaaaaaa-0000-4000-8000-000000000301", "reason": "เติมเงินทอน" }),
        )
        .await;
    room.ok(
        "BACKEND",
        DEVICES,
        "printerOnline",
        json!({ "online": false }),
    )
    .await;

    let expected: BTreeSet<&str> = SNAPSHOT_KEYS.into_iter().collect();
    for who in ["OWNER", "FRONT", "SYSTEM"] {
        let snapshot = room.ok(who, DEVICES, "snapshot", json!({})).await;
        let keys: BTreeSet<&str> = snapshot
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        assert_eq!(keys, expected, "{who}: the keys of a snapshot");
        assert_eq!(snapshot["jobs"], json!([job]), "{who}: the queued job");
        let kinds: Vec<&Value> = snapshot["events"]
            .as_array()
            .map(|events| events.iter().map(|e| &e["kind"]).collect())
            .unwrap_or_default();
        assert_eq!(
            kinds,
            vec![
                &json!("SIMULATED_PRINTER_CONNECTION"),
                &json!("DRAWER_REQUEST")
            ],
            "{who}: the events, newest first"
        );
        assert_eq!(snapshot["printerOnline"], json!(false), "{who}");
        assert_eq!(snapshot["printerModel"], json!(NEOCAL), "{who}");
        assert_eq!(snapshot["hasDrawer"], Value::Null, "{who}");
    }
    room.finish().await;
}
