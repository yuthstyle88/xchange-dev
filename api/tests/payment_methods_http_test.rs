//! Written before the implementation (row PM-API, design `docs/design/xchange-v3.md` §5.5.2).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `POST /api/payment-methods` — the owner switches each of the five channels on or off, for
//! BUY and SELL separately, per branch; every signed-in employee reads their own branch only;
//! only the owner sees the history of changes.
//!
//! `fixtures/payment_methods/v2_payment_methods_cases.json` is what the 10-06 bundle's own
//! `api()` answered, step by step (`api/tools/v2-fixtures/gen_payment_methods.mjs`): the
//! seventeen refusals of §5.5.2 in v2's order, and `list` / `save` for the owner and for
//! employees of `MAIN` and `AIRPORT`. Every step is sent here through the server's whole route
//! table with real sessions (`rpc_common`) and held to v2's status and body, with
//! `Cache-Control: no-store` on every answer.
//!
//! Two answers are the design's rather than the recording's:
//!
//! - Row 8, a well-formed branch that does not exist: §5.5.2 requires 400 `ไม่พบสาขา`. The
//!   recording has v2's 500 `โหลดการตั้งค่าไม่ได้ …` there, because v2's `model(branch)`
//!   (`server.mjs:7913`, the engine's `adminSnapshot`) throws the engine's own sentence for an
//!   unknown branch (`server.mjs:6798`) without a status before the route's check is reached.
//! - A body that is not JSON: §5.5.2 decides 400 `คำขอไม่ถูกต้อง` (v2 answered 500); listed in
//!   `docs/design/xchange-v3-known-differences.md`. Not in the recording.
//!
//! Row 17 (a database failure) is made by dropping the two tables of row SET under the server.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use std::collections::BTreeMap;

use compat_common::Person;
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde::Deserialize;
use serde_json::{Value, json};
use snapshot_common::time;

const ORIGIN: &str = "http://desk.test";
const ROUTE: &str = "/api/payment-methods";
const AIRPORT: &str = "AIRPORT";

const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
const OWNER_ONLY: &str = "เฉพาะเจ้าของระบบเท่านั้นที่ตั้งค่าวิธีชำระเงินได้";
const NO_SUCH_BRANCH: &str = "ไม่พบสาขา";
/// OR-4a's one new sentence (§10 `outOfScope`).
const OUT_OF_SCOPE: &str = "สาขานี้อยู่นอกขอบเขตของบัญชีนี้";
const NO_BUY_HERE: &str = "การจ่ายเงินบาทให้ลูกค้ารองรับเงินสดและโอนธนาคาร";
const ONE_PER_DIRECTION: &str = "ต้องเหลือช่องทางรับเงินและจ่ายเงินบาทอย่างน้อยประเภทละ 1 ช่องทาง";
const STALE: &str = "มีผู้แก้ไขการตั้งค่านี้แล้ว กรุณาโหลดล่าสุดก่อนบันทึก";

/// The seventeen refusals of §5.5.2, in v2's order: (row, status, sentence).
const REFUSALS: [(u8, u16, &str); 17] = [
    (1, 401, "กรุณาเข้าสู่ระบบ"),
    (2, 403, "Request rejected."),
    (3, 400, "คำขอใหญ่เกินกำหนด"),
    (4, 400, "คำขอไม่ถูกต้อง"),
    (5, 400, "สาขาไม่ถูกต้อง"),
    (6, 400, "คำขอไม่ถูกต้อง"),
    (7, 403, "เฉพาะเจ้าของระบบเท่านั้นที่ตั้งค่าวิธีชำระเงินได้"),
    (8, 400, "ไม่พบสาขา"),
    (9, 400, "กรุณาตั้งค่าให้ครบทั้ง 5 ช่องทาง"),
    (10, 400, "ข้อมูลช่องทางชำระเงินไม่ถูกต้อง"),
    (11, 400, "การจ่ายเงินบาทให้ลูกค้ารองรับเงินสดและโอนธนาคาร"),
    (
        12,
        400,
        "ช่องทางที่เปิดต้องเลือกการรับเงินหรือจ่ายเงินอย่างน้อยหนึ่งประเภท",
    ),
    (13, 400, "ข้อความยาวเกินกำหนดหรือไม่ถูกต้อง"),
    (
        14,
        400,
        "ต้องเหลือช่องทางรับเงินและจ่ายเงินบาทอย่างน้อยประเภทละ 1 ช่องทาง",
    ),
    (15, 400, "เวอร์ชันข้อมูลไม่ถูกต้อง"),
    (16, 409, "มีผู้แก้ไขการตั้งค่านี้แล้ว กรุณาโหลดล่าสุดก่อนบันทึก"),
    (
        17,
        500,
        "โหลดการตั้งค่าไม่ได้ กรุณาตรวจการเชื่อมต่อและติดตั้งตารางวิธีชำระเงิน",
    ),
];

