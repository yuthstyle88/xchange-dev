//! Ticket #277 (DRV-3, #274 decision C2) — `POST /api/devices`, the route the devices
//! screen and the staff pages ask for a device action through. Written before the
//! implementation. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Built from the design (`docs/design/xchange-v3.md` §5.4b row "route", §6), not from a
//! recording of v2's HTTP wrapper (C2): v2's payload field names and answer keys are reused
//! where `api/tests/fixtures/devices/v2_device_cases.json` shows them; the refusals are the
//! ticket's table. Who may do what is `Policy::allow_devices` as it is, checked against the
//! fixture's own `roles` lists. `setPrinter` is not this ticket's (DRV-3b) and is not pinned.
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real
//! sessions made at the real sign-in doors (`rpc_common`).
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use std::collections::BTreeSet;

use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};

const ORIGIN: &str = "http://desk.test";
const DEVICES: &str = "/api/devices";

const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
const STEP_NOT_ALLOWED: &str = "บทบาทนี้ไม่มีสิทธิ์ทำขั้นตอนนี้";
const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
const ORIGIN_REJECTED: &str = "ORIGIN_REJECTED";
const TOO_LARGE: &str = "ข้อมูลมากเกินไป";
const DRAWER_NEEDS_REASON: &str = "ระบุเหตุผลเปิดลิ้นชัก";
const REQUEST_CHANGED: &str = "คำขอเดิมมีข้อมูลเปลี่ยน";
const NO_SUCH_JOB: &str = "ไม่พบงาน";
const BAD_STATE: &str = "สถานะไม่ถูกต้อง";
const BAD_BRANCH: &str = "สาขาไม่ถูกต้อง";
const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
/// OR-4a's one new sentence (§10 `outOfScope`).
const OUT_OF_SCOPE: &str = "สาขานี้อยู่นอกขอบเขตของบัญชีนี้";

/// The keys of a job in an answer: v2's row (`gen_devices.mjs`) without its `request_hash`.
const JOB_KEYS: [&str; 9] = [
    "id",
    "request_id",
    "branch_id",
    "ref",
    "kind",
    "status",
    "actor",
    "created",
    "updated",
];

/// The five actions the role table guards, with a payload that is valid apart from the role.
fn guarded_calls() -> Vec<(&'static str, Value)> {
    vec![
        (
            "queue",
            json!({ "kind": "DRAWER_REQUEST", "ref": "", "requestId": request_id(900), "reason": "เติมเงินทอน" }),
        ),
        (
            "process",
            json!({ "id": "7e570001-de71-4ce5-8000-000000000001" }),
        ),
        (
            "retry",
            json!({ "id": "7e570001-de71-4ce5-8000-000000000001" }),
        ),
        ("printerOnline", json!({ "online": false })),
        ("drawerObservation", json!({ "ref": "", "state": "OPEN" })),
    ]
}

/// Who may do each guarded action, as v2 recorded it.
fn recorded_roles() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/devices/v2_device_cases.json"
    );
    let text = std::fs::read_to_string(path).expect("read v2_device_cases.json");
    let file: Value = serde_json::from_str(&text).expect("v2_device_cases.json is JSON");
    file["roles"].clone()
}

fn allowed(roles: &Value, action: &str) -> BTreeSet<String> {
    let mut may: BTreeSet<String> = roles[action]
        .as_array()
        .unwrap_or_else(|| panic!("the fixture lists the roles of {action}"))
        .iter()
        .map(|r| r.as_str().expect("a role name").to_string())
        .collect();
    // OR-4b (#767, design §4.2 C7): B1 merges BACKEND and MANAGER into BRANCH_MANAGER, so MANAGER
    // gains what BACKEND may on the device steps
    if may.contains("BACKEND")
        && ["process", "retry", "drawerObservation", "printerOnline"].contains(&action)
    {
        may.insert("MANAGER".to_string());
    }
    may
}

