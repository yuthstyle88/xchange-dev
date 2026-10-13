//! Ticket #485 (V1-kiosk-queue-views, `docs/design/xchange-v3-web.md` §12.18.2, §4.2): the two
//! views a v3 queue screen reads instead of v2's `notifications` and `list` on `POST
//! /api/kiosk`. Written before the implementation by the lane controller. Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! - `GET /api/v3/views/kiosk_queue` — a `PagedView` inheriting `(EntryPath::Kiosk, "list")`,
//!   filter `state` = `waiting` | `working` | `completed` (absent = `waiting`; anything else
//!   is refused naming `state`). Rows newest first (`created_at DESC, id DESC`), keyset paged by
//!   `before` (a row id; the page starts strictly after that row), at most `limit`. A row has
//!   exactly fifteen fields; their values are compat `list`'s for the same caller, except that
//!   `foreign` and `thb` are decimal strings with two places (W7).
//! - Nobody signed in is refused `kioskStaff` (`policy.rs:303-309`). Staff are answered for
//!   their own branch whatever they name (`Policy::branch`).
//! - Each of the five writers of `kiosk_requests` (customer `quote`, `confirm`; desk `accept`,
//!   `identityChecked`; the save's `complete_in`) moves the branch version, so the same `since`
//!   answers `changed: true` after it, and `changed: false` before it (§4.2).
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real
//! sessions made at the real sign-in doors (`kiosk_http_common::Counter`). Rows the read tests
//! need are put in `kiosk_requests` behind the routes, each with its own `created_at`; where
//! two rows share one on purpose, the test says so and `id` decides. The writer tests use the
//! routes only, and move the test's clock on before every write.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod kiosk_http_common;
mod rpc_common;
mod snapshot_common;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::http::Method;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use compat_common::Person;
use kiosk_http_common::{Counter, person};
use rpc_common::{Answer, Call};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xchange::presentation::http::v3::VIEWS;

/// Amended for QU-6 #1354 (ruling issuecomment-6117792804), setup only: `accept` starts the
/// request's queue ticket as the accepting teller (queue-tickets.md §4.5), and the queue serves
/// only a teller with an ACTIVE counter, so the accepting staff of `MAIN` get one (and the `users`
/// row it references), seeded as `kiosk_accept_queue_ticket_test.rs` seeds them.
async fn open_counters(pool: &sqlx::PgPool, tellers: &[&str]) {
    for teller in tellers {
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) \
             SELECT $2::uuid, organization_id, 'Teller', 'ACTIVE', 'test' FROM branches WHERE id = $1 \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(rpc_common::MAIN)
        .bind(teller)
        .execute(pool)
        .await
        .expect("the teller's user row");
        sqlx::query(
            "INSERT INTO queue_counters (organization_id, branch_id, teller_id, status) \
             SELECT organization_id, id, $2::uuid, 'ACTIVE' FROM branches WHERE id = $1 \
             ON CONFLICT (branch_id, teller_id) DO NOTHING",
        )
        .bind(rpc_common::MAIN)
        .bind(teller)
        .execute(pool)
        .await
        .expect("open the teller's counter");
    }
}

const QUEUE: &str = "kiosk_queue";

const MAIN: &str = "MAIN";
const B9: &str = "B9";
const B9_NAME: &str = "Nine";

/// The second front-desk employee: holds rows the first one does not.
const FRONT2: &str = "FRONT2";
/// A front-desk employee whose first password is unchanged: nobody on this path.
const MUST_CHANGE: &str = "FRONT_MUST_CHANGE";

const STAFF_REQUIRED: &str = "Staff access required.";
const INVALID_REQUEST: &str = "คำขอไม่ถูกต้อง";

/// The fifteen fields of a queue row (ticket §7) — none of `list`'s `paymentMethod`, `rate`,
/// `inputAmount`, `branchName`, `branch`, `created`.
const ROW_KEYS: [&str; 15] = [
    "id",
    "queue",
    "orderNo",
    "transactionNo",
    "status",
    "type",
    "currency",
    "foreign",
    "thb",
    "staff",
    "startedAt",
    "completedAt",
    "durationSeconds",
    "canAccept",
    "canContinue",
];
/// The fields whose values are compat `list`'s as they are.
const SAME_AS_LIST: [&str; 13] = [
    "id",
    "queue",
    "orderNo",
    "transactionNo",
    "status",
    "type",
    "currency",
    "staff",
    "startedAt",
    "completedAt",
    "durationSeconds",
    "canAccept",
    "canContinue",
];

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A request id in v2's shape; `n` makes it, so a test can name it again.
fn rid(n: u32) -> String {
    format!("{n:08x}-0000-4000-8000-0000000c{n:04x}")
}

fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("an object: {value}"))
        .keys()
        .cloned()
        .collect()
}

