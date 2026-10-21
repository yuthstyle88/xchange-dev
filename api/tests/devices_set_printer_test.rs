//! DRV-3b (#274) — `setPrinter` and `setDrawer` on `POST /api/devices`, and the two keys
//! `printerModel` and `hasDrawer` in `snapshot`. Written before the implementation. Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Built from the design (`docs/design/xchange-v3.md` §5.4b and §10 row DRV-3b, as amended by
//! design PR 295): both actions are the owner's only, every staff role is refused with the
//! path's own sentence, `setPrinter` reaches `DeviceService::set_printer(branch, kind, actor,
//! now)` and `setDrawer` reaches `DeviceService::set_drawer(branch, model, has_drawer, actor,
//! now)`. The refusal sentences are the service's (`BAD_PRINTER`, `NO_DRAWER`). `model` in the
//! snapshot stays T13b's admin snapshot and is never the printer model.
//!
//! The six actions DRV-3 answers keep their answers; `devices_route_test.rs` pins them and is
//! not touched. Everything goes through the server's whole route table
//! (`bootstrap::configure`) with real sessions made at the real sign-in doors (`rpc_common`).
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use xc_kernel::ids::BranchId;
use xchange::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use xchange::infrastructure::devices::printer_settings::{PrinterKind, PrinterSettings};

const ORIGIN: &str = "http://desk.test";
const DEVICES: &str = "/api/devices";

const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
const STEP_NOT_ALLOWED: &str = "บทบาทนี้ไม่มีสิทธิ์ทำขั้นตอนนี้";
/// `device_service.rs` `BAD_PRINTER`: DRV-1's and DRV-4's refusal of a printer setting.
const BAD_PRINTER: &str = "ตั้งค่าเครื่องพิมพ์ไม่ถูกต้อง";
/// `device_service.rs` `NO_DRAWER`: DRV-4's refusal of a drawer request where `hasDrawer = 0`.
const NO_DRAWER: &str = "สาขานี้ไม่มีลิ้นชัก";

/// The first model of the registry `DeviceService::new` keeps (`PrinterModels::built_in`).
const NEOCAL: &str = "NEOCAL_NTP_8220BT";
const MAIN: &str = "MAIN";

/// A request id the service accepts: 36 characters of `0-9 a-f -`.
fn request_id(n: u32) -> String {
    format!("bbbbbbbb-0000-4000-8000-{n:012x}")
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

    async fn ask(&self, who: &str, action: &str, payload: Value) -> Answer {
        self.app
            .send(Call::action(ORIGIN, DEVICES, action, payload).cookie(&self.cookies[who]))
            .await
    }

    /// The answer of a call that must be accepted, as JSON.
    async fn ok(&self, who: &str, action: &str, payload: Value) -> Value {
        let answer = self.ask(who, action, payload.clone()).await;
        assert_eq!(
            answer.status, 200,
            "{action} {payload} as {who}: {:.400}",
            answer.text
        );
        assert_eq!(
            answer.header("cache-control"),
            Some("no-store"),
            "{action} as {who}: never cached"
        );
        answer.json()
    }

    async fn snapshot(&self, who: &str) -> Value {
        self.ok(who, "snapshot", json!({})).await
    }

    /// Every row of `device_settings`, all branches, in a fixed order.
    async fn settings(&self) -> Vec<(String, String, String)> {
        sqlx::query_as::<_, (String, String, String)>(
            "SELECT branch_id, key, value FROM device_settings ORDER BY branch_id, key",
        )
        .fetch_all(&self.desk.db.pool)
        .await
        .expect("read device_settings")
    }

    /// The branch's printer as a transaction of its own reads it (DRV-1's reader).
    async fn printer(&self, branch_id: &str) -> PrinterSettings {
        let repo = PgDeviceRepository::new(self.desk.db.pool.clone());
        let mut tx = self.desk.db.pool.begin().await.expect("begin");
        let settings = PrinterSettings::read_in(&mut tx, &repo, &BranchId::new(branch_id))
            .await
            .expect("read the printer settings");
        tx.rollback().await.expect("rollback");
        settings
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn refused(answer: &Answer, status: u16, sentence: &str, label: &str) {
    answer.assert_refused(status, sentence, label);
    assert_eq!(
        answer.header("cache-control"),
        Some("no-store"),
        "{label}: a refusal is never cached"
    );
}

/// The value of a key that must be present (indexing a missing key would also give `null`).
fn present<'a>(snapshot: &'a Value, key: &str) -> &'a Value {
    snapshot
        .as_object()
        .unwrap_or_else(|| panic!("the snapshot is an object: {snapshot}"))
        .get(key)
        .unwrap_or_else(|| panic!("the snapshot carries `{key}`: {snapshot}"))
}

