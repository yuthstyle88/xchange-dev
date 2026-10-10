//! Ticket #464 (V2-transactions, `docs/design/xchange-v3-web.md` §4.2, §5.1, §12.1, §12.2): the
//! paged view the admin's transaction list reads instead of every exchange of the branch in
//! `adminSnapshot`. Written before the implementation by the lane controller. Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! - `GET /api/v3/views/admin_transactions` — a `PagedView` inheriting `(EntryPath::Rpc,
//!   "adminSnapshot")`. Rows are the shared `TransactionRow` (`dto/v3/transaction_row.rs`, V2-parts
//!   #476), the LIVE exchanges of the branch only, newest first (by when they were recorded,
//!   v2's `created`, `admin-html:58`), keyset paged by `before` (a row's `id`; the page starts
//!   strictly after that row), at most `limit` (≤ 100).
//! - Filter (§12.1 table, every key [required]), applied in SQL before the limit: `from`, `to`
//!   (the value date, both inclusive, `''`/absent = open, `Period::check`); `q` over id, customer
//!   name and currency, a literal case-folded substring (`lower()`, `%` `_` `\` escaped, no NFKC),
//!   at most 100 characters; `type` = `BUY` | `SELL`; `status` = `COMPLETED` | `CANCELLED` |
//!   `PENDING` (v3 stores an exchange only once it is saved, `snapshot.rs:14-16`, so every row is
//!   `COMPLETED`); `staff` (exact) and `staffQ` (case-folded substring) of who served. A value
//!   that does not parse is refused naming its key.
//! - W7, the controller's note on #464: every figure is a decimal string the **server** computes.
//!   One test per figure: `principal` (amount × rate, half up to the satang), `settlement`
//!   (principal − fee buying, + fee selling, `admin-html:55`), `referenceValue` (amount ×
//!   reference rate to the satang), `referenceSpread` (reference − principal buying, principal −
//!   reference selling) and `referencePercent` (spread ÷ reference × 100 to six places, half away
//!   from zero — v2's `toFixed(6)`, `admin-html:36`, without its float error); the rest of the row
//!   is the saved exchange as recorded (`TransactionRow::from_record`, design §12.2). Rates are
//!   the 8 places the database holds, with the row's `unitSize` (#781, rates.md §3.3, §4.9 R1;
//!   the parts show them through `formatRate`), money with two.
//! - §4.2: every writer of what the view reads (`transactions`, `transaction_files`) goes through
//!   `BranchTx` — one write per writer, then the same `since` answers `changed: true`. Writers: the
//!   exchange save (the workflow, the desk's `saveTransaction` on `/api/staff/rpc`, and the same
//!   save completing a kiosk request, which runs the kiosk save hooks `save_hooks.rs` inside it),
//!   and `adminAbandonRequest` (named by the ticket).
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real
//! sessions made at the real sign-in doors (`kiosk_http_common::Counter`). Exchanges whose figures
//! are asserted are saved through the real `SaveTransactionWorkflow`; rows the list tests need are
//! put in `transactions` behind the routes, each with its own `recorded_at` (where three share
//! one on purpose, the test says so and asserts no order between them). The writer tests use the
//! routes and move the test's clock on before every write.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`) for the view tests; nothing skips.

mod common;
mod compat_common;
mod kiosk_http_common;
mod rpc_common;
mod snapshot_common;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use actix_web::http::Method;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use compat_common::Person;
use kiosk_http_common::{Counter, person};
use rpc_common::{Answer, Call};
use serde_json::{Value, json};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::application::customer::service::CustomerService;
use xc_exchange::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xchange::presentation::http::v3::VIEWS;

const VIEW: &str = "admin_transactions";

const MAIN: &str = "MAIN";
const B9: &str = "B9";
const B9_NAME: &str = "Nine";

const INVALID_REQUEST: &str = "คำขอไม่ถูกต้อง";
const PERIOD_ORDER: &str = "วันที่เริ่มต้นต้องไม่เกินวันที่สิ้นสุด";

/// `TransactionRow`'s fields (design §12.2), and nothing else.
const ROW_FIELDS: [&str; 26] = [
    "id",
    "date",
    "created",
    "type",
    "currency",
    "amount",
    "rate",
    "unitSize",
    "principal",
    "fee",
    "bankFee",
    "settlement",
    "method",
    "customer",
    "customerId",
    "staff",
    "status",
    "notes",
    "cancelReason",
    "shiftId",
    "referenceRate",
    "referenceValue",
    "referenceSpread",
    "referencePercent",
    "paymentRef",
    "slipUrl",
];

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];
const PDF: &[u8] = b"%PDF-1.7 a slip\n";

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A request id in v2's shape, different on every call.
fn request_id() -> String {
    format!("00000000-0000-4000-8000-{:012x}", 0x464_0000_0000 + next())
}

fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("an object: {value}"))
        .keys()
        .cloned()
        .collect()
}

fn rows(data: &Value) -> &Vec<Value> {
    data.as_array()
        .unwrap_or_else(|| panic!("a page is an array: {data}"))
}

fn ids(data: &Value) -> Vec<String> {
    rows(data)
        .iter()
        .map(|r| {
            r["id"]
                .as_str()
                .unwrap_or_else(|| panic!("a row id: {r}"))
                .to_string()
        })
        .collect()
}

