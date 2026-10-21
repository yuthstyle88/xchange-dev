//! Ticket #466 (V2-customers, `docs/design/xchange-v3-web.md` §4.2, §5.1, §12.1, §12.14): the two
//! paged views the admin's customer menu reads instead of joining every customer of the snapshot
//! to every exchange in the browser. Written before the implementation by the lane controller
//! (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! v2 (`108-Plaza/xchange-v2-source@4d0bdbb`): `src/online/public/customer-core.js:1-8` joins the
//! snapshot's customers (`{id, name, nationality, created, member}`, `v2_json.rs` `customers_json`)
//! to its exchanges: `completed` = the customer's COMPLETED exchanges, `volume` = the sum of each
//! one's `amount × rate` rounded to the satang (`Math.round(t.amount*t.rate*100)`, summed, ÷ 100),
//! `lastAt` = the newest exchange's `created` (`''` when none); sorted by `lastAt` newest first,
//! then by name; searched over id, name and nationality, NFKC-normalised and case-folded
//! (`:6`). `customers.js:15-17` shows one customer's exchanges, newest first, searched over id,
//! currency and date.
//!
//! - `GET /api/v3/views/admin_customers` — a `PagedView` inheriting `(EntryPath::Rpc,
//!   "adminSnapshot")`: the LIVE customers of the branch in v2's order, keyset paged by the row's
//!   `id` (`before`), `limit` ≤ 100. A row is exactly `{id, name, nationality, created, member,
//!   completed, volume, lastAt}`: `created` (the customer's, shown by the history dialog as
//!   `เพิ่มเมื่อ`, `customers.js:16`) and `lastAt` are RFC 3339 instants, `lastAt` is `null` for
//!   a customer with no exchange; `completed` is a JSON number (a count, not money); `volume` is a
//!   decimal string computed by the server (W7). Filter `q` (design §12.1 row `admin_customers`).
//! - `GET /api/v3/views/admin_customer_history` — a `PagedView` inheriting the same row: one
//!   customer's LIVE exchanges in the branch, newest first, keyset paged by the row's `id`, each
//!   the shared `TransactionRow` (V2-parts #476). Filter `customer` (required — absent or empty is
//!   `400 invalidRequest` naming `customer`) and `q` over id, currency and date (`lower()`, no
//!   NFKC). The row's mapping is `TransactionRow::from_record`, #464's (blocked by): this file
//!   pins which rows, in which order, and the fields the history table shows — never the
//!   constructor.
//! - Both filters run in SQL (a filtered page is full when enough rows match); `q` is a literal
//!   substring (`%`, `_`, `\` match themselves); `q` longer than 100 characters is
//!   `400 invalidRequest` naming `q`, refused before the `since` shortcut.
//! - §4.2: every writer of what the views read goes through `BranchTx` — one write per writer,
//!   then the same `since` answers `changed: true`. Writers: the customer service
//!   (`CustomerService::reserve`), the exchange save (`SaveTransactionWorkflow`, which also writes
//!   `customers` through `confirm_in` / `record_exchange_in`), and the phone verification
//!   (`customerContactVerify`, `ContactVerify::verify_in`, which writes the verified `PHONE`
//!   identity the `member` flag is read from — BK-10 #1412 re-pinned this from v2's member issue
//!   and v2's member table).
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real
//! sessions made at the real sign-in doors (`rpc_common`); exchanges are saved through the real
//! `SaveTransactionWorkflow`, as `v3_admin_parts_views_test.rs` does. The test's clock is advanced
//! before every write, so no two writes share a time and no order rests on a frozen clock.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing skips.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
mod rpc_common;
mod shift_holder_common;
mod shift_setup_common;
mod snapshot_common;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use xc_auth::domain::access::{Actor, Role};
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_customer::application::customer::contact_verify::ContactVerify;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xchange::presentation::http::v3::VIEWS;
use xchange::presentation::test_callers;

const ORIGIN: &str = "http://desk.test";
const CUSTOMERS: &str = "admin_customers";
const HISTORY: &str = "admin_customer_history";

const MAIN: &str = "MAIN";
const B9: &str = "B9";
const B9_NAME: &str = "Nine";

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];

/// A row of `admin_customers`, and nothing else.
const CUSTOMER_FIELDS: [&str; 8] = [
    "id",
    "name",
    "nationality",
    "created",
    "member",
    "completed",
    "volume",
    "lastAt",
];

/// `TransactionRow`'s fields (design §12.2), the shape every row of the history has.
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

/// Thai `ำ` (SARA AM, U+0E33) composed, and the two code points NFKC turns it into.
const KAMRAI_COMPOSED: &str = "นายกำไร";
const KAM_DECOMPOSED: &str = "ก\u{0E4D}\u{0E32}";
const KAMRAI_DECOMPOSED: &str = "นางก\u{0E4D}\u{0E32}ลัง";
const KAM_COMPOSED: &str = "กำ";

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[allow(dead_code)]
fn request_id() -> String {
    format!("00000000-0000-4000-8000-{:012x}", next())
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

/// `%XX` of every byte that is not an unreserved ASCII character, for a query value.
fn enc(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A GET as the screen sends it: no body, no marker header.
fn get(path: &str) -> Call {
    let mut call = Call::post(ORIGIN, path, Vec::<u8>::new())
        .without("x-xchange-request")
        .without("content-type")
        .without("origin");
    call.method = Method::GET;
    call
}

fn view_path(view: &str, query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{view}")
    } else {
        format!("/api/v3/views/{view}?{query}")
    }
}

/// An RFC 3339 instant, as the row prints it.
fn instant(row: &Value, field: &str) -> DateTime<Utc> {
    let text = row[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} is a string: {row}"));
    DateTime::parse_from_rfc3339(text)
        .unwrap_or_else(|e| panic!("{field} is an RFC 3339 instant ({e}): {row}"))
        .with_timezone(&Utc)
}

/// A decimal string with two places, as satang. Panics on anything else.
fn satang(text: &str) -> i128 {
    let (whole, cents) = text
        .split_once('.')
        .unwrap_or_else(|| panic!("{text:?} has two decimal places"));
    assert_eq!(cents.len(), 2, "{text:?} has two decimal places");
    let whole: i128 = whole.parse().unwrap_or_else(|e| panic!("{text:?}: {e}"));
    let cents: i128 = cents.parse().unwrap_or_else(|e| panic!("{text:?}: {e}"));
    whole * 100 + cents
}

/// A person at the counter. `demo` = a DEMO customer.
#[derive(Clone)]
struct Who {
    name: String,
    passport: String,
    nationality: String,
    demo: bool,
}

impl Who {
    fn new(name: &str) -> Who {
        Who {
            name: name.to_owned(),
            passport: format!("AA{:07}", next()),
            nationality: "THA".to_owned(),
            demo: false,
        }
    }

    fn of(mut self, nationality: &str) -> Who {
        self.nationality = nationality.to_owned();
        self
    }

    fn demo(mut self) -> Who {
        self.demo = true;
        self
    }

    fn command(&self) -> ReserveCustomer {
        ReserveCustomer {
            name: self.name.clone(),
            passport_no: self.passport.clone(),
            nationality: self.nationality.clone(),
            birth_date: String::new(),
            passport_expiry: String::new(),
            mode: if self.demo { "DEMO" } else { "LIVE" }.to_owned(),
        }
    }
}

/// One exchange: USD 100 bought at 32.1, valued 2026-10-06, unless changed.
#[derive(Clone)]
struct Ex {
    currency: &'static str,
    amount: &'static str,
    value_date: &'static str,
    notes: &'static str,
    payment_ref: &'static str,
}

impl Ex {
    fn usd() -> Ex {
        Ex {
            currency: "USD",
            amount: "100",
            value_date: "2026-10-06",
            notes: "",
            payment_ref: "",
        }
    }
}

/// A customer as reserved.
#[derive(Clone)]
struct Reserved {
    id: String,
    at: DateTime<Utc>,
}

/// What a save left behind.
#[derive(Clone)]
struct Saved {
    id: String,
    customer_id: String,
    at: DateTime<Utc>,
}

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    t0: DateTime<Utc>,
}

