//! Ticket #465 (V2-slips, `docs/design/xchange-v3-web.md` §4.2, §5.1, §12.1, §12.14): the paged
//! view `admin_slips`. Written before the implementation by the lane controller (claude/7ca80c).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! - `GET /api/v3/views/admin_slips` — a `PagedView` inheriting `(EntryPath::Rpc, "adminSnapshot")`
//!   (ticket §7): the LIVE exchanges of the branch **with** a slip, newest first, keyset paged by
//!   the row's `id` (`before`), each row the shared `TransactionRow` (V2-parts #476).
//! - Filter (design §12.1 row `admin_slips`, v2 `src/online/public/slips.js:4`): `from`, `to`
//!   (the exchange's `date`, both inclusive, `Period`); `q`, a literal substring over id, customer,
//!   customer id, currency and payment reference, NFKC-normalised and case-folded on both sides;
//!   `status` = `COMPLETED` | `PENDING` | `CANCELLED`. Filtering happens in SQL: a filtered page is
//!   full when enough rows match. `q` longer than 100 characters is `400 invalidRequest`,
//!   `field: "q"`.
//! - §4.2: the one writer of what the view reads (`transactions`, `transaction_files`) is the
//!   exchange save (`SaveTransactionWorkflow` → `TransactionService::record_in`, inside its
//!   `BranchTx`): one save, then the same `since` answers `changed: true`.
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real
//! sessions made at the real sign-in doors (`rpc_common`); exchanges are saved through the real
//! `SaveTransactionWorkflow` behind the routes, as `v3_admin_parts_views_test.rs` does. The test's
//! clock is advanced before every write, so no two exchanges share a `created`.
//!
//! The figures of a row (`principal`, `settlement`, the reference figures) are not pinned here:
//! the row's constructor and its figures are the first exchange-listing view's to test (note on
//! #464). This file pins which rows, in which order, and the fields the slips table shows.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing skips.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
mod rpc_common;
mod snapshot_common;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, Utc};
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
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

const ORIGIN: &str = "http://desk.test";
const VIEW: &str = "admin_slips";

const MAIN: &str = "MAIN";
const B9: &str = "B9";
const B9_NAME: &str = "Nine";

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];
const PDF: &[u8] = b"%PDF-1.7 a slip\n";

/// `TransactionRow`'s fields (design §12.2), the shape every row of this view has.
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

fn view_path(query: &str) -> String {
    if query.is_empty() {
        format!("/api/v3/views/{VIEW}")
    } else {
        format!("/api/v3/views/{VIEW}?{query}")
    }
}

/// One exchange to save. `slip` = a payment slip attached.
#[derive(Clone)]
struct Spec {
    branch: &'static str,
    slip: bool,
    customer: String,
    passport: String,
    currency: &'static str,
    payment_ref: String,
    value_date: &'static str,
    notes: &'static str,
    demo: bool,
}

impl Spec {
    /// A LIVE purchase in `branch`, paid by transfer with a slip, valued 2026-10-06.
    fn slip(branch: &'static str) -> Spec {
        let n = next();
        Spec {
            branch,
            slip: true,
            customer: format!("Somchai Jaidee {n}"),
            passport: format!("AA{n:07}"),
            currency: "USD",
            payment_ref: format!("TRX-{n}"),
            value_date: "2026-10-06",
            notes: "",
            demo: false,
        }
    }

    /// The same, paid in cash with no slip.
    fn cash(branch: &'static str) -> Spec {
        Spec {
            slip: false,
            payment_ref: String::new(),
            ..Spec::slip(branch)
        }
    }
}

/// What a save left behind.
#[derive(Clone)]
struct Saved {
    id: String,
    customer_id: String,
    slip: Option<String>,
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

    /// The view as `who`, which must be answered 200 with JSON, never cached.
    async fn view(&self, who: &str, query: &str) -> Value {
        let path = view_path(query);
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
    async fn rows(&self, who: &str, query: &str) -> Vec<Value> {
        let body = self.view(who, query).await;
        assert_eq!(
            body["changed"],
            json!(true),
            "GET {}: {body}",
            view_path(query)
        );
        body.get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| panic!("GET {}: a list in data: {body}", view_path(query)))
    }