fn set_of(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

fn rows(data: &Value) -> &Vec<Value> {
    data.as_array()
        .unwrap_or_else(|| panic!("the queue's data is an array: {data}"))
}

fn ids(data: &Value) -> Vec<String> {
    rows(data)
        .iter()
        .map(|r| {
            r["id"]
                .as_str()
                .unwrap_or_else(|| panic!("a row's id: {r}"))
                .to_string()
        })
        .collect()
}

/// A compat money number as the view must print it: the same decimal, as a string with two
/// places. The number's own text is read as a decimal, never through `f64` arithmetic.
fn money_text(compat: &Value) -> Value {
    let n = compat
        .as_number()
        .unwrap_or_else(|| panic!("compat prints money as a JSON number: {compat}"));
    let d = Decimal::from_str(&n.to_string()).unwrap_or_else(|e| panic!("{n} is a decimal: {e}"));
    json!(format!("{d:.2}"))
}

/// A GET as a screen sends it: no body, no marker header.
fn get(path: &str) -> Call {
    let mut call = Call::post("http://desk.test", path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

fn path(view: &str, query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{view}")
    } else {
        format!("/api/v3/views/{view}?{query}")
    }
}

fn is_unchanged(body: &Value, version: i64, what: &str) {
    assert_eq!(
        body,
        &json!({ "version": version, "changed": false }),
        "{what}: exactly the version and changed: false, no data"
    );
}

/// A time as v2 printed it: `YYYY-MM-DDTHH:MM:SS.mmmZ`.
fn iso(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

// ── the rows ─────────────────────────────────────────────────────────────────

/// One `kiosk_requests` row put in behind the routes.
#[derive(Clone)]
struct Row {
    branch: &'static str,
    id: String,
    status: &'static str,
    created: String,
    staff: Option<String>,
    side: &'static str,
    currency: &'static str,
    foreign: &'static str,
    thb: &'static str,
    method: &'static str,
    started: Option<String>,
    completed: Option<String>,
    document: Option<String>,
}

impl Row {
    /// A WAITING BUY of USD in `MAIN`, id `rid(n)`, made `second` seconds after 04:00 of the
    /// test's day (the milliseconds from `n`).
    fn waiting(n: u32, second: u32) -> Row {
        Row {
            branch: MAIN,
            id: rid(n),
            status: "WAITING",
            created: format!(
                "2026-10-06T04:{:02}:{:02}.{:03}Z",
                second / 60,
                second % 60,
                n % 1000
            ),
            staff: None,
            side: "BUY",
            currency: "USD",
            foreign: "1234.50",
            thb: "39627.45",
            method: "Cash",
            started: None,
            completed: None,
            document: None,
        }
    }

    /// The request in `status`, held by the employee with id `staff`.
    fn held(mut self, status: &'static str, staff: &str) -> Row {
        self.status = status;
        self.staff = Some(staff.to_string());
        if status == "COMPLETED" {
            self.document = Some(format!("XC-20261006-{:06}", next()));
        }
        self
    }

    /// The request still a quote: never listed.
    fn quoted(mut self) -> Row {
        self.status = "QUOTED";
        self.staff = None;
        self
    }

    fn times(mut self, started: Option<&str>, completed: Option<&str>) -> Row {
        self.started = started.map(str::to_string);
        self.completed = completed.map(str::to_string);
        self
    }

    fn money(mut self, foreign: &'static str, thb: &'static str) -> Row {
        self.foreign = foreign;
        self.thb = thb;
        self
    }

    fn sell(mut self, currency: &'static str, method: &'static str) -> Row {
        self.side = "SELL";
        self.currency = currency;
        self.method = method;
        self
    }

    fn branch(mut self, branch: &'static str) -> Row {
        self.branch = branch;
        self
    }

    fn created(mut self, at: &str) -> Row {
        self.created = at.to_string();
        self
    }
}

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    counter: Counter,
    staff: Vec<Person>,
}

/// One employee of every staff role in `MAIN` (keys are the role names), a second FRONT
/// employee and a FRONT employee who must change the first password.
fn the_staff() -> Vec<Person> {
    let mut staff: Vec<Person> = Role::STAFF
        .iter()
        .enumerate()
        .map(|(i, role)| person(role.as_str(), role.as_str(), 0x31 + i as u8))
        .collect();
    staff.push(person(FRONT2, "FRONT", 0x41));
    let mut must_change = person(MUST_CHANGE, "FRONT", 0x42);
    must_change.must_change = true;
    staff.push(must_change);
    staff
}

impl Room {
    /// `MAIN` with the dollar at 32.1 / 32.6 and an open shift (`Counter::open`), and `B9`;
    /// the owner (key `owner`) and [`the_staff`] signed in.
    async fn open() -> Room {
        let staff = the_staff();
        let counter = Counter::open(&staff).await;
        open_counters(
            &counter.desk.db.pool,
            &[&staff
                .iter()
                .find(|p| p.key == "FRONT")
                .expect("FRONT is one of the staff")
                .id],
        )
        .await;
        counter.desk.db.seed_branch(B9, B9_NAME).await;
        Room { counter, staff }
    }

    fn id_of(&self, key: &str) -> String {
        self.staff
            .iter()
            .find(|p| p.key == key)
            .unwrap_or_else(|| panic!("{key} is one of the staff"))
            .id
            .clone()
    }

    fn username_of(&self, key: &str) -> String {
        self.staff
            .iter()
            .find(|p| p.key == key)
            .unwrap_or_else(|| panic!("{key} is one of the staff"))
            .username
            .clone()
    }

    /// The keys of everybody signed in, the owner first, then nobody (`""`).
    fn callers(&self) -> Vec<String> {
        let mut callers = vec!["owner".to_string()];
        callers.extend(self.staff.iter().map(|p| p.key.clone()));
        callers.push(String::new());
        callers
    }

    /// The actor `Policy` sees for a caller key on the path `Kiosk`.
    fn actor_of(&self, key: &str) -> Option<Actor> {
        match key {
            "" | MUST_CHANGE => None,
            "owner" => Some(Actor::owner("owner")),
            key => {
                let p = self
                    .staff
                    .iter()
                    .find(|p| p.key == key)
                    .unwrap_or_else(|| panic!("{key} is one of the staff"));
                Actor::staff(
                    p.id.clone(),
                    Role::parse(&p.role).expect("a staff role"),
                    BranchId::new(MAIN),
                )
            }
        }
    }

    async fn put(&self, row: &Row) {
        sqlx::query(
            "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, \
             currency, rate, foreign_amount, thb, input_amount, payment_method, submission_id, \
             document_ref) \
             VALUES ($1, $2, $3, $4::timestamptz, $5, $6, $7, 32.1, $8::numeric, $9::numeric, \
             $8::numeric, $10, $11, $11)",
        )
        .bind(row.branch)
        .bind(&row.id)
        .bind(row.status)
        .bind(&row.created)
        .bind(&row.staff)
        .bind(row.side)
        .bind(row.currency)
        .bind(row.foreign)
        .bind(row.thb)
        .bind(row.method)
        .bind(&row.document)
        .execute(&self.counter.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("put kiosk request {}: {e}", row.id));

        if row.started.is_some() || row.completed.is_some() {
            sqlx::query(
                "INSERT INTO queue_tickets (id, organization_id, branch_id, service_date, service_type, sequence, display_number, status, kiosk_request_id, counter_teller_id, issued_at, served_at, completed_at, idempotency_key) \
                 VALUES (gen_random_uuid(), (SELECT organization_id FROM branches WHERE id = $1), $1, current_date, 'WALK_IN', (SELECT COALESCE(MAX(sequence), 0) + 1 FROM queue_tickets WHERE branch_id = $1), 'A001', 'SERVING', $2, NULL, now(), $3::timestamptz, $4::timestamptz, $5)",
            )
            .bind(row.branch)
            .bind(&row.id)
            .bind(&row.started)
            .bind(&row.completed)
            .bind(format!("key-{}", row.id))
            .execute(&self.counter.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("put queue ticket for {}: {e}", row.id));
        }
    }

    async fn put_all(&self, rows: &[Row]) {
        for row in rows {
            self.put(row).await;
        }
    }

    /// `count` WAITING rows of `MAIN`, ids `rid(first..)`, one second apart, the last newest.
    async fn put_waiting(&self, first: u32, count: u32) -> Vec<String> {
        let mut made = Vec::new();
        for i in 0..count {
            let row = Row::waiting(first + i, 0).created(&format!(
                "2026-10-06T03:{:02}:{:02}.000Z",
                i / 60,
                i % 60
            ));
            self.put(&row).await;
            made.push(row.id);
        }
        made
    }

    async fn version(&self, branch: &str) -> i64 {
        BranchVersions::new(self.counter.desk.db.pool.clone())
            .current(&BranchId::new(branch))
            .await
            .expect("read the branch version")
    }

    /// `GET path` as `who` (`""` = nobody).
    async fn get(&self, who: &str, path: &str) -> Answer {
        let call = get(path);
        let call = if who.is_empty() {
            call
        } else {
            call.cookie(
                self.counter
                    .cookies
                    .get(who)
                    .unwrap_or_else(|| panic!("{who} is not signed in")),
            )
        };
        self.counter.app.send(call).await
    }

    /// A view as `who`, which must be answered 200 with JSON, never cached.
    async fn view(&self, who: &str, view: &str, query: &str) -> Value {
        let p = path(view, query);
        let answer = self.get(who, &p).await;
        assert_eq!(
            answer.status, 200,
            "GET {p} as {who:?}: {:.400}",
            answer.text
        );
        assert_eq!(
            answer.header("cache-control"),
            Some("no-store"),
            "GET {p}: never cached"
        );
        answer.json()
    }

    /// `data` of a view read with no `since`.
    async fn data(&self, who: &str, view: &str, query: &str) -> Value {
        let body = self.view(who, view, query).await;
        assert_eq!(
            body["changed"],
            json!(true),
            "GET {} as {who:?}: {body}",
            path(view, query)
        );
        body.get("data")
            .cloned()
            .unwrap_or_else(|| panic!("GET {}: data in {body}", path(view, query)))
    }

    /// One compat kiosk action as `who` (`"owner"` names `branch`; staff are on their own).
    async fn compat(&self, who: &str, action: &str, mut payload: Value, branch: &str) -> Value {
        if who == "owner" {
            payload["branchId"] = json!(branch);
        }
        let answer = self.counter.ask(Some(who), action, payload.clone()).await;
        assert_eq!(
            answer.status, 200,
            "compat {action} as {who} with {payload}: {:.400}",
            answer.text
        );
        answer.json()
    }

    /// Compat `list` page 0 for `state`: its rows.
    async fn compat_list(&self, who: &str, state: &str, branch: &str) -> Vec<Value> {
        let list = self
            .compat(who, "list", json!({ "filter": state, "page": 0 }), branch)
            .await;
        assert_eq!(list["hasMore"], json!(false), "one compat page holds them");
        list["requests"]
            .as_array()
            .unwrap_or_else(|| panic!("compat list's requests: {list}"))
            .clone()
    }

    fn advance(&self, seconds: i64) {
        self.counter.desk.clock.advance(Duration::seconds(seconds));
    }

    async fn finish(self) {
        self.counter.finish().await;
    }
}

/// Every view row against compat `list`'s row of the same id: the thirteen fields as they are,
/// `foreign` and `thb` as decimal strings of the same value, and the same order.
fn assert_rows_are_the_list_rows(view: &Value, list: &[Value], what: &str) {
    let view_rows = rows(view);
    let list_ids: Vec<&Value> = list.iter().map(|r| &r["id"]).collect();
    let view_ids: Vec<&Value> = view_rows.iter().map(|r| &r["id"]).collect();
    assert_eq!(
        view_ids, list_ids,
        "{what}: the same rows in the same order"
    );
    for (row, compat) in view_rows.iter().zip(list) {
        assert_eq!(
            keys(row),
            set_of(&ROW_KEYS),
            "{what}: the row's fields: {row}"
        );
        for key in SAME_AS_LIST {
            assert_eq!(
                row[key], compat[key],
                "{what}: {key} of {} — view {row}, compat {compat}",
                row["id"]
            );
        }
        for key in ["foreign", "thb"] {
            assert_eq!(
                row[key],
                money_text(&compat[key]),
                "{what}: {key} of {} is the decimal string of compat's {} (W7)",
                row["id"],
                compat[key]
            );
        }
    }
}

/// The rows the read tests share, in `MAIN`: three WAITING, two ACCEPTED (FRONT's and
/// FRONT2's), two ID_CHECKED (FRONT's and RATES's), two COMPLETED (FRONT's and BACKEND's), one
/// QUOTED, each with its own `created_at`; money with cents; one SELL by PromptPay.
fn the_day(room: &Room) -> Vec<Row> {
    let front = room.id_of("FRONT");
    let front2 = room.id_of(FRONT2);
    let rates = room.id_of("RATES");
    let backend = room.id_of("BACKEND");
    vec![
        Row::waiting(1, 1),
        Row::waiting(2, 2)
            .money("50.25", "1607.01")
            .sell("EUR", "PromptPay"),
        Row::waiting(3, 3).money("0.01", "0.32"),
        Row::waiting(4, 4)
            .held("ACCEPTED", &front)
            .times(Some("2026-10-06T04:10:00.123Z"), None),
        Row::waiting(5, 5)
            .held("ACCEPTED", &front2)
            .times(Some("2026-10-06T04:11:00.000Z"), None),
        Row::waiting(6, 6)
            .held("ID_CHECKED", &front)
            .times(Some("2026-10-06T04:12:00.500Z"), None),
        Row::waiting(7, 7)
            .held("ID_CHECKED", &rates)
            .times(Some("2026-10-06T04:13:00.000Z"), None),
        Row::waiting(8, 8).held("COMPLETED", &front).times(
            Some("2026-10-06T04:14:00.000Z"),
            Some("2026-10-06T04:16:30.999Z"),
        ),
        Row::waiting(9, 9)
            .held("COMPLETED", &backend)
            .times(
                Some("2026-10-06T04:15:00.000Z"),
                Some("2026-10-06T04:15:07.000Z"),
            )
            .money("99999999.99", "3209999999.68"),
        Row::waiting(10, 10).quoted(),
    ]
}

// ── who may read ─────────────────────────────────────────────────────────────

/// OR-4b (#767, D-4, design §2 and §02 "Server ตรวจ … branch scope"): an employee who names a
/// branch outside their scope is refused 403 `outOfScope`, where v3 answered their own branch.
fn assert_out_of_scope(answer: &rpc_common::Answer, label: &str) {
    assert_eq!(answer.status, 403, "{label}: {:.300}", answer.text);
    assert_eq!(
        answer.json()["error"]["code"],
        serde_json::json!("outOfScope"),
        "{label}: {:.300}",
        answer.text
    );
}

/// §8: nobody signed in — and an employee who must change the first password, who is nobody
/// on this path — gets the `kioskStaff` refusal from both views, whatever the query.
#[actix_web::test]
async fn nobody_signed_in_is_refused_kiosk_staff_on_both_views() {
    let room = Room::open().await;
    let refusal = json!({ "error": { "code": "kioskStaff", "message": STAFF_REQUIRED } });
    for who in ["", MUST_CHANGE] {
        for (view, query) in [
            (QUEUE, ""),
            (QUEUE, "branch=B9&state=working"),
            (QUEUE, "state=all"),
            (QUEUE, "limit=51&before=x"),
        ] {
            let answer = room.get(who, &path(view, query)).await;
            assert_eq!(
                answer.status, 400,
                "{view}?{query} as {who:?}: {:.300}",
                answer.text
            );
            assert_eq!(answer.json(), refusal, "{view}?{query} as {who:?}");
        }
    }
    room.finish().await;
}

/// W8: both views answer exactly whom `Policy::allow` lets call `notifications` and `list` on
/// the path `Kiosk` — the owner, every staff role — and refuse everybody else with the
/// policy's refusal. No role row of their own.
#[actix_web::test]
async fn both_views_answer_exactly_whom_the_kiosk_rows_allow() {
    let room = Room::open().await;
    let mut allowed = 0;
    for who in room.callers() {
        let actor = room.actor_of(&who);
        for (view, action) in [(QUEUE, "list")] {
            let policy = Policy::allow(actor.as_ref(), EntryPath::Kiosk, &Action::new(action));
            let answer = room.get(&who, &path(view, "")).await;
            match policy {
                Ok(()) => {
                    allowed += 1;
                    assert_eq!(
                        answer.status, 200,
                        "{view} as {who:?}: {:.300}",
                        answer.text
                    );
                    assert_eq!(answer.json()["changed"], json!(true), "{view} as {who:?}");
                }
                Err(e) => {
                    assert_eq!(
                        answer.status,
                        e.status(),
                        "{view} as {who:?}: {:.300}",
                        answer.text
                    );
                    assert_eq!(
                        answer.json()["error"]["message"],
                        json!(e.to_string()),
                        "{view} as {who:?}"
                    );
                }
            }
        }
    }
    // the owner, the nine roles and the second FRONT employee
    assert_eq!(allowed, 11, "every signed-in caller reads the queue view");
    room.finish().await;
}

// ── the queue's rows ─────────────────────────────────────────────────────────

/// §7, W7: a row has exactly the fifteen fields, and `foreign` / `thb` are decimal strings
/// with two places — never JSON numbers.
#[actix_web::test]
async fn a_queue_row_has_exactly_the_tickets_fields_and_money_as_decimal_strings() {
    let room = Room::open().await;
    room.put_all(&the_day(&room)).await;
    let waiting = room.data("FRONT", QUEUE, "state=waiting").await;
    let by_id = |id: &str| {
        rows(&waiting)
            .iter()
            .find(|r| r["id"] == json!(id))
            .unwrap_or_else(|| panic!("{id} among {waiting}"))
            .clone()
    };
    for row in rows(&waiting) {
        assert_eq!(keys(row), set_of(&ROW_KEYS), "{row}");
    }
    let one = by_id(&rid(1));
    assert_eq!(one["foreign"], json!("1234.50"), "{one}");
    assert_eq!(one["thb"], json!("39627.45"), "{one}");
    assert_eq!(one["type"], json!("BUY"), "{one}");
    assert_eq!(one["currency"], json!("USD"), "{one}");
    let two = by_id(&rid(2));
    assert_eq!(two["foreign"], json!("50.25"), "{two}");
    assert_eq!(two["thb"], json!("1607.01"), "{two}");
    assert_eq!(two["type"], json!("SELL"), "{two}");
    assert_eq!(two["currency"], json!("EUR"), "{two}");
    let three = by_id(&rid(3));
    assert_eq!(three["foreign"], json!("0.01"), "{three}");
    assert_eq!(three["thb"], json!("0.32"), "{three}");

    let completed = room.data("FRONT", QUEUE, "state=completed").await;
    let big = rows(&completed)
        .iter()
        .find(|r| r["id"] == json!(rid(9)))
        .unwrap_or_else(|| panic!("{} among {completed}", rid(9)));
    assert_eq!(big["foreign"], json!("99999999.99"), "{big}");
    assert_eq!(big["thb"], json!("3209999999.68"), "{big}");
    room.finish().await;
}

/// §7: for every state and every kind of caller, the rows are compat `list`'s rows for the
/// same caller — the same ids in the same order, the same `queue`, `orderNo`, `transactionNo`,
/// `status`, `type`, `currency`, `staff` (a username), times, `canAccept`, `canContinue` —
/// with money as decimal strings.
#[actix_web::test]
async fn queue_rows_are_the_compat_list_rows_with_money_as_decimal_strings() {
    let room = Room::open().await;
    room.put_all(&the_day(&room)).await;
    for who in [
        "owner", "FRONT", FRONT2, "BACKEND", "MANAGER", "RATES", "FINANCE",
    ] {
        for state in ["waiting", "working", "completed"] {
            let view = room
                .data(who, QUEUE, &format!("state={state}&limit=100"))
                .await;
            let list = room.compat_list(who, state, MAIN).await;
            assert!(!list.is_empty(), "the day has {state} rows");
            assert_rows_are_the_list_rows(&view, &list, &format!("{state} as {who}"));
        }
    }
    room.finish().await;
}

/// §12.18.2: `waiting` lists WAITING; `working` lists ACCEPTED and ID_CHECKED; `completed`
/// lists COMPLETED; a QUOTED request is in none of them.
#[actix_web::test]
async fn each_state_lists_only_its_statuses_and_quoted_is_never_listed() {
    let room = Room::open().await;
    room.put_all(&the_day(&room)).await;
    for (state, want) in [
        ("waiting", vec![rid(3), rid(2), rid(1)]),
        ("working", vec![rid(7), rid(6), rid(5), rid(4)]),
        ("completed", vec![rid(9), rid(8)]),
    ] {
        let data = room.data("owner", QUEUE, &format!("state={state}")).await;
        assert_eq!(ids(&data), want, "state={state}: {data}");
        assert!(
            !ids(&data).contains(&rid(10)),
            "state={state}: the QUOTED request is not listed"
        );
    }
    room.finish().await;
}

/// §12.18.2 (a v3 change): with no `state` the queue lists the waiting rows — not v2's `all`.
#[actix_web::test]
async fn no_state_means_waiting() {
    let room = Room::open().await;
    room.put_all(&the_day(&room)).await;
    for query in ["", "branch=MAIN", "limit=51"] {
        let data = room.data("FRONT", QUEUE, query).await;
        assert_eq!(
            ids(&data),
            vec![rid(3), rid(2), rid(1)],
            "{query:?}: the waiting rows only: {data}"
        );
    }
    room.finish().await;
}

/// §8: `state=all`, an empty value, another case and any other word are refused
/// `invalidRequest` naming `state`, with v2's sentence.
#[actix_web::test]
async fn state_all_or_any_other_value_is_refused_naming_the_field() {
    let room = Room::open().await;
    room.put_all(&the_day(&room)).await;
    let refusal = json!({ "error": {
        "code": "invalidRequest", "field": "state", "message": INVALID_REQUEST } });
    for query in [
        "state=all",
        "state=",
        "state=WAITING",
        "state=Working",
        "state=quoted",
        "state=done",
        "state=waiting%20",
        "branch=MAIN&state=all&limit=51",
    ] {
        for who in ["owner", "FRONT", "RATES"] {
            let answer = room.get(who, &path(QUEUE, query)).await;
            assert_eq!(answer.status, 400, "{query} as {who}: {:.300}", answer.text);
            assert_eq!(answer.json(), refusal, "{query} as {who}");
        }
    }
    room.finish().await;
}

// ── order and paging ─────────────────────────────────────────────────────────

/// §12.18.2: newest first by `created_at`; two rows made at the same instant (on purpose) are
/// ordered by `id`, the greater first — as `queue_page` orders them.
#[actix_web::test]
async fn rows_are_newest_first_with_id_breaking_a_tie() {
    let room = Room::open().await;
    let same = "2026-10-06T04:30:00.000Z";
    room.put_all(&[
        Row::waiting(0x21, 0).created("2026-10-06T04:29:59.999Z"),
        Row::waiting(0x24, 0).created(same),
        Row::waiting(0x22, 0).created(same),
        Row::waiting(0x23, 0).created(same),
        Row::waiting(0x20, 0).created("2026-10-06T04:30:00.001Z"),
    ])
    .await;
    let data = room.data("FRONT", QUEUE, "state=waiting").await;
    assert_eq!(
        ids(&data),
        vec![rid(0x20), rid(0x24), rid(0x23), rid(0x22), rid(0x21)],
        "{data}"
    );
    let list = room.compat_list("FRONT", "waiting", MAIN).await;
    assert_rows_are_the_list_rows(&data, &list, "the same order as compat list");
    room.finish().await;
}

/// §12.18.2: `before` is a row's id and the page starts strictly after that row in the order
/// — also when the boundary falls between two rows made at the same instant, and when the row
/// named has since left the state (it was accepted while the panel paged).
#[actix_web::test]
async fn before_starts_the_page_strictly_after_that_row() {
    let room = Room::open().await;
    let same = "2026-10-06T04:30:00.000Z";
    let front = room.id_of("FRONT");
    room.put_all(&[
        Row::waiting(0x31, 0).created("2026-10-06T04:31:00.000Z"),
        Row::waiting(0x34, 0).created(same),
        Row::waiting(0x33, 0).created(same),
        Row::waiting(0x32, 0).created(same),
        Row::waiting(0x35, 0).created("2026-10-06T04:29:00.000Z"),
        Row::waiting(0x36, 0).created("2026-10-06T04:28:00.000Z"),
        // accepted after the panel read it: no longer waiting, still a position in the order
        Row::waiting(0x37, 0)
            .created("2026-10-06T04:29:30.000Z")
            .held("ACCEPTED", &front),
    ])
    .await;
    let all = vec![
        rid(0x31),
        rid(0x34),
        rid(0x33),
        rid(0x32),
        rid(0x35),
        rid(0x36),
    ];
    assert_eq!(
        ids(&room.data("FRONT", QUEUE, "state=waiting").await),
        all,
        "the whole waiting queue"
    );

    // walk it two at a time: every row once, in order
    let mut walked: Vec<String> = Vec::new();
    let mut before: Option<String> = None;
    for _ in 0..5 {
        let query = match &before {
            None => "state=waiting&limit=2".to_string(),
            Some(id) => format!("state=waiting&limit=2&before={id}"),
        };
        let page = ids(&room.data("FRONT", QUEUE, &query).await);
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= 2, "{query}: {page:?}");
        before = page.last().cloned();
        walked.extend(page);
    }
    assert_eq!(
        walked, all,
        "pages of two, each strictly after the last row read"
    );

    // the boundary inside the tie: after 0x34 come 0x33 and 0x32, not 0x35
    assert_eq!(
        ids(&room
            .data(
                "FRONT",
                QUEUE,
                &format!("state=waiting&before={}", rid(0x34))
            )
            .await),
        vec![rid(0x33), rid(0x32), rid(0x35), rid(0x36)],
        "after a row of the tie: the rest of the tie, then older rows"
    );
    // a row named that is now ACCEPTED still positions the waiting page
    assert_eq!(
        ids(&room
            .data(
                "FRONT",
                QUEUE,
                &format!("state=waiting&before={}", rid(0x37))
            )
            .await),
        vec![rid(0x35), rid(0x36)],
        "after a row that left the state"
    );
    // the oldest row: nothing after it
    assert_eq!(
        ids(&room
            .data(
                "FRONT",
                QUEUE,
                &format!("state=waiting&before={}", rid(0x36))
            )
            .await),
        Vec::<String>::new(),
        "after the oldest row"
    );
    room.finish().await;
}

/// §9: a `before` that names no row of the branch read — an id of another branch, an id
/// nobody made — answers an empty page, never the branch's first page.
#[actix_web::test]
async fn before_naming_no_row_of_the_branch_gives_an_empty_page() {
    let room = Room::open().await;
    room.put_all(&[
        Row::waiting(0x41, 0).created("2026-10-06T04:00:00.000Z"),
        Row::waiting(0x42, 0).created("2026-10-06T04:01:00.000Z"),
        // B9's newest row: if it positioned MAIN's page, both MAIN rows would follow it
        Row::waiting(0x43, 0)
            .created("2026-10-06T05:00:00.000Z")
            .branch(B9),
    ])
    .await;
    for (who, query) in [
        ("owner", format!("state=waiting&before={}", rid(0x43))),
        (
            "owner",
            format!("branch=MAIN&state=waiting&before={}", rid(0x43)),
        ),
        ("FRONT", format!("state=waiting&before={}", rid(0x43))),
        (
            "FRONT",
            "state=waiting&before=00000000-0000-4000-8000-00000000ffff".to_string(),
        ),
        ("FRONT", "before=nobody-made-this".to_string()),
    ] {
        let data = room.data(who, QUEUE, &query).await;
        assert_eq!(data, json!([]), "{query} as {who}");
    }
    // the same id read in its own branch is a position there
    assert_eq!(
        room.data("owner", QUEUE, &format!("branch=B9&before={}", rid(0x43)))
            .await,
        json!([]),
        "B9 has nothing older"
    );
    room.finish().await;
}

/// §12.18.2, §4.2: a page holds at most `limit` rows — the panel asks 51 to show 50 and know
/// that a next page exists — and 50 when no limit is given; `limit` ≤ 100.
#[actix_web::test]
async fn limit_bounds_the_page_and_the_panel_reads_fifty_one() {
    let room = Room::open().await;
    let made = room.put_waiting(0x100, 103).await;
    let newest_first: Vec<String> = made.iter().rev().cloned().collect();

    let first = ids(&room.data("FRONT", QUEUE, "state=waiting&limit=51").await);
    assert_eq!(
        first,
        newest_first[..51].to_vec(),
        "limit=51: the 51 newest"
    );
    // the panel shows 50 and asks for the next page after its 50th row
    let second = ids(&room
        .data(
            "FRONT",
            QUEUE,
            &format!("state=waiting&limit=51&before={}", first[49]),
        )
        .await);
    assert_eq!(second, newest_first[50..101].to_vec(), "the next 51");
    let third = ids(&room
        .data(
            "FRONT",
            QUEUE,
            &format!("state=waiting&limit=51&before={}", second[49]),
        )
        .await);
    assert_eq!(third, newest_first[100..].to_vec(), "the last three");

    for (query, want) in [
        ("", 50),
        ("limit=1", 1),
        ("limit=50", 50),
        ("limit=100", 100),
    ] {
        let page = ids(&room.data("FRONT", QUEUE, query).await);
        assert_eq!(page, newest_first[..want].to_vec(), "{query:?}");
    }
    for query in ["limit=0", "limit=101", "limit=x"] {
        let answer = room.get("FRONT", &path(QUEUE, query)).await;
        assert_eq!(answer.status, 400, "{query}: {:.300}", answer.text);
        assert_eq!(
            answer.json()["error"]["code"],
            json!("invalidRequest"),
            "{query}"
        );
    }
    room.finish().await;
}

/// §9: a row with no work times (made before Q1's migration) answers `startedAt`,
/// `completedAt` and `durationSeconds` as `null`; a row with them answers v2's times and the
/// whole seconds between them, as compat `list` does.
#[actix_web::test]
async fn a_row_without_work_times_answers_null_times() {
    let room = Room::open().await;
    let front = room.id_of("FRONT");
    room.put_all(&[
        Row::waiting(0x51, 1).held("ACCEPTED", &front),
        Row::waiting(0x52, 2).held("COMPLETED", &front),
        Row::waiting(0x53, 3).held("COMPLETED", &front).times(
            Some("2026-10-06T04:20:00.250Z"),
            Some("2026-10-06T04:22:05.750Z"),
        ),
        Row::waiting(0x54, 4)
            .held("ID_CHECKED", &front)
            .times(Some("2026-10-06T04:21:00.000Z"), None),
    ])
    .await;
    let working = room.data("FRONT", QUEUE, "state=working").await;
    let completed = room.data("FRONT", QUEUE, "state=completed").await;
    let find = |data: &Value, n: u32| {
        rows(data)
            .iter()
            .find(|r| r["id"] == json!(rid(n)))
            .unwrap_or_else(|| panic!("{} in {data}", rid(n)))
            .clone()
    };
    for (row, what) in [
        (find(&working, 0x51), "an ACCEPTED row with no times"),
        (find(&completed, 0x52), "a COMPLETED row with no times"),
    ] {
        assert_eq!(row["startedAt"], Value::Null, "{what}: {row}");
        assert_eq!(row["completedAt"], Value::Null, "{what}: {row}");
        assert_eq!(row["durationSeconds"], Value::Null, "{what}: {row}");
    }
    let timed = find(&completed, 0x53);
    assert_eq!(
        timed["startedAt"],
        json!("2026-10-06T04:20:00.250Z"),
        "{timed}"
    );
    assert_eq!(
        timed["completedAt"],
        json!("2026-10-06T04:22:05.750Z"),
        "{timed}"
    );
    assert_eq!(timed["durationSeconds"], json!(125), "{timed}");
    let started = find(&working, 0x54);
    assert_eq!(
        started["startedAt"],
        json!("2026-10-06T04:21:00.000Z"),
        "{started}"
    );
    assert_eq!(started["completedAt"], Value::Null, "{started}");
    assert_eq!(started["durationSeconds"], Value::Null, "{started}");

    for (state, data) in [("working", &working), ("completed", &completed)] {
        let list = room.compat_list("FRONT", state, MAIN).await;
        assert_rows_are_the_list_rows(data, &list, state);
    }
    room.finish().await;
}

// ── who may claim ────────────────────────────────────────────────────────────

/// §9: a caller whose role may not claim (RATES, and the other roles `accept` refuses) sees
/// every row, but `canAccept` and `canContinue` are false on all of them — also on a row the
/// caller holds.
#[actix_web::test]
async fn a_rates_caller_sees_the_rows_but_may_claim_none() {
    let room = Room::open().await;
    room.put_all(&the_day(&room)).await;
    for who in [
        "RATES",
        "MONITOR",
        "FINANCE",
        "SYSTEM",
        "DOCUMENTS",
        "DOCUMENT_REVIEW",
    ] {
        assert!(
            Policy::allow(
                room.actor_of(who).as_ref(),
                EntryPath::Kiosk,
                &Action::new("accept")
            )
            .is_err(),
            "{who} may not claim"
        );
        let mut seen = 0;
        for state in ["waiting", "working", "completed"] {
            let data = room.data(who, QUEUE, &format!("state={state}")).await;
            for row in rows(&data) {
                seen += 1;
                assert_eq!(row["canAccept"], json!(false), "{who}: {row}");
                assert_eq!(row["canContinue"], json!(false), "{who}: {row}");
            }
        }
        assert_eq!(seen, 9, "{who} sees every listed row");
    }
    // RATES holds rid(7), ID_CHECKED: still no continuing
    let working = room.data("RATES", QUEUE, "state=working").await;
    let held = rows(&working)
        .iter()
        .find(|r| r["id"] == json!(rid(7)))
        .expect("RATES's row");
    assert_eq!(held["staff"], json!(room.username_of("RATES")), "{held}");
    assert_eq!(held["canContinue"], json!(false), "{held}");
    room.finish().await;
}

/// §7: a caller whose role may claim may accept every WAITING row, and continue only the
/// ACCEPTED / ID_CHECKED rows it holds — the holder compared by who holds it, printed by
/// username.
#[actix_web::test]
async fn a_claiming_caller_may_accept_waiting_rows_and_continue_only_its_own() {
    let room = Room::open().await;
    room.put_all(&the_day(&room)).await;
    let front = room.username_of("FRONT");
    let front2 = room.username_of(FRONT2);

    let waiting = room.data("FRONT", QUEUE, "state=waiting").await;
    for row in rows(&waiting) {
        assert_eq!(row["canAccept"], json!(true), "{row}");
        assert_eq!(row["canContinue"], json!(false), "{row}");
        assert_eq!(row["staff"], json!(""), "nobody holds a waiting row: {row}");
    }

    let continuing = |data: &Value| -> Vec<String> {
        rows(data)
            .iter()
            .filter(|r| r["canContinue"] == json!(true))
            .map(|r| r["id"].as_str().expect("an id").to_string())
            .collect()
    };
    let as_front = room.data("FRONT", QUEUE, "state=working").await;
    assert_eq!(continuing(&as_front), vec![rid(6), rid(4)], "{as_front}");
    let as_front2 = room.data(FRONT2, QUEUE, "state=working").await;
    assert_eq!(continuing(&as_front2), vec![rid(5)], "{as_front2}");
    let as_owner = room.data("owner", QUEUE, "state=working").await;
    assert_eq!(continuing(&as_owner), Vec::<String>::new(), "{as_owner}");
    for data in [&as_front, &as_front2, &as_owner] {
        for row in rows(data) {
            assert_eq!(row["canAccept"], json!(false), "not WAITING: {row}");
        }
        let staff: Vec<&Value> = rows(data).iter().map(|r| &r["staff"]).collect();
        assert_eq!(
            staff,
            vec![
                &json!(room.username_of("RATES")),
                &json!(front),
                &json!(front2),
                &json!(front)
            ],
            "the holders by username"
        );
    }
    let completed = room.data("FRONT", QUEUE, "state=completed").await;
    for row in rows(&completed) {
        assert_eq!(row["canAccept"], json!(false), "{row}");
        assert_eq!(row["canContinue"], json!(false), "{row}");
    }
    room.finish().await;
}

// ── branches ─────────────────────────────────────────────────────────────────

/// §9: staff naming another branch are answered for their own (`Policy::branch`); the owner
/// is answered for the branch named, and for `MAIN` when none is.
#[actix_web::test]
async fn staff_naming_another_branch_are_answered_for_their_own() {
    let room = Room::open().await;
    room.put_all(&[
        Row::waiting(0x61, 1),
        Row::waiting(0x62, 2).branch(B9),
        Row::waiting(0x63, 3).branch(B9),
    ])
    .await;
    // OR-4b (D-4): staff of MAIN naming B9 are refused; naming MAIN they read MAIN's
    for who in ["FRONT", "RATES"] {
        for view in [QUEUE] {
            assert_out_of_scope(
                &room.get(who, &path(view, "branch=B9")).await,
                &format!("{who} naming B9 on {view}"),
            );
        }
    }
    for (who, query, want) in [
        ("FRONT", "branch=MAIN", vec![rid(0x61)]),
        ("RATES", "branch=MAIN", vec![rid(0x61)]),
        ("FRONT", "", vec![rid(0x61)]),
        ("owner", "branch=B9", vec![rid(0x63), rid(0x62)]),
        ("owner", "", vec![rid(0x61)]),
    ] {
        let queue = room.data(who, QUEUE, query).await;
        assert_eq!(ids(&queue), want, "{QUEUE}?{query} as {who}");
    }
    room.finish().await;
}

// ── since ────────────────────────────────────────────────────────────────────

/// §4.2: `since` equal to the branch's version answers `{version, changed: false}` and loads
/// nothing — both views answer it with every table the views read renamed away, while a read
/// without `since` (or a queue page with `before`) then fails. An older `since` answers data.
#[actix_web::test]
async fn since_at_the_current_version_answers_unchanged_without_loading() {
    let room = Room::open().await;
    room.put_all(&the_day(&room)).await;
    let version = room.version(MAIN).await;
    for (view, query) in [(QUEUE, "state=working"), (QUEUE, "")] {
        let body = room.view("FRONT", view, query).await;
        assert_eq!(body["version"], json!(version), "{view}?{query}: {body}");
        assert_eq!(body["changed"], json!(true), "{view}?{query}");
        let sep = if query.is_empty() { "" } else { "&" };
        let same = room
            .view("FRONT", view, &format!("{query}{sep}since={version}"))
            .await;
        is_unchanged(&same, version, &format!("{view}?{query}"));
        let older = room
            .view("FRONT", view, &format!("{query}{sep}since={}", version - 1))
            .await;
        assert_eq!(
            older["changed"],
            json!(true),
            "{view}: an older since: {older}"
        );
        assert!(older.get("data").is_some(), "{view}: {older}");
    }
    // a queue page with `before` is never answered from `since`
    let paged = room
        .view(
            "FRONT",
            QUEUE,
            &format!("state=waiting&before={}&since={version}", rid(3)),
        )
        .await;
    assert_eq!(paged["changed"], json!(true), "{paged}");
    assert_eq!(ids(&paged["data"]), vec![rid(2), rid(1)], "{paged}");

    for table in ["kiosk_requests"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE {table} RENAME TO hidden_{table}"
        )))
        .execute(&room.counter.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("rename {table}: {e}"));
    }
    for view in [QUEUE] {
        let same = room.view("FRONT", view, &format!("since={version}")).await;
        is_unchanged(&same, version, &format!("{view} with its tables gone"));
        let loaded = room.get("FRONT", &path(view, "")).await;
        assert_eq!(
            loaded.status, 500,
            "{view}: a load now fails, so the unchanged answer loaded nothing: {:.300}",
            loaded.text
        );
    }
    let paged = room
        .get(
            "FRONT",
            &path(QUEUE, &format!("before={}&since={version}", rid(3))),
        )
        .await;
    assert_eq!(
        paged.status, 500,
        "a page with before loads: {:.300}",
        paged.text
    );
    for table in ["kiosk_requests"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE hidden_{table} RENAME TO {table}"
        )))
        .execute(&room.counter.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("rename {table} back: {e}"));
    }
    room.finish().await;
}