impl Room {
    /// `MAIN` (from the migration) and `B9`; the owner and one employee of every staff role in
    /// `MAIN`, all signed in; USD and EUR counter rates in both branches; a shift open in both.
    async fn open() -> Room {
        let desk = Desk::open().await;
        desk.db.seed_branch(B9, B9_NAME).await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let t0 = snapshot_common::time("2026-10-06T05:00:00.000Z");
        desk.clock.set(t0);
        let room = Room {
            desk,
            app,
            cookies,
            t0,
        };
        for branch in [MAIN, B9] {
            room.set_rate(branch, "USD", "32.1", "33").await;
            room.set_rate(branch, "EUR", "38.2", "39.1").await;
            room.open_shift(branch).await;
        }
        room
    }

    fn tick(&self) -> DateTime<Utc> {
        let t = self.t0 + Duration::seconds(next() as i64);
        self.desk.clock.set(t);
        t
    }

    fn cookie(&self, who: &str) -> String {
        match who.split_once('+') {
            Some((a, b)) => format!("{}; {}", self.cookies[a], self.cookies[b]),
            None => self.cookies[who].clone(),
        }
    }

    /// `GET path` as `who` (`""` = nobody; `OWNER+FRONT` = both cookies in one browser).
    async fn get(&self, who: &str, path: &str) -> Answer {
        let call = get(path);
        let call = if who.is_empty() {
            call
        } else {
            call.cookie(&self.cookie(who))
        };
        self.app.send(call).await
    }

    /// A view as `who`, which must be answered 200 with JSON, never cached.
    async fn view(&self, who: &str, view: &str, query: &str) -> Value {
        let path = view_path(view, query);
        let answer = self.get(who, &path).await;
        assert_eq!(
            answer.status, 200,
            "GET {path} as {who:?}: {:.400}",
            answer.text
        );
        assert_eq!(
            answer.header("cache-control"),
            Some("no-store"),
            "GET {path}: never cached"
        );
        answer.json()
    }

