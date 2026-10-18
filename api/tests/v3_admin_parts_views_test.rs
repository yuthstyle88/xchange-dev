//! Ticket #476 (V2-parts, `docs/design/xchange-v3-web.md` §5.1, §12.1, §12.2): the three bounded
//! views the shared admin parts read, and the shared exchange row. Written before the
//! implementation by the lane controller. Do not edit, delete or weaken; add tests in a file of
//! your own.
//!
//! - `GET /api/v3/views/transaction_documents?ref=` — a `BranchView` inheriting
//!   `(EntryPath::UserRoute, "documents")` (the row `/documents` checks): `{ref, files}`, the
//!   files `owners_page` lists for that exchange under `ใบเสร็จ —` / `ภาพรายละเอียดใบเสร็จ —`, each
//!   `{kind: "receipt" | "receiptImage", path, download}`; never the passport nor the payment
//!   proof. `ref` absent or empty → `400 invalidRequest`, `field: "ref"` (ticket §7).
//! - `GET /api/v3/views/admin_request_status?requestId=` — a `BranchView` inheriting
//!   `(EntryPath::Rpc, "adminSnapshot")`, `VERSIONED = false`: `{applied}`, the answer of the check
//!   `adminAbandonRequest` makes (`abandon_request.rs` `is_applied`), read-only; a request id not of
//!   the shape `read_request_id` accepts → `400`, `field: "requestId"`.
//! - `GET /api/v3/views/admin_shift_state` — a `BranchView` inheriting `(EntryPath::Rpc,
//!   "adminSnapshot")`, `NoFilter`: `{shiftOpen, deskUrl}`, `deskUrl` as the snapshot answers it.
//! - §4.2: every writer of what `transaction_documents` and `admin_shift_state` read goes through
//!   `BranchTx` — one write per writer, then the same `since` answers `changed: true`. The writers
//!   measured here: the exchange save (`SaveTransactionWorkflow`, which also writes the receipt
//!   file row inside its transaction, `file_service.rs` `store_receipt_in`), shift open and shift
//!   close (`/api/rpc`). The plain upload (`file_service.rs` `upload`) writes a file row that no
//!   exchange names yet, so it changes nothing these views answer and is not one of them.
//! - `TransactionRow` (`dto/v3/transaction_row.rs` → `generated/transaction_row.ts`): the
//!   ticket's 25 fields and #781's `unitSize`, money and rates decimal strings, `null` where v2 shows `—` / nothing. Read
//!   from the committed generated file, so this target compiles before the type exists; the drift
//!   test of `v3_types_test.rs` keeps that file equal to what the Rust type generates.
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real
//! sessions made at the real sign-in doors (`rpc_common`); writes are real `/api/rpc` actions,
//! exchanges are saved through the real `SaveTransactionWorkflow` behind the routes (as
//! `v3_devices_view_test.rs` does). The test's clock is advanced before every write.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`) for the view tests; nothing skips.
//!
//! Re-expected for RE-7 #604 by the ticket author (rates.md §8 migration 6, §9 RE-7): the counter
//! rates are published (`rates_re4e_publish::set_counter_rate`), not written to `rates`; the
//! `admin_request_status` tests that used `adminSaveRate` as a writer use `adminOpenShift`, and
//! "a saved rate" is no longer one of the writers `applied` finds.

mod common;
mod compat_common;
mod rates_re4_common;
mod rates_re4e_publish;
mod rpc_common;
mod shift_holder_common;
mod shift_setup_common;
mod snapshot_common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
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
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::exchange::commands::save_transaction::SaveTransaction;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xchange::presentation::http::v3::VIEWS;

const ORIGIN: &str = "http://desk.test";
const RPC: &str = "/api/rpc";

const MAIN: &str = "MAIN";
const B9: &str = "B9";
const B9_NAME: &str = "Nine";

const DOCUMENTS: &str = "transaction_documents";
const REQUEST_STATUS: &str = "admin_request_status";
const SHIFT_STATE: &str = "admin_shift_state";

const JPEG: &[u8] = &[
    0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 9,
];
const PDF: &[u8] = b"%PDF-1.7 a slip\n";