// ── the recording ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RecordedPerson {
    key: String,
    kind: String,
    id: String,
    username: String,
    role: String,
    #[serde(rename = "branchId")]
    branch_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Big {
    head: String,
    unit: String,
    count: usize,
    tail: String,
    units: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    n: usize,
    label: String,
    at: String,
    #[serde(rename = "as")]
    sent_as: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, Option<String>>,
    body: Option<Value>,
    raw: Option<String>,
    big: Option<Big>,
    #[serde(default)]
    database_down: bool,
    row: Option<u8>,
    status: u16,
    answer: Value,
}

#[derive(Debug, Deserialize)]
struct Recording {
    origin: String,
    route: String,
    branches: Vec<String>,
    people: Vec<RecordedPerson>,
    steps: Vec<Step>,
}

fn recording() -> Recording {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/payment_methods/v2_payment_methods_cases.json"
    );
    let text = std::fs::read_to_string(path).expect("read v2_payment_methods_cases.json");
    serde_json::from_str(&text).expect("v2_payment_methods_cases.json parses")
}

impl Step {
    fn text(&self) -> String {
        if let Some(raw) = &self.raw {
            return raw.clone();
        }
        if let Some(big) = &self.big {
            let text = format!("{}{}{}", big.head, big.unit.repeat(big.count), big.tail);
            assert_eq!(text.encode_utf16().count(), big.units, "#{}", self.n);
            return text;
        }
        self.body
            .as_ref()
            .unwrap_or_else(|| panic!("#{} has a body", self.n))
            .to_string()
    }
}