/// A request id the service accepts: 36 characters of `0-9 a-f -`.
fn request_id(n: u32) -> String {
    format!("aaaaaaaa-0000-4000-8000-{n:012x}")
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
        let answer = self.ask(who, action, payload).await;
        assert_eq!(
            answer.status, 200,
            "{action} as {who}: {:.400}",
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

    /// How many jobs and lines the device tables hold, all branches.
    async fn written(&self) -> (i64, i64) {
        let count = |sql: &'static str| {
            let pool = self.desk.db.pool.clone();
            async move {
                sqlx::query_scalar::<_, i64>(sql)
                    .fetch_one(&pool)
                    .await
                    .unwrap_or_else(|e| panic!("{sql}: {e}"))
            }
        };
        (
            count("SELECT count(*) FROM device_jobs").await,
            count("SELECT count(*) FROM device_events").await,
        )
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

fn assert_job_shape(job: &Value, label: &str) {
    let keys: BTreeSet<&str> = job
        .as_object()
        .unwrap_or_else(|| panic!("{label}: a job is an object: {job}"))
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        JOB_KEYS.into_iter().collect::<BTreeSet<_>>(),
        "{label}: the keys of a job"
    );
    for key in ["created", "updated"] {
        let at = job[key].as_str().unwrap_or("");
        assert!(
            at.len() == 24 && at.ends_with('Z') && at.as_bytes()[19] == b'.',
            "{label}: {key} is an instant with milliseconds, as v2 wrote it: {at:?}"
        );
    }
}

fn job_ids(snapshot: &Value) -> Vec<String> {
    snapshot["jobs"]
        .as_array()
        .expect("the snapshot's jobs")
        .iter()
        .map(|j| j["id"].as_str().expect("a job id").to_string())
        .collect()
}

fn job_in<'a>(snapshot: &'a Value, id: &str) -> &'a Value {
    snapshot["jobs"]
        .as_array()
        .expect("the snapshot's jobs")
        .iter()
        .find(|j| j["id"] == id)
        .unwrap_or_else(|| panic!("job {id} is in the snapshot: {snapshot}"))
}

async fn queue_drawer(room: &Room, who: &str, n: u32, reason: &str) -> Value {
    room.ok(
        who,
        "queue",
        json!({ "kind": "DRAWER_REQUEST", "ref": "", "requestId": request_id(n), "reason": reason }),
    )
    .await
}

// ── 1. snapshot ──────────────────────────────────────────────────────────────

#[actix_web::test]
async fn snapshot_answers_the_overview_to_every_signed_in_role() {
    let room = Room::open().await;
    for person in one_of_each_role() {
        let snapshot = room.snapshot(&person.key).await;
        assert!(
            snapshot["jobs"].as_array().is_some_and(Vec::is_empty),
            "{}: no job yet: {snapshot}",
            person.key
        );
        assert!(
            snapshot["events"].is_array(),
            "{}: the lines: {snapshot}",
            person.key
        );
        assert_eq!(
            snapshot["printerOnline"],
            json!(true),
            "{}: a printer nobody switched is online",
            person.key
        );
    }
    room.finish().await;
}

// ── 2–4. queue ───────────────────────────────────────────────────────────────