fn iso(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// A GET as the screen sends it: no body, no marker header.
fn get(path: &str) -> Call {
    let mut call = Call::post(kiosk_http_common::ORIGIN, path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

fn path(query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{VIEW}")
    } else {
        format!("/api/v3/views/{VIEW}?{query}")
    }
}

/// A query value as a URL carries it.
fn enc(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn is_unchanged(body: &Value, version: i64, what: &str) {
    assert_eq!(body["changed"], json!(false), "{what}: {body}");
    assert_eq!(body["version"], json!(version), "{what}: {body}");
    assert!(body.get("data").is_none(), "{what}: no data in {body}");
}

fn assert_refused(answer: &Answer, code: &str, field: &str, message: &str, what: &str) {
    assert_eq!(answer.status, 400, "{what}: {:.300}", answer.text);
    let body = answer.json();
    assert_eq!(body["error"]["code"], json!(code), "{what}: {body}");
    assert_eq!(body["error"]["field"], json!(field), "{what}: {body}");
    assert_eq!(body["error"]["message"], json!(message), "{what}: {body}");
    assert!(body.get("data").is_none(), "{what}: no data in {body}");
}

// ── rows put behind the routes ───────────────────────────────────────────────

/// One exchange as `transactions` stores it, put there directly for the list tests. The
/// figures are consistent (`principal` = amount × rate; `paid` / `received` as the save writes
/// them) but no test of this kind asserts a figure.
#[derive(Clone)]
struct Tx {
    id: String,
    branch: &'static str,
    mode: &'static str,
    side: &'static str,
    currency: &'static str,
    customer: String,
    staff: String,
    notes: String,
    date: String,
    recorded: String,
}

impl Tx {
    /// A LIVE purchase of 100 USD in `MAIN`, recorded `2026-10-06T03:00:<second>Z`.
    fn new(n: u32, second: u32) -> Tx {
        Tx {
            id: format!("XC-20261006-9{n:05}"),
            branch: MAIN,
            mode: "LIVE",
            side: "BUY",
            currency: "USD",
            customer: "Somchai Jaidee".to_string(),
            staff: "front".to_string(),
            notes: String::new(),
            date: "2026-10-06".to_string(),
            recorded: format!("2026-10-06T03:{:02}:{:02}.000Z", second / 60, second % 60),
        }
    }

    fn sell(mut self) -> Tx {
        self.side = "SELL";
        self
    }

    fn currency(mut self, code: &'static str) -> Tx {
        self.currency = code;
        self
    }

    fn customer(mut self, name: &str) -> Tx {
        self.customer = name.to_string();
        self
    }

    fn staff(mut self, name: &str) -> Tx {
        self.staff = name.to_string();
        self
    }

    fn notes(mut self, notes: &str) -> Tx {
        self.notes = notes.to_string();
        self
    }

    fn date(mut self, date: &str) -> Tx {
        self.date = date.to_string();
        self
    }

    fn recorded(mut self, at: &str) -> Tx {
        self.recorded = at.to_string();
        self
    }

    fn branch(mut self, branch: &'static str) -> Tx {
        self.branch = branch;
        self
    }

    fn demo(mut self) -> Tx {
        self.mode = "DEMO";
        self.id = self.id.replacen("XC-", "TEST-XC-", 1);
        self
    }
}

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    counter: Counter,
    staff: Vec<Person>,
}

/// One employee of every staff role in `MAIN`; keys are the role names.
fn the_staff() -> Vec<Person> {
    Role::STAFF
        .iter()
        .enumerate()
        .map(|(i, role)| person(role.as_str(), role.as_str(), 0x51 + i as u8))
        .collect()
}

/// A saved exchange's choices; everything else is fixed.
struct Save {
    side: &'static str,
    amount: &'static str,
    fee: &'static str,
    bank_fee: &'static str,
    reference: Option<&'static str>,
    transfer: bool,
    notes: &'static str,
    customer: &'static str,
    date: &'static str,
}

impl Save {
    fn buy(amount: &'static str) -> Save {
        Save {
            side: "BUY",
            amount,
            fee: "0",
            bank_fee: "0",
            reference: None,
            transfer: false,
            notes: "",
            customer: "Somchai Jaidee",
            date: "2026-10-06",
        }
    }

    fn sell(amount: &'static str) -> Save {
        Save {
            side: "SELL",
            ..Save::buy(amount)
        }
    }

    fn fee(mut self, fee: &'static str) -> Save {
        self.fee = fee;
        self
    }

    fn bank_fee(mut self, bank_fee: &'static str) -> Save {
        self.bank_fee = bank_fee;
        self
    }

    fn reference(mut self, rate: &'static str) -> Save {
        self.reference = Some(rate);
        self
    }

    fn transfer(mut self) -> Save {
        self.transfer = true;
        self
    }

    fn notes(mut self, notes: &'static str) -> Save {
        self.notes = notes;
        self
    }

    fn customer(mut self, name: &'static str) -> Save {
        self.customer = name;
        self
    }

    fn date(mut self, date: &'static str) -> Save {
        self.date = date;
        self
    }
}

/// What a save left behind.
struct Saved {
    id: String,
    customer_id: String,
    recorded: DateTime<Utc>,
    slip: Option<String>,
}

impl Room {
    /// `MAIN` with the dollar at 32.1 / 32.6 and an open shift (`Counter::open`); `B9` with an
    /// open shift; the owner (key `owner`) and [`the_staff`] signed in.
    async fn open() -> Room {
        let staff = the_staff();
        let counter = Counter::open(&staff).await;
        counter.desk.db.seed_branch(B9, B9_NAME).await;
        counter
            .owner_rpc(
                "adminOpenShift",
                json!({ "branchId": B9, "requestId": request_id(),
                        "opening": { "THB": 1000, "USD": 10 } }),
            )
            .await;
        Room { counter, staff }
    }

    fn username_of(&self, key: &str) -> String {
        self.staff
            .iter()
            .find(|p| p.key == key)
            .unwrap_or_else(|| panic!("{key} is one of the staff"))
            .username
            .clone()
    }

    /// The caller keys: the owner, every staff role, and nobody (`""`).
    fn callers(&self) -> Vec<String> {
        let mut callers = vec!["owner".to_string()];
        callers.extend(self.staff.iter().map(|p| p.key.clone()));
        callers.push(String::new());
        callers
    }

    /// The actor `Policy` sees for a caller key on the path `Rpc`.
    fn actor_of(&self, key: &str) -> Option<Actor> {
        match key {
            "" => None,
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

    fn advance(&self, seconds: i64) -> DateTime<Utc> {
        self.counter.desk.clock.advance(Duration::seconds(seconds));
        self.counter.now()
    }

    async fn version(&self, branch: &str) -> i64 {
        BranchVersions::new(self.counter.desk.db.pool.clone())
            .current(&BranchId::new(branch))
            .await
            .expect("read the branch version")
    }

    async fn shift_of(&self, branch: &str) -> String {
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = $1 AND status = 'OPEN'")
            .bind(branch)
            .fetch_one(&self.counter.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("the open shift of {branch}: {e}"))
    }

    fn person(name: &str) -> ReserveCustomer {
        ReserveCustomer {
            name: name.to_owned(),
            passport_no: "AA1234567".to_owned(),
            nationality: "THA".to_owned(),
            birth_date: String::new(),
            passport_expiry: String::new(),
            mode: "LIVE".to_owned(),
        }
    }

    async fn reserve(&self, branch: &str, name: &str, at: DateTime<Utc>) -> String {
        CustomerService::new(self.counter.desk.db.branch_tx())
            .reserve(&BranchId::new(branch), Self::person(name), at)
            .await
            .unwrap_or_else(|e| panic!("reserve {name} in {branch}: {e:?}"))
            .customer_id
    }

    /// Puts `txs` in `transactions` behind the routes, in the order given.
    async fn put_all(&self, txs: &[Tx]) {
        let at = self.counter.now();
        let main_customer = self.reserve(MAIN, "Somchai Jaidee", at).await;
        let b9_customer = self.reserve(B9, "Somchai Jaidee", at).await;
        let main_shift = self.shift_of(MAIN).await;
        let b9_shift = self.shift_of(B9).await;
        for tx in txs {
            let (customer, shift) = if tx.branch == MAIN {
                (&main_customer, &main_shift)
            } else {
                (&b9_customer, &b9_shift)
            };
            let shift = (tx.mode == "LIVE").then_some(shift.as_str());
            let (rate, principal) = if tx.side == "BUY" {
                ("32.1", "3210.00")
            } else {
                ("32.6", "3260.00")
            };
            let (paid, received) = if tx.side == "BUY" {
                (principal, "0")
            } else {
                ("0", principal)
            };
            sqlx::query(
                "INSERT INTO transactions (document_ref, branch_id, mode, shift_id, submission_id, \
                 payload_hash, customer_id, customer_name, nationality, type, code, amount, rate, \
                 reference_rate, principal, fee, bank_fee, received, paid, method, payment_ref, \
                 account, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
                 recorded_by) \
                 VALUES ($1, $2, $3, $4, $1, 'hash', $5, $6, 'THA', $7, $8, 100, $9::numeric, NULL, \
                 $10::numeric, 0, 0, $11::numeric, $12::numeric, 'Cash', '', 'Cash drawer', $13, \
                 'WALK-IN', $14, $15::date, $16::timestamptz, $16::timestamptz, 'owner')",
            )
            .bind(&tx.id)
            .bind(tx.branch)
            .bind(tx.mode)
            .bind(shift)
            .bind(customer)
            .bind(&tx.customer)
            .bind(tx.side)
            .bind(tx.currency)
            .bind(rate)
            .bind(principal)
            .bind(received)
            .bind(paid)
            .bind(&tx.staff)
            .bind(&tx.notes)
            .bind(&tx.date)
            .bind(&tx.recorded)
            .execute(&self.counter.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("put exchange {}: {e}", tx.id));
        }
    }

    /// One LIVE exchange of USD in `MAIN` saved through the real workflow behind the routes, at
    /// the counter rate (32.1 buying, 32.6 selling), by `EMP-464`, valued 2026-10-06 unless [`Save::date`] says otherwise.
    async fn save(&self, s: Save) -> Saved {
        let t = self.advance(7);
        let here = BranchId::new(MAIN);
        let customer_id = self.reserve(MAIN, s.customer, t).await;
        let passport = self
            .counter
            .desk
            .files
            .upload(&here, "owner", "image/jpeg", JPEG, t)
            .await
            .expect("upload a passport");
        let slip = if s.transfer {
            Some(
                self.counter
                    .desk
                    .files
                    .upload(&here, "owner", "application/pdf", PDF, t)
                    .await
                    .expect("upload a slip"),
            )
        } else {
            None
        };
        let cmd = SaveTransaction {
            customer: Self::person(s.customer),
            submission_id: format!("submission-464-{:012}", next()),
            customer_id: customer_id.clone(),
            side: s.side.to_owned(),
            currency: "USD".to_owned(),
            amount: s.amount.to_owned(),
            rate: if s.side == "BUY" { "32.1" } else { "32.6" }.to_owned(),
            fee: s.fee.to_owned(),
            bank_fee: s.bank_fee.to_owned(),
            method: if s.transfer { "Bank Transfer" } else { "Cash" }.to_owned(),
            payment_ref: if s.transfer { "TRX-464" } else { "" }.to_owned(),
            account: "Cash drawer 1".to_owned(),
            staff: "EMP-464".to_owned(),
            booking: "WALK-IN".to_owned(),
            notes: s.notes.to_owned(),
            value_date: s.date.to_owned(),
            paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
            payment_confirmed: true,
            identity_confirmed: true,
            passport: Some(passport),
            slip: slip.clone(),
            reference_rate: s.reference.map(str::to_owned),
        };
        let saved = self
            .counter
            .desk
            .workflow
            .save(&here, cmd, "owner", t)
            .await
            .unwrap_or_else(|e| panic!("save an exchange: {e:?}"));
        Saved {
            id: saved.document_ref,
            customer_id,
            recorded: t,
            slip,
        }
    }

    /// `GET path` as `who` (`""` = nobody).
    async fn get(&self, who: &str, p: &str) -> Answer {
        let call = get(p);
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

    /// The view as `who`, which must be answered 200 with JSON, never cached.
    async fn view(&self, who: &str, query: &str) -> Value {
        let p = path(query);
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

    /// `data` of the view read by the owner with no `since`.
    async fn data(&self, query: &str) -> Value {
        self.data_as("owner", query).await
    }

    async fn data_as(&self, who: &str, query: &str) -> Value {
        let body = self.view(who, query).await;
        assert_eq!(
            body["changed"],
            json!(true),
            "GET {} as {who:?}: {body}",
            path(query)
        );
        body.get("data")
            .cloned()
            .unwrap_or_else(|| panic!("GET {}: data in {body}", path(query)))
    }

    /// The row of `id` in `MAIN`'s first page.
    async fn row(&self, id: &str) -> Value {
        let data = self.data(&format!("branch={MAIN}&limit=100")).await;
        rows(&data)
            .iter()
            .find(|r| r["id"] == json!(id))
            .cloned()
            .unwrap_or_else(|| panic!("{id} in {data}"))
    }

    async fn finish(self) {
        self.counter.finish().await;
    }
}

// ── who may read it (W8) ─────────────────────────────────────────────────────

/// §4.4: the view asks `Policy::allow(actor, Rpc, adminSnapshot)` and nothing else, so every
/// caller kind gets exactly what that pair gives: 200 with data, or the policy's refusal.
#[actix_web::test]
async fn admin_transactions_answers_exactly_whom_the_snapshot_row_allows() {
    let room = Room::open().await;
    room.put_all(&[Tx::new(1, 1)]).await;
    let mut allowed = 0;
    let mut refused = 0;
    for who in room.callers() {
        let actor = room.actor_of(&who);
        let policy = Policy::allow(
            actor.as_ref(),
            EntryPath::Rpc,
            &Action::new("adminSnapshot"),
        );
        let answer = room.get(&who, &path("")).await;
        let what = format!("{VIEW} as {who:?}");
        match &policy {
            Ok(()) => {
                allowed += 1;
                assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
                let body = answer.json();
                assert_eq!(body["changed"], json!(true), "{what}: {body}");
                assert!(body["data"].is_array(), "{what}: a page in {body}");
            }
            Err(e) => {
                refused += 1;
                assert_eq!(answer.status, e.status(), "{what}: {:.300}", answer.text);
                let body = answer.json();
                assert_eq!(body["error"]["message"], json!(e.to_string()), "{what}");
                assert!(body.get("data").is_none(), "{what}: no data");
            }
        }
    }
    assert!(
        allowed > 0 && refused > 0,
        "{allowed} allowed, {refused} refused"
    );
    room.finish().await;
}

/// `Policy::branch`: staff are answered for their own branch whatever they name; the owner for
/// the branch named. No row of another branch is ever in a page.
#[actix_web::test]
async fn each_branch_lists_its_own_exchanges_and_staff_read_their_own_branch() {
    let room = Room::open().await;
    room.put_all(&[
        Tx::new(1, 1),
        Tx::new(2, 2).branch(B9),
        Tx::new(3, 3),
        Tx::new(4, 4).branch(B9),
    ])
    .await;
    let main = ids(&room.data(&format!("branch={MAIN}")).await);
    let nine = ids(&room.data(&format!("branch={B9}")).await);
    assert_eq!(main, ["XC-20261006-900003", "XC-20261006-900001"]);
    assert_eq!(nine, ["XC-20261006-900004", "XC-20261006-900002"]);
    let allowed_staff: Vec<String> = room
        .staff
        .iter()
        .map(|p| p.key.clone())
        .filter(|key| {
            Policy::allow(
                room.actor_of(key).as_ref(),
                EntryPath::Rpc,
                &Action::new("adminSnapshot"),
            )
            .is_ok()
        })
        .collect();
    for key in &allowed_staff {
        let named = ids(&room.data_as(key, &format!("branch={B9}")).await);
        assert_eq!(named, main, "{key} naming {B9} is answered for {MAIN}");
    }
    room.finish().await;
}

// ── the row (design §12.2, W7) ───────────────────────────────────────────────

/// The row is `TransactionRow`: its 26 fields and no other.
#[actix_web::test]
async fn a_row_has_exactly_the_transaction_row_fields() {
    let room = Room::open().await;
    let saved = room.save(Save::buy("100")).await;
    let row = room.row(&saved.id).await;
    let want: BTreeSet<String> = ROW_FIELDS.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(keys(&row), want, "{row}");
    room.finish().await;
}

/// `from_record`: what the save recorded, as v2 showed it. `created` is the time the exchange
/// was recorded (ISO, milliseconds, `Z`); `date` the value date; `staff` who the save names;
/// `status` `COMPLETED`; `shiftId` the open shift; nothing v2 shows as `—` is an empty string.
#[actix_web::test]
async fn a_row_is_the_saved_exchange_as_recorded() {
    let room = Room::open().await;
    // valued two days before it is recorded: `date` is the value date, not the day of `created`
    let cash = room
        .save(Save::buy("100").bank_fee("1.50").date("2026-10-04"))
        .await;
    let transfer = room
        .save(
            Save::buy("200")
                .transfer()
                .notes("ลูกค้าประจำ")
                .customer("Mary Jones"),
        )
        .await;
    let shift = room.shift_of(MAIN).await;

    let row = room.row(&cash.id).await;
    let want = json!({
        "id": cash.id,
        "date": "2026-10-04",
        "created": iso(cash.recorded),
        "type": "BUY",
        "currency": "USD",
        "amount": "100.00",
        "rate": "32.10000000",
        "unitSize": "1",
        "principal": "3210.00",
        "fee": "0.00",
        "bankFee": "1.50",
        "settlement": "3210.00",
        "method": "Cash",
        "customer": "Somchai Jaidee",
        "customerId": cash.customer_id,
        "staff": "EMP-464",
        "status": "COMPLETED",
        "notes": null,
        "cancelReason": null,
        "shiftId": shift,
        "referenceRate": null,
        "referenceValue": null,
        "referenceSpread": null,
        "referencePercent": null,
        "paymentRef": null,
        "slipUrl": null,
    });
    assert_eq!(row, want, "the cash purchase");

    let row = room.row(&transfer.id).await;
    let slip = transfer.slip.as_deref().expect("a slip was uploaded");
    assert_eq!(row["method"], json!("Bank Transfer"), "{row}");
    assert_eq!(row["paymentRef"], json!("TRX-464"), "{row}");
    assert_eq!(row["slipUrl"], json!(format!("/files/{slip}")), "{row}");
    assert_eq!(row["notes"], json!("ลูกค้าประจำ"), "{row}");
    assert_eq!(row["customer"], json!("Mary Jones"), "{row}");
    assert_eq!(row["customerId"], json!(transfer.customer_id), "{row}");
    assert_eq!(row["created"], json!(iso(transfer.recorded)), "{row}");
    assert_eq!(row["amount"], json!("200.00"), "{row}");
    assert_eq!(row["principal"], json!("6420.00"), "{row}");
    room.finish().await;
}

/// `principal` = amount × rate, half up to the satang, by the server: 10.05 × 32.1 = 322.605
/// → `322.61` (half to even and truncation both give 322.60).
#[actix_web::test]
async fn principal_is_amount_times_rate_half_up_to_the_satang() {
    let room = Room::open().await;
    let saved = room.save(Save::buy("10.05")).await;
    let row = room.row(&saved.id).await;
    assert_eq!(row["principal"], json!("322.61"), "{row}");
    assert_eq!(row["amount"], json!("10.05"), "{row}");
    assert_eq!(row["rate"], json!("32.10000000"), "{row}");
    room.finish().await;
}

/// `settlement` buying = principal − fee (`admin-html:55`): 322.61 − 5.25 = `317.36`.
#[actix_web::test]
async fn settlement_of_a_purchase_is_principal_less_fee() {
    let room = Room::open().await;
    let saved = room.save(Save::buy("10.05").fee("5.25")).await;
    let row = room.row(&saved.id).await;
    assert_eq!(row["settlement"], json!("317.36"), "{row}");
    assert_eq!(row["fee"], json!("5.25"), "{row}");
    assert_eq!(row["principal"], json!("322.61"), "{row}");
    room.finish().await;
}

/// `settlement` selling = principal + fee: 10.05 × 32.6 = 327.63, + 5.25 = `332.88`.
#[actix_web::test]
async fn settlement_of_a_sale_is_principal_plus_fee() {
    let room = Room::open().await;
    let saved = room.save(Save::sell("10.05").fee("5.25")).await;
    let row = room.row(&saved.id).await;
    assert_eq!(row["type"], json!("SELL"), "{row}");
    assert_eq!(row["rate"], json!("32.60000000"), "{row}");
    assert_eq!(row["principal"], json!("327.63"), "{row}");
    assert_eq!(row["settlement"], json!("332.88"), "{row}");
    room.finish().await;
}

/// `referenceValue` = amount × reference rate to the satang (`admin-html:36` `cents`):
/// 31.15 × 32.8731 = 1023.997065 → `1024.00` (truncation gives 1023.99); `referenceRate` is the
/// saved reference at the 8 places the database holds (#781).
#[actix_web::test]
async fn reference_value_is_amount_times_reference_rate_to_the_satang() {
    let room = Room::open().await;
    let saved = room.save(Save::buy("31.15").reference("32.8731")).await;
    let row = room.row(&saved.id).await;
    assert_eq!(row["referenceRate"], json!("32.87310000"), "{row}");
    assert_eq!(row["referenceValue"], json!("1024.00"), "{row}");
    room.finish().await;
}

/// `referenceSpread` (`admin-html:36`): buying, reference − principal (1024.00 − 999.92 =
/// `24.08`); selling, principal − reference (969.20 − 1024.00 = `-54.80`, signed).
#[actix_web::test]
async fn reference_spread_is_signed_from_the_shops_side() {
    let room = Room::open().await;
    let bought = room.save(Save::buy("31.15").reference("32.8731")).await;
    let sold = room.save(Save::sell("29.73").reference("34.4432")).await;
    let row = room.row(&bought.id).await;
    assert_eq!(row["principal"], json!("999.92"), "{row}");
    assert_eq!(row["referenceSpread"], json!("24.08"), "{row}");
    let row = room.row(&sold.id).await;
    assert_eq!(row["principal"], json!("969.20"), "{row}");
    assert_eq!(row["referenceValue"], json!("1024.00"), "{row}");
    assert_eq!(row["referenceSpread"], json!("-54.80"), "{row}");
    room.finish().await;
}

/// `referencePercent` = spread ÷ reference × 100 to six places, half away from zero: 24.08 ÷
/// 1024 × 100 = 2.3515625 exactly → `2.351563`; −54.80 ÷ 1024 × 100 = −5.3515625 → `-5.351563`.
/// (Half to even gives …562, truncation …562.) The web shows it as sent (#476 TransactionDetail).
#[actix_web::test]
async fn reference_percent_has_six_places_half_away_from_zero() {
    let room = Room::open().await;
    let bought = room.save(Save::buy("31.15").reference("32.8731")).await;
    let sold = room.save(Save::sell("29.73").reference("34.4432")).await;
    assert_eq!(
        room.row(&bought.id).await["referencePercent"],
        json!("2.351563")
    );
    assert_eq!(
        room.row(&sold.id).await["referencePercent"],
        json!("-5.351563")
    );
    room.finish().await;
}

/// No reference rate saved: the four reference figures are `null` (v2 shows no reference rows).
#[actix_web::test]
async fn no_reference_rate_means_no_reference_figures() {
    let room = Room::open().await;
    let saved = room.save(Save::sell("10").fee("1")).await;
    let row = room.row(&saved.id).await;
    for field in [
        "referenceRate",
        "referenceValue",
        "referenceSpread",
        "referencePercent",
    ] {
        assert_eq!(row[field], Value::Null, "{field} in {row}");
    }
    assert_eq!(row["settlement"], json!("327.00"), "{row}");
    room.finish().await;
}

// ── order and pages (§4.2) ───────────────────────────────────────────────────

/// Newest first by when the exchange was recorded (v2 sorts by `created`, `admin-html:58`), not
/// by its value date nor by its id.
#[actix_web::test]
async fn rows_are_newest_first_by_when_they_were_recorded() {
    let room = Room::open().await;
    room.put_all(&[
        Tx::new(3, 1).date("2026-10-06"),
        Tx::new(1, 2).date("2026-10-07"),
        Tx::new(2, 3).date("2026-10-05"),
    ])
    .await;
    let data = room.data(&format!("branch={MAIN}")).await;
    assert_eq!(
        ids(&data),
        [
            "XC-20261006-900002",
            "XC-20261006-900001",
            "XC-20261006-900003"
        ]
    );
    room.finish().await;
}

/// `limit` bounds a page; `before` starts the next page strictly after that row, until the
/// oldest: five rows read two at a time are 5 4 | 3 2 | 1 | (none).
#[actix_web::test]
async fn before_starts_the_page_strictly_after_that_row_and_limit_bounds_it() {
    let room = Room::open().await;
    let txs: Vec<Tx> = (1..=5).map(|n| Tx::new(n, n)).collect();
    room.put_all(&txs).await;
    let id = |n: u32| format!("XC-20261006-9{n:05}");
    let page = |before: Option<String>| match before {
        None => format!("branch={MAIN}&limit=2"),
        Some(b) => format!("branch={MAIN}&limit=2&before={b}"),
    };
    assert_eq!(ids(&room.data(&page(None)).await), [id(5), id(4)]);
    assert_eq!(ids(&room.data(&page(Some(id(4)))).await), [id(3), id(2)]);
    assert_eq!(ids(&room.data(&page(Some(id(2)))).await), [id(1)]);
    assert_eq!(
        ids(&room.data(&page(Some(id(1)))).await),
        Vec::<String>::new()
    );
    room.finish().await;
}

/// Exchanges recorded in the same millisecond (two desks at once): walked one row a page, each
/// is answered exactly once. Their order among themselves is not asserted.
#[actix_web::test]
async fn rows_recorded_at_the_same_instant_are_each_paged_exactly_once() {
    let room = Room::open().await;
    let same = "2026-10-06T04:00:00.000Z";
    room.put_all(&[
        Tx::new(1, 1),
        Tx::new(2, 0).recorded(same),
        Tx::new(3, 0).recorded(same),
        Tx::new(4, 0).recorded(same),
        Tx::new(5, 0).recorded("2026-10-06T05:00:00.000Z"),
    ])
    .await;
    let mut seen = Vec::new();
    let mut before: Option<String> = None;
    for _ in 0..10 {
        let q = match &before {
            None => format!("branch={MAIN}&limit=1"),
            Some(b) => format!("branch={MAIN}&limit=1&before={b}"),
        };
        let page = ids(&room.data(&q).await);
        assert!(page.len() <= 1, "limit=1: {page:?}");
        let Some(last) = page.last().cloned() else {
            break;
        };
        seen.push(last.clone());
        before = Some(last);
    }
    assert_eq!(seen.len(), 5, "every exchange once: {seen:?}");
    assert_eq!(seen.first().map(String::as_str), Some("XC-20261006-900005"));
    assert_eq!(seen.last().map(String::as_str), Some("XC-20261006-900001"));
    let unique: BTreeSet<&String> = seen.iter().collect();
    assert_eq!(unique.len(), 5, "no exchange twice: {seen:?}");
    room.finish().await;
}

/// `before` naming no exchange of the branch (unknown, or another branch's) answers an empty
/// page, never the other branch's rows (precedent: `kiosk_queue`, #485).
#[actix_web::test]
async fn before_naming_no_row_of_the_branch_gives_an_empty_page() {
    let room = Room::open().await;
    room.put_all(&[Tx::new(1, 1), Tx::new(2, 2).branch(B9)])
        .await;
    for before in ["XC-20991231-000001", "XC-20261006-900002"] {
        let data = room.data(&format!("branch={MAIN}&before={before}")).await;
        assert_eq!(ids(&data), Vec::<String>::new(), "before={before}");
    }
    room.finish().await;
}

/// `limit` ≤ 100 (§4.2): 100 is answered, 101 refused.
#[actix_web::test]
async fn a_limit_above_one_hundred_is_refused() {
    let room = Room::open().await;
    room.put_all(&[Tx::new(1, 1)]).await;
    room.data(&format!("branch={MAIN}&limit=100")).await;
    let answer = room
        .get("owner", &path(&format!("branch={MAIN}&limit=101")))
        .await;
    assert_eq!(answer.status, 400, "{:.300}", answer.text);
    room.finish().await;
}

/// The view is LIVE only (design §12.5): a DEMO exchange of the branch is never listed.
#[actix_web::test]
async fn demo_exchanges_are_never_listed() {
    let room = Room::open().await;
    room.put_all(&[Tx::new(1, 1), Tx::new(2, 2).demo(), Tx::new(3, 3)])
        .await;
    let data = room.data(&format!("branch={MAIN}")).await;
    assert_eq!(ids(&data), ["XC-20261006-900003", "XC-20261006-900001"]);
    room.finish().await;
}

// ── the filter (§12.1), in SQL ───────────────────────────────────────────────

/// `from` / `to` are the value date, both inclusive; `''` or absent is open on that side
/// (`admin-html:57`).
#[actix_web::test]
async fn from_and_to_are_inclusive_value_dates_open_when_empty() {
    let room = Room::open().await;
    room.put_all(&[
        Tx::new(4, 1).date("2026-10-04"),
        Tx::new(5, 2).date("2026-10-05"),
        Tx::new(6, 3).date("2026-10-06"),
    ])
    .await;
    let id = |n: u32| format!("XC-20261006-9{n:05}");
    for (query, want) in [
        ("from=2026-10-05&to=2026-10-05", vec![id(5)]),
        ("from=2026-10-05&to=2026-10-06", vec![id(6), id(5)]),
        ("from=2026-10-05", vec![id(6), id(5)]),
        ("to=2026-10-05", vec![id(5), id(4)]),
        ("from=&to=2026-10-04", vec![id(4)]),
        ("from=&to=", vec![id(6), id(5), id(4)]),
        ("from=2026-10-07", vec![]),
    ] {
        let data = room.data(&format!("branch={MAIN}&{query}")).await;
        assert_eq!(ids(&data), want, "{query}");
    }
    room.finish().await;
}

/// `Period::check`: `from` after `to` is `periodOrder` naming `to`; a date that is not
/// `YYYY-MM-DD` is `invalidRequest` naming it.
#[actix_web::test]
async fn a_bad_period_is_refused_naming_the_field() {
    let room = Room::open().await;
    let answer = room
        .get(
            "owner",
            &path(&format!("branch={MAIN}&from=2026-10-06&to=2026-10-05")),
        )
        .await;
    assert_refused(&answer, "periodOrder", "to", PERIOD_ORDER, "from > to");
    for (key, value) in [("from", "06/10/2026"), ("to", "2026-1-5")] {
        let answer = room
            .get(
                "owner",
                &path(&format!("branch={MAIN}&{key}={}", enc(value))),
            )
            .await;
        assert_refused(&answer, "invalidRequest", key, INVALID_REQUEST, key);
    }
    room.finish().await;
}

/// `q` searches the id, the customer's name and the currency (`admin-html:58`), case folded.
#[actix_web::test]
async fn q_matches_the_id_the_customer_or_the_currency_case_folded() {
    let room = Room::open().await;
    room.put_all(&[
        Tx::new(1, 1).customer("Somchai Jaidee"),
        Tx::new(2, 2).customer("Mary Jones").currency("EUR"),
        Tx::new(3, 3).customer("Kenji Sato").currency("JPY"),
    ])
    .await;
    let id = |n: u32| format!("XC-20261006-9{n:05}");
    for (q, want) in [
        ("somchai", vec![id(1)]),
        ("SOMCHAI", vec![id(1)]),
        ("ry jo", vec![id(2)]),
        ("eur", vec![id(2)]),
        ("Jpy", vec![id(3)]),
        ("900002", vec![id(2)]),
        ("xc-2026", vec![id(3), id(2), id(1)]),
        ("nobody", vec![]),
    ] {
        let data = room.data(&format!("branch={MAIN}&q={}", enc(q))).await;
        assert_eq!(ids(&data), want, "q={q}");
    }
    room.finish().await;
}

/// `q` is only those three: never who served, the notes, the customer number or the method.
#[actix_web::test]
async fn q_does_not_search_other_fields() {
    let room = Room::open().await;
    room.put_all(&[Tx::new(1, 1).staff("needle-staff").notes("needle-notes")])
        .await;
    for q in ["needle", "cash", "WALK-IN", "THA"] {
        let data = room.data(&format!("branch={MAIN}&q={}", enc(q))).await;
        assert_eq!(ids(&data), Vec::<String>::new(), "q={q}");
    }
    room.finish().await;
}

/// `q` is a literal substring (v2's `includes`): `%`, `_` and `\` match only themselves.
#[actix_web::test]
async fn q_is_literal_so_percent_underscore_and_backslash_match_only_themselves() {
    let room = Room::open().await;
    room.put_all(&[
        Tx::new(1, 1).customer("Plain Name"),
        Tx::new(2, 2).customer("Fifty% Off"),
        Tx::new(3, 3).customer("Under_Score"),
        Tx::new(4, 4).customer("Back\\Slash"),
    ])
    .await;
    let id = |n: u32| format!("XC-20261006-9{n:05}");
    for (q, want) in [
        ("%", vec![id(2)]),
        ("y% o", vec![id(2)]),
        ("_", vec![id(3)]),
        ("r_s", vec![id(3)]),
        ("\\", vec![id(4)]),
        ("k\\s", vec![id(4)]),
    ] {
        let data = room.data(&format!("branch={MAIN}&q={}", enc(q))).await;
        assert_eq!(ids(&data), want, "q={q}");
    }
    room.finish().await;
}

/// No NFKC for this view (§12.1 table: `lower()`, no NFKC): full-width letters are not their
/// ASCII forms.
#[actix_web::test]
async fn q_is_not_nfkc_normalised() {
    let room = Room::open().await;
    room.put_all(&[Tx::new(1, 1).customer("ＡＢＣ Trading")])
        .await;
    let data = room.data(&format!("branch={MAIN}&q=abc")).await;
    assert_eq!(ids(&data), Vec::<String>::new());
    let data = room.data(&format!("branch={MAIN}&q=trading")).await;
    assert_eq!(ids(&data), ["XC-20261006-900001"]);
    room.finish().await;
}

/// `q` longer than 100 characters (not bytes) is refused naming `q` (§12.1).
#[actix_web::test]
async fn q_longer_than_one_hundred_characters_is_refused_naming_q() {
    let room = Room::open().await;
    room.put_all(&[Tx::new(1, 1)]).await;
    let hundred = "ก".repeat(100);
    let data = room
        .data(&format!("branch={MAIN}&q={}", enc(&hundred)))
        .await;
    assert_eq!(ids(&data), Vec::<String>::new(), "100 characters are read");
    let answer = room
        .get(
            "owner",
            &path(&format!("branch={MAIN}&q={}", enc(&"ก".repeat(101)))),
        )
        .await;
    assert_refused(
        &answer,
        "invalidRequest",
        "q",
        INVALID_REQUEST,
        "101 characters",
    );
    room.finish().await;
}

/// `type` is `BUY` or `SELL`; anything else is refused naming `type`.
#[actix_web::test]
async fn type_lists_buys_or_sales_and_refuses_anything_else() {
    let room = Room::open().await;
    room.put_all(&[Tx::new(1, 1), Tx::new(2, 2).sell(), Tx::new(3, 3)])
        .await;
    let id = |n: u32| format!("XC-20261006-9{n:05}");
    assert_eq!(
        ids(&room.data(&format!("branch={MAIN}&type=BUY")).await),
        [id(3), id(1)]
    );
    assert_eq!(
        ids(&room.data(&format!("branch={MAIN}&type=SELL")).await),
        [id(2)]
    );
    assert_eq!(
        ids(&room.data(&format!("branch={MAIN}&type=")).await),
        [id(3), id(2), id(1)]
    );
    let answer = room
        .get("owner", &path(&format!("branch={MAIN}&type=SWAP")))
        .await;
    assert_refused(
        &answer,
        "invalidRequest",
        "type",
        INVALID_REQUEST,
        "type=SWAP",
    );
    room.finish().await;
}

/// `status` is `COMPLETED`, `CANCELLED` or `PENDING` (`admin-workspace.js:84`); every saved
/// exchange is `COMPLETED`, so the other two list nothing. Anything else is refused naming it.
#[actix_web::test]
async fn status_completed_lists_every_saved_exchange_and_the_others_none() {
    let room = Room::open().await;
    room.put_all(&[Tx::new(1, 1), Tx::new(2, 2).sell()]).await;
    let all = ["XC-20261006-900002", "XC-20261006-900001"];
    let completed = room.data(&format!("branch={MAIN}&status=COMPLETED")).await;
    assert_eq!(ids(&completed), all);
    for row in rows(&completed) {
        assert_eq!(row["status"], json!("COMPLETED"), "{row}");
    }
    for status in ["CANCELLED", "PENDING"] {
        let data = room.data(&format!("branch={MAIN}&status={status}")).await;
        assert_eq!(ids(&data), Vec::<String>::new(), "status={status}");
    }
    let answer = room
        .get("owner", &path(&format!("branch={MAIN}&status=DONE")))
        .await;
    assert_refused(
        &answer,
        "invalidRequest",
        "status",
        INVALID_REQUEST,
        "status=DONE",
    );
    room.finish().await;
}

/// `staff` is who served, exactly; `staffQ` a case-folded literal substring of it
/// (`employee-summary.js:8`).
#[actix_web::test]
async fn staff_is_exact_and_staff_q_a_case_folded_substring() {
    let room = Room::open().await;
    room.put_all(&[
        Tx::new(1, 1).staff("front"),
        Tx::new(2, 2).staff("front2"),
        Tx::new(3, 3).staff("Rates_Lead"),
        Tx::new(4, 4).staff("manager"),
    ])
    .await;
    let id = |n: u32| format!("XC-20261006-9{n:05}");
    for (query, want) in [
        ("staff=front", vec![id(1)]),
        ("staff=Front", vec![]),
        ("staff=fro", vec![]),
        ("staffQ=FRONT", vec![id(2), id(1)]),
        ("staffQ=lead", vec![id(3)]),
        ("staffQ=_", vec![id(3)]),
        ("staffQ=%25", vec![]),
        ("staff=", vec![id(4), id(3), id(2), id(1)]),
    ] {
        let data = room.data(&format!("branch={MAIN}&{query}")).await;
        assert_eq!(ids(&data), want, "{query}");
    }
    room.finish().await;
}

/// The filter is applied before the limit (in SQL, §12.1): three sales under three newer
/// purchases, read two at a time with `type=SELL`, fill the page with sales — a view that read
/// the newest two and filtered them would answer none. The filters also combine.
#[actix_web::test]
async fn the_filter_is_applied_before_the_limit_and_the_keys_combine() {
    let room = Room::open().await;
    room.put_all(&[
        Tx::new(1, 1).sell().customer("Mary Jones"),
        Tx::new(2, 2).sell().date("2026-10-05"),
        Tx::new(3, 3).sell(),
        Tx::new(4, 4),
        Tx::new(5, 5),
        Tx::new(6, 6),
    ])
    .await;
    let id = |n: u32| format!("XC-20261006-9{n:05}");
    assert_eq!(
        ids(&room.data(&format!("branch={MAIN}&type=SELL&limit=2")).await),
        [id(3), id(2)]
    );
    assert_eq!(
        ids(&room
            .data(&format!("branch={MAIN}&type=SELL&limit=2&before={}", id(2)))
            .await),
        [id(1)]
    );
    assert_eq!(
        ids(&room
            .data(&format!("branch={MAIN}&type=SELL&from=2026-10-06&q=mary"))
            .await),
        [id(1)]
    );
    assert_eq!(
        ids(&room.data(&format!("branch={MAIN}&type=BUY&q=mary")).await),
        Vec::<String>::new()
    );
    room.finish().await;
}

// ── the version (§4.1, §4.2) ─────────────────────────────────────────────────

/// `since` equal to the branch's version answers `changed: false` whatever the filter (the
/// cursor does not depend on the filter, §12.1); an older one answers the page.
#[actix_web::test]
async fn since_equal_to_the_branch_version_answers_unchanged_whatever_the_filter() {
    let room = Room::open().await;
    room.save(Save::buy("100")).await;
    let v = room.version(MAIN).await;
    for filter in [
        "",
        "&type=SELL",
        "&q=somchai&from=2026-10-01&status=COMPLETED",
    ] {
        let body = room
            .view("owner", &format!("branch={MAIN}&since={v}{filter}"))
            .await;
        is_unchanged(&body, v, filter);
    }
    let body = room
        .view("owner", &format!("branch={MAIN}&since={}", v - 1))
        .await;
    assert_eq!(body["changed"], json!(true), "{body}");
    assert_eq!(body["version"], json!(v), "{body}");
    assert_eq!(rows(&body["data"]).len(), 1, "{body}");
    room.finish().await;
}

/// The same `since` before and after one write: `changed: false`, then `changed: true` with a
/// higher version, and the new exchange first when `id` names one.
async fn moves_the_version(room: &Room, v0: i64, new_id: Option<&str>, what: &str) {
    let v1 = room.version(MAIN).await;
    assert!(v1 > v0, "{what} moved {MAIN}'s version: {v0} → {v1}");
    let body = room
        .view("owner", &format!("branch={MAIN}&since={v0}"))
        .await;
    assert_eq!(body["changed"], json!(true), "{what}: {body}");
    assert_eq!(body["version"], json!(v1), "{what}: {body}");
    if let Some(id) = new_id {
        assert_eq!(
            ids(&body["data"]).first().map(String::as_str),
            Some(id),
            "{what}: the new exchange first in {body}"
        );
    }
    let same = room
        .view("owner", &format!("branch={MAIN}&since={v1}"))
        .await;
    is_unchanged(&same, v1, &format!("{what}, at the new version"));
}

async fn unchanged_now(room: &Room) -> i64 {
    let v0 = room.version(MAIN).await;
    let body = room
        .view("owner", &format!("branch={MAIN}&since={v0}"))
        .await;
    is_unchanged(&body, v0, "before the write");
    v0
}

/// §4.2 writer: the exchange save (`SaveTransactionWorkflow`, `record_in` inside `BranchTx`).
#[actix_web::test]
async fn an_exchange_save_moves_the_version() {
    let room = Room::open().await;
    let v0 = unchanged_now(&room).await;
    let saved = room.save(Save::buy("100")).await;
    moves_the_version(&room, v0, Some(&saved.id), "the workflow's save").await;
    room.finish().await;
}

/// §4.2 writer: the desk's `saveTransaction` on `/api/staff/rpc` (the save with its steps).
#[actix_web::test]
async fn a_desk_save_moves_the_version() {
    let room = Room::open().await;
    let customer = staff_rpc(
        &room,
        "reserveCustomer",
        json!({ "name": "Customer 1", "passportNo": "P1000001", "nationality": "DEU",
                "birthDate": "1990-01-02", "passportExpiry": "2031-06-11", "mode": "LIVE" }),
    )
    .await["customerId"]
        .as_str()
        .expect("a customer number")
        .to_string();
    let passport = upload_passport(&room).await;
    room.advance(5);
    let v0 = unchanged_now(&room).await;
    let answer = staff_rpc(
        &room,
        "saveTransaction",
        save_payload(
            &room,
            &customer,
            &passport,
            "00000000-0000-4000-8000-0000004640d1",
            None,
        ),
    )
    .await;
    let id = answer["documentRef"]
        .as_str()
        .unwrap_or_else(|| panic!("a document ref in {answer}"))
        .to_string();
    moves_the_version(&room, v0, Some(&id), "the desk's saveTransaction").await;
    room.finish().await;
}

/// §4.2 writer: the save completing a kiosk request (the kiosk save hooks run inside it,
/// `save_hooks.rs` `complete_in`).
#[actix_web::test]
async fn a_kiosk_save_moves_the_version() {
    const WALKED: &str = "0000beef-0000-4000-8000-0000000a4641";
    let room = Room::open().await;
    let c = &room.counter;
    c.waiting_request(WALKED).await;
    room.advance(5);
    let accepted = c
        .ask(Some("FRONT"), "accept", json!({ "id": WALKED }))
        .await;
    assert_eq!(accepted.status, 200, "accept: {:.300}", accepted.text);
    room.advance(5);
    let checked = c
        .ask(
            Some("FRONT"),
            "identityChecked",
            json!({ "id": WALKED, "confirmed": true }),
        )
        .await;
    assert_eq!(
        checked.status, 200,
        "identityChecked: {:.300}",
        checked.text
    );
    let customer = staff_rpc(
        &room,
        "reserveCustomer",
        json!({ "name": "Customer 1", "passportNo": "P1000001", "nationality": "DEU",
                "birthDate": "1990-01-02", "passportExpiry": "2031-06-11", "mode": "LIVE" }),
    )
    .await["customerId"]
        .as_str()
        .expect("a customer number")
        .to_string();
    let passport = upload_passport(&room).await;
    room.advance(5);
    let v0 = unchanged_now(&room).await;
    staff_rpc(
        &room,
        "saveTransaction",
        save_payload(
            &room,
            &customer,
            &passport,
            "00000000-0000-4000-8000-0000004640d2",
            Some(WALKED),
        ),
    )
    .await;
    let status: String =
        sqlx::query_scalar("SELECT status FROM kiosk_requests WHERE branch_id = $1 AND id = $2")
            .bind(MAIN)
            .bind(WALKED)
            .fetch_one(&c.desk.db.pool)
            .await
            .expect("the kiosk request");
    assert_eq!(status, "COMPLETED", "the save ran the kiosk hooks");
    moves_the_version(&room, v0, None, "the kiosk save").await;
    room.finish().await;
}

/// §4.2 writer named by the ticket: `adminAbandonRequest` (inside `BranchTx::run`).
#[actix_web::test]
async fn an_abandoned_request_moves_the_version() {
    let room = Room::open().await;
    room.advance(5);
    let v0 = unchanged_now(&room).await;
    let answer = room
        .counter
        .owner_rpc(
            "adminAbandonRequest",
            json!({ "branchId": MAIN, "requestId": request_id() }),
        )
        .await;
    assert_eq!(answer["requestResolution"], json!("CANCELLED"), "{answer}");
    moves_the_version(&room, v0, None, "adminAbandonRequest").await;
    room.finish().await;
}

async fn staff_rpc(room: &Room, action: &str, payload: Value) -> Value {
    let c = &room.counter;
    let call =
        Call::action(&c.origin, "/api/staff/rpc", action, payload).cookie(&c.cookies["FRONT"]);
    let answer = c.app.send(call).await;
    assert_eq!(answer.status, 200, "{action}: {:.400}", answer.text);
    answer.json()
}

async fn upload_passport(room: &Room) -> String {
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, b'I', b'H', b'D', b'R',
    ];
    let c = &room.counter;
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

fn save_payload(
    room: &Room,
    customer: &str,
    passport: &str,
    submission: &str,
    kiosk: Option<&str>,
) -> Value {
    let now = room.counter.now();
    let mut payload = json!({
        "name": "Customer 1", "passportNo": "P1000001", "nationality": "DEU",
        "birthDate": "1990-01-02", "passportExpiry": "2031-06-11",
        "mode": "LIVE", "submissionId": submission,
        "customerId": customer, "type": "BUY", "currency": "USD",
        "amount": 100, "rate": 32.1, "fee": 0, "bankFee": 0,
        "method": "Cash", "ref": "", "account": "Cash Drawer",
        "staff": room.username_of("FRONT"), "booking": "WALK-IN",
        "notes": "", "valueDate": now.format("%Y-%m-%d").to_string(),
        "paidAt": iso(now), "paymentConfirmed": true,
        "identityConfirmed": true, "countConfirmed": true,
        "passport": { "uploadKey": passport },
    });
    if let Some(id) = kiosk {
        payload["kioskRequestId"] = json!(id);
    }
    payload
}

// ── the generated types (§5.1) ───────────────────────────────────────────────

/// A new empty directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "xc-v3-tx-{}-{nanos}-{}",
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

fn exported() -> (String, BTreeSet<String>) {
    let view = VIEWS.iter().find(|v| v.name == VIEW).unwrap_or_else(|| {
        panic!(
            "{VIEW} in the generated views {:?}",
            VIEWS.iter().map(|v| v.name).collect::<Vec<_>>()
        )
    });
    let scratch = Scratch::new();
    (view.export_types)(&scratch.0).unwrap_or_else(|e| panic!("export {VIEW}: {e}"));
    let files = std::fs::read_dir(&scratch.0)
        .expect("the scratch directory")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let text = std::fs::read_to_string(scratch.0.join(format!("{VIEW}.ts")))
        .unwrap_or_else(|e| panic!("{VIEW}.ts: {e}"));
    (text, files)
}

/// §5.1, §12.1: the view is in the generated table (a file in `http/v3/views/`, no shared list
/// edited) and exports its own `admin_transactions.ts`, whose filter type carries the seven wire
/// keys, `type` as `"BUY" | "SELL"` and `status` as the three states; its rows are the shared
/// `TransactionRow`, so it writes nothing else but `transaction_row.ts` / `common.ts`.
/// Needs no database.
#[test]
fn the_view_exports_its_filter_with_the_seven_keys() {
    let (text, files) = exported();
    for file in &files {
        assert!(
            file == "admin_transactions.ts" || file == "transaction_row.ts" || file == "common.ts",
            "{VIEW} writes {file}: {files:?}"
        );
    }
    for key in ["from", "to", "q", "type", "status", "staff", "staffQ"] {
        let declared = text.contains(&format!(" {key}: ")) || text.contains(&format!(" {key}?: "));
        assert!(declared, "{VIEW}.ts declares the key {key}: {text}");
    }
    assert!(text.contains("\"BUY\" | \"SELL\""), "{text}");
    assert!(
        text.contains("\"COMPLETED\" | \"CANCELLED\" | \"PENDING\""),
        "{text}"
    );
    assert!(!text.contains("bigint"), "{text}");
    assert!(
        !text.contains("export type TransactionRow"),
        "the row is the shared TransactionRow, not a copy: {text}"
    );
}

/// The committed `web/src/shared/api/generated/admin_transactions.ts` is what the view exports
/// (the drift gate of `v3_types_test.rs` checks every file; this names this one, so it fails
/// while the file is missing). Needs no database.
#[test]
fn the_committed_generated_file_is_the_views_export() {
    let (text, _) = exported();
    let committed = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../web/src/shared/api/generated")
        .join(format!("{VIEW}.ts"));
    let on_disk = std::fs::read_to_string(&committed)
        .unwrap_or_else(|e| panic!("{}: {e}", committed.display()));
    assert_eq!(
        on_disk,
        text,
        "{} is committed as exported",
        committed.display()
    );
}

/// `referenceSpread` zero gives "0.00" and `referencePercent` gives "0.000000" (six places).
#[actix_web::test]
async fn reference_spread_zero_gives_zero_percent_to_six_places() {
    let room = Room::open().await;
    // BUY 10 USD at 32.1, with reference rate 32.1000 -> principal 321.00, ref value 321.00 -> spread 0.00, percent 0.000000
    let saved = room.save(Save::buy("10").reference("32.1000")).await;
    let row = room.row(&saved.id).await;
    assert_eq!(row["referenceSpread"], json!("0.00"), "{row}");
    assert_eq!(row["referencePercent"], json!("0.000000"), "{row}");
    room.finish().await;
}

/// Reference percent ending in fewer than six places is padded to six places (1.25% -> "1.250000").
#[actix_web::test]
async fn reference_percent_padded_to_six_places_when_ending_early() {
    let room = Room::open().await;
    let shift = room.shift_of(MAIN).await;
    let customer = room.reserve(MAIN, "Test Cust", room.counter.now()).await;
    // BUY 100 USD at 7.9, reference rate 8.0:
    // principal = 100 * 7.9 = 790.00
    // ref_val = 100 * 8.0 = 800.00
    // spread = 800.00 - 790.00 = 10.00
    // percent = 10.00 / 800.00 * 100 = 1.25% -> "1.250000"
    sqlx::query(
        "INSERT INTO transactions (document_ref, branch_id, mode, shift_id, submission_id, \
         payload_hash, customer_id, customer_name, nationality, type, code, amount, rate, \
         reference_rate, principal, fee, bank_fee, received, paid, method, payment_ref, \
         account, staff_id, booking_no, notes, value_date, paid_at, recorded_at, \
         recorded_by) \
         VALUES ('XC-20261006-P01', $1, 'LIVE', $2, 'XC-20261006-P01', 'hash', $3, \
         'Test Cust', 'THA', 'BUY', 'USD', 100, 7.9, 8.0, 790.00, 0, 0, 790.00, 0, \
         'Cash', '', 'Cash drawer', 'front', 'WALK-IN', '', '2026-10-06'::date, \
         '2026-10-06T03:00:00Z'::timestamptz, '2026-10-06T03:00:00Z'::timestamptz, 'owner')",
    )
    .bind(MAIN)
    .bind(&shift)
    .bind(&customer)
    .execute(&room.counter.desk.db.pool)
    .await
    .unwrap();

    let row = room.row("XC-20261006-P01").await;
    assert_eq!(row["referenceSpread"], json!("10.00"), "{row}");
    assert_eq!(row["referencePercent"], json!("1.250000"), "{row}");
    room.finish().await;
}

/// Half-satang reference-value rounds half away from zero (32.1250 -> 32.13), pinning mutant P5.
#[actix_web::test]
async fn reference_value_half_away_from_zero_pins_half_satang() {
    let room = Room::open().await;
    // BUY 1 USD at counter rate 32.1 (principal 32.10).
    // Reference rate 32.1250:
    // raw_ref_val = 1 * 32.1250 = 32.1250
    // MidpointAwayFromZero: rounds to 32.13 (spread = 32.13 - 32.10 = 0.03)
    // MidpointNearestEven: rounds to 32.12 (spread = 32.12 - 32.10 = 0.02)
    let saved = room.save(Save::buy("1").reference("32.1250")).await;
    let row = room.row(&saved.id).await;
    assert_eq!(row["referenceValue"], json!("32.13"), "{row}");
    assert_eq!(row["referenceSpread"], json!("0.03"), "{row}");
    room.finish().await;
}