fn person(p: &RecordedPerson) -> Person {
    Person {
        key: p.key.clone(),
        kind: p.kind.clone(),
        id: p.id.clone(),
        username: p.username.clone(),
        name: p.username.clone(),
        role: p.role.clone(),
        branch_id: p.branch_id.clone(),
        must_change: false,
    }
}

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    async fn open(people: &[Person]) -> Room {
        let desk = Desk::open().await;
        desk.db.seed_branch(AIRPORT, "Airport").await;
        let app = rpc_app!(desk);
        desk.employ(people).await;
        let cookies = sign_in(&app, ORIGIN, people).await;
        Room { desk, app, cookies }
    }

    /// The owner and one employee of every staff role in `MAIN`, and a Front of `AIRPORT`.
    async fn everyone() -> Room {
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
        Room::open(&people).await
    }

    async fn send(&self, who: Option<&str>, body: &str) -> Answer {
        let mut call = Call::post(ORIGIN, ROUTE, body.to_string());
        if let Some(who) = who {
            call = call.cookie(&self.cookies[who]);
        }
        let answer = self.app.send(call).await;
        never_cached(&answer, body);
        answer
    }

    async fn ok(&self, who: &str, body: Value) -> Value {
        let answer = self.send(Some(who), &body.to_string()).await;
        assert_eq!(answer.status, 200, "{body} as {who}: {:.400}", answer.text);
        answer.json()
    }

    async fn list(&self, who: &str, branch: Option<&str>) -> Value {
        let payload = branch.map_or_else(|| json!({}), |b| json!({ "branchId": b }));
        self.ok(who, json!({ "action": "list", "payload": payload }))
            .await
    }

    async fn save(&self, who: &str, branch: &str, version: i64, methods: &Value) -> Answer {
        let body = json!({ "action": "save", "payload": {
            "branchId": branch, "version": version, "methods": methods } });
        self.send(Some(who), &body.to_string()).await
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn never_cached(answer: &Answer, what: &str) {
    assert_eq!(
        answer.header("cache-control"),
        Some("no-store"),
        "{what:.200}: every answer carries Cache-Control: no-store"
    );
    assert_eq!(
        answer.header("content-type"),
        Some("application/json"),
        "{what:.200}"
    );
}

/// v2's defaults (`payment-settings.mjs:3`) as the first step of the recording shows them.
fn defaults() -> Value {
    recording().steps[0].answer["methods"].clone()
}

fn with(rows: &Value, id: &str, change: Value) -> Value {
    let mut rows = rows.clone();
    for row in rows.as_array_mut().expect("five rows") {
        if row["id"] == json!(id) {
            for (k, v) in change.as_object().expect("an object of changes") {
                row[k] = v.clone();
            }
        }
    }
    rows
}

// ── the recording, step by step ──────────────────────────────────────────────

#[test]
fn the_recording_has_every_refusal_of_the_design_in_its_place() {
    let recording = recording();
    assert_eq!(recording.origin, ORIGIN);
    assert_eq!(recording.route, ROUTE);
    assert_eq!(recording.branches, vec!["MAIN", AIRPORT]);
    let mut seen = Vec::new();
    for step in &recording.steps {
        let Some(row) = step.row else { continue };
        let (_, status, sentence) = REFUSALS[usize::from(row) - 1];
        if row == 8 {
            // v2 never reaches row 8: see the module comment
            assert_eq!(step.status, 500, "#{} {}", step.n, step.label);
        } else {
            assert_eq!(
                (step.status, step.answer.clone()),
                (status, json!({ "error": sentence })),
                "#{} {}",
                step.n,
                step.label
            );
        }
        if !seen.contains(&row) {
            seen.push(row);
        }
    }
    assert_eq!(
        seen,
        (1..=17).collect::<Vec<u8>>(),
        "each row, in v2's order"
    );
    assert_eq!(recording.steps.len(), 97);
}

#[actix_web::test]
async fn every_recorded_step_is_answered_as_v2_answered_it() {
    let recording = recording();
    let people: Vec<Person> = recording.people.iter().map(person).collect();
    let room = Room::open(&people).await;
    for step in &recording.steps {
        room.desk.clock.set(time(&step.at));
        if step.database_down {
            for table in ["branch_setting_history", "branch_settings"] {
                sqlx::query(sqlx::AssertSqlSafe(format!("DROP TABLE IF EXISTS {table}")))
                    .execute(&room.desk.db.pool)
                    .await
                    .expect("take the payment tables away");
            }
        }
        let mut call = Call::post(ORIGIN, ROUTE, step.text());
        for (name, value) in &step.headers {
            call = match value {
                Some(value) => call.header(name, value),
                None => call.without(name),
            };
        }
        if let Some(who) = &step.sent_as {
            call = call.cookie(&room.cookies[who]);
        }
        let answer = room.app.send(call).await;
        let label = format!("#{} {}", step.n, step.label);
        never_cached(&answer, &label);
        let (status, expected) = if step.row == Some(8) {
            (400, json!({ "error": NO_SUCH_BRANCH }))
        } else if names_another_branch(step) {
            // OR-4b (#767, D-4, §02 "Server ตรวจ … branch scope"): an employee v2 let through
            // naming a branch other than their own (#5, #6) is refused as out of scope
            (403, json!({ "error": OUT_OF_SCOPE }))
        } else {
            (step.status, step.answer.clone())
        };
        assert_eq!(answer.status, status, "{label}: {:.400}", answer.text);
        assert_eq!(answer.json(), expected, "{label}");
    }
    room.finish().await;
}

/// OR-4b (D-4): an employee's step v2 accepted that names a branch other than their own
/// (`AIRPORT_FRONT`'s is `AIRPORT`, everybody else's `MAIN`).
fn names_another_branch(step: &Step) -> bool {
    let Some(who) = step.sent_as.as_deref().filter(|w| *w != "OWNER") else {
        return false;
    };
    let own = if who.starts_with("AIRPORT") {
        AIRPORT
    } else {
        "MAIN"
    };
    let named = serde_json::from_str::<Value>(&step.text())
        .ok()
        .and_then(|b| b["payload"]["branchId"].as_str().map(str::to_owned))
        .unwrap_or_default();
    step.status == 200 && !named.is_empty() && named != own
}

// ── what the owner asked for, said once each ─────────────────────────────────

/// "พนักงานอ่านค่าได้เฉพาะสาขาที่กำหนด": whatever branch an employee names, they read their own.
#[actix_web::test]
async fn an_employee_reads_only_their_own_branch() {
    let room = Room::everyone().await;
    let main_rows = with(&defaults(), "Credit Card", json!({ "enabled": false }));
    let airport_rows = with(&defaults(), "PromptPay", json!({ "enabled": false }));
    assert_eq!(room.save("OWNER", "MAIN", 0, &main_rows).await.status, 200);
    assert_eq!(
        room.save("OWNER", AIRPORT, 0, &airport_rows).await.status,
        200
    );

    // OR-4b (#767, D-4, §02 "Server ตรวจ … branch scope"): naming no branch or their own, an
    // employee reads their own; naming any other (even a malformed one) is refused as out of scope
    for named in [None, Some("MAIN"), Some(AIRPORT)] {
        if named != Some("MAIN") {
            let airport = room.list("AIRPORT_FRONT", named).await;
            assert_eq!(airport["branchId"], json!(AIRPORT), "named {named:?}");
            assert_eq!(airport["methods"], airport_rows, "named {named:?}");
        }
        if named != Some(AIRPORT) {
            let main = room.list("FRONT", named).await;
            assert_eq!(main["branchId"], json!("MAIN"), "named {named:?}");
            assert_eq!(main["methods"], main_rows, "named {named:?}");
        }
    }
    for (who, named) in [
        ("AIRPORT_FRONT", "MAIN"),
        ("FRONT", AIRPORT),
        ("AIRPORT_FRONT", "NOWHERE"),
        ("FRONT", "NOWHERE"),
        ("AIRPORT_FRONT", "bad branch!"),
        ("FRONT", "bad branch!"),
    ] {
        let body = json!({ "action": "list", "payload": { "branchId": named } });
        room.send(Some(who), &body.to_string())
            .await
            .assert_refused(403, OUT_OF_SCOPE, &format!("{who} naming {named}"));
    }
    room.finish().await;
}

/// "ประวัติการตั้งค่า" is for whoever holds `SystemSettings` (OR-7.3 #1022): the owner and SYSTEM (an ADMIN, B1) read
/// the rows with the history and may edit; an employee of any other role reads the rows with no history and cannot
/// edit.
#[actix_web::test]
async fn only_the_owner_sees_the_history() {
    let room = Room::everyone().await;
    let rows = with(&defaults(), "Alipay", json!({ "enabled": false }));
    assert_eq!(room.save("OWNER", "MAIN", 0, &rows).await.status, 200);

    let owner = room.list("OWNER", None).await;
    assert_eq!(owner["canEdit"], json!(true));
    assert_eq!(owner["history"].as_array().map(Vec::len), Some(1));
    assert_eq!(owner["history"][0]["after_methods"], rows);
    assert_eq!(owner["history"][0]["before_methods"], defaults());

    // OR-7.3 (#1022): `canEdit` and the history follow `SystemSettings`, the permission `save` checks. B1 makes
    // SYSTEM an ADMIN, which holds it (and already saves, C5 below), so SYSTEM now edits and sees the history
    for person in one_of_each_role()
        .iter()
        .filter(|p| !p.is_owner() && p.key != "SYSTEM")
    {
        let read = room.list(&person.key, None).await;
        assert_eq!(read["canEdit"], json!(false), "{}", person.key);
        assert_eq!(read["history"], json!([]), "{}", person.key);
        assert_eq!(read["methods"], rows, "{}", person.key);
        assert_eq!(read["version"], json!(1), "{}", person.key);
    }
    let system = room.list("SYSTEM", None).await;
    assert_eq!(system["canEdit"], json!(true), "SYSTEM (ADMIN) edits");
    assert_eq!(system["history"].as_array().map(Vec::len), Some(1));
    assert_eq!(system["methods"], rows, "SYSTEM");
    assert_eq!(system["version"], json!(1), "SYSTEM");
    room.finish().await;
}

#[actix_web::test]
async fn only_the_owner_saves_and_a_refused_save_changes_nothing() {
    let room = Room::everyone().await;
    let rows = with(&defaults(), "Cash", json!({ "sell": false }));
    // OR-4b (#767, design §4.2 C5): SystemSettings is ADMIN's too, and B1 makes SYSTEM an ADMIN;
    // SYSTEM saves below
    for person in one_of_each_role()
        .iter()
        .filter(|p| !p.is_owner() && p.key != "SYSTEM")
    {
        room.save(&person.key, "MAIN", 0, &rows)
            .await
            .assert_refused(403, OWNER_ONLY, &person.key);
    }
    let after = room.list("OWNER", None).await;
    assert_eq!(after["version"], json!(0));
    assert_eq!(after["methods"], defaults());
    assert_eq!(after["history"], json!([]));
    let saved = room.save("SYSTEM", "MAIN", 0, &rows).await;
    assert_eq!(saved.status, 200, "SYSTEM saves (C5): {:.300}", saved.text);
    room.finish().await;
}

/// "BUY รองรับเงินสดและโอนธนาคาร" and "ต้องเหลืออย่างน้อยหนึ่งช่องทางต่อทิศทาง".
#[actix_web::test]
async fn buy_is_cash_or_transfer_and_each_direction_keeps_a_channel() {
    let room = Room::everyone().await;
    for id in ["Credit Card", "Alipay", "PromptPay"] {
        room.save(
            "OWNER",
            "MAIN",
            0,
            &with(&defaults(), id, json!({ "buy": true })),
        )
        .await
        .assert_refused(400, NO_BUY_HERE, id);
    }
    let no_buy = with(
        &with(&defaults(), "Cash", json!({ "buy": false })),
        "Bank Transfer",
        json!({ "enabled": false }),
    );
    room.save("OWNER", "MAIN", 0, &no_buy).await.assert_refused(
        400,
        ONE_PER_DIRECTION,
        "no BUY left",
    );
    let mut no_sell = defaults();
    for row in no_sell.as_array_mut().expect("rows") {
        row["sell"] = json!(false);
        row["enabled"] = json!(row["buy"] == json!(true));
    }
    room.save("OWNER", "MAIN", 0, &no_sell)
        .await
        .assert_refused(400, ONE_PER_DIRECTION, "no SELL left");
    assert_eq!(room.list("OWNER", None).await["version"], json!(0));

    // the last channel of each direction may stay on alone
    let mut transfer_only = defaults();
    for row in transfer_only.as_array_mut().expect("rows") {
        row["enabled"] = json!(row["id"] == json!("Bank Transfer"));
    }
    let saved = room.save("OWNER", "MAIN", 0, &transfer_only).await;
    assert_eq!(saved.status, 200, "{}", saved.text);
    assert_eq!(saved.json()["methods"], transfer_only);
    room.finish().await;
}

#[actix_web::test]
async fn a_stale_save_is_refused_and_the_newer_one_stays() {
    let room = Room::everyone().await;
    let first = with(&defaults(), "Alipay", json!({ "enabled": false }));
    let second = with(&defaults(), "PromptPay", json!({ "enabled": false }));
    assert_eq!(room.save("OWNER", "MAIN", 0, &first).await.status, 200);
    room.save("OWNER", "MAIN", 0, &second).await.assert_refused(
        409,
        STALE,
        "a save from the version before",
    );
    let after = room.list("OWNER", None).await;
    assert_eq!(after["version"], json!(1));
    assert_eq!(after["methods"], first);
    assert_eq!(after["history"].as_array().map(Vec::len), Some(1));
    // and AIRPORT's own version 0 is not stale there
    assert_eq!(room.save("OWNER", AIRPORT, 0, &second).await.status, 200);
    room.finish().await;
}

/// §5.5.2: a body that is not JSON is answered as `/api/rpc` answers it — after the sign-in and
/// origin checks, before anything else.
#[actix_web::test]
async fn a_body_that_is_not_json_is_a_bad_request() {
    let room = Room::everyone().await;
    for body in ["{", "action=list", "", "{\"action\":\"list\",}"] {
        room.send(Some("OWNER"), body)
            .await
            .assert_refused(400, BAD_REQUEST, body);
        room.send(Some("FRONT"), body)
            .await
            .assert_refused(400, BAD_REQUEST, body);
        room.send(None, body)
            .await
            .assert_refused(401, SIGN_IN, body);
    }
    room.finish().await;
}