    /// The rows of a page, which must be answered `changed: true`.
    async fn rows(&self, who: &str, view: &str, query: &str) -> Vec<Value> {
        let body = self.view(who, view, query).await;
        assert_eq!(
            body["changed"],
            json!(true),
            "GET {}: {body}",
            view_path(view, query)
        );
        body.get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| panic!("GET {}: a list in data: {body}", view_path(view, query)))
    }

    /// The ids of a page, in the order answered.
    async fn ids(&self, who: &str, view: &str, query: &str) -> Vec<String> {
        self.rows(who, view, query)
            .await
            .iter()
            .map(|r| {
                r["id"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a string id: {r}"))
                    .to_owned()
            })
            .collect()
    }

    /// The owner's `admin_customers` ids of `MAIN` (limit 100) with `extra` query keys.
    async fn customers(&self, extra: &str) -> Vec<String> {
        let query = if extra.is_empty() {
            "branch=MAIN&limit=100".to_owned()
        } else {
            format!("branch=MAIN&limit=100&{extra}")
        };
        self.ids("OWNER", CUSTOMERS, &query).await
    }

    /// The owner's row of `id` in `MAIN`'s customers.
    async fn customer(&self, id: &str) -> Value {
        let rows = self.rows("OWNER", CUSTOMERS, "branch=MAIN&limit=100").await;
        rows.iter()
            .find(|r| r["id"] == json!(id))
            .cloned()
            .unwrap_or_else(|| panic!("{id} among {rows:?}"))
    }

    /// The owner's `admin_customer_history` ids of `customer` in `MAIN` (limit 100).
    async fn history(&self, customer: &str, extra: &str) -> Vec<String> {
        let mut query = format!("branch=MAIN&limit=100&customer={}", enc(customer));
        if !extra.is_empty() {
            query.push('&');
            query.push_str(extra);
        }
        self.ids("OWNER", HISTORY, &query).await
    }

    async fn version(&self, branch: &str) -> i64 {
        BranchVersions::new(self.desk.db.pool.clone())
            .current(&BranchId::new(branch))
            .await
            .expect("read the branch version")
    }

    async fn set_rate(&self, branch: &str, code: &str, buy: &str, sell: &str) {
        // Re-expected for RE-7 #604 by the ticket author (rates.md §8 migration 6 drops `rates`):
        // the rate is published, per 1 × unit_size, over the current publication; no `rates` row.
        rates_re4e_publish::set_counter_rate(&self.desk.db.pool, branch, code, buy, sell).await;
    }

    async fn open_shift(&self, branch: &str) {
        let t = self.tick();
        shift_setup_common::open_shift_by_count(
            &self.desk.db.pool,
            branch,
            json!({ "THB": 100000, "USD": 1000, "EUR": 1000 }),
            t,
        )
        .await;
    }

    /// Reserves `who` in `branch` through the customer service (the same customer again when
    /// already reserved there).
    async fn reserve(&self, branch: &'static str, who: &Who) -> Reserved {
        let t = self.tick();
        let id = CustomerService::new(self.desk.db.branch_tx())
            .reserve(&BranchId::new(branch), who.command(), t)
            .await
            .unwrap_or_else(|e| panic!("reserve {} in {branch}: {e:?}", who.name))
            .customer_id;
        Reserved { id, at: t }
    }

    /// Saves one exchange of `who` through the real workflow behind the routes.
    async fn save(&self, branch: &'static str, who: &Who, ex: Ex) -> Saved {
        let customer = self.reserve(branch, who).await;
        let t = self.tick();
        let here = BranchId::new(branch);
        let saver = shift_holder_common::teller_of_open_shift(&self.desk.db.pool, branch).await;
        let passport = self
            .desk
            .files
            .upload(&here, &saver, "image/jpeg", JPEG, t)
            .await
            .expect("upload a passport");
        let cmd = SaveTransaction {
            customer: who.command(),
            submission_id: format!("submission-{branch}-{:012}", next()),
            customer_id: customer.id.clone(),
            side: "BUY".to_owned(),
            currency: ex.currency.to_owned(),
            amount: ex.amount.to_owned(),
            rate: if ex.currency == "EUR" { "38.2" } else { "32.1" }.to_owned(),
            fee: "0".to_owned(),
            bank_fee: "0".to_owned(),
            method: "Cash".to_owned(),
            payment_ref: ex.payment_ref.to_owned(),
            account: "Cash drawer 1".to_owned(),
            staff: "EMP-466".to_owned(),
            booking: "WALK-IN".to_owned(),
            notes: ex.notes.to_owned(),
            value_date: ex.value_date.to_owned(),
            paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
            payment_confirmed: true,
            identity_confirmed: true,
            passport: Some(passport),
            slip: None,
            reference_rate: None,
        };
        let saved = self
            .desk
            .workflow
            .save(&here, cmd, &saver, t)
            .await
            .unwrap_or_else(|e| panic!("save an exchange of {} in {branch}: {e:?}", who.name));
        Saved {
            id: saved.document_ref,
            customer_id: customer.id,
            at: t,
        }
    }

    /// BK-10 (#1412): a Teller of `branch` verifies `customer`'s phone through `ContactVerify`,
    /// inside `BranchTx::run` on `branch` as the `customerContactVerify` command seam runs it.
    async fn verify_phone(&self, branch: &str, customer: &str) {
        let t = self.tick();
        let here = BranchId::new(branch);
        let who = xc_auth::domain::access::Actor::new(
            "00000000-0000-4000-8000-0000000000aa",
            "00000000-0000-0000-0000-000000000001",
            vec![xc_auth::domain::access::ScopedRole::new(
                xc_auth::domain::access::Role::Teller,
                Some(here.clone()),
                DateTime::<Utc>::UNIX_EPOCH,
                None,
            )],
            None,
        );
        let service = ContactVerify::new(self.desk.db.branch_tx());
        let customer = customer.to_owned();
        let acting = here.clone();
        self.desk
            .db
            .branch_tx()
            .run(&here, move |tx| {
                Box::pin(async move {
                    service
                        .verify_in(tx, &who, &acting, &customer, "081-234-5678", t)
                        .await
                        .map_err(|e| pos_error::AppError::Internal(format!("{e:?}")))
                })
            })
            .await
            .unwrap_or_else(|e| panic!("verify a phone at {branch}: {e:?}"));
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

const STAFF: [(&str, &[Role]); 9] = [
    ("FRONT", &[Role::Teller]),
    ("BACKEND", &[Role::BranchManager]),
    ("MANAGER", &[Role::BranchManager]),
    ("RATES", &[Role::RateManager]),
    ("MONITOR", &[Role::VerificationReviewer]),
    ("FINANCE", &[Role::Finance]),
    ("SYSTEM", &[Role::Admin]),
    ("DOCUMENTS", &[Role::VerificationReviewer]),
    ("DOCUMENT_REVIEW", &[Role::VerificationReviewer]),
];

/// The actor `Policy` sees for each caller key of [`Room`] on a path that is not `StaffRpc`.
fn callers() -> Vec<(String, Option<Actor>)> {
    let mut callers: Vec<(String, Option<Actor>)> = vec![
        (String::new(), None),
        ("OWNER".to_string(), Some(test_callers::owner("owner"))),
    ];
    for (name, roles) in STAFF {
        let staff = Some(test_callers::staff("employee", roles, BranchId::new(MAIN)));
        callers.push((name.to_string(), staff));
        callers.push((format!("OWNER+{name}"), Some(test_callers::owner("owner"))));
    }
    callers
}

fn assert_refused(answer: &Answer, code: &str, field: &str, what: &str) {
    assert_eq!(answer.status, 400, "{what}: {:.300}", answer.text);
    let body = answer.json();
    assert_eq!(body["error"]["code"], json!(code), "{what}: {body}");
    assert_eq!(body["error"]["field"], json!(field), "{what}: {body}");
    assert!(body.get("data").is_none(), "{what}: no data in {body}");
}

fn rev(saved: &[&Saved]) -> Vec<String> {
    saved.iter().rev().map(|s| s.id.clone()).collect()
}

fn ids_of(reserved: &[&Reserved]) -> Vec<String> {
    reserved.iter().map(|r| r.id.clone()).collect()
}

/// Every caller kind, as `Policy::authorize(actor, Rpc, adminSnapshot, ..)` answers it.
async fn answers_exactly_whom_the_snapshot_row_allows(room: &Room, path: &str) {
    let mut allowed = 0;
    let mut refused = 0;
    for (who, actor) in callers() {
        let policy = Policy::authorize(
            actor.as_ref(),
            EntryPath::Rpc,
            &Action::new("adminSnapshot"),
            None,
            Utc::now(),
        );
        let answer = room.get(&who, path).await;
        let what = format!("{path} as {who:?}");
        match &policy {
            Ok(_) => {
                allowed += 1;
                assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
                let body = answer.json();
                assert_eq!(body["changed"], json!(true), "{what}: {body}");
                assert!(body["data"].is_array(), "{what}: a list in data: {body}");
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
    assert!(allowed > 0, "someone may read {path}");
    assert!(refused > 0, "nobody signed in is refused {path}");
}

// ── registration and who may read ────────────────────────────────────────────

#[test]
fn both_views_are_in_the_generated_view_table() {
    for view in [CUSTOMERS, HISTORY] {
        assert!(
            VIEWS.iter().any(|v| v.name == view),
            "{view} is listed by the generator from http/v3/views/ (no shared file edited): {:?}",
            VIEWS.iter().map(|v| v.name).collect::<Vec<_>>()
        );
    }
}

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

#[actix_web::test]
async fn admin_customers_answers_exactly_whom_the_snapshot_row_allows() {
    let room = Room::open().await;
    room.save(MAIN, &Who::new("Somchai Jaidee"), Ex::usd())
        .await;
    answers_exactly_whom_the_snapshot_row_allows(&room, &view_path(CUSTOMERS, "limit=50")).await;
    // FRONT works in MAIN. OR-4b (D-4): naming B9 is refused; naming MAIN is answered (restored
    // from the retired `only_the_live_customers_of_the_branch_…`, controller ruling on #767)
    assert_out_of_scope(
        &room
            .get("FRONT", &view_path(CUSTOMERS, "branch=B9&limit=50"))
            .await,
        "FRONT naming B9",
    );
    let answer = room
        .get("FRONT", &view_path(CUSTOMERS, "branch=MAIN&limit=50"))
        .await;
    assert_eq!(
        answer.status, 200,
        "FRONT naming MAIN: {:.300}",
        answer.text
    );
    room.finish().await;
}

#[actix_web::test]
async fn admin_customer_history_answers_exactly_whom_the_snapshot_row_allows() {
    let room = Room::open().await;
    let saved = room
        .save(MAIN, &Who::new("Somchai Jaidee"), Ex::usd())
        .await;
    let path = view_path(
        HISTORY,
        &format!("limit=50&customer={}", enc(&saved.customer_id)),
    );
    answers_exactly_whom_the_snapshot_row_allows(&room, &path).await;
    room.finish().await;
}

// ── admin_customers: the row ─────────────────────────────────────────────────

/// `customer-core.js:3`: `completed` counts the customer's exchanges, `volume` sums each one's
/// `amount × rate` rounded to the satang first. 10.05 × 32.1 = 322.605 → 322.61, twice = 645.22
/// (rounding the exact sum 645.21 instead would be wrong), plus 100 × 32.1 = 3210.00.
#[actix_web::test]
async fn a_customer_row_has_exactly_its_fields_and_the_figures_of_its_exchanges() {
    let room = Room::open().await;
    let malee = Who::new("Malee Rakdee").of("JPN");
    let reserved = room.reserve(MAIN, &malee).await;
    let odd = Ex {
        amount: "10.05",
        ..Ex::usd()
    };
    room.save(MAIN, &malee, odd.clone()).await;
    room.save(MAIN, &malee, odd).await;
    let last = room.save(MAIN, &malee, Ex::usd()).await;
    assert_eq!(last.customer_id, reserved.id, "the same customer each time");
    // another customer's exchange counts for nobody else
    room.save(MAIN, &Who::new("Other Person"), Ex::usd()).await;

    let row = room.customer(&reserved.id).await;
    assert_eq!(keys(&row), set_of(&CUSTOMER_FIELDS), "exactly: {row}");
    assert_eq!(row["name"], json!("Malee Rakdee"), "{row}");
    assert_eq!(
        row["nationality"],
        json!("JPN"),
        "as stored (upper case): {row}"
    );
    assert_eq!(
        instant(&row, "created"),
        reserved.at,
        "created is when the customer was first recorded: {row}"
    );
    assert_eq!(row["member"], json!(false), "{row}");
    assert_eq!(
        row["completed"],
        json!(3),
        "a JSON number, every saved exchange of the customer: {row}"
    );
    assert_eq!(
        row["volume"],
        json!("3855.22"),
        "a decimal string, Σ round(amount × rate, 2): 322.61 + 322.61 + 3210.00: {row}"
    );
    assert_eq!(
        instant(&row, "lastAt"),
        last.at,
        "lastAt is when the newest exchange was recorded: {row}"
    );
    room.finish().await;
}

/// Ticket §9: a customer with no exchange has `completed 0`, volume `0.00` and no last activity.
#[actix_web::test]
async fn a_customer_with_no_exchange_has_completed_0_volume_0_00_and_no_last_activity() {
    let room = Room::open().await;
    let reserved = room.reserve(MAIN, &Who::new("Never Exchanged")).await;
    let row = room.customer(&reserved.id).await;
    assert_eq!(keys(&row), set_of(&CUSTOMER_FIELDS), "exactly: {row}");
    assert_eq!(row["completed"], json!(0), "{row}");
    assert_eq!(row["volume"], json!("0.00"), "{row}");
    assert_eq!(
        row["lastAt"],
        Value::Null,
        "null, not a time and not missing: {row}"
    );
    assert_eq!(instant(&row, "created"), reserved.at, "{row}");
    room.finish().await;
}

/// `customer-core.js:3`: newest activity first, then the name; ticket §9: a customer with no
/// exchange is sorted last. Names that tie keep the order they were recorded in (v2's sort is
/// stable over the snapshot's list, which is in the order customers were recorded).
#[actix_web::test]
async fn customers_are_ordered_by_last_activity_then_name_with_no_exchange_last() {
    let room = Room::open().await;
    let charlie = room.reserve(MAIN, &Who::new("Charlie")).await;
    let anan = Who::new("Anan");
    let boon = Who::new("Boon");
    let anan_first = room.save(MAIN, &anan, Ex::usd()).await;
    let boon_last = room.save(MAIN, &boon, Ex::usd()).await;
    let alpha = room.reserve(MAIN, &Who::new("Alpha")).await;
    let bravo_1 = room.reserve(MAIN, &Who::new("Bravo")).await;
    let bravo_2 = room.reserve(MAIN, &Who::new("Bravo")).await;
    assert_ne!(bravo_1.id, bravo_2.id, "two customers named Bravo");

    let expected = vec![
        boon_last.customer_id.clone(),
        anan_first.customer_id.clone(),
        alpha.id.clone(),
        bravo_1.id.clone(),
        bravo_2.id.clone(),
        charlie.id.clone(),
    ];
    assert_eq!(
        room.customers("").await,
        expected,
        "Boon's exchange is newer"
    );

    // Anan exchanges again: now the newest activity
    room.save(MAIN, &anan, Ex::usd()).await;
    let ids = room.customers("").await;
    assert_eq!(
        ids[..2].to_vec(),
        vec![
            anan_first.customer_id.clone(),
            boon_last.customer_id.clone()
        ],
        "lastAt is the newest exchange, whatever came before: {ids:?}"
    );
    room.finish().await;
}

// CI-4 (#651): `only_the_live_customers_of_the_branch_are_listed_staff_reading_their_own_branch` retired
// (#554 §6): `admin_customers` lists every LIVE customer of the organisation (owner, 2026-10-09: "Yes, all
// company customers"; #554 §4.4); its figures stay the branch's.

/// `customers.js:11` shows `Member` for a customer the snapshot marks (`member`, T15-g).
/// Re-pinned for BK-10 #1412 by claude/29df2ec5 (ruling 1, issuecomment-6140233542): the member
/// code is dropped; a member is a customer whose phone staff verified (`customerContactVerify`).
#[actix_web::test]
async fn a_customer_with_a_verified_phone_is_flagged_member() {
    let room = Room::open().await;
    let somchai = room
        .save(MAIN, &Who::new("Somchai Jaidee"), Ex::usd())
        .await;
    let other = room.reserve(MAIN, &Who::new("Not A Member")).await;
    room.verify_phone(MAIN, &somchai.customer_id).await;
    let row = room.customer(&somchai.customer_id).await;
    assert_eq!(row["member"], json!(true), "{row}");
    let row = room.customer(&other.id).await;
    assert_eq!(row["member"], json!(false), "{row}");
    room.finish().await;
}

// ── admin_customers: paging ──────────────────────────────────────────────────

#[actix_web::test]
async fn customers_page_by_the_last_id_across_ties_and_customers_with_no_exchange() {
    let room = Room::open().await;
    let older = room.save(MAIN, &Who::new("Zed Older"), Ex::usd()).await;
    let newer = room.save(MAIN, &Who::new("Yan Newer"), Ex::usd()).await;
    let delta = room.reserve(MAIN, &Who::new("Delta")).await;
    let bravo_1 = room.reserve(MAIN, &Who::new("Bravo")).await;
    let bravo_2 = room.reserve(MAIN, &Who::new("Bravo")).await;
    let all = vec![
        newer.customer_id.clone(),
        older.customer_id.clone(),
        bravo_1.id.clone(),
        bravo_2.id.clone(),
        delta.id.clone(),
    ];
    assert_eq!(room.customers("").await, all, "the whole order");

    let mut walked = Vec::new();
    let mut before: Option<String> = None;
    for _ in 0..4 {
        let mut query = "branch=MAIN&limit=2".to_owned();
        if let Some(b) = &before {
            query.push_str(&format!("&before={}", enc(b)));
        }
        let page = room.ids("OWNER", CUSTOMERS, &query).await;
        if page.is_empty() {
            break;
        }
        assert!(page.len() <= 2, "at most limit rows: {page:?}");
        before = page.last().cloned();
        walked.extend(page);
    }
    assert_eq!(
        walked, all,
        "pages of 2 by the last id: every customer once, in order, across the Bravo tie and \
         the customers with no exchange"
    );

    let answer = room
        .get("OWNER", &view_path(CUSTOMERS, "branch=MAIN&limit=101"))
        .await;
    assert_eq!(answer.status, 400, "limit above 100: {:.300}", answer.text);
    room.finish().await;
}

// ── admin_customers: the filter, in SQL ──────────────────────────────────────

#[actix_web::test]
async fn customers_q_searches_id_name_and_nationality_only() {
    let room = Room::open().await;
    let somsri = room
        .save(
            MAIN,
            &Who::new("Somsri Boonmee"),
            Ex {
                currency: "EUR",
                notes: "note-only-word",
                payment_ref: "PAYREF-ONLY",
                ..Ex::usd()
            },
        )
        .await;
    let ken_who = Who::new("Ken Tanaka").of("JPN");
    let ken = room.reserve(MAIN, &ken_who).await;

    let cases: Vec<(String, Vec<String>, &str)> = vec![
        (
            somsri.customer_id.clone(),
            vec![somsri.customer_id.clone()],
            "the id",
        ),
        (
            "Somsri".to_owned(),
            vec![somsri.customer_id.clone()],
            "the name",
        ),
        ("jpn".to_owned(), vec![ken.id.clone()], "the nationality"),
        (
            "CX-".to_owned(),
            vec![somsri.customer_id.clone(), ken.id.clone()],
            "both ids, in the list's order",
        ),
        (
            ken_who.passport.clone(),
            vec![],
            "the passport is not searched",
        ),
        (
            "EUR".to_owned(),
            vec![],
            "the exchanges' currency is not searched",
        ),
        (
            "note-only-word".to_owned(),
            vec![],
            "the notes are not searched",
        ),
        (
            "PAYREF".to_owned(),
            vec![],
            "the payment reference is not searched",
        ),
        ("no-such-thing".to_owned(), vec![], "nothing matches"),
    ];
    for (q, expected, what) in cases {
        let ids = room.customers(&format!("q={}", enc(&q))).await;
        assert_eq!(ids, expected, "q={q:?}: {what}");
    }
    room.finish().await;
}

#[actix_web::test]
async fn customers_q_is_case_folded_and_nfkc_normalised_on_both_sides() {
    let room = Room::open().await;
    let latin = room.reserve(MAIN, &Who::new("Somchai Jaidee")).await;
    let composed = room.reserve(MAIN, &Who::new(KAMRAI_COMPOSED)).await;
    let decomposed = room.reserve(MAIN, &Who::new(KAMRAI_DECOMPOSED)).await;
    // the three have no exchange, so they are listed by name: Latin before Thai
    let thai_by_name = room.customers(&format!("q={}", enc(KAM_COMPOSED))).await;

    let cases: Vec<(&str, Vec<String>, &str)> = vec![
        ("SOMCHAI", ids_of(&[&latin]), "upper case finds mixed case"),
        ("jaidee", ids_of(&[&latin]), "lower case finds mixed case"),
        // full-width Latin letters (U+FF33 …) are SOM under NFKC
        (
            "\u{FF33}\u{FF2F}\u{FF2D}chai",
            ids_of(&[&latin]),
            "full-width",
        ),
        (
            KAM_DECOMPOSED,
            thai_by_name.clone(),
            "the decomposed Thai finds the stored composed ำ (NFKC on the stored side)",
        ),
    ];
    for (q, expected, what) in cases {
        let ids = room.customers(&format!("q={}", enc(q))).await;
        assert_eq!(ids, expected, "q={q:?}: {what}");
    }
    let mut found: Vec<String> = thai_by_name.clone();
    found.sort();
    let mut both = ids_of(&[&composed, &decomposed]);
    both.sort();
    assert_eq!(
        found, both,
        "the composed ำ finds the stored decomposed form too (NFKC on the query side)"
    );
    room.finish().await;
}

#[actix_web::test]
async fn customers_q_is_a_literal_substring_not_a_pattern() {
    let room = Room::open().await;
    let plain = room.reserve(MAIN, &Who::new("Wanida Sukjai")).await;
    let marked = room.reserve(MAIN, &Who::new("Pim 50%_off\\x")).await;
    let cases: Vec<(&str, Vec<String>)> = vec![
        ("%", ids_of(&[&marked])),
        ("_", ids_of(&[&marked])),
        ("\\", ids_of(&[&marked])),
        ("50%_off\\x", ids_of(&[&marked])),
        ("Wanida%", vec![]),
        ("Wan_da", vec![]),
        // Pim before Wanida by name; neither has an exchange
        ("i", ids_of(&[&marked, &plain])),
    ];
    for (q, expected) in cases {
        let ids = room.customers(&format!("q={}", enc(q))).await;
        assert_eq!(ids, expected, "q={q:?} is matched literally");
    }
    room.finish().await;
}

#[actix_web::test]
async fn customers_refuse_q_longer_than_100_characters_naming_q() {
    let room = Room::open().await;
    room.reserve(MAIN, &Who::new("Somchai Jaidee")).await;
    let v = room.version(MAIN).await;
    let long = "a".repeat(101);
    for query in [
        format!("branch=MAIN&q={long}"),
        format!("branch=MAIN&q={long}&since={v}"),
    ] {
        let answer = room.get("OWNER", &view_path(CUSTOMERS, &query)).await;
        assert_refused(&answer, "invalidRequest", "q", "q of 101 characters");
    }
    let ids = room.customers(&format!("q={}", "a".repeat(100))).await;
    assert!(ids.is_empty(), "q of 100 characters is read: {ids:?}");
    room.finish().await;
}

#[actix_web::test]
async fn a_filtered_customer_page_is_full() {
    let room = Room::open().await;
    let mut wanted = Vec::new();
    for i in 0..4 {
        // between every two wanted customers, one who does not match: a page filtered after it
        // was read would come back short
        // (none has an exchange, so they are listed by name: K0a, K0b, K1a, K1b, …)
        wanted.push(
            room.reserve(MAIN, &Who::new(&format!("K{i}a Kanya Target")))
                .await,
        );
        room.reserve(MAIN, &Who::new(&format!("K{i}b Other Person")))
            .await;
    }
    let names_in_order = ids_of(&wanted.iter().collect::<Vec<_>>());
    let first = room
        .ids("OWNER", CUSTOMERS, "branch=MAIN&limit=2&q=kanya%20target")
        .await;
    assert_eq!(first, names_in_order[0..2].to_vec(), "a full first page");
    let second = room
        .ids(
            "OWNER",
            CUSTOMERS,
            &format!(
                "branch=MAIN&limit=2&q=kanya%20target&before={}",
                enc(&first[1])
            ),
        )
        .await;
    assert_eq!(second, names_in_order[2..4].to_vec(), "a full second page");
    room.finish().await;
}

// ── admin_customer_history ───────────────────────────────────────────────────

/// Design §12.1 row `admin_customer_history`: `customer` is required.
#[actix_web::test]
async fn history_refuses_a_missing_or_empty_customer_naming_customer() {
    let room = Room::open().await;
    room.save(MAIN, &Who::new("Somchai Jaidee"), Ex::usd())
        .await;
    let v = room.version(MAIN).await;
    for query in [
        "branch=MAIN".to_owned(),
        "branch=MAIN&customer=".to_owned(),
        format!("branch=MAIN&since={v}"),
        format!("branch=MAIN&customer=&since={v}"),
    ] {
        let answer = room.get("OWNER", &view_path(HISTORY, &query)).await;
        assert_refused(&answer, "invalidRequest", "customer", &query);
    }
    room.finish().await;
}

#[actix_web::test]
async fn history_lists_that_customers_live_exchanges_newest_first_as_transaction_rows() {
    let room = Room::open().await;
    let malee = Who::new("Malee Rakdee");
    let a = room
        .save(
            MAIN,
            &malee,
            Ex {
                currency: "EUR",
                value_date: "2026-10-05",
                ..Ex::usd()
            },
        )
        .await;
    room.save(MAIN, &Who::new("Somebody Else"), Ex::usd()).await;
    let b = room.save(MAIN, &malee, Ex::usd()).await;
    room.save(B9, &Who::new("Nine Person"), Ex::usd()).await;

    let rows = room
        .rows(
            "OWNER",
            HISTORY,
            &format!("branch=MAIN&limit=50&customer={}", enc(&a.customer_id)),
        )
        .await;
    let ids: Vec<&str> = rows.iter().filter_map(|r| r["id"].as_str()).collect();
    assert_eq!(
        ids,
        vec![b.id.as_str(), a.id.as_str()],
        "that customer's only, newest first: {rows:?}"
    );
    let row = &rows[1];
    assert_eq!(
        keys(row),
        set_of(&ROW_FIELDS),
        "the shared TransactionRow: {row}"
    );
    assert_eq!(row["customerId"], json!(a.customer_id), "{row}");
    assert_eq!(row["customer"], json!("Malee Rakdee"), "{row}");
    assert_eq!(row["currency"], json!("EUR"), "{row}");
    assert_eq!(row["type"], json!("BUY"), "{row}");
    assert_eq!(row["date"], json!("2026-10-05"), "{row}");
    assert_eq!(
        row["status"],
        json!("COMPLETED"),
        "v3 stores an exchange once saved: {row}"
    );
    assert_eq!(
        instant(row, "created"),
        a.at,
        "created is when the exchange was recorded: {row}"
    );
    for money in ["amount", "rate", "principal", "settlement"] {
        assert!(
            row[money].is_string(),
            "{money} is a decimal string, never a JSON number (W7): {row}"
        );
    }

    // a DEMO customer's exchanges are never listed, not even by its own id
    let demo = room
        .save(MAIN, &Who::new("Demo Person").demo(), Ex::usd())
        .await;
    assert!(
        room.history(&demo.customer_id, "").await.is_empty(),
        "LIVE only (design §12.5)"
    );
    room.finish().await;
}

#[actix_web::test]
async fn history_is_read_from_the_callers_branch_and_an_unknown_customer_has_none() {
    let room = Room::open().await;
    let main = room.save(MAIN, &Who::new("Main Person"), Ex::usd()).await;
    let nine = room.save(B9, &Who::new("Nine Person"), Ex::usd()).await;

    let q = |branch: &str, customer: &str| {
        format!("branch={branch}&limit=50&customer={}", enc(customer))
    };
    assert_eq!(
        room.ids("OWNER", HISTORY, &q(B9, &nine.customer_id)).await,
        vec![nine.id.clone()],
        "the owner naming B9"
    );
    assert!(
        room.ids("OWNER", HISTORY, &q(MAIN, &nine.customer_id))
            .await
            .is_empty(),
        "B9's customer has no exchange in MAIN"
    );
    // FRONT works in MAIN. OR-4b (D-4): naming B9 is refused; naming MAIN reads MAIN
    assert_out_of_scope(
        &room
            .get("FRONT", &view_path(HISTORY, &q(B9, &nine.customer_id)))
            .await,
        "FRONT naming B9",
    );
    assert!(
        room.ids("FRONT", HISTORY, &q(MAIN, &nine.customer_id))
            .await
            .is_empty(),
        "FRONT is answered from MAIN"
    );
    assert_eq!(
        room.ids("FRONT", HISTORY, &q(MAIN, &main.customer_id))
            .await,
        vec![main.id.clone()],
        "FRONT naming MAIN reads MAIN's customer"
    );
    assert!(
        room.history("CX-99999999", "").await.is_empty(),
        "an unknown customer: an empty page, not a refusal"
    );
    room.finish().await;
}

#[actix_web::test]
async fn history_pages_by_the_last_id_with_other_customers_in_between() {
    let room = Room::open().await;
    let malee = Who::new("Malee Rakdee");
    let mut saved = Vec::new();
    for i in 0..5 {
        saved.push(room.save(MAIN, &malee, Ex::usd()).await);
        room.save(MAIN, &Who::new(&format!("Other {i}")), Ex::usd())
            .await;
    }
    let customer = saved[0].customer_id.clone();
    let newest_first: Vec<String> = saved.iter().rev().map(|s| s.id.clone()).collect();
    let page = |before: Option<&str>| {
        let mut query = format!("branch=MAIN&limit=2&customer={}", enc(&customer));
        if let Some(b) = before {
            query.push_str(&format!("&before={}", enc(b)));
        }
        query
    };
    let first = room.ids("OWNER", HISTORY, &page(None)).await;
    assert_eq!(first, newest_first[0..2].to_vec(), "a full first page");
    let second = room.ids("OWNER", HISTORY, &page(Some(&first[1]))).await;
    assert_eq!(second, newest_first[2..4].to_vec(), "the next two");
    let third = room.ids("OWNER", HISTORY, &page(Some(&second[1]))).await;
    assert_eq!(
        third,
        newest_first[4..5].to_vec(),
        "the last one, a short page"
    );
    let after = room.ids("OWNER", HISTORY, &page(Some(&third[0]))).await;
    assert!(after.is_empty(), "nothing older: {after:?}");
    assert_eq!(room.history(&customer, "").await, newest_first, "limit 100");
    room.finish().await;
}

/// `customers.js:17`: the history is searched over the exchange's id, currency and date only.
#[actix_web::test]
async fn history_q_searches_id_currency_and_date_only() {
    let room = Room::open().await;
    let malee = Who::new("Malee Rakdee");
    let usd = room
        .save(
            MAIN,
            &malee,
            Ex {
                value_date: "2026-10-04",
                notes: "note-only-word",
                payment_ref: "PAYREF-ONLY",
                ..Ex::usd()
            },
        )
        .await;
    let eur = room
        .save(
            MAIN,
            &malee,
            Ex {
                currency: "EUR",
                value_date: "2026-10-05",
                ..Ex::usd()
            },
        )
        .await;
    let customer = usd.customer_id.clone();
    let cases: Vec<(String, Vec<String>, &str)> = vec![
        (usd.id.clone(), vec![usd.id.clone()], "the id"),
        (
            "eur".to_owned(),
            vec![eur.id.clone()],
            "the currency, case-folded",
        ),
        (
            "Usd".to_owned(),
            vec![usd.id.clone()],
            "the currency, case-folded",
        ),
        ("2026-10-04".to_owned(), vec![usd.id.clone()], "the date"),
        ("2026-10".to_owned(), rev(&[&usd, &eur]), "both dates"),
        (
            "Malee".to_owned(),
            vec![],
            "the customer's name is not searched",
        ),
        (
            "note-only-word".to_owned(),
            vec![],
            "the notes are not searched",
        ),
        (
            "PAYREF".to_owned(),
            vec![],
            "the payment reference is not searched",
        ),
        ("EMP-466".to_owned(), vec![], "the staff is not searched"),
        (
            "%".to_owned(),
            vec![],
            "a literal %, which nothing contains",
        ),
        (
            "_".to_owned(),
            vec![],
            "a literal _, which nothing contains",
        ),
        ("2026-10-0_".to_owned(), vec![], "_ is not a wildcard"),
    ];
    for (q, expected, what) in cases {
        let ids = room.history(&customer, &format!("q={}", enc(&q))).await;
        assert_eq!(ids, expected, "q={q:?}: {what}");
    }
    room.finish().await;
}

#[actix_web::test]
async fn history_refuses_q_longer_than_100_characters_naming_q() {
    let room = Room::open().await;
    let saved = room
        .save(MAIN, &Who::new("Somchai Jaidee"), Ex::usd())
        .await;
    let v = room.version(MAIN).await;
    let long = "a".repeat(101);
    let c = enc(&saved.customer_id);
    for query in [
        format!("branch=MAIN&customer={c}&q={long}"),
        format!("branch=MAIN&customer={c}&q={long}&since={v}"),
    ] {
        let answer = room.get("OWNER", &view_path(HISTORY, &query)).await;
        assert_refused(&answer, "invalidRequest", "q", "q of 101 characters");
    }
    assert!(
        room.history(&saved.customer_id, &format!("q={}", "a".repeat(100)))
            .await
            .is_empty(),
        "q of 100 characters is read"
    );
    room.finish().await;
}

/// W7: the list's `volume` is the sum of the history's `principal`s, both from the server — the
/// screen shows the two side by side (`customers.js:16-17`).
#[actix_web::test]
async fn the_volume_is_the_sum_of_the_principals_the_history_shows() {
    let room = Room::open().await;
    let malee = Who::new("Malee Rakdee");
    for amount in ["10.05", "10.05", "1234.57", "1.01"] {
        room.save(
            MAIN,
            &malee,
            Ex {
                amount,
                ..Ex::usd()
            },
        )
        .await;
    }
    let eur = room
        .save(
            MAIN,
            &malee,
            Ex {
                currency: "EUR",
                amount: "7.77",
                ..Ex::usd()
            },
        )
        .await;
    let rows = room
        .rows(
            "OWNER",
            HISTORY,
            &format!("branch=MAIN&limit=100&customer={}", enc(&eur.customer_id)),
        )
        .await;
    assert_eq!(rows.len(), 5, "{rows:?}");
    let sum: i128 = rows
        .iter()
        .map(|r| {
            satang(
                r["principal"]
                    .as_str()
                    .unwrap_or_else(|| panic!("principal: {r}")),
            )
        })
        .sum();
    let row = room.customer(&eur.customer_id).await;
    let volume = row["volume"]
        .as_str()
        .unwrap_or_else(|| panic!("volume: {row}"));
    assert_eq!(
        satang(volume),
        sum,
        "volume {volume} = Σ principal: {rows:?}"
    );
    // 322.61 + 322.61 + 39629.70 + 32.42 + 296.81
    assert_eq!(volume, "40604.15", "{row}");
    room.finish().await;
}

// ── §4.2: the version moves with every writer ────────────────────────────────

fn assert_unchanged(body: &Value, v: i64, what: &str) {
    assert_eq!(
        body,
        &json!({ "version": v, "changed": false }),
        "{what}: an equal since is exactly the version and changed: false"
    );
}

#[actix_web::test]
async fn a_customer_reservation_moves_the_version_admin_customers_answers() {
    let room = Room::open().await;
    let first = room.reserve(MAIN, &Who::new("First Person")).await;
    let v = room.version(MAIN).await;
    let since = format!("branch=MAIN&limit=50&since={v}");
    assert_unchanged(&room.view("OWNER", CUSTOMERS, &since).await, v, "before");

    // the customer service, the one writer of a new customer
    let second = room.reserve(MAIN, &Who::new("Alpha Second")).await;
    let now = room.version(MAIN).await;
    assert!(
        now > v,
        "the reservation moved MAIN's version ({v} → {now})"
    );
    let body = room.view("OWNER", CUSTOMERS, &since).await;
    assert_eq!(body["version"], json!(now), "{body}");
    assert_eq!(body["changed"], json!(true), "{body}");
    let ids: Vec<&str> = body["data"]
        .as_array()
        .unwrap_or_else(|| panic!("a list: {body}"))
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert_eq!(ids, vec![second.id.as_str(), first.id.as_str()], "{body}");
    room.finish().await;
}

#[actix_web::test]
async fn an_exchange_save_moves_the_version_both_views_answer() {
    let room = Room::open().await;
    let malee = Who::new("Malee Rakdee");
    let first = room.save(MAIN, &malee, Ex::usd()).await;
    let v = room.version(MAIN).await;
    let c = enc(&first.customer_id);
    let list = format!("branch=MAIN&limit=50&since={v}");
    let history = format!("branch=MAIN&limit=50&customer={c}&since={v}");
    assert_unchanged(&room.view("OWNER", CUSTOMERS, &list).await, v, "list");
    assert_unchanged(&room.view("OWNER", HISTORY, &history).await, v, "history");

    let second = room.save(MAIN, &malee, Ex::usd()).await;
    let now = room.version(MAIN).await;
    assert!(now > v, "the save moved MAIN's version ({v} → {now})");

    let body = room.view("OWNER", CUSTOMERS, &list).await;
    assert_eq!(body["version"], json!(now), "{body}");
    assert_eq!(body["changed"], json!(true), "{body}");
    assert_eq!(body["data"][0]["completed"], json!(2), "{body}");

    let body = room.view("OWNER", HISTORY, &history).await;
    assert_eq!(body["version"], json!(now), "{body}");
    assert_eq!(body["changed"], json!(true), "{body}");
    let ids: Vec<&str> = body["data"]
        .as_array()
        .unwrap_or_else(|| panic!("a list: {body}"))
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert_eq!(ids, vec![second.id.as_str(), first.id.as_str()], "{body}");
    room.finish().await;
}

/// Re-pinned for BK-10 #1412 by claude/29df2ec5 (ruling 1, issuecomment-6140233542): the writer of
/// the `member` flag is now the phone verification (`customerContactVerify`), not the member issue.
#[actix_web::test]
async fn verifying_a_phone_moves_the_version_admin_customers_answers() {
    let room = Room::open().await;
    let somchai = room.reserve(MAIN, &Who::new("Somchai Jaidee")).await;
    let v = room.version(MAIN).await;
    let since = format!("branch=MAIN&limit=50&since={v}");
    assert_unchanged(&room.view("OWNER", CUSTOMERS, &since).await, v, "before");

    // the phone verification, the one writer of the member flag
    room.verify_phone(MAIN, &somchai.id).await;
    let now = room.version(MAIN).await;
    assert!(
        now > v,
        "verifying a phone moved MAIN's version ({v} → {now})"
    );
    let body = room.view("OWNER", CUSTOMERS, &since).await;
    assert_eq!(body["changed"], json!(true), "{body}");
    assert_eq!(body["data"][0]["id"], json!(somchai.id), "{body}");
    assert_eq!(body["data"][0]["member"], json!(true), "{body}");
    room.finish().await;
}

#[actix_web::test]
async fn demo_exchanges_are_not_counted_in_completed_volume_or_last_activity() {
    let room = Room::open().await;
    let somchai = room.reserve(MAIN, &Who::new("Somchai Demo")).await;

    let demo_ref = format!("TX-DEMO-{}", next());
    sqlx::query(
        "INSERT INTO transactions (document_ref, branch_id, mode, submission_id, payload_hash, customer_id, customer_name, \
             nationality, staff_id, booking_no, notes, value_date, paid_at, recorded_at, recorded_by, \
             completed_at) \
         VALUES ($1, $2, 'DEMO', $3, 'payload-hash', $4, 'Somchai Demo', 'THA', 'owner', '', '', \
             DATE '2026-10-06', now(), now(), 'owner', now())",
    )
    .bind(&demo_ref)
    .bind(MAIN)
    .bind(format!("submission-demo-{}", next()))
    .bind(&somchai.id)
    .execute(&room.desk.db.pool)
    .await
    .expect("insert demo transaction");
    seed_lines(
        &room.desk.db.pool,
        &[demo_ref],
        ("SELL", "USD", "1000.00", "32.0", "32000.00"),
    )
    .await;

    let row = room.customer(&somchai.id).await;
    assert_eq!(
        row["completed"],
        json!(0),
        "DEMO exchanges are not counted in completed"
    );
    assert_eq!(
        row["volume"],
        json!("0.00"),
        "DEMO exchanges are not counted in volume"
    );
    assert_eq!(
        row["lastAt"],
        Value::Null,
        "DEMO exchanges do not update lastAt"
    );

    let hist = room.history(&somchai.id, "").await;
    assert!(hist.is_empty(), "DEMO transactions are not in history");
    room.finish().await;
}

/// Re-pinned for BK-10 #1412 by claude/29df2ec5 (ruling 1, issuecomment-6140233542): the flag was
/// per branch (v2's member table of that branch); it is now company-wide (design §6, PR 639 K1).
/// A customer with no verified identity flags nobody anywhere; a phone verified at B9 flags the
/// customer at MAIN and at B9.
#[actix_web::test]
async fn member_flag_is_company_wide() {
    let room = Room::open().await;
    let somchai = room.reserve(MAIN, &Who::new("Somchai Cross Branch")).await;

    let row_before = room.customer(&somchai.id).await;
    assert_eq!(
        row_before["member"],
        json!(false),
        "a customer with no verified identity is not flagged in MAIN"
    );

    room.verify_phone(B9, &somchai.id).await;
    let row_after = room.customer(&somchai.id).await;
    assert_eq!(
        row_after["member"],
        json!(true),
        "a phone verified at B9 flags the customer in MAIN"
    );
    let in_b9 = room
        .rows("OWNER", CUSTOMERS, "branch=B9&limit=100")
        .await
        .into_iter()
        .find(|r| r["id"] == json!(somchai.id))
        .unwrap_or_else(|| panic!("{} among B9's customers", somchai.id));
    assert_eq!(in_b9["member"], json!(true), "and in B9: {in_b9}");
    room.finish().await;
}

#[actix_web::test]
async fn volume_never_prints_hash_fill_on_very_large_sums() {
    let room = Room::open().await;
    let rich = room.reserve(MAIN, &Who::new("Rich Customer")).await;
    let batch = next() as i64;

    sqlx::query(
        "INSERT INTO transactions (document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
             customer_id, customer_name, nationality, staff_id, booking_no, notes, \
             value_date, paid_at, recorded_at, recorded_by, completed_at) \
         SELECT 'TX-LARGE-' || $2 || '-' || n, $3, 'LIVE', \
             (SELECT id FROM shifts WHERE branch_id = $3 LIMIT 1), \
             'sub-' || $2 || '-' || n, 'hash', \
             $1, 'Rich Customer', 'THA', \
             'owner', '', '', DATE '2026-10-06', now(), now(), 'owner', now() \
         FROM generate_series(1, 150) AS n",
    )
    .bind(&rich.id)
    .bind(batch)
    .bind(MAIN)
    .execute(&room.desk.db.pool)
    .await
    .expect("insert 150 large transactions");
    let large: Vec<String> = (1..=150).map(|n| format!("TX-LARGE-{batch}-{n}")).collect();
    seed_lines(
        &room.desk.db.pool,
        &large,
        (
            "SELL",
            "USD",
            "9000000000000000.00",
            "100.0",
            "900000000000000000.00",
        ),
    )
    .await;

    let row = room.customer(&rich.id).await;
    let volume = row["volume"].as_str().expect("volume is a string");
    assert!(
        !volume.contains('#'),
        "volume must never contain '#' fill: got {volume}"
    );
    assert_eq!(volume, "135000000000000000000.00");
    room.finish().await;
}

#[actix_web::test]
async fn full_width_wildcards_are_treated_as_literals_not_wildcards() {
    let room = Room::open().await;
    let plain = room.reserve(MAIN, &Who::new("Alice Normal")).await;
    let marked = room.reserve(MAIN, &Who::new("Bob 100% Cotton")).await;

    // q = '％' (full-width percent) normalises to '%' in NFKC.
    // Must be treated as a literal search for '%', matching only Bob and not Alice.
    let ids_percent = room.customers(&format!("q={}", enc("％"))).await;
    assert_eq!(
        ids_percent,
        ids_of(&[&marked]),
        "q=％ must match only records containing literal %, never everyone"
    );
    assert!(
        !ids_percent.contains(&plain.id),
        "Alice Normal must not match literal %"
    );

    // Full-width underscore and backslash match nothing when no customer has '_' or '\'
    let ids_underscore = room.customers(&format!("q={}", enc("＿"))).await;
    assert!(
        ids_underscore.is_empty(),
        "q=＿ must match nothing, got {ids_underscore:?}"
    );

    let ids_backslash = room.customers(&format!("q={}", enc("＼"))).await;
    assert!(
        ids_backslash.is_empty(),
        "q=＼ must match nothing, got {ids_backslash:?}"
    );

    room.finish().await;
}

/// The `transaction_lines` + `transaction_payments` rows of the exchanges `refs`, cash, no fee,
/// all with the same `(side, code, quantity, rate per 1, thb)` (TX-9 #677: the figures live on the
/// line and the payment only; `transactions` keeps the header).
async fn seed_lines(
    pool: &sqlx::PgPool,
    refs: &[String],
    (side, code, quantity, rate, thb): (&str, &str, &str, &str, &str),
) {
    sqlx::query(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, quote_line_id, \
            shop_side, currency, denomination_id, quantity, unit_size, rate_snapshot, \
            reference_rate, thb_amount, fee_thb, settle_thb, cost_basis_thb, \
            realized_margin_thb, line_no, created_at, updated_at) \
         SELECT gen_random_uuid(), t.organization_id, t.id, NULL, $2, $3, NULL, $4::numeric, 1, \
            $5::numeric, NULL, $6::numeric, 0, $6::numeric, NULL, NULL, 1, \
            t.recorded_at, t.recorded_at \
         FROM transactions t WHERE t.document_ref = ANY($1)",
    )
    .bind(refs)
    .bind(side)
    .bind(code)
    .bind(quantity)
    .bind(rate)
    .bind(thb)
    .execute(pool)
    .await
    .expect("seed transaction_lines");
    sqlx::query(
        "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, \
            method, reference, account, slip_file_key, amount_thb, bank_fee_thb, paid_at, \
            created_at, updated_at) \
         SELECT gen_random_uuid(), t.organization_id, t.id, 1, 'Cash', '', '', NULL, \
            $2::numeric, 0, t.paid_at, t.recorded_at, t.recorded_at \
         FROM transactions t WHERE t.document_ref = ANY($1)",
    )
    .bind(refs)
    .bind(thb)
    .execute(pool)
    .await
    .expect("seed transaction_payments");
}