    /// The ids of a page, in the order answered.
    async fn ids(&self, who: &str, query: &str) -> Vec<String> {
        self.rows(who, query)
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

    async fn version(&self, branch: &str) -> i64 {
        BranchVersions::new(self.desk.db.pool.clone())
            .current(&BranchId::new(branch))
            .await
            .expect("read the branch version")
    }

    async fn set_rate(&self, branch: &str, code: &str, buy: &str, sell: &str) {
        sqlx::query(
            "INSERT INTO rates (branch_id, code, buy, sell, updated_at, updated_by, request_id) \
             VALUES ($1, $2, $3::numeric, $4::numeric, now(), 'test', $5)",
        )
        .bind(branch)
        .bind(code)
        .bind(buy)
        .bind(sell)
        .bind(request_id())
        .execute(&self.desk.db.pool)
        .await
        .expect("insert a counter rate");
        // #759 (RE-4e; rates.md D-3, §4.9 R4): the counter rate is the current publication.
        rates_re4e_publish::publish_newest_rates(&self.desk.db.pool, branch).await;
    }

    async fn open_shift(&self, branch: &str) {
        self.tick();
        let call = Call::action(
            ORIGIN,
            rpc_common::RPC,
            "adminOpenShift",
            json!({ "branchId": branch, "requestId": request_id(),
                    "opening": { "THB": 100000, "USD": 1000, "EUR": 1000 } }),
        )
        .cookie(&self.cookie("OWNER"));
        let answer = self.app.send(call).await;
        assert_eq!(
            answer.status, 200,
            "open a shift in {branch}: {:.400}",
            answer.text
        );
    }

    /// Saves one exchange through the real workflow behind the routes.
    async fn save(&self, spec: Spec) -> Saved {
        let t = self.tick();
        let here = BranchId::new(spec.branch);
        let mode = if spec.demo { "DEMO" } else { "LIVE" };
        let person = ReserveCustomer {
            name: spec.customer.clone(),
            passport_no: spec.passport.clone(),
            nationality: "THA".to_owned(),
            birth_date: String::new(),
            passport_expiry: String::new(),
            mode: mode.to_owned(),
        };
        let customer = CustomerService::new(self.desk.db.branch_tx())
            .reserve(&here, person.clone(), t)
            .await
            .expect("reserve the customer")
            .customer_id;
        let passport = self
            .desk
            .files
            .upload(&here, "owner", "image/jpeg", JPEG, t)
            .await
            .expect("upload a passport");
        let slip = if spec.slip {
            Some(
                self.desk
                    .files
                    .upload(&here, "owner", "application/pdf", PDF, t)
                    .await
                    .expect("upload a slip"),
            )
        } else {
            None
        };
        let transfer = !spec.payment_ref.is_empty();
        let cmd = SaveTransaction {
            customer: person,
            submission_id: format!("submission-{}-{:012}", spec.branch, next()),
            customer_id: customer.clone(),
            side: "BUY".to_owned(),
            currency: spec.currency.to_owned(),
            amount: "100".to_owned(),
            rate: if spec.currency == "EUR" {
                "38.2"
            } else {
                "32.1"
            }
            .to_owned(),
            fee: "0".to_owned(),
            bank_fee: "0".to_owned(),
            method: if transfer { "Bank Transfer" } else { "Cash" }.to_owned(),
            payment_ref: spec.payment_ref.clone(),
            account: "Cash drawer 1".to_owned(),
            staff: "EMP-001".to_owned(),
            booking: "WALK-IN".to_owned(),
            notes: spec.notes.to_owned(),
            value_date: spec.value_date.to_owned(),
            paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
            payment_confirmed: true,
            identity_confirmed: true,
            passport: Some(passport),
            slip: slip.clone(),
            reference_rate: None,
        };
        let saved = self
            .desk
            .workflow
            .save(&here, cmd, "owner", t)
            .await
            .unwrap_or_else(|e| panic!("save an exchange in {}: {e:?}", spec.branch));
        Saved {
            id: saved.document_ref,
            customer_id: customer,
            slip,
            at: t,
        }
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// The actor `Policy` sees for each caller key of [`Room`] on a path that is not `StaffRpc`.
fn callers() -> Vec<(String, Option<Actor>)> {
    let mut callers: Vec<(String, Option<Actor>)> = vec![
        (String::new(), None),
        ("OWNER".to_string(), Some(Actor::owner("owner"))),
    ];
    for role in Role::STAFF {
        let staff = Actor::staff("employee", role, BranchId::new(MAIN));
        callers.push((role.as_str().to_string(), staff));
        callers.push((
            format!("OWNER+{}", role.as_str()),
            Some(Actor::owner("owner")),
        ));
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

// ── registration and who may read ────────────────────────────────────────────

#[test]
fn admin_slips_is_in_the_generated_view_table() {
    assert!(
        VIEWS.iter().any(|v| v.name == VIEW),
        "{VIEW} is listed by the generator from http/v3/views/ (no shared file edited): {:?}",
        VIEWS.iter().map(|v| v.name).collect::<Vec<_>>()
    );
}

#[actix_web::test]
async fn admin_slips_answers_exactly_whom_the_snapshot_row_allows() {
    let room = Room::open().await;
    room.save(Spec::slip(MAIN)).await;
    let mut allowed = 0;
    let mut refused = 0;
    for (who, actor) in callers() {
        let policy = Policy::allow(
            actor.as_ref(),
            EntryPath::Rpc,
            &Action::new("adminSnapshot"),
        );
        let answer = room.get(&who, &view_path("limit=50")).await;
        let what = format!("{VIEW} as {who:?}");
        match &policy {
            Ok(()) => {
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
    assert!(allowed > 0, "someone may read it");
    assert!(refused > 0, "nobody signed in is refused");
    room.finish().await;
}

// ── which rows ───────────────────────────────────────────────────────────────

#[actix_web::test]
async fn admin_slips_lists_only_the_exchanges_with_a_slip_newest_first() {
    let room = Room::open().await;
    let a = room.save(Spec::slip(MAIN)).await;
    let _cash1 = room.save(Spec::cash(MAIN)).await;
    let b = room.save(Spec::slip(MAIN)).await;
    // a cash payment may still carry a slip: it is listed
    let c = room
        .save(Spec {
            payment_ref: String::new(),
            ..Spec::slip(MAIN)
        })
        .await;
    let _cash2 = room.save(Spec::cash(MAIN)).await;

    let ids = room.ids("OWNER", "branch=MAIN&limit=50").await;
    assert_eq!(ids, rev(&[&a, &b, &c]), "slips only, newest first");
    room.finish().await;
}

#[actix_web::test]
async fn admin_slips_row_is_the_transaction_row_with_the_fields_the_slips_table_shows() {
    let room = Room::open().await;
    let saved = room
        .save(Spec {
            customer: "Malee Rakdee".to_owned(),
            currency: "EUR",
            payment_ref: "KB-778899".to_owned(),
            value_date: "2026-10-05",
            ..Spec::slip(MAIN)
        })
        .await;
    let rows = room.rows("OWNER", "branch=MAIN&limit=50").await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(
        keys(row),
        set_of(&ROW_FIELDS),
        "the shared TransactionRow: {row}"
    );
    assert_eq!(row["id"], json!(saved.id), "{row}");
    assert_eq!(row["customer"], json!("Malee Rakdee"), "{row}");
    assert_eq!(row["customerId"], json!(saved.customer_id), "{row}");
    assert_eq!(row["currency"], json!("EUR"), "{row}");
    assert_eq!(row["type"], json!("BUY"), "{row}");
    assert_eq!(row["date"], json!("2026-10-05"), "{row}");
    assert_eq!(
        row["status"],
        json!("COMPLETED"),
        "v3 stores an exchange once saved: {row}"
    );
    assert_eq!(row["paymentRef"], json!("KB-778899"), "{row}");
    let slip = saved.slip.as_deref().expect("saved with a slip");
    assert_eq!(
        row["slipUrl"],
        json!(format!("/files/{slip}")),
        "the slip is a file under /files/ (slips.js:5): {row}"
    );
    let created = row["created"]
        .as_str()
        .unwrap_or_else(|| panic!("created is a string: {row}"));
    let created = DateTime::parse_from_rfc3339(created)
        .unwrap_or_else(|e| panic!("created is an RFC 3339 instant ({e}): {row}"));
    assert_eq!(
        created.with_timezone(&Utc),
        saved.at,
        "created is when the exchange was recorded: {row}"
    );
    for money in [
        "amount",
        "rate",
        "principal",
        "fee",
        "bankFee",
        "settlement",
    ] {
        assert!(
            row[money].is_string(),
            "{money} is a decimal string, never a JSON number (W7): {row}"
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn admin_slips_never_lists_a_demo_exchange() {
    let room = Room::open().await;
    let live = room.save(Spec::slip(MAIN)).await;
    let demo = room
        .save(Spec {
            demo: true,
            ..Spec::slip(MAIN)
        })
        .await;
    let ids = room.ids("OWNER", "branch=MAIN&limit=50").await;
    assert_eq!(ids, vec![live.id.clone()], "LIVE only (design §12.5)");
    let ids = room
        .ids(
            "OWNER",
            &format!("branch=MAIN&limit=50&q={}", enc(&demo.id)),
        )
        .await;
    assert!(
        ids.is_empty(),
        "the DEMO exchange is not found by its id: {ids:?}"
    );
    room.finish().await;
}

#[actix_web::test]
async fn admin_slips_answers_staff_from_their_own_branch_and_the_owner_from_the_branch_named() {
    let room = Room::open().await;
    let main = room.save(Spec::slip(MAIN)).await;
    let nine = room.save(Spec::slip(B9)).await;

    // FRONT works in MAIN: naming B9 changes nothing
    let ids = room.ids("FRONT", "branch=B9&limit=50").await;
    assert_eq!(ids, vec![main.id.clone()], "FRONT is answered from MAIN");
    let ids = room.ids("OWNER", "branch=B9&limit=50").await;
    assert_eq!(ids, vec![nine.id.clone()], "the owner naming B9");
    let ids = room.ids("OWNER", "branch=MAIN&limit=50").await;
    assert_eq!(ids, vec![main.id.clone()], "the owner naming MAIN");
    room.finish().await;
}

// ── paging ───────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn admin_slips_pages_by_the_last_id_with_rows_without_a_slip_in_between() {
    let room = Room::open().await;
    let mut slips = Vec::new();
    for _ in 0..5 {
        slips.push(room.save(Spec::slip(MAIN)).await);
        room.save(Spec::cash(MAIN)).await;
    }
    let newest_first: Vec<String> = slips.iter().rev().map(|s| s.id.clone()).collect();

    let first = room.ids("OWNER", "branch=MAIN&limit=2").await;
    assert_eq!(
        first,
        newest_first[0..2].to_vec(),
        "a page of 2 is 2 slips, the slip-less rows between them do not count"
    );
    let second = room
        .ids(
            "OWNER",
            &format!("branch=MAIN&limit=2&before={}", enc(&first[1])),
        )
        .await;
    assert_eq!(second, newest_first[2..4].to_vec(), "the next two");
    let third = room
        .ids(
            "OWNER",
            &format!("branch=MAIN&limit=2&before={}", enc(&second[1])),
        )
        .await;
    assert_eq!(
        third,
        newest_first[4..5].to_vec(),
        "the last one, a short page"
    );
    let after = room
        .ids(
            "OWNER",
            &format!("branch=MAIN&limit=2&before={}", enc(&third[0])),
        )
        .await;
    assert!(after.is_empty(), "nothing older: {after:?}");

    let all = room.ids("OWNER", "branch=MAIN&limit=100").await;
    assert_eq!(all, newest_first, "limit 100 is allowed (design §4.2)");
    let answer = room.get("OWNER", &view_path("branch=MAIN&limit=101")).await;
    assert_eq!(answer.status, 400, "limit above 100: {:.300}", answer.text);
    room.finish().await;
}

// ── the filter, in SQL ───────────────────────────────────────────────────────

#[actix_web::test]
async fn admin_slips_q_searches_id_customer_customer_id_currency_and_payment_reference() {
    let room = Room::open().await;
    let usd = room
        .save(Spec {
            customer: "Somsri Boonmee".to_owned(),
            payment_ref: "Pay-Ref-Alpha".to_owned(),
            ..Spec::slip(MAIN)
        })
        .await;
    let eur = room
        .save(Spec {
            customer: "Anan Chaiyo".to_owned(),
            currency: "EUR",
            payment_ref: "Pay-Ref-Beta".to_owned(),
            notes: "note-only-word",
            ..Spec::slip(MAIN)
        })
        .await;

    let cases: Vec<(String, Vec<String>, &str)> = vec![
        (usd.id.clone(), vec![usd.id.clone()], "the id"),
        ("Somsri".to_owned(), vec![usd.id.clone()], "the customer"),
        (
            eur.customer_id.clone(),
            vec![eur.id.clone()],
            "the customer id",
        ),
        ("EUR".to_owned(), vec![eur.id.clone()], "the currency"),
        (
            "Ref-Alpha".to_owned(),
            vec![usd.id.clone()],
            "the payment reference",
        ),
        (
            "Pay-Ref".to_owned(),
            rev(&[&usd, &eur]),
            "both payment references",
        ),
        (
            "note-only-word".to_owned(),
            vec![],
            "the notes are not searched",
        ),
        ("EMP-001".to_owned(), vec![], "the staff is not searched"),
        (
            "Bank Transfer".to_owned(),
            vec![],
            "the method is not searched",
        ),
        ("no-such-thing".to_owned(), vec![], "nothing matches"),
    ];
    for (q, expected, what) in cases {
        let ids = room
            .ids("OWNER", &format!("branch=MAIN&limit=50&q={}", enc(&q)))
            .await;
        assert_eq!(ids, expected, "q={q:?}: {what}");
    }
    room.finish().await;
}

#[actix_web::test]
async fn admin_slips_q_is_case_folded_and_nfkc_normalised_on_both_sides() {
    let room = Room::open().await;
    let latin = room
        .save(Spec {
            customer: "Somchai Jaidee".to_owned(),
            payment_ref: "Trx-Ab12".to_owned(),
            ..Spec::slip(MAIN)
        })
        .await;
    let composed = room
        .save(Spec {
            customer: KAMRAI_COMPOSED.to_owned(),
            ..Spec::slip(MAIN)
        })
        .await;
    let decomposed = room
        .save(Spec {
            customer: KAMRAI_DECOMPOSED.to_owned(),
            ..Spec::slip(MAIN)
        })
        .await;

    let cases: Vec<(&str, Vec<String>, &str)> = vec![
        (
            "SOMCHAI",
            vec![latin.id.clone()],
            "upper case finds mixed case",
        ),
        (
            "trx-ab12",
            vec![latin.id.clone()],
            "lower case finds mixed case",
        ),
        // full-width Latin letters (U+FF34 …) are TRX under NFKC
        (
            "\u{FF34}\u{FF32}\u{FF38}-ab",
            vec![latin.id.clone()],
            "full-width",
        ),
        (
            KAM_DECOMPOSED,
            rev(&[&composed, &decomposed]),
            "the decomposed Thai finds the stored composed ำ (NFKC on the stored side)",
        ),
        (
            KAM_COMPOSED,
            rev(&[&composed, &decomposed]),
            "the composed ำ finds the stored decomposed form (NFKC on the query side)",
        ),
    ];
    for (q, expected, what) in cases {
        let ids = room
            .ids("OWNER", &format!("branch=MAIN&limit=50&q={}", enc(q)))
            .await;
        assert_eq!(ids, expected, "q={q:?}: {what}");
    }
    room.finish().await;
}

#[actix_web::test]
async fn admin_slips_q_is_a_literal_substring_not_a_pattern() {
    let room = Room::open().await;
    let plain = room
        .save(Spec {
            customer: "Wanida Sukjai".to_owned(),
            payment_ref: "REF-PLAIN".to_owned(),
            ..Spec::slip(MAIN)
        })
        .await;
    let marked = room
        .save(Spec {
            customer: "Pim 50%_off\\x".to_owned(),
            payment_ref: "REF-MARKED".to_owned(),
            ..Spec::slip(MAIN)
        })
        .await;
    let cases: Vec<(&str, Vec<String>)> = vec![
        ("%", vec![marked.id.clone()]),
        ("_", vec![marked.id.clone()]),
        ("\\", vec![marked.id.clone()]),
        ("50%_off\\x", vec![marked.id.clone()]),
        ("Wanida%", vec![]),
        ("Wan_da", vec![]),
        ("REF-", rev(&[&plain, &marked])),
    ];
    for (q, expected) in cases {
        let ids = room
            .ids("OWNER", &format!("branch=MAIN&limit=50&q={}", enc(q)))
            .await;
        assert_eq!(ids, expected, "q={q:?} is matched literally");
    }
    room.finish().await;
}

#[actix_web::test]
async fn admin_slips_refuses_q_longer_than_100_characters_naming_q() {
    let room = Room::open().await;
    let v = room.version(MAIN).await;
    let long = "a".repeat(101);
    for query in [
        format!("branch=MAIN&q={long}"),
        format!("branch=MAIN&q={long}&since={v}"),
    ] {
        let answer = room.get("OWNER", &view_path(&query)).await;
        assert_refused(&answer, "invalidRequest", "q", "q of 101 characters");
    }
    let ids = room
        .ids("OWNER", &format!("branch=MAIN&q={}", "a".repeat(100)))
        .await;
    assert!(ids.is_empty(), "q of 100 characters is read: {ids:?}");
    room.finish().await;
}

#[actix_web::test]
async fn admin_slips_status_filters_by_the_exchange_status() {
    let room = Room::open().await;
    let a = room.save(Spec::slip(MAIN)).await;
    let b = room.save(Spec::slip(MAIN)).await;
    let all = rev(&[&a, &b]);
    for (status, expected) in [
        ("", all.clone()),
        ("COMPLETED", all.clone()),
        ("PENDING", vec![]),
        ("CANCELLED", vec![]),
    ] {
        let ids = room
            .ids("OWNER", &format!("branch=MAIN&limit=50&status={status}"))
            .await;
        assert_eq!(
            ids, expected,
            "status={status:?} (every v3 exchange is COMPLETED)"
        );
    }
    for bad in ["DONE", "completed", "BUY"] {
        let answer = room
            .get("OWNER", &view_path(&format!("branch=MAIN&status={bad}")))
            .await;
        assert_refused(&answer, "invalidRequest", "status", bad);
    }
    room.finish().await;
}

#[actix_web::test]
async fn admin_slips_period_is_the_exchange_date_both_ends_inclusive() {
    let room = Room::open().await;
    let d4 = room
        .save(Spec {
            value_date: "2026-10-04",
            ..Spec::slip(MAIN)
        })
        .await;
    let d5 = room
        .save(Spec {
            value_date: "2026-10-05",
            ..Spec::slip(MAIN)
        })
        .await;
    let d6 = room
        .save(Spec {
            value_date: "2026-10-06",
            ..Spec::slip(MAIN)
        })
        .await;
    for (query, expected) in [
        ("from=2026-10-05&to=2026-10-05", vec![d5.id.clone()]),
        ("from=2026-10-05", rev(&[&d5, &d6])),
        ("to=2026-10-05", rev(&[&d4, &d5])),
        ("from=2026-10-04&to=2026-10-06", rev(&[&d4, &d5, &d6])),
        ("from=&to=", rev(&[&d4, &d5, &d6])),
        ("from=2026-10-07", vec![]),
    ] {
        let ids = room
            .ids("OWNER", &format!("branch=MAIN&limit=50&{query}"))
            .await;
        assert_eq!(ids, expected, "{query}");
    }
    let answer = room
        .get(
            "OWNER",
            &view_path("branch=MAIN&from=2026-10-06&to=2026-10-05"),
        )
        .await;
    assert_refused(&answer, "periodOrder", "to", "from after to");
    let answer = room
        .get("OWNER", &view_path("branch=MAIN&from=2026-13-01"))
        .await;
    assert_refused(&answer, "invalidRequest", "from", "a malformed from");
    room.finish().await;
}

#[actix_web::test]
async fn admin_slips_filters_combine_and_a_filtered_page_is_full() {
    let room = Room::open().await;
    let mut wanted = Vec::new();
    for i in 0..4 {
        // between every two wanted rows: one slip of another customer, one of another day, one
        // without a slip — a page filtered after it was read would come back short
        wanted.push(
            room.save(Spec {
                customer: format!("Kanya Target {i}"),
                value_date: "2026-10-05",
                ..Spec::slip(MAIN)
            })
            .await,
        );
        room.save(Spec {
            customer: format!("Other Person {i}"),
            value_date: "2026-10-05",
            ..Spec::slip(MAIN)
        })
        .await;
        room.save(Spec {
            customer: format!("Kanya Target late {i}"),
            value_date: "2026-10-06",
            ..Spec::slip(MAIN)
        })
        .await;
        room.save(Spec {
            customer: format!("Kanya Target cash {i}"),
            value_date: "2026-10-05",
            ..Spec::cash(MAIN)
        })
        .await;
    }
    let newest_first: Vec<String> = wanted.iter().rev().map(|s| s.id.clone()).collect();
    let filter = "q=kanya%20target&status=COMPLETED&from=2026-10-05&to=2026-10-05";

    let first = room
        .ids("OWNER", &format!("branch=MAIN&limit=2&{filter}"))
        .await;
    assert_eq!(first, newest_first[0..2].to_vec(), "a full first page");
    let second = room
        .ids(
            "OWNER",
            &format!("branch=MAIN&limit=2&{filter}&before={}", enc(&first[1])),
        )
        .await;
    assert_eq!(second, newest_first[2..4].to_vec(), "a full second page");
    let ids = room
        .ids(
            "OWNER",
            "branch=MAIN&limit=50&q=kanya%20target&status=PENDING",
        )
        .await;
    assert!(ids.is_empty(), "status is ANDed with q: {ids:?}");
    room.finish().await;
}

// ── §4.2: the version moves with every writer ────────────────────────────────

#[actix_web::test]
async fn an_exchange_save_moves_the_version_admin_slips_answers() {
    let room = Room::open().await;
    let first = room.save(Spec::slip(MAIN)).await;
    let v = room.version(MAIN).await;
    let body = room
        .view("OWNER", &format!("branch=MAIN&limit=50&since={v}"))
        .await;
    assert_eq!(
        body,
        json!({ "version": v, "changed": false }),
        "an equal since: exactly the version and changed: false"
    );

    // the one writer of transactions / transaction_files, with a slip
    let second = room.save(Spec::slip(MAIN)).await;
    let now = room.version(MAIN).await;
    assert!(now > v, "the save moved MAIN's version ({v} → {now})");
    let body = room
        .view("OWNER", &format!("branch=MAIN&limit=50&since={v}"))
        .await;
    assert_eq!(body["version"], json!(now), "{body}");
    assert_eq!(body["changed"], json!(true), "{body}");
    let ids: Vec<&str> = body["data"]
        .as_array()
        .unwrap_or_else(|| panic!("a list: {body}"))
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert_eq!(ids, vec![second.id.as_str(), first.id.as_str()], "{body}");

    // and without one: the version moves too, whatever the view's rows
    let v = now;
    room.save(Spec::cash(MAIN)).await;
    let body = room
        .view("OWNER", &format!("branch=MAIN&limit=50&since={v}"))
        .await;
    assert_eq!(body["changed"], json!(true), "a cash save moved it: {body}");
    room.finish().await;
}