// ── the five writers of kiosk_requests ───────────────────────────────────────

/// The writers, in the order a request meets them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Writer {
    Quote,
    Confirm,
    Accept,
    IdentityChecked,
    Save,
}

const WALKED: &str = "0000beef-0000-4000-8000-00000000a001";

/// A walk of one kiosk request through the routes, up to and including `last`. Before the
/// write of `last`, the version `v0` is read and both views must answer `since=v0` with
/// `changed: false`; after it, the version must have risen and both views must answer that
/// same `since` with `changed: true` and data.
async fn walk_to(last: Writer) {
    let room = Room::open().await;
    let c = &room.counter;
    let mut customer = String::new();
    let mut passport = String::new();
    for writer in [
        Writer::Quote,
        Writer::Confirm,
        Writer::Accept,
        Writer::IdentityChecked,
        Writer::Save,
    ] {
        if writer > last {
            break;
        }
        if writer == Writer::Save {
            // what a save needs, before the version under test is read
            let reserved = room
                .compat_rpc(
                    "reserveCustomer",
                    json!({ "name": "Customer 1", "passportNo": "P1000001",
                            "nationality": "DEU", "birthDate": "1990-01-02",
                            "passportExpiry": "2031-06-11", "mode": "LIVE" }),
                )
                .await;
            customer = reserved["customerId"]
                .as_str()
                .expect("a customer number")
                .to_string();
            passport = room.upload_passport().await;
        }
        room.advance(5);
        let v0 = room.version(MAIN).await;
        if writer == last {
            for view in [QUEUE] {
                let before = room.view("FRONT", view, &format!("since={v0}")).await;
                is_unchanged(&before, v0, &format!("{view} before {writer:?}"));
            }
        }
        let answer = match writer {
            Writer::Quote => {
                c.hands.ids.push(WALKED);
                c.ask(
                    None,
                    "quote",
                    json!({ "type": "BUY", "currency": "USD", "amount": 100 }),
                )
                .await
            }
            Writer::Confirm => c.ask(None, "confirm", json!({ "id": WALKED })).await,
            Writer::Accept => {
                c.ask(Some("FRONT"), "accept", json!({ "id": WALKED }))
                    .await
            }
            Writer::IdentityChecked => {
                c.ask(
                    Some("FRONT"),
                    "identityChecked",
                    json!({ "id": WALKED, "confirmed": true }),
                )
                .await
            }
            Writer::Save => {
                let now = c.now();
                let call = Call::action(
                    &c.origin,
                    "/api/staff/rpc",
                    "saveTransaction",
                    json!({
                        "name": "Customer 1", "passportNo": "P1000001", "nationality": "DEU",
                        "birthDate": "1990-01-02", "passportExpiry": "2031-06-11",
                        "mode": "LIVE", "submissionId": "00000000-0000-4000-8000-00000000b001",
                        "customerId": customer, "type": "BUY", "currency": "USD",
                        "amount": 100, "rate": 32.1, "fee": 0, "bankFee": 0,
                        "method": "Cash", "ref": "", "account": "Cash Drawer",
                        "staff": room.username_of("FRONT"), "booking": "WALK-IN",
                        "notes": "", "valueDate": now.format("%Y-%m-%d").to_string(),
                        "paidAt": iso(now), "paymentConfirmed": true,
                        "identityConfirmed": true, "countConfirmed": true,
                        "passport": { "uploadKey": passport }, "kioskRequestId": WALKED,
                    }),
                )
                .cookie(&c.cookies["FRONT"]);
                c.app.send(call).await
            }
        };
        assert_eq!(answer.status, 200, "{writer:?}: {:.400}", answer.text);
        if writer != last {
            continue;
        }
        let v1 = room.version(MAIN).await;
        assert!(v1 > v0, "{writer:?} moved MAIN's version: {v0} → {v1}");
        let mut seen_in = Vec::new();
        for state in ["waiting", "working", "completed"] {
            let queue = room
                .view("FRONT", QUEUE, &format!("state={state}&since={v0}"))
                .await;
            assert_eq!(
                queue["changed"],
                json!(true),
                "{state} after {writer:?}: {queue}"
            );
            assert_eq!(queue["version"], json!(v1), "{state} after {writer:?}");
            let found = rows(&queue["data"])
                .iter()
                .find(|r| r["id"] == json!(WALKED))
                .cloned();
            if let Some(row) = found {
                seen_in.push((state, row));
            }
            let same = room
                .view("FRONT", QUEUE, &format!("state={state}&since={v1}"))
                .await;
            is_unchanged(&same, v1, &format!("{state} at the new version"));
        }

        // what the write left, as the views show it
        match writer {
            Writer::Quote => {
                assert!(
                    seen_in.is_empty(),
                    "a QUOTED request is not listed: {seen_in:?}"
                );
            }
            Writer::Confirm => {
                assert_eq!(seen_in.len(), 1, "{seen_in:?}");
                assert_eq!(seen_in[0].0, "waiting");
                assert_eq!(seen_in[0].1["canAccept"], json!(true), "{seen_in:?}");
                assert_eq!(seen_in[0].1["foreign"], json!("100.00"), "{seen_in:?}");
                assert_eq!(seen_in[0].1["thb"], json!("3210.00"), "{seen_in:?}");
            }
            Writer::Accept | Writer::IdentityChecked => {
                assert_eq!(seen_in.len(), 1, "{seen_in:?}");
                assert_eq!(seen_in[0].0, "working");
                let status = if writer == Writer::Accept {
                    "ACCEPTED"
                } else {
                    "ID_CHECKED"
                };
                assert_eq!(seen_in[0].1["status"], json!(status), "{seen_in:?}");
                assert_eq!(seen_in[0].1["canContinue"], json!(true), "{seen_in:?}");
                assert_eq!(
                    seen_in[0].1["staff"],
                    json!(room.username_of("FRONT")),
                    "{seen_in:?}"
                );
            }
            Writer::Save => {
                assert_eq!(seen_in.len(), 1, "{seen_in:?}");
                assert_eq!(seen_in[0].0, "completed");
                assert_eq!(seen_in[0].1["status"], json!("COMPLETED"), "{seen_in:?}");
                assert!(
                    seen_in[0].1["transactionNo"].is_string(),
                    "linked to the exchange: {seen_in:?}"
                );
            }
        }
    }
    room.finish().await;
}