/// `TransactionRow`'s fields (ticket §7, design §12.2).
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
/// The fields v2 shows as `—` or not at all when the exchange has none: `null` on the wire.
const ROW_NULLABLE: [&str; 9] = [
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

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A request id in v2's shape, different on every call.
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

/// What a save left behind.
struct Saved {
    document_ref: String,
    passport: String,
    slip: Option<String>,
    receipt: String,
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
    /// `MAIN`, all signed in; the USD counter rate 32.1 / 33 in both branches.
    async fn open() -> Room {
        let mut desk = Desk::open().await;
        desk.workflow =
            xchange::application::workflows::save_transaction::SaveTransactionWorkflow::new(
                desk.db.branch_tx(),
                xc_shift::application::shift::service::ShiftService::new(
                    std::sync::Arc::new(
                        xc_cash::infrastructure::ledger::ledger_shift_repository::ledger_shift_repository(
                            desk.db.pool.clone(),
                        ),
                    ),
                    desk.db.branch_tx(),
                ),
                desk.files.clone(),
            );
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

    /// `data` of a view whose answer must be `changed: true`.
    async fn data(&self, who: &str, view: &str, query: &str) -> Value {
        let body = self.view(who, view, query).await;
        assert_eq!(
            body["changed"],
            json!(true),
            "GET {}: {body}",
            view_path(view, query)
        );
        body.get("data")
            .cloned()
            .unwrap_or_else(|| panic!("GET {}: data in {body}", view_path(view, query)))
    }

    async fn post(&self, who: &str, action: &str, payload: Value) -> Answer {
        self.tick();
        let call = Call::action(ORIGIN, RPC, action, payload).cookie(&self.cookie(who));
        self.app.send(call).await
    }

    async fn ok(&self, who: &str, action: &str, payload: Value) -> Value {
        let answer = self.post(who, action, payload.clone()).await;
        assert_eq!(
            answer.status, 200,
            "{action} as {who} with {payload}: {:.400}",
            answer.text
        );
        answer.json()
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

    /// Opens a shift in `branch` by count; returns the shift's `open_request_id`.
    async fn open_shift(&self, branch: &str) -> String {
        let shift_id = self.open_shift_by_count(branch).await;
        sqlx::query_scalar("SELECT open_request_id FROM shifts WHERE id = $1")
            .bind(&shift_id)
            .fetch_one(&self.desk.db.pool)
            .await
            .expect("open_request_id")
    }

    async fn open_shift_by_count(&self, branch: &str) -> String {
        let t = self.tick();
        shift_setup_common::open_shift_by_count(
            &self.desk.db.pool,
            branch,
            json!({ "THB": 100000, "USD": 1000 }),
            t,
        )
        .await
    }

    async fn open_shift_id(&self, branch: &str) -> String {
        sqlx::query_scalar("SELECT id FROM shifts WHERE branch_id = $1 AND status = 'OPEN'")
            .bind(branch)
            .fetch_one(&self.desk.db.pool)
            .await
            .expect("an open shift")
    }

    async fn expected_of_open_shift(&self, branch: &str) -> Value {
        use xc_cash::infrastructure::ledger::shift_balances::PgLedgerShiftBalances;
        use xc_shift::domain::shift::repositories::shift_balances::ShiftBalances;
        let shift_id = self.open_shift_id(branch).await;
        let rows = PgLedgerShiftBalances::new(self.desk.db.pool.clone())
            .shift_balances(&shift_id)
            .await
            .expect("read the open shift's balances");
        Value::Object(
            rows.into_iter()
                .map(|b| {
                    (
                        b.currency.as_str().to_owned(),
                        Value::String(b.expected.to_string()),
                    )
                })
                .collect(),
        )
    }

    /// Closes the open shift of `branch` by count, counting what is expected; returns the shift's `close_request_id`.
    async fn close_shift(&self, branch: &str) -> String {
        let shift = self.open_shift_id(branch).await;
        let expected = self.expected_of_open_shift(branch).await;
        shift_setup_common::close_shift_by_count_with_note(
            &self.desk.db.pool,
            branch,
            json!(expected),
            "นับเงินปิดกะ",
            self.tick(),
        )
        .await;
        sqlx::query(
            "UPDATE shifts SET close_request_id = (SELECT id::text FROM cash_counts WHERE shift_id = $1 AND kind = 'CLOSE') \
             WHERE id = $1 AND close_request_id IS NULL",
        )
        .bind(&shift)
        .execute(&self.desk.db.pool)
        .await
        .expect("set close_request_id from close count");
        sqlx::query_scalar("SELECT close_request_id FROM shifts WHERE id = $1")
            .bind(&shift)
            .fetch_one(&self.desk.db.pool)
            .await
            .expect("close_request_id")
    }

    /// One LIVE purchase of 100 USD saved through the real workflow behind the routes (the
    /// branch needs an open shift); with a payment slip when `slip`.
    async fn save(&self, branch: &str, slip: bool) -> Saved {
        let t = self.tick();
        let here = BranchId::new(branch);
        let person = ReserveCustomer {
            name: "Somchai Jaidee".to_owned(),
            passport_no: "AA1234567".to_owned(),
            nationality: "THA".to_owned(),
            birth_date: String::new(),
            passport_expiry: String::new(),
            mode: "LIVE".to_owned(),
        };
        let customer = CustomerService::new(self.desk.db.branch_tx())
            .reserve(&here, person.clone(), t)
            .await
            .expect("reserve the customer")
            .customer_id;
        let saver = shift_holder_common::teller_of_open_shift(&self.desk.db.pool, branch).await;
        let passport = self
            .desk
            .files
            .upload(&here, &saver, "image/jpeg", JPEG, t)
            .await
            .expect("upload a passport");
        let slip_key = if slip {
            Some(
                self.desk
                    .files
                    .upload(&here, &saver, "application/pdf", PDF, t)
                    .await
                    .expect("upload a slip"),
            )
        } else {
            None
        };
        let cmd = SaveTransaction {
            customer: person,
            submission_id: format!("submission-{branch}-{:012}", next()),
            customer_id: customer,
            side: "BUY".to_owned(),
            currency: "USD".to_owned(),
            amount: "100".to_owned(),
            rate: "32.1".to_owned(),
            fee: "0".to_owned(),
            bank_fee: "0".to_owned(),
            method: if slip { "Bank Transfer" } else { "Cash" }.to_owned(),
            payment_ref: if slip { "TRX-1" } else { "" }.to_owned(),
            account: "Cash drawer 1".to_owned(),
            staff: "EMP-001".to_owned(),
            booking: "WALK-IN".to_owned(),
            notes: String::new(),
            value_date: "2026-10-06".to_owned(),
            paid_at: "2026-10-06T05:30:00.000Z".to_owned(),
            payment_confirmed: true,
            identity_confirmed: true,
            passport: Some(passport.clone()),
            slip: slip_key.clone(),
            reference_rate: None,
        };
        let saved = self
            .desk
            .workflow
            .save(&here, cmd, &saver, t)
            .await
            .unwrap_or_else(|e| panic!("save an exchange in {branch}: {e:?}"));
        let receipt = saved
            .receipt_url
            .strip_prefix("/files/")
            .unwrap_or_else(|| panic!("a receipt url: {}", saved.receipt_url))
            .to_owned();
        Saved {
            document_ref: saved.document_ref,
            passport,
            slip: slip_key,
            receipt,
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
        // the owner and an employee in one browser: a view not on StaffRpc answers the owner
        callers.push((
            format!("OWNER+{}", role.as_str()),
            Some(Actor::owner("owner")),
        ));
    }
    callers
}

/// Every caller kind gets what `Policy::allow(actor, path, action)` gives: 200 with data when it
/// allows, its refusal otherwise.
async fn answers_exactly_whom_the_row_allows(
    room: &Room,
    view: &str,
    query: &str,
    path: EntryPath,
    action: &str,
) -> (usize, usize) {
    let mut allowed = 0;
    let mut refused = 0;
    for (who, actor) in callers() {
        let policy = Policy::allow(actor.as_ref(), path, &Action::new(action));
        let answer = room.get(&who, &view_path(view, query)).await;
        let what = format!("{view} as {who:?}");
        match &policy {
            Ok(()) => {
                allowed += 1;
                assert_eq!(answer.status, 200, "{what}: {:.300}", answer.text);
                let body = answer.json();
                assert_eq!(body["changed"], json!(true), "{what}: {body}");
                assert!(body.get("data").is_some(), "{what}: data in {body}");
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
    (allowed, refused)
}

fn assert_refused_naming(answer: &Answer, field: &str, what: &str) {
    assert_eq!(answer.status, 400, "{what}: {:.300}", answer.text);
    let body = answer.json();
    assert_eq!(
        body["error"]["code"],
        json!("invalidRequest"),
        "{what}: {body}"
    );
    assert_eq!(body["error"]["field"], json!(field), "{what}: {body}");
    assert!(body.get("data").is_none(), "{what}: no data in {body}");
}

// ── transaction_documents ────────────────────────────────────────────────────

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
async fn transaction_documents_answers_exactly_whom_the_documents_route_allows() {
    let room = Room::open().await;
    room.open_shift_by_count(MAIN).await;
    let saved = room.save(MAIN, false).await;
    let (allowed, refused) = answers_exactly_whom_the_row_allows(
        &room,
        DOCUMENTS,
        &format!("ref={}", saved.document_ref),
        EntryPath::UserRoute,
        "documents",
    )
    .await;
    assert_eq!(
        allowed, 19,
        "the owner, the nine roles, the owner beside each"
    );
    assert_eq!(refused, 1, "nobody signed in");
    room.finish().await;
}

#[actix_web::test]
async fn transaction_documents_lists_the_exchanges_receipt_and_nothing_else() {
    let room = Room::open().await;
    room.open_shift_by_count(MAIN).await;
    let saved = room.save(MAIN, true).await;
    let data = room
        .data(
            "OWNER",
            DOCUMENTS,
            &format!("branch=MAIN&ref={}", saved.document_ref),
        )
        .await;
    assert_eq!(keys(&data), set_of(&["ref", "files"]), "{data}");
    assert_eq!(data["ref"], json!(saved.document_ref), "{data}");
    let files = data["files"].as_array().expect("files is a list");
    assert_eq!(
        files.len(),
        1,
        "one receipt, as owners_page lists it: {data}"
    );
    assert_eq!(
        keys(&files[0]),
        set_of(&["kind", "path", "download"]),
        "{data}"
    );
    assert_eq!(files[0]["kind"], json!("receipt"), "{data}");
    assert_eq!(
        files[0]["path"],
        json!(format!("/files/{}", saved.receipt)),
        "{data}"
    );
    assert_eq!(
        files[0]["download"],
        json!(format!("{}_RECEIPT_FULL.html", saved.document_ref)),
        "the download name owners_page gives the receipt: {data}"
    );
    let text = data.to_string();
    assert!(
        !text.contains(&saved.passport),
        "the passport is not a receipt: {data}"
    );
    let slip = saved.slip.as_deref().expect("saved with a slip");
    assert!(
        !text.contains(slip),
        "the payment proof is not a receipt: {data}"
    );
    room.finish().await;
}

#[actix_web::test]
async fn transaction_documents_refuses_a_missing_or_empty_ref_naming_ref() {
    let room = Room::open().await;
    let v = room.version(MAIN).await;
    for query in [
        "branch=MAIN".to_string(),
        "branch=MAIN&ref=".to_string(),
        format!("branch=MAIN&ref=&since={v}"),
        format!("since={v}"),
    ] {
        let answer = room.get("OWNER", &view_path(DOCUMENTS, &query)).await;
        assert_refused_naming(&answer, "ref", &query);
    }
    let answer = room.get("FRONT", &view_path(DOCUMENTS, "")).await;
    assert_refused_naming(&answer, "ref", "FRONT, no query");
    room.finish().await;
}

#[actix_web::test]
async fn transaction_documents_answers_staff_from_their_own_branch() {
    let room = Room::open().await;
    room.open_shift_by_count(MAIN).await;
    room.open_shift_by_count(B9).await;
    let main = room.save(MAIN, false).await;
    let nine = room.save(B9, false).await;

    // FRONT works in MAIN. OR-4b (D-4): naming B9 is refused; naming MAIN reads MAIN
    assert_out_of_scope(
        &room
            .get(
                "FRONT",
                &view_path(DOCUMENTS, &format!("branch=B9&ref={}", main.document_ref)),
            )
            .await,
        "FRONT naming B9",
    );
    let data = room
        .data(
            "FRONT",
            DOCUMENTS,
            &format!("branch=MAIN&ref={}", main.document_ref),
        )
        .await;
    assert_eq!(
        data["files"][0]["path"],
        json!(format!("/files/{}", main.receipt)),
        "FRONT is answered from MAIN: {data}"
    );

    // the owner reads the branch named
    let data = room
        .data(
            "OWNER",
            DOCUMENTS,
            &format!("branch=B9&ref={}", nine.document_ref),
        )
        .await;
    assert_eq!(
        data["files"][0]["path"],
        json!(format!("/files/{}", nine.receipt)),
        "the owner naming B9: {data}"
    );

    // B9's exchange is not MAIN's: asked in MAIN, B9's receipt is never answered
    for (who, query) in [
        ("OWNER", format!("branch=MAIN&ref={}", nine.document_ref)),
        ("FRONT", format!("branch=B9&ref={}", nine.document_ref)),
    ] {
        let answer = room.get(who, &view_path(DOCUMENTS, &query)).await;
        assert!(
            !answer.text.contains(&nine.receipt),
            "{who} {query}: another branch's receipt: {:.300}",
            answer.text
        );
    }
    room.finish().await;
}

/// §4.2: the exchange save (with its receipt row, inside the save's `BranchTx`) moves the version
/// the view answers; an equal `since` is unchanged before it.
#[actix_web::test]
async fn an_exchange_save_moves_the_version_transaction_documents_answers() {
    let room = Room::open().await;
    room.open_shift_by_count(MAIN).await;
    let first = room.save(MAIN, false).await;
    let v = room.version(MAIN).await;
    let body = room
        .view(
            "OWNER",
            DOCUMENTS,
            &format!("branch=MAIN&ref={}&since={v}", first.document_ref),
        )
        .await;
    assert_eq!(
        body,
        json!({ "version": v, "changed": false }),
        "an equal since: exactly the version and changed: false"
    );

    let second = room.save(MAIN, false).await;
    let now = room.version(MAIN).await;
    assert!(now > v, "the save moved MAIN's version ({v} → {now})");
    let body = room
        .view(
            "OWNER",
            DOCUMENTS,
            &format!("branch=MAIN&ref={}&since={v}", second.document_ref),
        )
        .await;
    assert_eq!(body["version"], json!(now), "{body}");
    assert_eq!(body["changed"], json!(true), "{body}");
    assert_eq!(
        body["data"]["files"][0]["path"],
        json!(format!("/files/{}", second.receipt)),
        "{body}"
    );
    room.finish().await;
}

// ── admin_request_status ─────────────────────────────────────────────────────

#[actix_web::test]
async fn admin_request_status_answers_exactly_whom_the_snapshot_row_allows() {
    let room = Room::open().await;
    let (allowed, refused) = answers_exactly_whom_the_row_allows(
        &room,
        REQUEST_STATUS,
        &format!("requestId={}", request_id()),
        EntryPath::Rpc,
        "adminSnapshot",
    )
    .await;
    assert_eq!(allowed, 19);
    assert_eq!(refused, 1);
    room.finish().await;
}

#[actix_web::test]
async fn admin_request_status_refuses_a_request_id_of_the_wrong_shape_naming_it() {
    let room = Room::open().await;
    let v = room.version(MAIN).await;
    let too_long = "a".repeat(61);
    for query in [
        "branch=MAIN".to_string(),
        "branch=MAIN&requestId=".to_string(),
        "branch=MAIN&requestId=short-id-15chr".to_string(),
        format!("branch=MAIN&requestId={too_long}"),
        "branch=MAIN&requestId=has_an_underscore-0001".to_string(),
        "branch=MAIN&requestId=has%20a%20space-00000001".to_string(),
        format!("branch=MAIN&since={v}"),
    ] {
        let answer = room.get("OWNER", &view_path(REQUEST_STATUS, &query)).await;
        assert_refused_naming(&answer, "requestId", &query);
    }
    // the shortest and the longest shape read_request_id accepts
    for id in ["abcdefgh-1234567".to_string(), "b".repeat(60)] {
        let data = room
            .data(
                "OWNER",
                REQUEST_STATUS,
                &format!("branch=MAIN&requestId={id}"),
            )
            .await;
        assert_eq!(data, json!({ "applied": false }), "{id}");
    }
    room.finish().await;
}

async fn seed_movement(
    pool: &sqlx::PgPool,
    branch: &str,
    shift_id: &str,
    code: &str,
    direction: &str,
    amount: &str,
    reason: &str,
) -> (String, String) {
    use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
    use xc_cash::domain::posting::{CashPlace, CashPostings, moves};
    use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
    use xc_kernel::money::CurrencyCode;
    let id = format!("MV-{:012}", next());
    let req_id = request_id();
    let mut tx = pool.begin().await.expect("begin tx");
    sqlx::query(
        "INSERT INTO stock_movements (id, branch_id, shift_id, code, direction, amount, reason, request_id, at, by) \
         VALUES ($1, $2, $3, $4, $5, $6::numeric, $7, $8, now(), 'fn1')",
    )
    .bind(&id)
    .bind(branch)
    .bind(shift_id)
    .bind(code)
    .bind(direction)
    .bind(amount)
    .bind(reason)
    .bind(&req_id)
    .execute(&mut *tx)
    .await
    .expect("insert stock_movement");
    let (safe, drawer) = (
        CashPlace::Safe(BranchId::new(branch)),
        CashPlace::Drawer(shift_id.to_string()),
    );
    let (from, to) = if direction == "IN" {
        (safe, drawer)
    } else {
        (drawer, safe)
    };
    let source = SourceRef {
        source_type: SourceType::Transfer,
        source_id: req_id.clone(),
        source_key: "move".to_string(),
    };
    let moved = [(
        CurrencyCode::parse(code).expect("valid currency"),
        amount.parse().expect("valid decimal"),
    )];
    LedgerCashPostings::new()
        .post_in(&mut tx, &moves(&from, &to, &moved, &source), None)
        .await
        .expect("post ledger movement");
    sqlx::query(
        "INSERT INTO branch_versions (branch_id, version) VALUES ($1, 1) \
         ON CONFLICT (branch_id) DO UPDATE SET version = branch_versions.version + 1",
    )
    .bind(branch)
    .execute(&mut *tx)
    .await
    .expect("bump branch_versions");
    tx.commit().await.expect("commit seed_movement");
    (id, req_id)
}

/// `applied` is the check `adminAbandonRequest` makes: an opened shift, a stock movement, a closed
/// shift in the branch, or a branch anywhere, made with that request id. (Before RE-7 also a rate
/// saved directly; RE-7 #604 removes that save.)
#[actix_web::test]
async fn admin_request_status_is_applied_exactly_when_abandon_would_find_the_request() {
    let room = Room::open().await;
    let ask = |id: String| {
        let room = &room;
        async move {
            room.data(
                "OWNER",
                REQUEST_STATUS,
                &format!("branch=MAIN&requestId={id}"),
            )
            .await
        }
    };
    assert_eq!(ask(request_id()).await, json!({ "applied": false }));

    // RE-7 #604 (re-expected by the ticket author): "a saved rate" is gone with `adminSaveRate`
    // and `rates` (rates.md §8 migration 6, §9 RE-7); the other writers still count

    let opened = room.open_shift(MAIN).await;
    assert_eq!(
        ask(opened).await,
        json!({ "applied": true }),
        "an opened shift"
    );
    let shift = room.open_shift_id(MAIN).await;
    let (_movement_id, moved) = seed_movement(
        &room.desk.db.pool,
        MAIN,
        &shift,
        "USD",
        "IN",
        "10",
        "from the safe",
    )
    .await;
    assert_eq!(
        ask(moved).await,
        json!({ "applied": true }),
        "a stock movement"
    );
    let closed = room.close_shift(MAIN).await;
    assert_eq!(
        ask(closed).await,
        json!({ "applied": true }),
        "a closed shift"
    );

    let branch = request_id();
    room.ok(
        "OWNER",
        "adminCreateBranch",
        json!({ "requestId": branch, "name": "Airport" }),
    )
    .await;
    assert_eq!(
        ask(branch).await,
        json!({ "applied": true }),
        "a branch made with it, asked from MAIN"
    );

    assert_eq!(ask(request_id()).await, json!({ "applied": false }));
    room.finish().await;
}

#[actix_web::test]
async fn admin_request_status_is_unversioned_and_writes_nothing() {
    let room = Room::open().await;
    let id = request_id();
    let v = room.version(MAIN).await;
    for _ in 0..2 {
        let body = room
            .view(
                "OWNER",
                REQUEST_STATUS,
                &format!("branch=MAIN&requestId={id}&since={v}"),
            )
            .await;
        assert_eq!(
            body,
            json!({ "version": v, "changed": true, "data": { "applied": false } }),
            "VERSIONED = false: an equal since still answers"
        );
    }
    assert_eq!(room.version(MAIN).await, v, "reading moved no version");
    let abandoned: i64 = sqlx::query_scalar("SELECT count(*) FROM abandoned_requests")
        .fetch_one(&room.desk.db.pool)
        .await
        .expect("count abandoned requests");
    assert_eq!(abandoned, 0, "asking gives nothing up");

    // the id asked about is still free to be used (RE-7 #604: by a shift opened with it, where
    // it was a direct rate save before RE-7 removed that action; re-expected for CA-10 #626 by the
    // ticket author: by a branch created with it, the direct shift open being retired too, design
    // `cash-shift-transfer.md` §6)
    room.ok(
        "OWNER",
        "adminCreateBranch",
        json!({ "requestId": id, "name": "Request Status Branch" }),
    )
    .await;
    let data = room
        .data(
            "OWNER",
            REQUEST_STATUS,
            &format!("branch=MAIN&requestId={id}"),
        )
        .await;
    assert_eq!(data, json!({ "applied": true }));
    room.finish().await;
}

#[actix_web::test]
async fn admin_request_status_answers_staff_for_their_own_branch() {
    let room = Room::open().await;
    // RE-7 #604 (re-expected by the ticket author): a shift opened in B9, where it was a direct
    // rate save in B9 before RE-7 removed that action
    let id = room.open_shift(B9).await;
    let q = |branch: &str| format!("branch={branch}&requestId={id}");
    assert_eq!(
        room.data("OWNER", REQUEST_STATUS, &q(B9)).await,
        json!({ "applied": true }),
        "the owner naming B9"
    );
    assert_eq!(
        room.data("OWNER", REQUEST_STATUS, &q(MAIN)).await,
        json!({ "applied": false }),
        "a shift of B9 is not MAIN's"
    );
    // OR-4b (D-4): FRONT works in MAIN; naming B9 is refused, naming MAIN reads MAIN
    assert_out_of_scope(
        &room.get("FRONT", &view_path(REQUEST_STATUS, &q(B9))).await,
        "FRONT naming B9",
    );
    assert_eq!(
        room.data("FRONT", REQUEST_STATUS, &q(MAIN)).await,
        json!({ "applied": false }),
        "FRONT works in MAIN"
    );
    room.finish().await;
}

// ── admin_shift_state ────────────────────────────────────────────────────────

#[actix_web::test]
async fn admin_shift_state_answers_exactly_whom_the_snapshot_row_allows() {
    let room = Room::open().await;
    let (allowed, refused) = answers_exactly_whom_the_row_allows(
        &room,
        SHIFT_STATE,
        "",
        EntryPath::Rpc,
        "adminSnapshot",
    )
    .await;
    assert_eq!(allowed, 19);
    assert_eq!(refused, 1);
    room.finish().await;
}

/// §4.2: shift open and shift close move the version the view answers.
#[actix_web::test]
async fn admin_shift_state_follows_open_and_close_and_each_moves_the_version() {
    let room = Room::open().await;
    let data = room.data("OWNER", SHIFT_STATE, "branch=MAIN").await;
    assert_eq!(
        data,
        json!({ "shiftOpen": false, "deskUrl": "/desk?branch=MAIN", "openShifts": [] })
    );

    let v = room.version(MAIN).await;
    let body = room
        .view("OWNER", SHIFT_STATE, &format!("branch=MAIN&since={v}"))
        .await;
    assert_eq!(body, json!({ "version": v, "changed": false }));

    room.open_shift(MAIN).await;
    let opened = room.version(MAIN).await;
    assert!(opened > v, "opening moved MAIN's version");
    let body = room
        .view("OWNER", SHIFT_STATE, &format!("branch=MAIN&since={v}"))
        .await;
    // CA-10p: the open shift is listed with its teller (design §4.7)
    assert_eq!(
        body["version"],
        json!(opened),
        "after adminOpenShift: {body}"
    );
    assert_eq!(body["changed"], json!(true), "after adminOpenShift: {body}");
    assert_eq!(
        body["data"]["shiftOpen"],
        json!(true),
        "after adminOpenShift: {body}"
    );
    assert_eq!(
        body["data"]["deskUrl"],
        json!("/desk?branch=MAIN"),
        "{body}"
    );
    assert_open_shifts(&room, MAIN, &body["data"]["openShifts"], 1).await;

    room.close_shift(MAIN).await;
    let closed = room.version(MAIN).await;
    assert!(closed > opened, "closing moved MAIN's version");
    let body = room
        .view("OWNER", SHIFT_STATE, &format!("branch=MAIN&since={opened}"))
        .await;
    assert_eq!(
        body,
        json!({ "version": closed, "changed": true,
                "data": { "shiftOpen": false, "deskUrl": "/desk?branch=MAIN", "openShifts": [] } }),
        "after adminCloseShift"
    );
    room.finish().await;
}

#[actix_web::test]
async fn admin_shift_state_is_per_branch_and_staff_read_their_own() {
    let room = Room::open().await;
    room.open_shift_by_count(B9).await;
    let b9 = room.data("OWNER", SHIFT_STATE, "branch=B9").await;
    assert_eq!(b9["shiftOpen"], json!(true), "{b9}");
    assert_eq!(b9["deskUrl"], json!("/desk?branch=B9"), "{b9}");
    assert_open_shifts(&room, B9, &b9["openShifts"], 1).await;
    assert_eq!(
        room.data("OWNER", SHIFT_STATE, "branch=MAIN").await,
        json!({ "shiftOpen": false, "deskUrl": "/desk?branch=MAIN", "openShifts": [] })
    );
    assert_eq!(
        room.data("OWNER", SHIFT_STATE, "").await,
        json!({ "shiftOpen": false, "deskUrl": "/desk?branch=MAIN", "openShifts": [] }),
        "no branch named: MAIN"
    );
    // OR-4b (D-4): FRONT works in MAIN; naming B9 is refused, naming MAIN reads MAIN
    assert_out_of_scope(
        &room
            .get("FRONT", &view_path(SHIFT_STATE, "branch=B9"))
            .await,
        "FRONT naming B9",
    );
    assert_eq!(
        room.data("FRONT", SHIFT_STATE, "branch=MAIN").await,
        json!({ "shiftOpen": false, "deskUrl": "/desk?branch=MAIN", "openShifts": [] }),
        "FRONT works in MAIN"
    );
    room.finish().await;
}

#[actix_web::test]
async fn admin_shift_state_desk_url_is_the_snapshots() {
    let room = Room::open().await;
    for branch in [MAIN, B9] {
        let snapshot = room
            .ok("OWNER", "adminSnapshot", json!({ "branchId": branch }))
            .await;
        let data = room
            .data("OWNER", SHIFT_STATE, &format!("branch={branch}"))
            .await;
        assert_eq!(
            data["deskUrl"], snapshot["deskUrl"],
            "{branch}: the snapshot's deskUrl"
        );
    }
    room.finish().await;
}

// ── the generated types ──────────────────────────────────────────────────────

fn exported(view: &str) -> BTreeMap<String, String> {
    let entry = VIEWS
        .iter()
        .find(|v| v.name == view)
        .unwrap_or_else(|| panic!("the view {view} is in the generated table"));
    let dir: PathBuf =
        std::env::temp_dir().join(format!("xc-476-{view}-{}-{}", std::process::id(), next()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    (entry.export_types)(&dir).unwrap_or_else(|e| panic!("export {view}: {e}"));
    let mut files = BTreeMap::new();
    for e in std::fs::read_dir(&dir).expect("read the scratch directory") {
        let e = e.expect("an entry");
        let name = e.file_name().to_string_lossy().into_owned();
        files.insert(
            name,
            std::fs::read_to_string(e.path()).expect("read an exported file"),
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    files
}

/// `field: type` of an object type in generated TypeScript, as written.
fn has_field(ts: &str, field: &str, types: &[&str]) {
    let found = types.iter().any(|ty| {
        ts.contains(&format!("{field}: {ty},")) || ts.contains(&format!("{field}: {ty} "))
    });
    assert!(found, "`{field}: one of {types:?}` in:\n{ts}");
}

#[test]
fn the_three_views_are_in_the_generated_table() {
    let names: Vec<&str> = VIEWS.iter().map(|v| v.name).collect();
    for view in [DOCUMENTS, REQUEST_STATUS, SHIFT_STATE] {
        assert!(names.contains(&view), "{view} in {names:?}");
    }
}

#[test]
fn the_views_types_carry_the_tickets_fields() {
    let documents = exported(DOCUMENTS);
    let ts = documents
        .get("transaction_documents.ts")
        .unwrap_or_else(|| panic!("transaction_documents.ts in {:?}", documents.keys()));
    has_field(ts, "ref", &["string"]);
    has_field(ts, "path", &["string"]);
    assert!(
        ts.contains("\"receipt\" | \"receiptImage\""),
        "kind is \"receipt\" | \"receiptImage\":\n{ts}"
    );
    assert!(
        ts.contains("download?: string") || ts.contains("download: string | null"),
        "download is an optional string:\n{ts}"
    );

    let status = exported(REQUEST_STATUS);
    let ts = status
        .get("admin_request_status.ts")
        .unwrap_or_else(|| panic!("admin_request_status.ts in {:?}", status.keys()));
    has_field(ts, "applied", &["boolean"]);
    has_field(ts, "requestId", &["string"]);

    let shift = exported(SHIFT_STATE);
    let ts = shift
        .get("admin_shift_state.ts")
        .unwrap_or_else(|| panic!("admin_shift_state.ts in {:?}", shift.keys()));
    has_field(ts, "shiftOpen", &["boolean"]);
    has_field(ts, "deskUrl", &["string"]);
    // CA-10p (design §4.7): one shared row type, exported once into `common.ts` (a view writes only its own
    // file and `common.ts`, v3_types_test `each_view_writes_only_its_own_file_and_common`)
    has_field(ts, "openShifts", &["Array<OpenShiftDto>", "OpenShiftDto[]"]);
    assert!(!ts.contains("bigint"), "{ts}");
    let rows = exported(SHIFT_STATE);
    let common = rows
        .get("common.ts")
        .unwrap_or_else(|| panic!("common.ts in {:?}", rows.keys()));
    assert!(
        common.contains("export type OpenShiftDto"),
        "OpenShiftDto is in common.ts:\n{common}"
    );
    for field in ["shiftId", "tellerId", "tellerName", "openedAt"] {
        has_field(common, field, &["string"]);
    }
}

/// `TransactionRow` as committed in `generated/transaction_row.ts` (kept equal to the Rust type by
/// `v3_types_test.rs`): exactly the ticket's fields, every one a string (money and rates are
/// decimal strings, W7), `| null` exactly on the fields v2 shows as `—` or not at all.
#[test]
fn transaction_row_has_the_tickets_fields_as_strings() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../web/src/shared/api/generated/transaction_row.ts");
    let ts = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is committed: {e}", path.display()));
    let start = ts
        .find("export type TransactionRow = {")
        .unwrap_or_else(|| panic!("export type TransactionRow in:\n{ts}"));
    let body = &ts[start + "export type TransactionRow = {".len()..];
    let body = &body[..body
        .find("};")
        .unwrap_or_else(|| panic!("the end of TransactionRow:\n{ts}"))];
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    for part in body.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (name, ty) = part
            .split_once(':')
            .unwrap_or_else(|| panic!("`name: type` in {part:?}"));
        fields.insert(name.trim().to_owned(), ty.trim().to_owned());
    }
    let names: BTreeSet<String> = fields.keys().cloned().collect();
    assert_eq!(names, set_of(&ROW_FIELDS), "TransactionRow's fields:\n{ts}");
    for (name, ty) in &fields {
        let nullable = ROW_NULLABLE.contains(&name.as_str());
        let base = ty.strip_suffix(" | null").unwrap_or(ty);
        assert!(
            ["string", "Money", "Rate"].contains(&base),
            "{name}: a string on the wire (W7), got `{ty}`"
        );
        assert_eq!(
            ty.ends_with(" | null"),
            nullable,
            "{name}: `| null` exactly where v2 shows — or nothing, got `{ty}`"
        );
    }
    for money in [
        "amount",
        "principal",
        "fee",
        "bankFee",
        "settlement",
        "rate",
    ] {
        assert!(
            !fields[money].contains("number"),
            "{money} is a decimal string"
        );
    }
}

// ── CA-10p: the branch's OPEN shifts, each with its teller (design cash-shift-transfer.md §4.7 @ 4ed83b68) ──
//
// Tests written before the implementation (ticket CA-10p) — do not edit or delete.

/// The branch's OPEN shifts as the database holds them, in the view's order (`opened_at`, then id).
async fn open_shifts_in_db(
    room: &Room,
    branch: &str,
) -> Vec<(String, String, String, DateTime<Utc>)> {
    sqlx::query_as(
        "SELECT s.id, s.teller_id::text, u.name, s.opened_at FROM shifts s JOIN users u ON u.id = s.teller_id \
         WHERE s.branch_id = $1 AND s.status = 'OPEN' ORDER BY s.opened_at, s.id",
    )
    .bind(branch)
    .fetch_all(&room.desk.db.pool)
    .await
    .expect("read the branch's OPEN shifts")
}

/// `openShifts` is exactly the branch's OPEN shifts (`want` of them), each `{ shiftId, tellerId, tellerName,
/// openedAt }`, `openedAt` RFC 3339 for the stored instant.
async fn assert_open_shifts(room: &Room, branch: &str, list: &Value, want: usize) {
    let db = open_shifts_in_db(room, branch).await;
    assert_eq!(
        db.len(),
        want,
        "the seed opened {want} shift(s) at {branch}"
    );
    let got = list
        .as_array()
        .unwrap_or_else(|| panic!("openShifts is a list: {list}"));
    assert_eq!(got.len(), db.len(), "{list}");
    for (row, (id, teller, name, at)) in got.iter().zip(&db) {
        let fields: BTreeSet<&str> = row
            .as_object()
            .unwrap_or_else(|| panic!("a row is an object: {row}"))
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            fields,
            BTreeSet::from(["openedAt", "shiftId", "tellerId", "tellerName"]),
            "{row}"
        );
        assert_eq!(row["shiftId"], json!(id), "{row}");
        assert_eq!(row["tellerId"], json!(teller), "{row}");
        assert_eq!(row["tellerName"], json!(name), "{row}");
        let opened =
            DateTime::parse_from_rfc3339(row["openedAt"].as_str().expect("openedAt is a string"))
                .unwrap_or_else(|e| panic!("openedAt is RFC 3339: {row}: {e}"))
                .with_timezone(&Utc);
        assert_eq!(opened, *at, "{row}");
    }
}

/// Two tellers at one branch: both listed, in `opened_at` order, and `shiftOpen` is true. Seeded by SQL so
/// the second shift needs no second sign-in; `opened_by_count` set because the v2 per-branch index binds
/// marker-less shifts until CA-10's M3 (CA-10 drops it from this seed).
#[actix_web::test]
async fn admin_shift_state_lists_every_open_shift_of_the_branch_with_its_teller() {
    let room = Room::open().await;
    let pool = &room.desk.db.pool;
    let org: uuid::Uuid = sqlx::query_scalar("SELECT organization_id FROM branches WHERE id = $1")
        .bind(B9)
        .fetch_one(pool)
        .await
        .expect("B9's organisation");
    // opened 20 minutes ago, then 10 minutes ago: the earlier one is listed first
    for (name, minutes) in [("CA10p Earlier", 20), ("CA10p Later", 10)] {
        let teller = uuid::Uuid::new_v4();
        let count = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, organization_id, name, status, created_by) VALUES ($1, $2, $3, 'ACTIVE', 'test')",
        )
        .bind(teller)
        .bind(org)
        .bind(name)
        .execute(pool)
        .await
        .expect("insert a teller");
        sqlx::query(
            "INSERT INTO cash_counts (id, organization_id, branch_id, kind, counted_by, counted_at, idempotency_key) \
             VALUES ($1, $2, $3, 'OPEN', $4, now(), $5)",
        )
        .bind(count)
        .bind(org)
        .bind(B9)
        .bind(teller)
        .bind(format!("idem-{count}"))
        .execute(pool)
        .await
        .expect("insert the OPEN count");
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', now() - make_interval(mins => $3), 'test', $1, $4)",
        )
        .bind(format!("SH-ca10p-{count}"))
        .bind(B9)
        .bind(minutes)
        .bind(teller)
        .execute(pool)
        .await
        .expect("insert the OPEN shift");
    }
    let data = room.data("OWNER", SHIFT_STATE, "branch=B9").await;
    assert_eq!(data["shiftOpen"], json!(true), "{data}");
    assert_open_shifts(&room, B9, &data["openShifts"], 2).await;
    assert_eq!(
        data["openShifts"][0]["tellerName"],
        json!("CA10p Earlier"),
        "{data}"
    );
    room.finish().await;
}
