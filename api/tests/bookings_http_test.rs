//! Written before the implementation (ticket T15-f, #413). Do not edit, delete or weaken; add
//! tests in a file of your own.
//!
//! `POST /api/bookings` (the owner) and `POST /api/staff/bookings` (the counter roles), design
//! `docs/design/xchange-v3.md` §5.6.3 (the envelope rules, *The booking as printed*, the action
//! table), §5.6.4 (*Routes, roles and branch*), §5.6.5 (`BookingsRoute`, `BookingJson`), §5.6.7
//! and §5.6.8 (the rows naming this file). Every request goes through the server's whole route
//! table (`bootstrap::configure`) with real sessions (`rpc_common`), on a PostgreSQL schema of
//! the test's own.
//!
//! Symptom (#377): the kept admin menu *การจองออนไลน์* prints the parser's error because both
//! paths answer the plain-text `Not found`. So every answer here is read as JSON and its
//! `Content-Type` checked, refusals included.
//!
//! What is pinned, and where it comes from (v2 = `108-Plaza/xchange-v2-source` at `4d0bdbb`):
//!
//! - Owner route, in order: nobody → 401 `กรุณาเข้าสู่ระบบ`, an employee → 403
//!   `เฉพาะเจ้าของระบบเท่านั้น`, both with **no** `Cache-Control` (v2 `src/server.mjs:205-206`,
//!   the router before the handler); then Origin / `X-Xchange-Request` → 400 `ORIGIN_REJECTED`;
//!   body over 8,000 UTF-16 units → `ข้อมูลมากเกินไป`; not JSON or a non-object `payload` →
//!   `คำขอไม่ถูกต้อง`; then the branch (`payload.branchId`, `MAIN` when absent, `null` or `""`);
//!   then the action (v2 `src/online/app/api/bookings/route.ts:4`, `src/online/lib/bookings.ts:55`).
//! - Staff route, in order (§5.6.4): Origin → 400 `คำขอไม่ถูกต้อง`, size, JSON, then
//!   `Policy::allow(…, EntryPath::StaffBookings, action)`, then the employee's own branch.
//! - Every other refusal is `{"error": <sentence>}` with `Cache-Control: no-store`, and so is
//!   every 200 (v2 `route.ts:4`); the shared 500 carries none (§5.6.3 (b)).
//! - The branch must exist before any action answers (§5.6.3 "Common to every action"; v2 runs
//!   `adminSnapshot` first, `bookings.ts:55`).
//! - The answers: `list` (`bookings.ts:71-75`), `get` (`:82-83`), `cancel` (`:84`), `confirm`
//!   (`:85-89`), `member` (`:78-80`), `settings` (`:77`), `registrations` (`:56-58`),
//!   `approveRegistration` (`:60-61`), any other name (`:91`); the booking as printed is
//!   `publicBooking` (`src/online/lib/booking-rules.mjs:8`). `create` is new (§5.6.3, owner
//!   2026-10-08); its statuses are the owner's: CONFIRMED for a walk-in, PENDING for a member.
//! - The route parses the payloads into `CreateBooking`, `ConfirmBooking` and `IssueMember`
//!   (lane controller on #413 and #409): create refusals 1–6, confirm's unreadable `expiresAt`
//!   (RFC 3339 text only, §5.6.7), member refusals 2–4, with the phone sentence
//!   `เบอร์ติดต่อไม่ถูกต้อง`. `minutes: 30.0` is refused (lane controller on #413).
//! - Money and rates print as JSON numbers in their shortest form (`1000`, `250.5`, `36.25`;
//!   never `1000.0`, never a string, as `money_json` / `rate_json` print them), and are compared
//!   here by their JSON text. Times print as `time_json` (UTC, milliseconds).
//! - The kept screen (`src/online/public/bookings.js:5`, the five requests it builds) and the
//!   desk's hand-off (`src/online/lib/index-html.ts:1`, `{action:'get',payload:{branchId,id}}`)
//!   are sent byte for byte as the browser sends them.
//!
//! The route reads `now` from `web::Data<dyn RpcClock>` when one is registered, as
//! `payment_methods.rs` does (the test's clock). Bookings made by a test are made at distinct
//! instants (the clock moves one second before each), so `created_at desc` orders them; nothing
//! is ordered by a time two rows share.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.
//!
//! Amended for #758 (RE-4d) by the ticket author (claude/e5ab6c), `docs/design/blueprint/rates.md`
//! D-3, §4.8 and §4.9 R1/R4: the counter rate a booking is created and confirmed at is the line
//! of the branch's current publication, no longer the newest `rates` row. `Room::open` publishes
//! the same counter rates it writes to `rates` (all unit 1, so every rate here reads as before).
//!
//! Re-expected for RE-7 #604 by the ticket author: `Room::open` no longer writes those rates to
//! `rates` (rates.md §8 migration 6 drops it); it publishes them, as before, and nothing else.

mod common;
mod compat_common;
mod rates_re4_common;
mod rpc_common;
mod snapshot_common;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use compat_common::Person;
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use snapshot_common::time;
use xchange::presentation::http::rpc::RpcClock;

const ORIGIN: &str = "http://desk.test";
const OWNER_ROUTE: &str = "/api/v2/bookings";
const STAFF_ROUTE: &str = "/api/staff/bookings";
const MAIN: &str = "MAIN";
const AIRPORT: &str = "AIRPORT";
const NOW: &str = "2026-10-06T05:00:00.000Z";

const SIGN_IN: &str = "กรุณาเข้าสู่ระบบ";
const OWNER_ONLY: &str = "เฉพาะเจ้าของระบบเท่านั้น";
const ORIGIN_REJECTED: &str = "ORIGIN_REJECTED";
const TOO_MUCH: &str = "ข้อมูลมากเกินไป";
const BAD_REQUEST: &str = "คำขอไม่ถูกต้อง";
const BAD_BRANCH: &str = "สาขาไม่ถูกต้อง";
const NO_SUCH_BRANCH: &str = "ไม่พบสาขาที่เลือก กรุณารีเฟรช";
const EMPLOYEE_AUTH_REQUIRED: &str = "EMPLOYEE_AUTH_REQUIRED";
const ROLE_NOT_ALLOWED: &str = "บทบาทนี้ไม่มีสิทธิ์ทำรายการ";
const SERVER_ERROR: &str = "เซิร์ฟเวอร์ขัดข้อง";

const FILL_IN: &str = "กรอกข้อมูลให้ครบและไม่เกินกำหนด";
const BAD_PHONE: &str = "เบอร์ติดต่อไม่ถูกต้อง";
const INCOMPLETE: &str = "ข้อมูลรายการไม่ครบ";
const BAD_NUMBER: &str = "จำนวนเงินหรือเรทไม่ถูกต้อง";
const TOO_MANY_DECIMALS: &str = "ทศนิยมเกินกำหนด";
const CHANGED: &str = "รายการเดิมมีข้อมูลเปลี่ยน ห้ามใช้รหัสส่งซ้ำกับข้อมูลใหม่";
const NOT_FOUND: &str = "ไม่พบการจอง";
const EXPIRY: &str = "วันหมดอายุต้องอยู่ในอนาคต";
const ONLY_PENDING_MEMBER: &str = "ยืนยันได้เฉพาะคำขอ Member ที่รอยืนยัน";
const RATE_MOVED: &str = "เรทเปลี่ยน กรุณาโหลดใหม่";
const CLOSED: &str = "ปิดรับจองใหม่";
const NOT_IDENTIFIED: &str = "ต้องเลือกลูกค้าและยืนยันตรวจตัวตนหน้าร้านก่อน";
const BAD_MEMBER_CODE: &str = "รหัสสมาชิกไม่ถูกต้อง";
const BAD_RULES: &str = "กำหนดเวลา 5\u{2013}1440 นาที";
const NO_REGISTRATION: &str = "ไม่พบการลงทะเบียนของสาขานี้";

/// The counter roles (owner 2026-10-08). OR-4b (#767, design §4.2 C9): `BookingsDesk` is TELLER's,
/// and B1 makes FRONT a TELLER, so the counter is FRONT.
const COUNTER: [&str; 1] = ["FRONT"];
/// BACKEND and MANAGER: counter roles in v3, BRANCH_MANAGER under B1, refused the desk bookings by
/// C9 (§03 B02: the user of the desk bookings is TELLER).
const EX_COUNTER: [&str; 2] = ["BACKEND", "MANAGER"];
/// Every other staff role.
const NOT_COUNTER: [&str; 6] = [
    "RATES",
    "MONITOR",
    "FINANCE",
    "SYSTEM",
    "DOCUMENTS",
    "DOCUMENT_REVIEW",
];
/// The actions only the owner runs.
const OWNER_ACTIONS: [&str; 4] = ["member", "settings", "registrations", "approveRegistration"];
const ALL_ACTIONS: [&str; 9] = [
    "list",
    "get",
    "create",
    "confirm",
    "cancel",
    "member",
    "settings",
    "registrations",
    "approveRegistration",
];

/// Live customers: (id, branch, name).
const SOMCHAI: (&str, &str, &str) = ("C-MAIN-1", MAIN, "สมชาย ใจดี");
const SUDA: (&str, &str, &str) = ("C-MAIN-2", MAIN, "สุดา รักไทย");
const AIR_CUSTOMER: (&str, &str, &str) = ("C-AIR-1", AIRPORT, "Airport Customer");
/// A DEMO customer of `MAIN`.
const DEMO: &str = "C-MAIN-DEMO";

const SECRET_A: &str = "6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b";
const SECRET_B: &str = "0a1b2c3d-4e5f-4061-8273-a4b5c6d7e8f9";

// ── the shop ─────────────────────────────────────────────────────────────────

fn at(text: &str) -> DateTime<Utc> {
    time(text)
}

fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    /// `MAIN` and `AIRPORT`, each with an OPEN shift and counter rates (in `rates` and published,
    /// RE-4d); the owner, one employee
    /// of every staff role in `MAIN`, and a FRONT of `AIRPORT`; three LIVE customers and a DEMO
    /// one. No `bookingRules` stored. The clock at [`NOW`].
    async fn open() -> Room {
        let desk = Desk::open().await;
        desk.db.seed_branch(AIRPORT, "Airport").await;
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
        let app = rpc_app!(desk);
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let room = Room { desk, app, cookies };

        room.seed_shift("SH-MAIN", MAIN).await;
        room.seed_shift("SH-AIR", AIRPORT).await;
        // RE-4d: the counter is the current publication's line (rates D-3, §4.8). RE-7 #604: the
        // same rates are no longer also written to `rates` (dropped, rates.md §8 migration 6).
        let pool = room.desk.db.pool.clone();
        rates_re4_common::publish(
            &pool,
            MAIN,
            &[("USD", "36.25", "36.75"), ("EUR", "39.5", "40.1")],
        )
        .await;
        rates_re4_common::publish(&pool, AIRPORT, &[("USD", "36.1", "36.9")]).await;
        for (id, branch, name) in [SOMCHAI, SUDA, AIR_CUSTOMER] {
            room.seed_customer(id, branch, name, "LIVE").await;
        }
        room.seed_customer(DEMO, MAIN, "Demo Customer", "DEMO")
            .await;
        room.desk.clock.set(at(NOW));
        room
    }

    async fn seed_customer(&self, id: &str, branch: &str, name: &str, mode: &str) {
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT $1, b.organization_id, $1, $3, $4, $5 FROM branches b WHERE b.id = $2",
        )
        .bind(id)
        .bind(branch)
        .bind(name)
        .bind(mode)
        .bind(at("2026-10-06T04:00:00.000Z"))
        .execute(&self.desk.db.pool)
        .await
        .expect("insert into customers");
    }

    async fn seed_shift(&self, id: &str, branch: &str) {
        let query = if branch == MAIN {
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, closed_at, closed_by, \
                 open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', $3, 'owner', NULL, NULL, $4, (SELECT u.id FROM users u WHERE u.legacy_employee_id IS NULL AND EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL)))"
        } else {
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, closed_at, closed_by, \
                 open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', $3, 'owner', NULL, NULL, $4, (SELECT id FROM users WHERE legacy_employee_id = '00000000-0000-4000-8000-0000000000a1'))"
        };
        sqlx::query(query)
            .bind(id)
            .bind(branch)
            .bind(at("2026-10-06T04:00:00.000Z"))
            .bind(format!("open-{id}"))
            .execute(&self.desk.db.pool)
            .await
            .expect("insert into shifts");
    }

    fn now(&self) -> DateTime<Utc> {
        self.desk.clock.now()
    }

    /// `text` POSTed to `route` as the kept screens send it, with `who`'s session.
    async fn post(&self, route: &str, who: Option<&str>, text: impl Into<String>) -> Answer {
        let mut call = Call::post(ORIGIN, route, text.into());
        if let Some(who) = who {
            call = call.cookie(&self.cookies[who]);
        }
        self.app.send(call).await
    }

    async fn act(&self, route: &str, who: Option<&str>, action: &str, payload: Value) -> Answer {
        self.post(route, who, body(action, &payload)).await
    }

    /// A 200 answer of `action`, as JSON.
    async fn ok(&self, route: &str, who: &str, action: &str, payload: Value) -> Value {
        let label = format!("{who} {action} {payload} on {route}");
        let answer = self.act(route, Some(who), action, payload).await;
        answered(&answer, &label)
    }

    /// `action` refused with `sentence` (400 unless the sentence says otherwise), `no-store`.
    async fn refused(
        &self,
        route: &str,
        who: Option<&str>,
        action: &str,
        payload: Value,
        status: u16,
        sentence: &str,
    ) {
        let label = format!("{who:?} {action} {payload} on {route}");
        let answer = self.act(route, who, action, payload).await;
        assert_refusal(&answer, status, sentence, true, &label);
    }

    /// `create`, after the clock moves one second; the `booking` of `{saved: true, booking}`.
    async fn create(&self, route: &str, who: &str, payload: Value) -> Value {
        self.desk.clock.advance(Duration::seconds(1));
        let answer = self.ok(route, who, "create", payload.clone()).await;
        assert_eq!(answer["saved"], json!(true), "create {payload}: {answer}");
        assert_eq!(
            keys(&answer),
            vec!["booking", "saved"],
            "create answers {{saved, booking}}: {answer}"
        );
        answer["booking"].clone()
    }

    /// The owner issues a member code for `customer` of `MAIN`.
    async fn issue_member(&self, customer: &str, secret: &str) -> Value {
        self.desk.clock.advance(Duration::seconds(1));
        self.ok(
            OWNER_ROUTE,
            "OWNER",
            "member",
            json!({ "branchId": MAIN, "customerId": customer, "identityConfirmed": true,
                    "phone": "081-234-5678", "memberSecret": secret }),
        )
        .await
    }

    async fn list(&self, route: &str, who: &str, branch: Option<&str>) -> Value {
        let payload = branch.map_or_else(|| json!({}), |b| json!({ "branchId": b }));
        self.ok(route, who, "list", payload).await
    }

    async fn get(&self, route: &str, who: &str, branch: &str, id: &str) -> Value {
        self.ok(route, who, "get", json!({ "branchId": branch, "id": id }))
            .await
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

fn body(action: &str, payload: &Value) -> String {
    json!({ "action": action, "payload": payload }).to_string()
}

/// A walk-in's `create` payload in `MAIN`: BUY 1,000 USD at the counter's 36.25.
fn walk_in(request_id: &str) -> Value {
    json!({
        "branchId": MAIN,
        "requestId": request_id,
        "name": "Somchai Jaidee",
        "phone": "081-234-5678",
        "type": "BUY",
        "currency": "USD",
        "amount": 1000,
        "expectedRate": 36.25
    })
}

fn with(payload: &Value, key: &str, value: Value) -> Value {
    let mut payload = payload.clone();
    payload[key] = value;
    payload
}

fn without(payload: &Value, key: &str) -> Value {
    let mut payload = payload.clone();
    payload.as_object_mut().expect("an object").remove(key);
    payload
}

fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .unwrap_or_else(|| panic!("an object: {value}"))
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

/// The JSON text of a number: `1000`, `250.5`, `36.25`.
fn number_text(value: &Value, label: &str) -> String {
    assert!(value.is_number(), "{label}: a JSON number, got {value}");
    value.to_string()
}

fn is_json(answer: &Answer) -> bool {
    answer
        .header("content-type")
        .is_some_and(|t| t.starts_with("application/json"))
}

/// A 200 that is JSON and never cached.
fn answered(answer: &Answer, label: &str) -> Value {
    assert_eq!(answer.status, 200, "{label}: {:.400}", answer.text);
    assert!(
        is_json(answer),
        "{label}: Content-Type {:?}",
        answer.header("content-type")
    );
    assert_eq!(answer.header("cache-control"), Some("no-store"), "{label}");
    answer.json()
}

/// `status` and exactly `{"error": sentence}`, as JSON; `Cache-Control: no-store` present when
/// `no_store`, absent otherwise.
fn assert_refusal(answer: &Answer, status: u16, sentence: &str, no_store: bool, label: &str) {
    assert_eq!(answer.status, status, "{label}: {:.400}", answer.text);
    assert!(
        is_json(answer),
        "{label}: Content-Type {:?}, body {:.200}",
        answer.header("content-type"),
        answer.text
    );
    assert_eq!(answer.json(), json!({ "error": sentence }), "{label}");
    let expected = no_store.then_some("no-store");
    assert_eq!(answer.header("cache-control"), expected, "{label}");
}

/// A body of exactly `units` UTF-16 units: an owner `list` of `MAIN` padded with Thai letters
/// (one unit, three bytes each), so a byte count is not a unit count.
fn padded(units: usize) -> String {
    let head = r#"{"action":"list","payload":{"branchId":"MAIN","pad":""#;
    let tail = r#""}}"#;
    let fixed = head.encode_utf16().count() + tail.encode_utf16().count();
    let text = format!("{head}{}{tail}", "ก".repeat(units - fixed));
    assert_eq!(text.encode_utf16().count(), units);
    text
}

fn booking_keys() -> Vec<String> {
    let mut keys: Vec<String> = [
        "id",
        "branchId",
        "name",
        "phone",
        "member",
        "type",
        "currency",
        "amount",
        "rate",
        "status",
        "createdAt",
        "expiresAt",
        "transactionRef",
    ]
    .iter()
    .map(|k| k.to_string())
    .collect();
    keys.sort();
    keys
}

fn assert_booking_id(id: &Value, label: &str) {
    let id = id.as_str().unwrap_or_else(|| panic!("{label}: id is text"));
    assert!(
        id.starts_with("BK-") && id.len() > 3,
        "{label}: id BK-<uuid>, got {id}"
    );
}

// ── the owner route: the envelope ────────────────────────────────────────────

/// §5.6.4, the owner column, steps 1–5 in order; each step is shown to come before the next by
/// a request that would also fail the later ones.
#[actix_web::test]
async fn owner_route_refusal_order() {
    let room = Room::open().await;
    let no_marker = |c: Call| c.without("x-xchange-request");

    // 1. nobody → 401, before the Origin check, the size and the JSON; no Cache-Control
    for call in [
        Call::post(ORIGIN, OWNER_ROUTE, body("list", &json!({}))),
        no_marker(Call::post(ORIGIN, OWNER_ROUTE, "not json")),
        Call::post(ORIGIN, OWNER_ROUTE, "x".repeat(9_000)).header("origin", "http://evil.test"),
    ] {
        let answer = room.app.send(call).await;
        assert_refusal(&answer, 401, SIGN_IN, false, "nobody");
    }

    // 1. an employee of any role → 403, before the Origin check; no Cache-Control
    for role in COUNTER
        .iter()
        .chain(EX_COUNTER.iter())
        .chain(NOT_COUNTER.iter())
    {
        for call in [
            Call::post(ORIGIN, OWNER_ROUTE, body("list", &json!({}))),
            no_marker(Call::post(ORIGIN, OWNER_ROUTE, "not json")),
        ] {
            let answer = room.app.send(call.cookie(&room.cookies[*role])).await;
            assert_refusal(&answer, 403, OWNER_ONLY, false, role);
        }
    }

    // 2. the owner, Origin / X-Xchange-Request → 400 ORIGIN_REJECTED, before the size
    let owner = &room.cookies["OWNER"];
    let big = "x".repeat(9_000);
    for (label, call) in [
        (
            "no marker",
            no_marker(Call::post(ORIGIN, OWNER_ROUTE, big.clone())),
        ),
        (
            "marker 0",
            Call::post(ORIGIN, OWNER_ROUTE, big.clone()).header("x-xchange-request", "0"),
        ),
        (
            "no Origin",
            Call::post(ORIGIN, OWNER_ROUTE, big.clone()).without("origin"),
        ),
        (
            "another Origin",
            Call::post(ORIGIN, OWNER_ROUTE, big.clone()).header("origin", "http://evil.test"),
        ),
    ] {
        let answer = room.app.send(call.cookie(owner)).await;
        assert_refusal(&answer, 400, ORIGIN_REJECTED, true, label);
    }

    // 3. over 8,000 UTF-16 units → too much, before the JSON; 8,000 exactly passes
    let answer = room.post(OWNER_ROUTE, Some("OWNER"), padded(8_001)).await;
    assert_refusal(&answer, 400, TOO_MUCH, true, "8,001 units");
    let answer = room
        .post(OWNER_ROUTE, Some("OWNER"), "{".repeat(8_001))
        .await;
    assert_refusal(&answer, 400, TOO_MUCH, true, "8,001 units, not JSON");
    let answer = room.post(OWNER_ROUTE, Some("OWNER"), padded(8_000)).await;
    let listed = answered(&answer, "8,000 units");
    assert_eq!(listed["bookings"], json!([]), "8,000 units is a list");

    // 4. not JSON, or a payload neither absent, null nor an object
    for text in [
        "not json".to_string(),
        "{".to_string(),
        String::new(),
        "[]".to_string(),
        r#"{"action":"list","payload":"x"}"#.to_string(),
        r#"{"action":"list","payload":[]}"#.to_string(),
        r#"{"action":"list","payload":5}"#.to_string(),
        r#"{"action":"list","payload":true}"#.to_string(),
        r#"{"action":"list","payload":"x","branchId":"bad branch!"}"#.to_string(),
    ] {
        let answer = room.post(OWNER_ROUTE, Some("OWNER"), text.clone()).await;
        assert_refusal(&answer, 400, BAD_REQUEST, true, &text);
    }
    for text in [
        r#"{"action":"list"}"#,
        r#"{"action":"list","payload":null}"#,
        r#"{"action":"list","payload":{}}"#,
    ] {
        let answer = room.post(OWNER_ROUTE, Some("OWNER"), text).await;
        let listed = answered(&answer, text);
        assert_eq!(listed["bookings"], json!([]), "{text}: MAIN's list");
    }

    // 5. the branch: a non-empty string of 1–70 of A-Za-z0-9-, else MAIN when absent, null or ""
    for branch in [
        json!("bad branch!"),
        json!("MAIN!"),
        json!("x".repeat(71)),
        json!(5),
    ] {
        room.refused(
            OWNER_ROUTE,
            Some("OWNER"),
            "list",
            json!({ "branchId": branch }),
            400,
            BAD_BRANCH,
        )
        .await;
        // before the action is judged
        room.refused(
            OWNER_ROUTE,
            Some("OWNER"),
            "nope",
            json!({ "branchId": branch }),
            400,
            BAD_BRANCH,
        )
        .await;
    }
    for branch in [Value::Null, json!("")] {
        let listed = room
            .ok(OWNER_ROUTE, "OWNER", "list", json!({ "branchId": branch }))
            .await;
        assert_eq!(listed["rules"], json!({ "enabled": 1, "minutes": 30 }));
    }
    // 70 characters is a well-formed branch that does not exist
    room.refused(
        OWNER_ROUTE,
        Some("OWNER"),
        "list",
        json!({ "branchId": "A".repeat(70) }),
        400,
        NO_SUCH_BRANCH,
    )
    .await;

    // 6. the action: unknown, missing or not text → คำขอไม่ถูกต้อง at once, even naming an id
    for text in [
        r#"{"payload":{"branchId":"MAIN"}}"#,
        r#"{"action":5,"payload":{"branchId":"MAIN"}}"#,
        r#"{"action":"List","payload":{"branchId":"MAIN"}}"#,
        r#"{"action":"nope","payload":{"branchId":"MAIN","id":"BK-1"}}"#,
        r#"{"action":"publicCreate","payload":{"branchId":"MAIN"}}"#,
    ] {
        let answer = room.post(OWNER_ROUTE, Some("OWNER"), text).await;
        assert_refusal(&answer, 400, BAD_REQUEST, true, text);
    }
    room.finish().await;
}

/// §5.6.3 "Common to every action": a well-formed branch that does not exist answers
/// `ไม่พบสาขาที่เลือก กรุณารีเฟรช` before any of the action's own refusals, for all nine actions.
#[actix_web::test]
async fn the_branch_must_exist_before_any_action_answers() {
    let room = Room::open().await;
    for action in ALL_ACTIONS {
        // payloads every action would otherwise refuse (or, for list/registrations, answer)
        room.refused(
            OWNER_ROUTE,
            Some("OWNER"),
            action,
            json!({ "branchId": "NOWHERE", "id": 5, "requestId": 5, "minutes": 1,
                    "customerId": 5, "expiresAt": "never" }),
            400,
            NO_SUCH_BRANCH,
        )
        .await;
    }
    room.finish().await;
}

// ── the staff route: the envelope and the policy ─────────────────────────────

/// §5.6.4, the staff column, steps 1–5 in order.
#[actix_web::test]
async fn staff_route_refusal_order() {
    let room = Room::open().await;

    // 1. Origin / X-Xchange-Request → 400 คำขอไม่ถูกต้อง, before the session is read
    for who in [None, Some("FRONT")] {
        for (label, call) in [
            (
                "no marker",
                Call::post(ORIGIN, STAFF_ROUTE, "x".repeat(9_000)).without("x-xchange-request"),
            ),
            (
                "no Origin",
                Call::post(ORIGIN, STAFF_ROUTE, "x".repeat(9_000)).without("origin"),
            ),
            (
                "another Origin",
                Call::post(ORIGIN, STAFF_ROUTE, "x".repeat(9_000))
                    .header("origin", "http://evil.test"),
            ),
        ] {
            let call = match who {
                Some(who) => call.cookie(&room.cookies[who]),
                None => call,
            };
            let answer = room.app.send(call).await;
            assert_refusal(&answer, 400, BAD_REQUEST, true, &format!("{who:?} {label}"));
        }
    }

    // 2. over 8,000 UTF-16 units, before the session; 8,000 exactly reaches the policy
    let answer = room.post(STAFF_ROUTE, None, padded(8_001)).await;
    assert_refusal(&answer, 400, TOO_MUCH, true, "nobody, 8,001 units");
    let answer = room.post(STAFF_ROUTE, None, padded(8_000)).await;
    assert_refusal(
        &answer,
        401,
        EMPLOYEE_AUTH_REQUIRED,
        true,
        "nobody, 8,000 units",
    );
    let answer = room.post(STAFF_ROUTE, Some("FRONT"), padded(8_000)).await;
    answered(&answer, "FRONT, 8,000 units");

    // 3. not JSON or a non-object payload, before the session
    for text in [
        "not json",
        "{",
        r#"{"action":"list","payload":"x"}"#,
        r#"{"action":"list","payload":[]}"#,
    ] {
        let answer = room.post(STAFF_ROUTE, None, text).await;
        assert_refusal(&answer, 400, BAD_REQUEST, true, text);
    }

    // 4. the policy (T15-d), with Cache-Control: no-store on each refusal
    room.refused(
        STAFF_ROUTE,
        None,
        "list",
        json!({}),
        401,
        EMPLOYEE_AUTH_REQUIRED,
    )
    .await;
    // C9: the ex-counter roles are refused here with the other roles
    for role in NOT_COUNTER.iter().chain(EX_COUNTER.iter()) {
        for action in ["list", "get", "create", "member"] {
            room.refused(
                STAFF_ROUTE,
                Some(role),
                action,
                json!({}),
                400,
                ROLE_NOT_ALLOWED,
            )
            .await;
        }
    }
    // C9: an ex-counter role is refused by role before the name is read
    for role in EX_COUNTER {
        for action in OWNER_ACTIONS.into_iter().chain(["nope", "List", ""]) {
            room.refused(
                STAFF_ROUTE,
                Some(role),
                action,
                json!({}),
                400,
                ROLE_NOT_ALLOWED,
            )
            .await;
        }
    }
    for role in COUNTER {
        for action in OWNER_ACTIONS {
            room.refused(STAFF_ROUTE, Some(role), action, json!({}), 403, OWNER_ONLY)
                .await;
        }
        for action in ["nope", "List", ""] {
            room.refused(STAFF_ROUTE, Some(role), action, json!({}), 400, BAD_REQUEST)
                .await;
        }
    }

    // 5. the branch is the employee's own, whatever is named — never สาขาไม่ถูกต้อง here
    for branch in [json!("bad branch!"), json!(5), json!("NOWHERE")] {
        let listed = room
            .ok(STAFF_ROUTE, "FRONT", "list", json!({ "branchId": branch }))
            .await;
        assert_eq!(listed["rules"], json!({ "enabled": 1, "minutes": 30 }));
    }
    room.finish().await;
}

/// §5.6.8: `/api/staff/bookings` with only an owner's session → 401 `EMPLOYEE_AUTH_REQUIRED`.
/// OR-4b (#767, design §4.2 C1): a staff path admits whoever holds its role; the owner's user
/// (EXECUTIVE + ADMIN) holds no `BookingsDesk`, so it is refused by role (400) instead.
#[actix_web::test]
async fn staff_route_refuses_an_owner_session() {
    let room = Room::open().await;
    for action in ALL_ACTIONS {
        room.refused(
            STAFF_ROUTE,
            Some("OWNER"),
            action,
            json!({ "branchId": MAIN }),
            400,
            ROLE_NOT_ALLOWED,
        )
        .await;
    }
    room.finish().await;
}

// ── every refusal is JSON ────────────────────────────────────────────────────

/// A case: (route, who, action, payload, status, sentence, no-store).
type Refusal = (
    &'static str,
    Option<&'static str>,
    &'static str,
    Value,
    u16,
    &'static str,
    bool,
);

/// §5.6.8 `every_refusal_is_json`: each refusal of either route is `{"error"}` as JSON — never
/// the plain-text 404 of #377 — with `Cache-Control: no-store` on all but the owner route's
/// step-1 401/403 (and the 500, below).
#[actix_web::test]
async fn every_refusal_is_json() {
    let room = Room::open().await;
    let walk = room
        .create(OWNER_ROUTE, "OWNER", walk_in("req-walk-1"))
        .await;
    let id = walk["id"].as_str().expect("an id").to_string();

    let cases: Vec<Refusal> = vec![
        (OWNER_ROUTE, None, "list", json!({}), 401, SIGN_IN, false),
        (
            OWNER_ROUTE,
            Some("FRONT"),
            "list",
            json!({}),
            403,
            OWNER_ONLY,
            false,
        ),
        (
            OWNER_ROUTE,
            Some("OWNER"),
            "list",
            json!({ "branchId": "bad!" }),
            400,
            BAD_BRANCH,
            true,
        ),
        (
            OWNER_ROUTE,
            Some("OWNER"),
            "list",
            json!({ "branchId": "NOWHERE" }),
            400,
            NO_SUCH_BRANCH,
            true,
        ),
        (
            OWNER_ROUTE,
            Some("OWNER"),
            "nope",
            json!({}),
            400,
            BAD_REQUEST,
            true,
        ),
        (
            OWNER_ROUTE,
            Some("OWNER"),
            "get",
            json!({ "id": "BK-none" }),
            400,
            NOT_FOUND,
            true,
        ),
        (
            OWNER_ROUTE,
            Some("OWNER"),
            "get",
            json!({}),
            400,
            FILL_IN,
            true,
        ),
        (
            OWNER_ROUTE,
            Some("OWNER"),
            "create",
            without(&walk_in("req-x"), "phone"),
            400,
            BAD_PHONE,
            true,
        ),
        (
            OWNER_ROUTE,
            Some("OWNER"),
            "confirm",
            json!({ "id": id, "expiresAt": "2026-10-07T05:00:00.000Z", "expectedRate": 36.25 }),
            400,
            ONLY_PENDING_MEMBER,
            true,
        ),
        (
            OWNER_ROUTE,
            Some("OWNER"),
            "cancel",
            json!({ "id": "BK-none" }),
            400,
            NOT_FOUND,
            true,
        ),
        (
            OWNER_ROUTE,
            Some("OWNER"),
            "member",
            json!({ "customerId": SOMCHAI.0 }),
            400,
            NOT_IDENTIFIED,
            true,
        ),
        (
            OWNER_ROUTE,
            Some("OWNER"),
            "settings",
            json!({ "minutes": 4, "enabled": true }),
            400,
            BAD_RULES,
            true,
        ),
        (
            OWNER_ROUTE,
            Some("OWNER"),
            "approveRegistration",
            json!({ "id": "REG-1" }),
            400,
            NO_REGISTRATION,
            true,
        ),
        (
            STAFF_ROUTE,
            None,
            "list",
            json!({}),
            401,
            EMPLOYEE_AUTH_REQUIRED,
            true,
        ),
        // C1: the owner's user is refused by role on a staff path
        (
            STAFF_ROUTE,
            Some("OWNER"),
            "list",
            json!({}),
            400,
            ROLE_NOT_ALLOWED,
            true,
        ),
        (
            STAFF_ROUTE,
            Some("RATES"),
            "list",
            json!({}),
            400,
            ROLE_NOT_ALLOWED,
            true,
        ),
        (
            STAFF_ROUTE,
            Some("FRONT"),
            "settings",
            json!({}),
            403,
            OWNER_ONLY,
            true,
        ),
        (
            STAFF_ROUTE,
            Some("FRONT"),
            "nope",
            json!({}),
            400,
            BAD_REQUEST,
            true,
        ),
        (
            STAFF_ROUTE,
            Some("FRONT"),
            "get",
            json!({ "id": "BK-none" }),
            400,
            NOT_FOUND,
            true,
        ),
        (
            STAFF_ROUTE,
            Some("FRONT"),
            "create",
            with(&walk_in("req-y"), "currency", json!("THB")),
            400,
            INCOMPLETE,
            true,
        ),
        // C9: the counter is FRONT
        (
            STAFF_ROUTE,
            Some("FRONT"),
            "confirm",
            json!({ "id": id, "expiresAt": "tomorrow" }),
            400,
            EXPIRY,
            true,
        ),
        (
            STAFF_ROUTE,
            Some("FRONT"),
            "cancel",
            json!({ "id": 5 }),
            400,
            FILL_IN,
            true,
        ),
    ];
    for (route, who, action, payload, status, sentence, no_store) in cases {
        let label = format!("{who:?} {action} {payload} on {route}");
        let answer = room.act(route, who, action, payload).await;
        assert_refusal(&answer, status, sentence, no_store, &label);
    }

    // the envelope's own refusals, as raw bodies
    for (route, who, text, sentence) in [
        (OWNER_ROUTE, Some("OWNER"), "nope", BAD_REQUEST),
        (OWNER_ROUTE, Some("OWNER"), r#"{"payload":[]}"#, BAD_REQUEST),
        (STAFF_ROUTE, Some("FRONT"), "nope", BAD_REQUEST),
        (STAFF_ROUTE, None, r#"{"payload":"x"}"#, BAD_REQUEST),
    ] {
        let answer = room.post(route, who, text).await;
        assert_refusal(&answer, 400, sentence, true, &format!("{text} on {route}"));
    }
    let answer = room.post(OWNER_ROUTE, Some("OWNER"), padded(8_001)).await;
    assert_refusal(&answer, 400, TOO_MUCH, true, "owner, too much");
    let answer = room.post(STAFF_ROUTE, Some("FRONT"), padded(8_001)).await;
    assert_refusal(&answer, 400, TOO_MUCH, true, "staff, too much");
    room.finish().await;
}

/// §5.6.3 (b): a database failure is the shared 500 `{"error": "เซิร์ฟเวอร์ขัดข้อง"}`, as JSON,
/// with no `Cache-Control`, on both routes.
#[actix_web::test]
async fn a_database_failure_is_the_shared_500() {
    let room = Room::open().await;
    sqlx::query("DROP TABLE v2_bookings CASCADE")
        .execute(&room.desk.db.pool)
        .await
        .expect("take the bookings table away");
    for (route, who) in [(OWNER_ROUTE, "OWNER"), (STAFF_ROUTE, "FRONT")] {
        let answer = room
            .act(route, Some(who), "list", json!({ "branchId": MAIN }))
            .await;
        assert_refusal(&answer, 500, SERVER_ERROR, false, route);
    }
    room.finish().await;
}

// ── list, settings and the rules ─────────────────────────────────────────────

/// §5.6.8 `list_default_rules_shape`: with no `bookingRules` stored, `list` prints
/// `{enabled: 1, minutes: 30}` without `branch_id`; a stored document prints
/// `{branch_id, enabled: 1|0, minutes}`; per branch.
#[actix_web::test]
async fn list_default_rules_shape() {
    let room = Room::open().await;
    let empty = json!({
        "bookings": [],
        "members": [],
        "rules": { "enabled": 1, "minutes": 30 },
        "publicUrl": ""
    });
    assert_eq!(room.list(OWNER_ROUTE, "OWNER", None).await, empty);
    assert_eq!(room.list(OWNER_ROUTE, "OWNER", Some(AIRPORT)).await, empty);
    // C9: an ex-counter role is refused the list
    for role in EX_COUNTER {
        room.refused(
            STAFF_ROUTE,
            Some(role),
            "list",
            json!({}),
            400,
            ROLE_NOT_ALLOWED,
        )
        .await;
    }
    for role in COUNTER {
        assert_eq!(room.list(STAFF_ROUTE, role, None).await, empty, "{role}");
    }

    let saved = room
        .ok(
            OWNER_ROUTE,
            "OWNER",
            "settings",
            json!({ "branchId": MAIN, "minutes": 45, "enabled": false }),
        )
        .await;
    assert_eq!(saved, json!({ "saved": true }));
    let rules = json!({ "branch_id": MAIN, "enabled": 0, "minutes": 45 });
    assert_eq!(room.list(OWNER_ROUTE, "OWNER", None).await["rules"], rules);
    assert_eq!(room.list(STAFF_ROUTE, "FRONT", None).await["rules"], rules);
    // AIRPORT still has none
    assert_eq!(
        room.list(OWNER_ROUTE, "OWNER", Some(AIRPORT)).await["rules"],
        json!({ "enabled": 1, "minutes": 30 })
    );
    assert_eq!(
        room.list(STAFF_ROUTE, "AIRPORT_FRONT", Some(MAIN)).await["rules"],
        json!({ "enabled": 1, "minutes": 30 })
    );

    // last write wins; enabled prints 1
    room.ok(
        OWNER_ROUTE,
        "OWNER",
        "settings",
        json!({ "branchId": MAIN, "minutes": 1440, "enabled": true }),
    )
    .await;
    assert_eq!(
        room.list(OWNER_ROUTE, "OWNER", None).await["rules"],
        json!({ "branch_id": MAIN, "enabled": 1, "minutes": 1440 })
    );
    room.finish().await;
}

/// `settings` (§5.6.3, v2 `bookings.ts:77`): `minutes` a JSON integer 5–1440 and `enabled` a
/// JSON boolean, else `กำหนดเวลา 5–1440 นาที`; `30.0` is refused (lane controller on #413); a
/// refused save changes nothing. And the rules are what `create` uses.
#[actix_web::test]
async fn settings_refusals_and_their_effect_on_create() {
    let room = Room::open().await;
    for (minutes, enabled) in [
        (json!(4), json!(true)),
        (json!(1441), json!(true)),
        (json!(30.0), json!(true)),
        (json!(30.5), json!(true)),
        (json!("30"), json!(true)),
        (json!(-30), json!(true)),
        (Value::Null, json!(true)),
        (json!(30), json!(1)),
        (json!(30), json!("true")),
        (json!(30), Value::Null),
    ] {
        room.refused(
            OWNER_ROUTE,
            Some("OWNER"),
            "settings",
            json!({ "branchId": MAIN, "minutes": minutes, "enabled": enabled }),
            400,
            BAD_RULES,
        )
        .await;
    }
    room.refused(
        OWNER_ROUTE,
        Some("OWNER"),
        "settings",
        json!({ "branchId": MAIN }),
        400,
        BAD_RULES,
    )
    .await;
    assert_eq!(
        room.list(OWNER_ROUTE, "OWNER", None).await["rules"],
        json!({ "enabled": 1, "minutes": 30 }),
        "a refused save stores nothing"
    );

    room.ok(
        OWNER_ROUTE,
        "OWNER",
        "settings",
        json!({ "branchId": MAIN, "minutes": 5, "enabled": true }),
    )
    .await;
    let booking = room.create(OWNER_ROUTE, "OWNER", walk_in("req-five")).await;
    assert_eq!(
        booking["expiresAt"],
        json!(stamp(room.now() + Duration::minutes(5))),
        "create uses the branch's minutes"
    );

    room.ok(
        OWNER_ROUTE,
        "OWNER",
        "settings",
        json!({ "branchId": MAIN, "minutes": 60, "enabled": false }),
    )
    .await;
    room.refused(
        OWNER_ROUTE,
        Some("OWNER"),
        "create",
        walk_in("req-closed"),
        400,
        CLOSED,
    )
    .await;
    // AIRPORT's rules are its own
    let airport = room
        .create(
            STAFF_ROUTE,
            "AIRPORT_FRONT",
            with(&walk_in("req-air"), "expectedRate", json!(36.1)),
        )
        .await;
    assert_eq!(
        airport["expiresAt"],
        json!(stamp(room.now() + Duration::minutes(30)))
    );
    room.finish().await;
}

/// `list` (v2 `bookings.ts:71-75`): bookings newest first, each as `publicBooking` prints it
/// (`booking-rules.mjs:8`), `EXPIRED` shown from the instant of `expiresAt`; members with v2's
/// snake_case keys, in `verified_at` order.
#[actix_web::test]
async fn list_prints_bookings_newest_first_and_members() {
    let room = Room::open().await;
    let first = room
        .create(OWNER_ROUTE, "OWNER", walk_in("req-list-1"))
        .await;
    let second = room
        .create(
            OWNER_ROUTE,
            "OWNER",
            with(
                &with(&walk_in("req-list-2"), "type", json!("SELL")),
                "expectedRate",
                json!(36.75),
            ),
        )
        .await;
    let third = room
        .create(
            STAFF_ROUTE,
            "FRONT",
            with(
                &with(&walk_in("req-list-3"), "currency", json!("EUR")),
                "expectedRate",
                json!(39.5),
            ),
        )
        .await;

    let listed = room.list(OWNER_ROUTE, "OWNER", None).await;
    assert_eq!(
        keys(&listed),
        vec!["bookings", "members", "publicUrl", "rules"]
    );
    assert_eq!(
        listed["bookings"],
        json!([third, second, first]),
        "newest first, each as create printed it"
    );
    for b in listed["bookings"].as_array().expect("bookings") {
        assert_eq!(keys(b), booking_keys(), "{b}");
    }
    assert_eq!(listed["publicUrl"], json!(""));

    // EXPIRED from the instant of the first booking's expiry, never stored
    room.desk
        .clock
        .set(time(first["expiresAt"].as_str().expect("an expiry")));
    let listed = room.list(STAFF_ROUTE, "FRONT", None).await;
    let statuses: Vec<&Value> = listed["bookings"]
        .as_array()
        .expect("bookings")
        .iter()
        .map(|b| &b["status"])
        .collect();
    assert_eq!(
        statuses,
        vec![&json!("CONFIRMED"), &json!("CONFIRMED"), &json!("EXPIRED")]
    );

    // members: verified_at order, v2's keys
    room.desk.clock.set(at(NOW));
    room.issue_member(SUDA.0, SECRET_B).await;
    let suda_at = stamp(room.now());
    room.issue_member(SOMCHAI.0, SECRET_A).await;
    let somchai_at = stamp(room.now());
    let members = room.list(OWNER_ROUTE, "OWNER", None).await["members"].clone();
    let members = members.as_array().expect("members").clone();
    assert_eq!(members.len(), 2);
    for (m, (customer, name, verified)) in members.iter().zip([
        (SUDA.0, SUDA.2, &suda_at),
        (SOMCHAI.0, SOMCHAI.2, &somchai_at),
    ]) {
        assert_eq!(
            keys(m),
            vec!["customer_id", "id", "name", "phone", "verified_at"]
        );
        assert!(
            m["id"].as_str().is_some_and(|id| id.starts_with("MB-")),
            "{m}"
        );
        assert_eq!(m["customer_id"], json!(customer));
        assert_eq!(m["name"], json!(name));
        assert_eq!(m["phone"], json!("081-234-5678"));
        assert_eq!(m["verified_at"], json!(verified));
    }
    room.finish().await;
}

// ── create ───────────────────────────────────────────────────────────────────

/// `create` (§5.6.3, new; statuses owner 2026-10-08): a walk-in is CONFIRMED at the counter rate
/// until now + the branch's minutes; a member's booking is PENDING with no expiry. The answer is
/// `{saved: true, booking}`, the booking as printed.
#[actix_web::test]
async fn create_answers_the_booking() {
    let room = Room::open().await;
    let booking = room
        .create(
            OWNER_ROUTE,
            "OWNER",
            with(
                &with(
                    &walk_in("req-create-1"),
                    "name",
                    json!("  Somchai Jaidee  "),
                ),
                "phone",
                json!(" 081-234-5678 "),
            ),
        )
        .await;
    assert_booking_id(&booking["id"], "walk-in");
    let created = stamp(room.now());
    assert_eq!(
        booking,
        json!({
            "id": booking["id"],
            "branchId": MAIN,
            "name": "Somchai Jaidee",
            "phone": "081-234-5678",
            "member": false,
            "type": "BUY",
            "currency": "USD",
            "amount": 1000,
            "rate": 36.25,
            "status": "CONFIRMED",
            "createdAt": created,
            "expiresAt": stamp(room.now() + Duration::minutes(30)),
            "transactionRef": null
        })
    );
    assert_eq!(number_text(&booking["amount"], "amount"), "1000");
    assert_eq!(number_text(&booking["rate"], "rate"), "36.25");

    // SELL at the counter's sell rate; an amount with a fraction prints its shortest form
    let sell = room
        .create(
            OWNER_ROUTE,
            "OWNER",
            json!({ "branchId": MAIN, "requestId": "req-create-2", "name": "Ann", "phone": "+66 81 234 5678",
                    "type": "SELL", "currency": "EUR", "amount": 250.5, "expectedRate": 40.1 }),
        )
        .await;
    assert_eq!(sell["type"], json!("SELL"));
    assert_eq!(sell["currency"], json!("EUR"));
    assert_eq!(number_text(&sell["amount"], "amount"), "250.5");
    assert_eq!(number_text(&sell["rate"], "rate"), "40.1");
    assert_eq!(sell["status"], json!("CONFIRMED"));

    // the counter rate is the branch's own
    room.refused(
        OWNER_ROUTE,
        Some("OWNER"),
        "create",
        with(&walk_in("req-create-3"), "branchId", json!(AIRPORT)),
        400,
        RATE_MOVED,
    )
    .await;
    let airport = room
        .create(
            OWNER_ROUTE,
            "OWNER",
            with(
                &with(&walk_in("req-create-3"), "branchId", json!(AIRPORT)),
                "expectedRate",
                json!(36.1),
            ),
        )
        .await;
    assert_eq!(airport["branchId"], json!(AIRPORT));
    assert_eq!(number_text(&airport["rate"], "rate"), "36.1");

    // a member's booking waits for the counter
    room.issue_member(SOMCHAI.0, SECRET_A).await;
    let member = room
        .create(
            STAFF_ROUTE,
            "FRONT",
            with(&walk_in("req-create-4"), "customerId", json!(SOMCHAI.0)),
        )
        .await;
    assert_booking_id(&member["id"], "member");
    assert_eq!(member["member"], json!(true));
    assert_eq!(member["status"], json!("PENDING"));
    assert_eq!(member["expiresAt"], Value::Null);
    assert_eq!(number_text(&member["rate"], "rate"), "36.25");
    assert_eq!(member["createdAt"], json!(stamp(room.now())));

    // refusal 8: a customer with no member row in this branch
    room.refused(
        OWNER_ROUTE,
        Some("OWNER"),
        "create",
        with(&walk_in("req-create-5"), "customerId", json!(SUDA.0)),
        400,
        "ไม่พบ Member ของลูกค้านี้ในสาขานี้",
    )
    .await;
    room.finish().await;
}

/// `create` refusals 1–6, parsed at the route (lane controller on #413 and #409), in the order
/// of §5.6.3: each payload breaks the step named and every later one, so only the order gives
/// its sentence. Nothing is written. On both routes.
#[actix_web::test]
async fn create_payload_refusals_in_order() {
    let room = Room::open().await;
    let good = walk_in("req-order-1");
    let broken = json!({
        "branchId": MAIN,
        "requestId": "",
        "name": "",
        "phone": "12",
        "type": "HOLD",
        "currency": "THB",
        "amount": 0,
        "expectedRate": 0
    });
    // step by step: fix the earlier fields from `good`, keep the later ones broken
    let fields = [
        "requestId",
        "name",
        "phone",
        "type",
        "currency",
        "amount",
        "expectedRate",
    ];
    let sentences = [
        FILL_IN, FILL_IN, BAD_PHONE, INCOMPLETE, INCOMPLETE, BAD_NUMBER, BAD_NUMBER,
    ];
    for (i, sentence) in sentences.iter().enumerate() {
        let mut payload = broken.clone();
        for field in &fields[..i] {
            payload[*field] = good[*field].clone();
        }
        for (route, who) in [(OWNER_ROUTE, "OWNER"), (STAFF_ROUTE, "FRONT")] {
            room.refused(route, Some(who), "create", payload.clone(), 400, sentence)
                .await;
        }
    }

    // each field alone
    let cases: Vec<(&str, Value, &str)> = vec![
        // 1 requestId: text, trimmed, 1–60
        ("requestId", Value::Null, FILL_IN),
        ("requestId", json!("   "), FILL_IN),
        ("requestId", json!(5), FILL_IN),
        ("requestId", json!("r".repeat(61)), FILL_IN),
        // 2 name: text, trimmed, 1–100
        ("name", Value::Null, FILL_IN),
        ("name", json!(""), FILL_IN),
        ("name", json!(5), FILL_IN),
        ("name", json!("ก".repeat(101)), FILL_IN),
        // 3 phone: text ≤ 30, only +0-9 ()-, at least 7 digits
        ("phone", Value::Null, BAD_PHONE),
        ("phone", json!("12-34-56"), BAD_PHONE),
        ("phone", json!("081-234-5678 ext"), BAD_PHONE),
        (
            "phone",
            json!(format!("0812345678{}", " ".repeat(21)) + "9"),
            BAD_PHONE,
        ),
        // 4 type BUY|SELL, currency a stock currency other than THB
        ("type", Value::Null, INCOMPLETE),
        ("type", json!("buy"), INCOMPLETE),
        ("type", json!("HOLD"), INCOMPLETE),
        ("currency", Value::Null, INCOMPLETE),
        ("currency", json!("THB"), INCOMPLETE),
        ("currency", json!("ZZZ"), INCOMPLETE),
        ("currency", json!("usd"), INCOMPLETE),
        // 5 amount: Money::parse of the number's text, not zero
        ("amount", json!(0), BAD_NUMBER),
        ("amount", json!(-1), BAD_NUMBER),
        ("amount", json!(100_000_001), BAD_NUMBER),
        ("amount", json!(1000.555), TOO_MANY_DECIMALS),
        // 6 expectedRate: the number's text, a rate per 1. Amended for #758 (RE-4d, rates §4.9 R3
        // and the ticket author's ruling 4): a per-1 rate carries up to 8 + log10(unit_size)
        // places, 11 for KRW per 1000, so `TOO_MANY_DECIMALS` starts at 12 places, not 9.
        ("expectedRate", json!(0), BAD_NUMBER),
        ("expectedRate", json!(-36.25), BAD_NUMBER),
        ("expectedRate", json!(0.025123456789), TOO_MANY_DECIMALS),
    ];
    for (field, value, sentence) in cases {
        let payload = if value.is_null() {
            without(&good, field)
        } else {
            with(&good, field, value)
        };
        // C9: the counter is FRONT
        for (route, who) in [(OWNER_ROUTE, "OWNER"), (STAFF_ROUTE, "FRONT")] {
            room.refused(route, Some(who), "create", payload.clone(), 400, sentence)
                .await;
        }
    }

    // nothing was written
    assert_eq!(
        room.list(OWNER_ROUTE, "OWNER", None).await["bookings"],
        json!([])
    );
    // and the edges that pass: 60 characters, 100 characters, 30 characters, 7 digits
    let edge = room
        .create(
            OWNER_ROUTE,
            "OWNER",
            json!({ "branchId": MAIN, "requestId": "r".repeat(60), "name": "ก".repeat(100),
                    "phone": format!("1234567{}", "-".repeat(23)), "type": "BUY", "currency": "USD",
                    "amount": 0.01, "expectedRate": 36.25 }),
        )
        .await;
    assert_eq!(edge["name"], json!("ก".repeat(100)));
    assert_eq!(number_text(&edge["amount"], "amount"), "0.01");
    room.finish().await;
}

/// §5.6.3 refusal 7: the request hash is over the parsed command, so a repeat that differs only
/// in what parsing removes (spaces around text, `1000.0` for `1000`) is the stored booking, as
/// stored; another command under the same `requestId` is `CHANGED`.
#[actix_web::test]
async fn create_is_idempotent_over_the_parsed_command() {
    let room = Room::open().await;
    let first = room.create(OWNER_ROUTE, "OWNER", walk_in("req-same")).await;
    let repeat = with(
        &with(&walk_in("req-same"), "name", json!("  Somchai Jaidee ")),
        "amount",
        json!(1000.0),
    );
    let again = room.create(OWNER_ROUTE, "OWNER", repeat.clone()).await;
    assert_eq!(again, first, "the stored booking, as stored");
    let again = room.create(STAFF_ROUTE, "FRONT", repeat).await;
    assert_eq!(again, first, "the same request on the staff route");

    room.refused(
        OWNER_ROUTE,
        Some("OWNER"),
        "create",
        with(&walk_in("req-same"), "amount", json!(1001)),
        400,
        CHANGED,
    )
    .await;
    assert_eq!(
        room.list(OWNER_ROUTE, "OWNER", None).await["bookings"],
        json!([first])
    );
    room.finish().await;
}

// ── get, confirm, cancel ─────────────────────────────────────────────────────

/// `get` (v2 `bookings.ts:82-83`): the booking plus `customerId`, the member's customer or
/// `null`; `id` not text ≤ 60 → `กรอกข้อมูลให้ครบและไม่เกินกำหนด`; not in the branch →
/// `ไม่พบการจอง`.
#[actix_web::test]
async fn get_answers_the_booking_and_customer_id() {
    let room = Room::open().await;
    let walk = room
        .create(OWNER_ROUTE, "OWNER", walk_in("req-get-1"))
        .await;
    room.issue_member(SOMCHAI.0, SECRET_A).await;
    let member = room
        .create(
            OWNER_ROUTE,
            "OWNER",
            with(&walk_in("req-get-2"), "customerId", json!(SOMCHAI.0)),
        )
        .await;

    // C9: the counter is FRONT
    for (route, who) in [(OWNER_ROUTE, "OWNER"), (STAFF_ROUTE, "FRONT")] {
        let got = room
            .get(route, who, MAIN, walk["id"].as_str().expect("id"))
            .await;
        let mut expected = walk.clone();
        expected["customerId"] = Value::Null;
        assert_eq!(got, expected, "{who}");

        let got = room
            .get(route, who, MAIN, member["id"].as_str().expect("id"))
            .await;
        let mut expected = member.clone();
        expected["customerId"] = json!(SOMCHAI.0);
        assert_eq!(got, expected, "{who}");

        for id in [Value::Null, json!(""), json!(5), json!("x".repeat(61))] {
            room.refused(
                route,
                Some(who),
                "get",
                json!({ "branchId": MAIN, "id": id }),
                400,
                FILL_IN,
            )
            .await;
        }
        room.refused(
            route,
            Some(who),
            "get",
            json!({ "branchId": MAIN, "id": "BK-none" }),
            400,
            NOT_FOUND,
        )
        .await;
    }
    room.finish().await;
}

/// `confirm` (v2 `bookings.ts:85-89`, §5.6.3): refusals 1–2 as `get`, then `expiresAt` —
/// RFC 3339 text only, in the future (an unreadable time is parsed at the route as "no time",
/// lane controller on #413) — then the machine's rules. A confirmed member booking is CONFIRMED
/// at the counter rate until the instant sent, printed in UTC.
#[actix_web::test]
async fn confirm_reads_rfc3339_only() {
    let room = Room::open().await;
    room.issue_member(SOMCHAI.0, SECRET_A).await;
    let member = room
        .create(
            OWNER_ROUTE,
            "OWNER",
            with(&walk_in("req-confirm-1"), "customerId", json!(SOMCHAI.0)),
        )
        .await;
    let id = member["id"].as_str().expect("id").to_string();
    let walk = room
        .create(OWNER_ROUTE, "OWNER", walk_in("req-confirm-2"))
        .await;
    let now = room.now();

    // 1–2 before 3
    for (bad_id, sentence) in [
        (Value::Null, FILL_IN),
        (json!(5), FILL_IN),
        (json!("x".repeat(61)), FILL_IN),
        (json!("BK-none"), NOT_FOUND),
    ] {
        room.refused(
            OWNER_ROUTE,
            Some("OWNER"),
            "confirm",
            json!({ "branchId": MAIN, "id": bad_id, "expiresAt": "tomorrow", "expectedRate": 0 }),
            400,
            sentence,
        )
        .await;
    }

    // 3: not RFC 3339 text, or not after now
    for expires in [
        Value::Null,
        json!(""),
        json!("tomorrow"),
        json!(now.timestamp_millis() + 3_600_000),
        json!("2026-10-06"),
        json!("2026-10-06T07:00:00"),
        json!("06 Oct 2026 07:00:00 GMT"),
        json!(true),
        json!(stamp(now)),
        json!(stamp(now - Duration::minutes(1))),
    ] {
        for (route, who) in [(OWNER_ROUTE, "OWNER"), (STAFF_ROUTE, "FRONT")] {
            room.refused(
                route,
                Some(who),
                "confirm",
                json!({ "branchId": MAIN, "id": id, "expiresAt": expires, "expectedRate": 36.25 }),
                400,
                EXPIRY,
            )
            .await;
        }
    }
    let missing = json!({ "branchId": MAIN, "id": id, "expectedRate": 36.25 });
    room.refused(OWNER_ROUTE, Some("OWNER"), "confirm", missing, 400, EXPIRY)
        .await;

    // 5: only a member's PENDING booking
    room.refused(
        OWNER_ROUTE,
        Some("OWNER"),
        "confirm",
        json!({ "branchId": MAIN, "id": walk["id"], "expiresAt": "2026-10-06T07:00:00.000Z",
                "expectedRate": 36.25 }),
        400,
        ONLY_PENDING_MEMBER,
    )
    .await;

    // 6: the counter rate, as a number; missing or another is "the rate moved"
    for rate in [Value::Null, json!(36.3), json!(0)] {
        let mut payload =
            json!({ "branchId": MAIN, "id": id, "expiresAt": "2026-10-06T07:00:00.000Z" });
        if !rate.is_null() {
            payload["expectedRate"] = rate;
        }
        room.refused(
            OWNER_ROUTE,
            Some("OWNER"),
            "confirm",
            payload,
            400,
            RATE_MOVED,
        )
        .await;
    }
    assert_eq!(
        room.get(OWNER_ROUTE, "OWNER", MAIN, &id).await["status"],
        json!("PENDING"),
        "a refused confirm writes nothing"
    );

    // confirmed: an offset is read as the instant it names and printed in UTC
    let saved = room
        .ok(
            STAFF_ROUTE,
            "FRONT",
            "confirm",
            json!({ "branchId": MAIN, "id": id, "expiresAt": "2026-10-06T14:00:00+07:00",
                    "expectedRate": 36.25 }),
        )
        .await;
    assert_eq!(saved, json!({ "saved": true }));
    let got = room.get(OWNER_ROUTE, "OWNER", MAIN, &id).await;
    assert_eq!(got["status"], json!("CONFIRMED"));
    assert_eq!(got["expiresAt"], json!("2026-10-06T07:00:00.000Z"));
    assert_eq!(number_text(&got["rate"], "rate"), "36.25");
    assert_eq!(got["customerId"], json!(SOMCHAI.0));

    // the same instant again is {saved: true}; another is refused
    let again = room
        .ok(
            OWNER_ROUTE,
            "OWNER",
            "confirm",
            json!({ "branchId": MAIN, "id": id, "expiresAt": "2026-10-06T07:00:00.000Z" }),
        )
        .await;
    assert_eq!(again, json!({ "saved": true }));
    room.refused(
        OWNER_ROUTE,
        Some("OWNER"),
        "confirm",
        json!({ "branchId": MAIN, "id": id, "expiresAt": "2026-10-06T08:00:00.000Z",
                "expectedRate": 36.25 }),
        400,
        ONLY_PENDING_MEMBER,
    )
    .await;
    room.finish().await;
}

/// `cancel` (v2 `bookings.ts:84`): `{saved: true}`, CANCELLED; again → `{saved: true}`; an
/// EXPIRED booking can be cancelled.
#[actix_web::test]
async fn cancel_answers_saved() {
    let room = Room::open().await;
    let walk = room
        .create(OWNER_ROUTE, "OWNER", walk_in("req-cancel-1"))
        .await;
    let id = walk["id"].as_str().expect("id").to_string();
    // C9: the counter is FRONT
    for (route, who) in [(STAFF_ROUTE, "FRONT"), (OWNER_ROUTE, "OWNER")] {
        let saved = room
            .ok(route, who, "cancel", json!({ "branchId": MAIN, "id": id }))
            .await;
        assert_eq!(saved, json!({ "saved": true }), "{who}");
        assert_eq!(
            room.get(OWNER_ROUTE, "OWNER", MAIN, &id).await["status"],
            json!("CANCELLED")
        );
    }
    for bad_id in [Value::Null, json!(5), json!("")] {
        room.refused(
            OWNER_ROUTE,
            Some("OWNER"),
            "cancel",
            json!({ "branchId": MAIN, "id": bad_id }),
            400,
            FILL_IN,
        )
        .await;
    }
    room.refused(
        STAFF_ROUTE,
        Some("FRONT"),
        "cancel",
        json!({ "id": "BK-none" }),
        400,
        NOT_FOUND,
    )
    .await;

    let expiring = room
        .create(OWNER_ROUTE, "OWNER", walk_in("req-cancel-2"))
        .await;
    let expiring_id = expiring["id"].as_str().expect("id").to_string();
    room.desk.clock.advance(Duration::minutes(30));
    assert_eq!(
        room.get(STAFF_ROUTE, "FRONT", MAIN, &expiring_id).await["status"],
        json!("EXPIRED")
    );
    room.ok(STAFF_ROUTE, "FRONT", "cancel", json!({ "id": expiring_id }))
        .await;
    assert_eq!(
        room.get(STAFF_ROUTE, "FRONT", MAIN, &expiring_id).await["status"],
        json!("CANCELLED")
    );
    room.finish().await;
}

// ── member, registrations ────────────────────────────────────────────────────

/// `member` (v2 `bookings.ts:78-80`, §5.6.3): refusal 1 (no LIVE customer of the branch, or
/// `identityConfirmed` not JSON `true`) before 2 (phone) before 3 (secret text ≤ 80) before 4
/// (`^[a-f0-9-]{36}$`); 5, another member's code. Refusals 2–4 are parsed at the route (lane
/// controller on #413 and #409). The answer is `{saved: true, name, memberCode}`, `name` the
/// customer's.
#[actix_web::test]
async fn member_refusals_in_order_and_the_answer() {
    let room = Room::open().await;
    let good = json!({ "branchId": MAIN, "customerId": SOMCHAI.0, "identityConfirmed": true,
                       "phone": "081-234-5678", "memberSecret": SECRET_A });
    let mut cases: Vec<(Value, &str)> = Vec::new();
    // 1, with phone and secret broken too
    for confirmed in [json!(false), json!("true"), json!(1), Value::Null] {
        let mut p = with(
            &with(&good, "phone", json!("12")),
            "memberSecret",
            json!("x"),
        );
        if confirmed.is_null() {
            p = without(&p, "identityConfirmed");
        } else {
            p["identityConfirmed"] = confirmed;
        }
        cases.push((p, NOT_IDENTIFIED));
    }
    // CI-4 (#651): AIR_CUSTOMER left this list: a customer reserved at another branch is the organisation's
    // customer and is identified here too (#554 K1).
    for customer in [json!("C-NOBODY"), json!(DEMO), Value::Null] {
        let p = if customer.is_null() {
            without(&good, "customerId")
        } else {
            with(&good, "customerId", customer)
        };
        cases.push((p, NOT_IDENTIFIED));
    }
    // 2 before 3 and 4
    for phone in [
        Value::Null,
        json!("12-34-56"),
        json!("081-234-5678 ext"),
        json!(812345678),
    ] {
        let p = with(&with(&good, "phone", phone), "memberSecret", json!(""));
        cases.push((p, BAD_PHONE));
    }
    // 3 before 4
    for secret in [Value::Null, json!(""), json!(5), json!("a".repeat(81))] {
        cases.push((with(&good, "memberSecret", secret), FILL_IN));
    }
    // 4
    for secret in [
        json!("not-a-uuid"),
        json!(SECRET_A.to_uppercase()),
        json!("6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4g"),
        json!(format!("{SECRET_A}0")),
    ] {
        cases.push((with(&good, "memberSecret", secret), BAD_MEMBER_CODE));
    }
    for (payload, sentence) in cases {
        room.refused(OWNER_ROUTE, Some("OWNER"), "member", payload, 400, sentence)
            .await;
    }
    assert_eq!(
        room.list(OWNER_ROUTE, "OWNER", None).await["members"],
        json!([]),
        "nothing was written"
    );

    let saved = room
        .ok(
            OWNER_ROUTE,
            "OWNER",
            "member",
            with(&good, "phone", json!(" 081-234-5678 ")),
        )
        .await;
    assert_eq!(
        saved,
        json!({ "saved": true, "name": SOMCHAI.2, "memberCode": SECRET_A })
    );
    let members = room.list(OWNER_ROUTE, "OWNER", None).await["members"].clone();
    assert_eq!(members[0]["phone"], json!("081-234-5678"));
    assert_eq!(members[0]["customer_id"], json!(SOMCHAI.0));

    // 5: the code is another member's
    room.refused(
        OWNER_ROUTE,
        Some("OWNER"),
        "member",
        with(&good, "customerId", json!(SUDA.0)),
        400,
        BAD_MEMBER_CODE,
    )
    .await;
    room.finish().await;
}

/// `registrations` and `approveRegistration` (§5.6.2: not ported, no registration exists).
#[actix_web::test]
async fn registrations_are_empty() {
    let room = Room::open().await;
    for (page, printed) in [
        (Value::Null, 0),
        (json!(2), 2),
        (json!(2.7), 2),
        (json!(-5), 0),
        (json!(100_001), 100_000),
        (json!("x"), 0),
    ] {
        let mut payload = json!({ "branchId": MAIN, "query": "สม" });
        if !page.is_null() {
            payload["page"] = page.clone();
        }
        let answer = room
            .ok(OWNER_ROUTE, "OWNER", "registrations", payload)
            .await;
        assert_eq!(
            answer,
            json!({ "items": [], "hasMore": false, "page": printed }),
            "page {page}"
        );
    }
    for (id, sentence) in [
        (Value::Null, FILL_IN),
        (json!(5), FILL_IN),
        (json!("x".repeat(61)), FILL_IN),
        (json!("REG-1"), NO_REGISTRATION),
    ] {
        room.refused(
            OWNER_ROUTE,
            Some("OWNER"),
            "approveRegistration",
            json!({ "branchId": MAIN, "id": id, "customerId": SOMCHAI.0,
                    "identityConfirmed": true, "memberSecret": SECRET_A }),
            400,
            sentence,
        )
        .await;
    }
    room.finish().await;
}

// ── roles and branches ───────────────────────────────────────────────────────

/// Owner 2026-10-08: FRONT, BACKEND and MANAGER may `list`, `get`, `create`, `confirm`,
/// `cancel` on `/api/staff/bookings`.
#[actix_web::test]
async fn counter_roles_run_the_five_actions() {
    let room = Room::open().await;
    room.issue_member(SOMCHAI.0, SECRET_A).await;
    // C9: an ex-counter role runs none of the five
    for role in EX_COUNTER {
        for action in ["list", "get", "create", "confirm", "cancel"] {
            room.refused(
                STAFF_ROUTE,
                Some(role),
                action,
                json!({}),
                400,
                ROLE_NOT_ALLOWED,
            )
            .await;
        }
    }
    for role in COUNTER {
        let walk = room
            .create(STAFF_ROUTE, role, walk_in(&format!("req-{role}-walk")))
            .await;
        assert_eq!(walk["status"], json!("CONFIRMED"), "{role}");
        let walk_id = walk["id"].as_str().expect("id").to_string();
        let listed = room.list(STAFF_ROUTE, role, None).await;
        assert_eq!(listed["bookings"][0], walk, "{role}: newest first");
        let got = room.get(STAFF_ROUTE, role, MAIN, &walk_id).await;
        assert_eq!(got["id"], json!(walk_id), "{role}");

        let member = room
            .create(
                STAFF_ROUTE,
                role,
                with(
                    &walk_in(&format!("req-{role}-member")),
                    "customerId",
                    json!(SOMCHAI.0),
                ),
            )
            .await;
        assert_eq!(member["status"], json!("PENDING"), "{role}");
        let member_id = member["id"].as_str().expect("id").to_string();
        let saved = room
            .ok(
                STAFF_ROUTE,
                role,
                "confirm",
                json!({ "id": member_id, "expiresAt": "2026-10-06T09:00:00.000Z",
                        "expectedRate": 36.25 }),
            )
            .await;
        assert_eq!(saved, json!({ "saved": true }), "{role}");
        assert_eq!(
            room.get(STAFF_ROUTE, role, MAIN, &member_id).await["status"],
            json!("CONFIRMED"),
            "{role}"
        );

        let saved = room
            .ok(STAFF_ROUTE, role, "cancel", json!({ "id": walk_id }))
            .await;
        assert_eq!(saved, json!({ "saved": true }), "{role}");
        assert_eq!(
            room.get(STAFF_ROUTE, role, MAIN, &walk_id).await["status"],
            json!("CANCELLED"),
            "{role}"
        );
    }
    room.finish().await;
}

/// §5.6.8 `staff_branch_is_forced`: an employee naming another branch on `/api/staff/bookings`
/// runs on their own.
#[actix_web::test]
async fn staff_branch_is_forced() {
    let room = Room::open().await;
    let main = room
        .create(
            STAFF_ROUTE,
            "FRONT",
            with(&walk_in("req-forced-1"), "branchId", json!(AIRPORT)),
        )
        .await;
    assert_eq!(main["branchId"], json!(MAIN));
    assert_eq!(number_text(&main["rate"], "rate"), "36.25", "MAIN's rate");

    let airport = room
        .create(
            STAFF_ROUTE,
            "AIRPORT_FRONT",
            with(
                &with(&walk_in("req-forced-2"), "branchId", json!(MAIN)),
                "expectedRate",
                json!(36.1),
            ),
        )
        .await;
    assert_eq!(airport["branchId"], json!(AIRPORT));

    for named in [
        None,
        Some(MAIN),
        Some(AIRPORT),
        Some("NOWHERE"),
        Some("bad branch!"),
    ] {
        let listed = room.list(STAFF_ROUTE, "FRONT", named).await;
        assert_eq!(listed["bookings"], json!([main]), "FRONT naming {named:?}");
        let listed = room.list(STAFF_ROUTE, "AIRPORT_FRONT", named).await;
        assert_eq!(
            listed["bookings"],
            json!([airport]),
            "AIRPORT_FRONT naming {named:?}"
        );
    }
    room.finish().await;
}

/// §5.6.8 `staff_cannot_read_another_branch`: a booking id from another branch answers
/// `ไม่พบการจอง` on both routes, and nothing happens to that booking.
#[actix_web::test]
async fn staff_cannot_read_another_branch() {
    let room = Room::open().await;
    room.issue_member(SOMCHAI.0, SECRET_A).await;
    let main = room
        .create(
            OWNER_ROUTE,
            "OWNER",
            with(&walk_in("req-iso-1"), "customerId", json!(SOMCHAI.0)),
        )
        .await;
    let airport = room
        .create(
            OWNER_ROUTE,
            "OWNER",
            with(
                &with(&walk_in("req-iso-2"), "branchId", json!(AIRPORT)),
                "expectedRate",
                json!(36.1),
            ),
        )
        .await;
    let main_id = main["id"].as_str().expect("id").to_string();
    let air_id = airport["id"].as_str().expect("id").to_string();

    // the lists hold different bookings
    assert_eq!(
        room.list(OWNER_ROUTE, "OWNER", Some(MAIN)).await["bookings"],
        json!([main])
    );
    assert_eq!(
        room.list(OWNER_ROUTE, "OWNER", Some(AIRPORT)).await["bookings"],
        json!([airport])
    );

    for (route, who, branch, id) in [
        (STAFF_ROUTE, "FRONT", AIRPORT, &air_id),
        // C9: the counter is FRONT
        (STAFF_ROUTE, "FRONT", MAIN, &air_id),
        (STAFF_ROUTE, "AIRPORT_FRONT", MAIN, &main_id),
        (OWNER_ROUTE, "OWNER", MAIN, &air_id),
        (OWNER_ROUTE, "OWNER", AIRPORT, &main_id),
    ] {
        for action in ["get", "cancel"] {
            room.refused(
                route,
                Some(who),
                action,
                json!({ "branchId": branch, "id": id }),
                400,
                NOT_FOUND,
            )
            .await;
        }
        room.refused(
            route,
            Some(who),
            "confirm",
            json!({ "branchId": branch, "id": id, "expiresAt": "2026-10-06T09:00:00.000Z",
                    "expectedRate": 36.25 }),
            400,
            NOT_FOUND,
        )
        .await;
    }

    // untouched
    let mut expected = main.clone();
    expected["customerId"] = json!(SOMCHAI.0);
    assert_eq!(
        room.get(OWNER_ROUTE, "OWNER", MAIN, &main_id).await,
        expected
    );
    let mut expected = airport.clone();
    expected["customerId"] = Value::Null;
    assert_eq!(
        room.get(STAFF_ROUTE, "AIRPORT_FRONT", AIRPORT, &air_id)
            .await,
        expected
    );
    room.finish().await;
}

// ── the kept pages ───────────────────────────────────────────────────────────

/// §5.6.8 `kept_screen_shapes`: the five requests of the kept `bookings.js` exactly as its
/// `bookingApi` builds them (v2 `src/online/public/bookings.js:5`: `branchId` first, then the
/// action's fields), and the fields its drawing reads (`:9-13`, `:16`).
#[actix_web::test]
async fn kept_screen_shapes() {
    let room = Room::open().await;

    // list — drawBookings reads rules.minutes, rules.enabled, members.length, publicUrl
    let answer = room
        .post(
            OWNER_ROUTE,
            Some("OWNER"),
            r#"{"action":"list","payload":{"branchId":"MAIN"}}"#,
        )
        .await;
    let listed = answered(&answer, "list");
    assert!(listed["bookings"].is_array());
    assert!(listed["members"].is_array());
    assert_eq!(number_text(&listed["rules"]["minutes"], "minutes"), "30");
    assert_eq!(listed["rules"]["enabled"], json!(1));
    assert_eq!(listed["publicUrl"], json!(""));

    // settings — {minutes: Number(input), enabled: checkbox}
    let answer = room
        .post(
            OWNER_ROUTE,
            Some("OWNER"),
            r#"{"action":"settings","payload":{"branchId":"MAIN","minutes":45,"enabled":true}}"#,
        )
        .await;
    assert_eq!(answered(&answer, "settings"), json!({ "saved": true }));

    // member — {customerId, phone, identityConfirmed: checkbox, memberSecret: randomUUID()}
    let answer = room
        .post(
            OWNER_ROUTE,
            Some("OWNER"),
            format!(
                r#"{{"action":"member","payload":{{"branchId":"MAIN","customerId":"{}","phone":"0812345678","identityConfirmed":true,"memberSecret":"{SECRET_A}"}}}}"#,
                SOMCHAI.0
            ),
        )
        .await;
    assert_eq!(
        answered(&answer, "member"),
        json!({ "saved": true, "name": SOMCHAI.2, "memberCode": SECRET_A })
    );

    // a member's booking (create has no form yet: API-only) and a walk-in
    let member = room
        .create(
            OWNER_ROUTE,
            "OWNER",
            with(&walk_in("req-screen-1"), "customerId", json!(SOMCHAI.0)),
        )
        .await;
    let walk = room
        .create(OWNER_ROUTE, "OWNER", walk_in("req-screen-2"))
        .await;

    // the rows drawBookings draws
    let answer = room
        .post(
            OWNER_ROUTE,
            Some("OWNER"),
            r#"{"action":"list","payload":{"branchId":"MAIN"}}"#,
        )
        .await;
    let listed = answered(&answer, "list");
    assert_eq!(listed["rules"]["minutes"], json!(45));
    assert_eq!(listed["members"].as_array().map(Vec::len), Some(1));
    for b in listed["bookings"].as_array().expect("bookings") {
        for key in ["id", "name", "phone", "type", "currency", "status"] {
            assert!(b[key].is_string(), "{key} is text: {b}");
        }
        assert!(b["member"].is_boolean(), "{b}");
        assert!(b["amount"].is_number() && b["rate"].is_number(), "{b}");
        assert!(
            b["expiresAt"].is_string() || b["expiresAt"].is_null(),
            "{b}"
        );
        assert!(
            ["PENDING", "CONFIRMED", "EXPIRED", "USED", "CANCELLED"]
                .contains(&b["status"].as_str().expect("status")),
            "{b}"
        );
    }
    assert_eq!(listed["bookings"][0]["id"], walk["id"]);
    assert_eq!(listed["bookings"][1]["id"], member["id"]);
    assert_eq!(listed["bookings"][1]["expiresAt"], Value::Null);

    // confirm — {id, expiresAt: new Date(input).toISOString(), expectedRate: the state's rate}
    let answer = room
        .post(
            OWNER_ROUTE,
            Some("OWNER"),
            format!(
                r#"{{"action":"confirm","payload":{{"branchId":"MAIN","id":"{}","expiresAt":"2026-10-06T08:00:00.000Z","expectedRate":36.25}}}}"#,
                member["id"].as_str().expect("id")
            ),
        )
        .await;
    assert_eq!(answered(&answer, "confirm"), json!({ "saved": true }));

    // cancel — {id}
    let answer = room
        .post(
            OWNER_ROUTE,
            Some("OWNER"),
            format!(
                r#"{{"action":"cancel","payload":{{"branchId":"MAIN","id":"{}"}}}}"#,
                walk["id"].as_str().expect("id")
            ),
        )
        .await;
    assert_eq!(answered(&answer, "cancel"), json!({ "saved": true }));

    let listed = room.list(OWNER_ROUTE, "OWNER", None).await;
    assert_eq!(listed["bookings"][0]["status"], json!("CANCELLED"));
    assert_eq!(listed["bookings"][1]["status"], json!("CONFIRMED"));
    assert_eq!(
        listed["bookings"][1]["expiresAt"],
        json!("2026-10-06T08:00:00.000Z")
    );
    room.finish().await;
}

/// §5.6.8 `staff_desk_get`: the desk opened with `?booking=<id>` on `/staff/desk` sends
/// `{action:'get',payload:{branchId:deskBranch,id}}` (v2 `src/online/lib/index-html.ts:1`) to
/// `/api/staff/bookings`, and reads `status`, `type`, `currency`, `amount`, `rate`, and the
/// member's `customerId`.
#[actix_web::test]
async fn staff_desk_get() {
    let room = Room::open().await;
    room.issue_member(SOMCHAI.0, SECRET_A).await;
    let member = room
        .create(
            OWNER_ROUTE,
            "OWNER",
            with(&walk_in("req-desk-1"), "customerId", json!(SOMCHAI.0)),
        )
        .await;
    let id = member["id"].as_str().expect("id").to_string();
    room.ok(
        OWNER_ROUTE,
        "OWNER",
        "confirm",
        json!({ "branchId": MAIN, "id": id, "expiresAt": "2026-10-06T09:00:00.000Z",
                "expectedRate": 36.25 }),
    )
    .await;
    let walk = room
        .create(OWNER_ROUTE, "OWNER", walk_in("req-desk-2"))
        .await;

    // C9: an ex-counter role gets no booking
    for role in EX_COUNTER {
        room.refused(
            STAFF_ROUTE,
            Some(role),
            "get",
            json!({ "branchId": MAIN, "id": id }),
            400,
            ROLE_NOT_ALLOWED,
        )
        .await;
    }
    for role in COUNTER {
        let answer = room
            .post(
                STAFF_ROUTE,
                Some(role),
                format!(r#"{{"action":"get","payload":{{"branchId":"MAIN","id":"{id}"}}}}"#),
            )
            .await;
        let got = answered(&answer, role);
        let mut expected = member.clone();
        expected["status"] = json!("CONFIRMED");
        expected["expiresAt"] = json!("2026-10-06T09:00:00.000Z");
        expected["customerId"] = json!(SOMCHAI.0);
        assert_eq!(got, expected, "{role}");
        assert_eq!(number_text(&got["amount"], "amount"), "1000");
        assert_eq!(number_text(&got["rate"], "rate"), "36.25");

        let answer = room
            .post(
                STAFF_ROUTE,
                Some(role),
                format!(
                    r#"{{"action":"get","payload":{{"branchId":"MAIN","id":"{}"}}}}"#,
                    walk["id"].as_str().expect("id")
                ),
            )
            .await;
        let got = answered(&answer, role);
        assert_eq!(got["status"], json!("CONFIRMED"));
        assert_eq!(got["customerId"], Value::Null);
    }
    room.finish().await;
}