#[actix_web::test]
async fn queue_answers_the_queued_job_to_every_role_on_its_list() {
    let room = Room::open().await;
    let roles = recorded_roles();
    let mut queued = Vec::new();
    for (n, who) in allowed(&roles, "queue").iter().enumerate() {
        let n = n as u32 + 1;
        let job = queue_drawer(&room, who, n, "ลูกค้าขอแลกเหรียญ").await;
        assert_job_shape(&job, who);
        assert_eq!(job["kind"], "DRAWER_REQUEST", "{who}");
        assert_eq!(job["status"], "QUEUED", "{who}");
        assert_eq!(job["ref"], "", "{who}");
        assert_eq!(job["branch_id"], "MAIN", "{who}");
        assert_eq!(job["request_id"], request_id(n), "{who}");
        queued.push(job["id"].as_str().expect("a job id").to_string());
    }
    assert_eq!(queued.len(), 4, "OWNER, FRONT, BACKEND, MANAGER");
    let ids = job_ids(&room.snapshot("OWNER").await);
    for id in &queued {
        assert!(
            ids.contains(id),
            "the queued job {id} is in the snapshot: {ids:?}"
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn a_drawer_request_without_a_reason_is_refused_and_queues_nothing() {
    let room = Room::open().await;
    for (n, reason) in [(1, ""), (2, "   "), (3, "\t\n")] {
        let answer = room
            .ask(
                "FRONT",
                "queue",
                json!({ "kind": "DRAWER_REQUEST", "ref": "", "requestId": request_id(n), "reason": reason }),
            )
            .await;
        refused(
            &answer,
            400,
            DRAWER_NEEDS_REASON,
            &format!("reason {reason:?}"),
        );
    }
    let answer = room
        .ask(
            "FRONT",
            "queue",
            json!({ "kind": "DRAWER_REQUEST", "ref": "", "requestId": request_id(4) }),
        )
        .await;
    refused(&answer, 400, DRAWER_NEEDS_REASON, "no reason field at all");
    assert_eq!(room.written().await, (0, 0), "a refusal writes nothing");
    room.finish().await;
}

#[actix_web::test]
async fn the_same_request_twice_is_the_same_job() {
    let room = Room::open().await;
    let first = queue_drawer(&room, "FRONT", 7, "เติมเงินทอน").await;
    let again = queue_drawer(&room, "FRONT", 7, "เติมเงินทอน").await;
    assert_eq!(
        again, first,
        "the same request again answers the job it made"
    );
    let changed = room
        .ask(
            "FRONT",
            "queue",
            json!({ "kind": "DRAWER_REQUEST", "ref": "", "requestId": request_id(7), "reason": "อีกเหตุผล" }),
        )
        .await;
    refused(
        &changed,
        400,
        REQUEST_CHANGED,
        "the same request id, another reason",
    );
    assert_eq!(job_ids(&room.snapshot("OWNER").await).len(), 1, "one job");
    room.finish().await;
}

// ── 5–6. process, retry, the printer switch ─────────────────────────────────

#[actix_web::test]
async fn process_carries_a_queued_job_out() {
    let room = Room::open().await;
    let queued = queue_drawer(&room, "MANAGER", 1, "เปิดตรวจนับ").await;
    let id = queued["id"].as_str().expect("a job id").to_string();

    let done = room.ok("FRONT", "process", json!({ "id": id })).await;
    assert_job_shape(&done, "process");
    assert_eq!(done["id"], id.as_str());
    assert_eq!(
        done["status"], "COMMAND_SIMULATED",
        "a simulated drawer kick"
    );
    assert_eq!(
        job_in(&room.snapshot("OWNER").await, &id)["status"],
        "COMMAND_SIMULATED"
    );

    let missing = room
        .ask(
            "BACKEND",
            "process",
            json!({ "id": "7e570009-de71-4ce5-8000-000000000009" }),
        )
        .await;
    refused(&missing, 400, NO_SUCH_JOB, "a job nobody queued");
    room.finish().await;
}

#[actix_web::test]
async fn a_failed_job_can_be_retried_and_the_printer_switch_shows() {
    let room = Room::open().await;
    let off = room
        .ok("BACKEND", "printerOnline", json!({ "online": false }))
        .await;
    assert_eq!(off, json!({ "saved": true }), "printerOnline answers saved");
    assert_eq!(room.snapshot("FRONT").await["printerOnline"], json!(false));

    let queued = queue_drawer(&room, "OWNER", 1, "เปิดตรวจนับ").await;
    let id = queued["id"].as_str().expect("a job id").to_string();
    let failed = room.ok("OWNER", "process", json!({ "id": id })).await;
    assert_eq!(failed["status"], "FAILED", "the printer is off");

    let retried = room.ok("FRONT", "retry", json!({ "id": id })).await;
    assert_eq!(retried, json!({ "saved": true }), "retry answers saved");
    assert_eq!(
        job_in(&room.snapshot("OWNER").await, &id)["status"],
        "QUEUED"
    );

    room.ok("OWNER", "printerOnline", json!({ "online": true }))
        .await;
    assert_eq!(room.snapshot("BACKEND").await["printerOnline"], json!(true));
    room.finish().await;
}

// ── 7. drawerObservation ─────────────────────────────────────────────────────

#[actix_web::test]
async fn a_drawer_observation_is_written_down() {
    let room = Room::open().await;
    let saved = room
        .ok(
            "FRONT",
            "drawerObservation",
            json!({ "ref": "", "state": "OPEN" }),
        )
        .await;
    assert_eq!(saved, json!({ "saved": true }));
    let snapshot = room.snapshot("OWNER").await;
    let events = snapshot["events"].as_array().expect("the snapshot's lines");
    assert_eq!(events.len(), 1, "one line: {snapshot}");
    assert_eq!(events[0]["kind"], "DRAWER_MANUAL_OBSERVATION");
    assert_eq!(events[0]["ref"], "");
    assert!(
        events[0]["at"].is_string(),
        "a line has its time: {snapshot}"
    );

    let ajar = room
        .ask(
            "BACKEND",
            "drawerObservation",
            json!({ "ref": "", "state": "AJAR" }),
        )
        .await;
    refused(
        &ajar,
        400,
        BAD_STATE,
        "a state that is neither OPEN nor CLOSED",
    );
    room.finish().await;
}

// ── 8–10. who may ask ────────────────────────────────────────────────────────

#[actix_web::test]
async fn each_action_is_refused_to_a_role_outside_its_list() {
    let room = Room::open().await;
    let roles = recorded_roles();
    let mut checked = 0;
    for (action, payload) in guarded_calls() {
        let may = allowed(&roles, action);
        for person in one_of_each_role() {
            if may.contains(&person.key) {
                continue;
            }
            let answer = room.ask(&person.key, action, payload.clone()).await;
            refused(
                &answer,
                400,
                STEP_NOT_ALLOWED,
                &format!("{action} as {}", person.key),
            );
            checked += 1;
        }
    }
    assert!(checked >= 30, "every refused cell is asked: {checked}");
    assert_eq!(room.written().await, (0, 0), "a refusal writes nothing");
    room.finish().await;
}

#[actix_web::test]
async fn nobody_signed_in_is_asked_to_sign_in() {
    let room = Room::open().await;
    let mut calls = guarded_calls();
    calls.push(("snapshot", json!({})));
    calls.push(("nope", json!({})));
    for (action, payload) in calls {
        let answer = room
            .app
            .send(Call::action(ORIGIN, DEVICES, action, payload))
            .await;
        refused(&answer, 401, SIGN_IN, &format!("{action} with no session"));
    }
    assert_eq!(room.written().await, (0, 0), "nothing is written");
    room.finish().await;
}

#[actix_web::test]
async fn an_unknown_action_is_a_bad_request() {
    let room = Room::open().await;
    for person in one_of_each_role() {
        for action in ["nope", "", "Snapshot"] {
            let answer = room.ask(&person.key, action, json!({})).await;
            refused(
                &answer,
                400,
                BAD_REQUEST,
                &format!("{action:?} as {}", person.key),
            );
        }
    }
    assert_eq!(room.written().await, (0, 0), "nothing is written");
    room.finish().await;
}

// ── 11. the branch ───────────────────────────────────────────────────────────

#[actix_web::test]
async fn the_branch_is_the_policys() {
    let room = Room::open().await;
    // OR-4b (#767, D-4, §02 "Server ตรวจ … branch scope"): a staff member naming a branch outside
    // their scope is refused; naming their own, they work on it
    let outside = room
        .ask(
            "FRONT",
            "queue",
            json!({ "kind": "DRAWER_REQUEST", "ref": "", "requestId": request_id(1),
                    "reason": "เติมเงินทอน", "branchId": "B404" }),
        )
        .await;
    refused(
        &outside,
        403,
        OUT_OF_SCOPE,
        "a staff member names another branch",
    );
    let job = room
        .ok(
            "FRONT",
            "queue",
            json!({ "kind": "DRAWER_REQUEST", "ref": "", "requestId": request_id(1),
                    "reason": "เติมเงินทอน", "branchId": "MAIN" }),
        )
        .await;
    assert_eq!(
        job["branch_id"], "MAIN",
        "a staff member works on their own branch"
    );

    let unknown = room
        .ask("OWNER", "snapshot", json!({ "branchId": "B404" }))
        .await;
    refused(
        &unknown,
        400,
        NO_SUCH_BRANCH,
        "the owner names a branch that does not exist",
    );
    let malformed = room
        .ask("OWNER", "snapshot", json!({ "branchId": "no such branch" }))
        .await;
    refused(
        &malformed,
        400,
        BAD_BRANCH,
        "the owner names a malformed branch",
    );
    let main = room
        .ok("OWNER", "snapshot", json!({ "branchId": "MAIN" }))
        .await;
    assert_eq!(job_ids(&main).len(), 1, "the owner sees MAIN's job: {main}");
    room.finish().await;
}

// ── 12–13. the request rules ─────────────────────────────────────────────────

#[actix_web::test]
async fn a_write_without_the_marker_header_is_refused() {
    let room = Room::open().await;
    let call = Call::action(
        ORIGIN,
        DEVICES,
        "drawerObservation",
        json!({ "ref": "", "state": "OPEN" }),
    )
    .without("x-xchange-request")
    .cookie(&room.cookies["OWNER"]);
    let answer = room.app.send(call).await;
    refused(&answer, 400, ORIGIN_REJECTED, "no X-Xchange-Request");

    let call = Call::action(
        ORIGIN,
        DEVICES,
        "drawerObservation",
        json!({ "ref": "", "state": "OPEN" }),
    )
    .header("origin", "http://elsewhere.test")
    .cookie(&room.cookies["OWNER"]);
    let answer = room.app.send(call).await;
    refused(&answer, 400, ORIGIN_REJECTED, "another origin");
    assert_eq!(room.written().await, (0, 0), "nothing is written");
    room.finish().await;
}

/// A body of exactly `units` characters (all ASCII, so as many UTF-16 units).
fn body_of_length(units: usize) -> String {
    let head = r#"{"action":"drawerObservation","payload":{"ref":"","state":"OPEN","pad":""#;
    let tail = r#""}}"#;
    let body = format!(
        "{head}{}{tail}",
        "x".repeat(units - head.len() - tail.len())
    );
    assert_eq!(body.len(), units);
    body
}

#[actix_web::test]
async fn the_body_limit_is_twenty_thousand_units() {
    let room = Room::open().await;
    let at_limit = room
        .app
        .send(Call::post(ORIGIN, DEVICES, body_of_length(20_000)).cookie(&room.cookies["FRONT"]))
        .await;
    assert_eq!(
        at_limit.status, 200,
        "20,000 units are read: {:.300}",
        at_limit.text
    );
    assert_eq!(at_limit.json(), json!({ "saved": true }));

    let over = room
        .app
        .send(Call::post(ORIGIN, DEVICES, body_of_length(20_001)).cookie(&room.cookies["FRONT"]))
        .await;
    refused(&over, 400, TOO_LARGE, "20,001 units");
    assert_eq!(
        room.written().await,
        (0, 1),
        "only the first one wrote its line"
    );
    room.finish().await;
}