fn network(host: &str, port: u16) -> PrinterSettings {
    PrinterSettings {
        online: true,
        kind: PrinterKind::Network(host.to_string(), port),
    }
}

fn simulated() -> PrinterSettings {
    PrinterSettings {
        online: true,
        kind: PrinterKind::Simulated,
    }
}

/// The staff roles: everyone `one_of_each_role` makes except the owner.
fn staff_keys() -> Vec<String> {
    one_of_each_role()
        .into_iter()
        .map(|p| p.key)
        .filter(|k| k != "OWNER")
        .collect()
}

// ── 1–4. setPrinter ──────────────────────────────────────────────────────────

#[actix_web::test]
async fn the_owner_sets_a_network_printer_and_then_a_simulated_one() {
    let room = Room::open().await;
    assert_eq!(room.printer(MAIN).await, simulated(), "nobody set it yet");

    let saved = room
        .ok(
            "OWNER",
            "setPrinter",
            json!({ "kind": "NETWORK", "host": "192.168.1.50", "port": 9101 }),
        )
        .await;
    assert_eq!(saved, json!({ "saved": true }), "setPrinter answers saved");
    assert_eq!(
        room.printer(MAIN).await,
        network("192.168.1.50", 9101),
        "the branch reads the printer the owner set"
    );

    let saved = room
        .ok("OWNER", "setPrinter", json!({ "kind": "SIMULATED" }))
        .await;
    assert_eq!(saved, json!({ "saved": true }));
    assert_eq!(
        room.printer(MAIN).await,
        simulated(),
        "SIMULATED is the simulated printer again"
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_network_printer_named_without_a_port_listens_on_9100() {
    let room = Room::open().await;
    let saved = room
        .ok(
            "OWNER",
            "setPrinter",
            json!({ "kind": "NETWORK", "host": "printer.branch.local" }),
        )
        .await;
    assert_eq!(saved, json!({ "saved": true }));
    assert_eq!(
        room.printer(MAIN).await,
        network("printer.branch.local", 9100),
        "no port is the default port (printer_settings.rs DEFAULT_PORT)"
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_network_printer_that_cannot_be_reached_is_refused_and_nothing_is_stored() {
    let room = Room::open().await;
    let before = room.settings().await;
    let cases = [
        ("no host", json!({ "kind": "NETWORK", "port": 9100 })),
        (
            "blank host",
            json!({ "kind": "NETWORK", "host": "   ", "port": 9100 }),
        ),
        (
            "host not text",
            json!({ "kind": "NETWORK", "host": 42, "port": 9100 }),
        ),
        (
            "port 0",
            json!({ "kind": "NETWORK", "host": "192.168.1.50", "port": 0 }),
        ),
        (
            "port 65536",
            json!({ "kind": "NETWORK", "host": "192.168.1.50", "port": 65536 }),
        ),
        (
            "port -1",
            json!({ "kind": "NETWORK", "host": "192.168.1.50", "port": -1 }),
        ),
        (
            "port 1.5",
            json!({ "kind": "NETWORK", "host": "192.168.1.50", "port": 1.5 }),
        ),
        (
            "port as text",
            json!({ "kind": "NETWORK", "host": "192.168.1.50", "port": "9100" }),
        ),
    ];
    for (label, payload) in cases {
        let answer = room.ask("OWNER", "setPrinter", payload).await;
        refused(&answer, 400, BAD_PRINTER, label);
    }
    assert_eq!(room.settings().await, before, "a refusal stores nothing");
    assert_eq!(room.printer(MAIN).await, simulated());
    room.finish().await;
}

#[actix_web::test]
async fn an_unknown_printer_kind_is_refused_and_nothing_is_stored() {
    let room = Room::open().await;
    let before = room.settings().await;
    let cases = [
        ("kind USB", json!({ "kind": "USB" })),
        ("kind empty", json!({ "kind": "" })),
        (
            "kind in lower case",
            json!({ "kind": "network", "host": "192.168.1.50", "port": 9100 }),
        ),
        ("no kind", json!({ "host": "192.168.1.50", "port": 9100 })),
        ("kind not text", json!({ "kind": 1 })),
    ];
    for (label, payload) in cases {
        let answer = room.ask("OWNER", "setPrinter", payload).await;
        refused(&answer, 400, BAD_PRINTER, label);
    }
    assert_eq!(room.settings().await, before, "a refusal stores nothing");
    room.finish().await;
}

/// `SIMULATION` is the driver's `mode()` (the word every v2 queue line carries), not a stored
/// printer kind: `printer_settings.rs` reads only `SIMULATED` and `NETWORK`.
#[actix_web::test]
async fn simulation_is_the_driver_mode_not_a_printer_kind() {
    let room = Room::open().await;
    let before = room.settings().await;
    let answer = room
        .ask("OWNER", "setPrinter", json!({ "kind": "SIMULATION" }))
        .await;
    refused(&answer, 400, BAD_PRINTER, "kind SIMULATION");
    assert_eq!(room.settings().await, before, "a refusal stores nothing");
    room.finish().await;
}

// ── 5–8. setDrawer and the snapshot ──────────────────────────────────────────

#[actix_web::test]
async fn the_snapshot_shows_the_first_model_and_no_drawer_answer_until_the_owner_sets_them() {
    let room = Room::open().await;
    for who in ["OWNER", "FRONT"] {
        let snapshot = room.snapshot(who).await;
        assert_eq!(
            present(&snapshot, "printerModel"),
            &json!(NEOCAL),
            "{who}: no setting is the first model"
        );
        assert_eq!(
            present(&snapshot, "hasDrawer"),
            &Value::Null,
            "{who}: nobody said whether there is a drawer"
        );
        assert!(
            !snapshot["model"].is_string(),
            "{who}: `model` is T13b's admin snapshot, never the printer model: {snapshot}"
        );
    }

    let saved = room
        .ok(
            "OWNER",
            "setDrawer",
            json!({ "printerModel": NEOCAL, "hasDrawer": false }),
        )
        .await;
    assert_eq!(saved, json!({ "saved": true }), "setDrawer answers saved");
    for who in ["OWNER", "FRONT"] {
        let snapshot = room.snapshot(who).await;
        assert_eq!(present(&snapshot, "printerModel"), &json!(NEOCAL), "{who}");
        assert_eq!(
            present(&snapshot, "hasDrawer"),
            &json!(false),
            "{who}: the branch has no drawer"
        );
    }

    room.ok(
        "OWNER",
        "setDrawer",
        json!({ "printerModel": NEOCAL, "hasDrawer": true }),
    )
    .await;
    let snapshot = room.snapshot("BACKEND").await;
    assert_eq!(present(&snapshot, "hasDrawer"), &json!(true));
    assert_eq!(
        present(&snapshot, "printerOnline"),
        &json!(true),
        "the snapshot keeps DRV-3's keys"
    );
    assert!(snapshot["jobs"].is_array() && snapshot["events"].is_array());
    room.finish().await;
}

#[actix_web::test]
async fn an_unknown_printer_model_is_refused_and_nothing_is_stored() {
    let room = Room::open().await;
    let before = room.settings().await;
    let cases = [
        (
            "a model the registry does not hold",
            json!({ "printerModel": "EPSON_TM_T82", "hasDrawer": true }),
        ),
        (
            "the id in lower case",
            json!({ "printerModel": "neocal_ntp_8220bt", "hasDrawer": true }),
        ),
        (
            "the model's name, not its id",
            json!({ "printerModel": "NEOCAL NTP-8220BT", "hasDrawer": true }),
        ),
        ("empty", json!({ "printerModel": "", "hasDrawer": true })),
        ("no printerModel", json!({ "hasDrawer": true })),
    ];
    for (label, payload) in cases {
        let answer = room.ask("OWNER", "setDrawer", payload).await;
        refused(&answer, 400, BAD_PRINTER, label);
    }
    assert_eq!(room.settings().await, before, "a refusal stores nothing");
    let snapshot = room.snapshot("OWNER").await;
    assert_eq!(present(&snapshot, "printerModel"), &json!(NEOCAL));
    assert_eq!(present(&snapshot, "hasDrawer"), &Value::Null);
    room.finish().await;
}

/// `DeviceService::set_drawer` takes `has_drawer: bool` (DRV-4), so the route has no value to
/// hand it for `null`: only the JSON `true` and `false` are answers, everything else is refused.
#[actix_web::test]
async fn has_drawer_is_true_or_false_and_nothing_else() {
    let room = Room::open().await;
    let before = room.settings().await;
    let cases = [
        (
            "null",
            json!({ "printerModel": NEOCAL, "hasDrawer": Value::Null }),
        ),
        ("missing", json!({ "printerModel": NEOCAL })),
        (
            "the text true",
            json!({ "printerModel": NEOCAL, "hasDrawer": "true" }),
        ),
        (
            "the number 1",
            json!({ "printerModel": NEOCAL, "hasDrawer": 1 }),
        ),
        (
            "the text 0",
            json!({ "printerModel": NEOCAL, "hasDrawer": "0" }),
        ),
    ];
    for (label, payload) in cases {
        let answer = room.ask("OWNER", "setDrawer", payload).await;
        refused(&answer, 400, BAD_PRINTER, &format!("hasDrawer {label}"));
    }
    assert_eq!(room.settings().await, before, "a refusal stores nothing");
    assert_eq!(
        present(&room.snapshot("OWNER").await, "hasDrawer"),
        &Value::Null
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_branch_the_owner_set_without_a_drawer_refuses_a_drawer_request() {
    let room = Room::open().await;
    room.ok(
        "OWNER",
        "setDrawer",
        json!({ "printerModel": NEOCAL, "hasDrawer": false }),
    )
    .await;
    let answer = room
        .ask(
            "FRONT",
            "queue",
            json!({ "kind": "DRAWER_REQUEST", "ref": "", "requestId": request_id(1), "reason": "เติมเงินทอน" }),
        )
        .await;
    refused(
        &answer,
        400,
        NO_DRAWER,
        "setDrawer writes the switch the queue reads",
    );

    room.ok(
        "OWNER",
        "setDrawer",
        json!({ "printerModel": NEOCAL, "hasDrawer": true }),
    )
    .await;
    let job = room
        .ok(
            "FRONT",
            "queue",
            json!({ "kind": "DRAWER_REQUEST", "ref": "", "requestId": request_id(2), "reason": "เติมเงินทอน" }),
        )
        .await;
    assert_eq!(
        job["status"],
        json!("QUEUED"),
        "a drawer: the request queues"
    );
    room.finish().await;
}

// ── 9–10. who may ask ────────────────────────────────────────────────────────

#[actix_web::test]
async fn every_staff_role_is_refused_both_settings_and_nothing_is_stored() {
    let room = Room::open().await;
    let before = room.settings().await;
    let calls = [
        (
            "setPrinter",
            json!({ "kind": "NETWORK", "host": "192.168.1.50", "port": 9100 }),
        ),
        (
            "setDrawer",
            json!({ "printerModel": NEOCAL, "hasDrawer": false }),
        ),
    ];
    let mut checked = 0;
    // OR-4b (#767, design §4.2 C5): SystemSettings is ADMIN's too (§02 ADMIN "ตั้งค่าระบบ"), and B1
    // makes SYSTEM an ADMIN; SYSTEM is asked below
    for who in staff_keys().into_iter().filter(|k| k != "SYSTEM") {
        for (action, payload) in &calls {
            let answer = room.ask(&who, action, payload.clone()).await;
            refused(
                &answer,
                400,
                STEP_NOT_ALLOWED,
                &format!("{action} as {who}"),
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 16, "eight staff roles, two actions");
    assert_eq!(room.settings().await, before, "a refusal stores nothing");
    // C5: SYSTEM sets both
    for (action, payload) in &calls {
        let answer = room.ask("SYSTEM", action, payload.clone()).await;
        assert_eq!(
            answer.status, 200,
            "{action} as SYSTEM (C5): {:.300}",
            answer.text
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn nobody_signed_in_is_asked_to_sign_in_for_either_setting() {
    let room = Room::open().await;
    let before = room.settings().await;
    let calls = [
        (
            "setPrinter",
            json!({ "kind": "NETWORK", "host": "192.168.1.50", "port": 9100 }),
        ),
        (
            "setDrawer",
            json!({ "printerModel": NEOCAL, "hasDrawer": false }),
        ),
    ];
    for (action, payload) in calls {
        let answer = room
            .app
            .send(Call::action(ORIGIN, DEVICES, action, payload))
            .await;
        refused(&answer, 401, SIGN_IN, &format!("{action} with no session"));
    }
    assert_eq!(room.settings().await, before, "nothing is stored");
    room.finish().await;
}