impl Room {
    /// One `/api/staff/rpc` action as FRONT.
    async fn compat_rpc(&self, action: &str, payload: Value) -> Value {
        let c = &self.counter;
        let call =
            Call::action(&c.origin, "/api/staff/rpc", action, payload).cookie(&c.cookies["FRONT"]);
        let answer = c.app.send(call).await;
        assert_eq!(answer.status, 200, "{action}: {:.400}", answer.text);
        answer.json()
    }

    /// A passport picture uploaded by FRONT: its key.
    async fn upload_passport(&self) -> String {
        const PNG: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, b'I', b'H', b'D', b'R',
        ];
        let c = &self.counter;
        let call = Call::post(&c.origin, "/api/upload?branch=MAIN", PNG)
            .header("content-type", "image/png")
            .cookie(&c.cookies["FRONT"]);
        let answer = c.app.send(call).await;
        assert_eq!(answer.status, 200, "the passport: {:.300}", answer.text);
        answer.json()["uploadKey"]
            .as_str()
            .expect("an upload key")
            .to_string()
    }
}

/// §4.2 writer 1: the customer's `quote` (`customer.rs:49`).
#[actix_web::test]
async fn a_customer_quote_moves_both_views() {
    walk_to(Writer::Quote).await;
}

/// §4.2 writer 2: the customer's `confirm` (`customer.rs:97`).
#[actix_web::test]
async fn a_customer_confirm_moves_both_views() {
    walk_to(Writer::Confirm).await;
}

/// §4.2 writer 3: the desk's `accept` (`desk.rs:78`).
#[actix_web::test]
async fn a_desk_accept_moves_both_views() {
    walk_to(Writer::Accept).await;
}

/// §4.2 writer 4: the desk's `identityChecked` (`desk.rs:153`).
#[actix_web::test]
async fn a_desk_identity_check_moves_both_views() {
    walk_to(Writer::IdentityChecked).await;
}

/// §4.2 writer 5: the save's `complete_in` (`save_hooks.rs:34`), through a LIVE
/// `saveTransaction` naming the request.
#[actix_web::test]
async fn a_kiosk_save_moves_both_views() {
    walk_to(Writer::Save).await;
}

// ── the generated types ──────────────────────────────────────────────────────

/// A new empty directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "xc-v3-kq-{}-{nanos}-{}",
            std::process::id(),
            next()
        ));
        std::fs::create_dir_all(&dir).expect("create a scratch directory");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// §5.1, done criterion 4: both views are in the generated table, each exports its own file;
/// the queue's money is `Money` (a decimal string), `type` is `"BUY" | "SELL"`, its filter
/// offers exactly the three states, `serverNow` is a string, and `durationSeconds`, `count`
/// and `workingCount` are `number` — never `bigint` — as #486's `wire.ts` declares them.
/// Needs no database.
#[test]
fn the_generated_types_carry_money_as_strings_and_the_three_states() {
    let names: Vec<&str> = VIEWS.iter().map(|v| v.name).collect();
    assert!(
        names.contains(&QUEUE),
        "{QUEUE} in the generated views {names:?}"
    );
    let text_of = |name: &str| {
        let view = VIEWS
            .iter()
            .find(|v| v.name == name)
            .unwrap_or_else(|| panic!("{name}"));
        let scratch = Scratch::new();
        (view.export_types)(&scratch.0).unwrap_or_else(|e| panic!("export {name}: {e}"));
        std::fs::read_to_string(scratch.0.join(format!("{name}.ts")))
            .unwrap_or_else(|e| panic!("{name}.ts: {e}"))
    };
    let queue = text_of(QUEUE);
    for want in [
        "foreign: Money",
        "thb: Money",
        "\"waiting\"",
        "\"working\"",
        "\"completed\"",
        "\"BUY\"",
        "\"SELL\"",
        "durationSeconds: number | null",
    ] {
        assert!(queue.contains(want), "{QUEUE}.ts has {want}: {queue}");
    }
    for never in [
        "\"all\"",
        "\"QUOTED\"",
        "bigint",
        "foreign: number",
        "thb: number",
        "type: string",
    ] {
        assert!(!queue.contains(never), "{QUEUE}.ts has no {never}: {queue}");
    }
}
