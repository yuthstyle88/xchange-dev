//! Ticket #463 (V2-bookings, `docs/design/xchange-v3-web.md` §4.2, §5.1, §12.1, §12.14, §12.17):
//! the views the v3 bookings menu reads. Written before the implementation by the lane
//! controller (claude/7ca80c). Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Both views live in the ticket's one view file `http/v3/views/admin_bookings.rs` (its
//! `register` registers both; its `export_types` writes both views' DTOs to
//! `generated/admin_bookings.ts`), so no file outside the ticket's scope is touched:
//!
//! - `GET /api/v3/views/admin_bookings` — a `PagedView`, `INHERITS = (EntryPath::OwnerRoute,
//!   "/api/bookings")`, the very row `POST /api/bookings` checks (`http/bookings.rs`
//!   `owner_bookings`, step 1). Rows are #413's `list` rows (`dto/booking.rs` `booking_json`,
//!   the same 13 keys) with money and rates as **decimal strings** (`Money` / `Rate` as they
//!   print: `"1000.00"`, `"36.2500"`, W7), plus `counterRate` and `counterPrincipal` (design
//!   §12.14): on a PENDING row, the branch's latest counter rate for the booking's currency and
//!   side (BUY → `buy`, SELL → `sell`, as `PgBookingPorts::counter_rate_in` reads it) and the
//!   amount at that rate; `null` on every other row and when the currency has no counter rate.
//!   Newest first (`created_at desc, id desc`), keyset paging by `before` = a row's `id`.
//!   `status` is the status as shown at the server's clock (`Booking::shown_status`): a CONFIRMED
//!   booking past its `expiresAt` reads `EXPIRED`. Filter `q` (design §12.1, `bookings.js:7`):
//!   a literal, case-folded (`lower()`) substring of id, name or phone, in SQL, with `%`, `_` and
//!   `\` taken literally; `''` or absent = no filter; over 100 characters → `400
//!   invalidRequest`, `field: "q"`.
//! - `GET /api/v3/views/admin_booking_settings` — a `BranchView`, the same `INHERITS`,
//!   `NoFilter`: what `bookings.js` reads beside the rows (`:8`, `:9`, `:16`) and what the
//!   create form needs (design §12.17): `{rules: {enabled, minutes}, memberCount, publicUrl,
//!   counterRates: [{currency, buy, sell}]}` — the stored `bookingRules` or v2's default
//!   `{enabled: true, minutes: 30}`; the branch's verified members, counted; `publicUrl` always
//!   `""` (`xchange-v3.md:1832`); one entry per currency with a
//!   counter rate in the branch, the latest row (as `counter_rate_in` reads it), sorted by code,
//!   rates as decimal strings. Bounded by the branch's currencies. The branch's name (the
//!   settings title, `bookings.js:16`) is not here: a menu reads names from the shell's
//!   `admin_branches` (lane controller on #462, decision 2).
//! - §4.2: every writer of what the two views read goes through `BranchTx` — one write per
//!   writer, then the same `since` answers `changed: true` with the write in `data`: `create`,
//!   `confirm`, `cancel`, `settings`, `member` (`/api/bookings`), `adminSaveRate` (the counter
//!   rates, `/api/rpc`), and the desk's hand-off (#414: a save naming the booking marks it
//!   `USED`).
//!
//! The view has no request clock (`PagedView::page` takes none), so it reads the server's
//! clock; the room's route clock is therefore set to the real time (whole seconds) before
//! anyone signs in, and moved one second before every write, so no two bookings share a
//! `created_at` and nothing is ordered by a frozen time.
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real
//! sessions made at the real sign-in doors (`rpc_common`); writes are the real routes.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`) for the view tests; nothing skips.
//!
//! Amended for #758 (RE-4d) by its ticket author (claude/e5ab6c), who owns this file for RE-4
//! (ticket author's ruling 2 on #758), `docs/design/blueprint/rates.md` D-3, §4.8, §4.9 R3 and R6:
//! - the counter rates both views show are the lines of the branch's current publication, per 1
//!   through `CompatRate::per_one` (the screen sends them back as `/api/bookings`'
//!   `expectedRate`, per 1 by R3), and `counterPrincipal` is `ExchangeArithmetic::thb` at the
//!   line's rate and unit. `Room::set_rate` therefore also publishes the branch's lines with the
//!   new one, and a per-1 rate prints exactly, at least 4 places (`"36.2500"`, `"0.2310"`);
//! - `a_pending_row_carries_the_counter_rate_and_principal`: after the counter moves, confirm is
//!   refused and the row keeps its create-time line (R6), where it was confirmed at the new rate;
//! - `every_writer_of_the_views_tables_moves_the_version`: the counter rates' writer is a
//!   publication through the rate routes, no longer `adminSaveRate` (D-3: a direct save is not
//!   the counter rate).
//!
//! Re-expected for RE-7 #604 by the ticket author (rates.md §8 migration 6): `Room::set_rate` and
//! `the_counter_rates_are_the_published_lines_per_1` no longer write `rates` rows; the
//! publications they make are unchanged.
//!
//! Amended for #943 (RF-4) by its up-front test author (claude/dell-claude-4, on the lane
//! controller's order): `admin_booking_settings` also answers `freshness` (`RateFreshnessDto`,
//! RF-1), so the two key-set pins below (`settings_answers_rules_members_public_url_and_counter_rates`
//! and `the_generated_types_carry_the_tickets_fields`) name it. Its values are pinned in
//! `v3_admin_booking_settings_freshness_test.rs`.

mod common;
mod compat_common;
mod rates_re4_common;
mod rpc_common;
mod snapshot_common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use actix_web::http::Method;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use compat_common::Fixture;
use rpc_common::{Answer, App, Call, Cookies, Desk, Replay, one_of_each_role, sign_in};
use serde_json::{Value, json};
use snapshot_common::time;
use xc_auth::domain::policy::entities::policy::Policy;
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::branch_versions::BranchVersions;
use xc_kernel::ids::BranchId;
use xchange::presentation::http::rpc::RpcClock;
use xchange::presentation::http::v3::VIEWS;

const ORIGIN: &str = "http://desk.test";
const ROUTE: &str = "/api/bookings";
const RPC: &str = "/api/rpc";
/// `BookingError::RateMoved` (T15-a), the refusal of a moved counter.
const RATE_MOVED: &str = "เรทเปลี่ยน กรุณาโหลดใหม่";

const MAIN: &str = "MAIN";
const AIRPORT: &str = "AIRPORT";
const AIRPORT_NAME: &str = "Airport";

const BOOKINGS: &str = "admin_bookings";
const SETTINGS: &str = "admin_booking_settings";
/// The row the owner route checks (`http/bookings.rs`, `owner_bookings` step 1).
const INHERITS_PATH: EntryPath = EntryPath::OwnerRoute;
const INHERITS_ACTION: &str = "/api/bookings";

/// LIVE customers of `MAIN`: (id, name).
const SOMCHAI: (&str, &str) = ("C-MAIN-1", "สมชาย ใจดี");
const SUDA: (&str, &str) = ("C-MAIN-2", "สุดา รักไทย");

const SECRET_A: &str = "6f1c2a3b-4d5e-4f60-8a7b-9c0d1e2f3a4b";
const SECRET_B: &str = "0a1b2c3d-4e5f-4061-8273-a4b5c6d7e8f9";

/// #413's 13 keys (`booking_json`) and the two of design §12.14.
const ROW_FIELDS: [&str; 15] = [
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
    "counterRate",
    "counterPrincipal",
];

static NEXT: AtomicU64 = AtomicU64::new(1);

fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A request id, different on every call.
fn request_id() -> String {
    format!("00000000-0000-4000-8000-{:012x}", next())
}

fn stamp(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The real time, whole seconds.
fn real_now() -> DateTime<Utc> {
    DateTime::from_timestamp(Utc::now().timestamp(), 0).expect("a time")
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

/// `text` as one query value.
fn enc(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn ids(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .map(|r| {
            r["id"]
                .as_str()
                .unwrap_or_else(|| panic!("an id: {r}"))
                .to_owned()
        })
        .collect()
}

fn names(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .map(|r| {
            r["name"]
                .as_str()
                .unwrap_or_else(|| panic!("a name: {r}"))
                .to_owned()
        })
        .collect()
}

/// A MANUAL draft line of the rate routes (`v3_rate_engine_routes_test.rs` `manual_line`).
fn manual_line(currency: &str, buy: &str, sell: &str) -> Value {
    json!({
        "currency": currency,
        "buyMode": "MANUAL",
        "sellMode": "MANUAL",
        "manualBuy": buy,
        "manualSell": sell,
        "reason": "published by the RE-4d test",
    })
}

fn row<'a>(rows: &'a [Value], id: &str) -> &'a Value {
    rows.iter()
        .find(|r| r["id"] == json!(id))
        .unwrap_or_else(|| panic!("{id} in {rows:?}"))
}

/// A walk-in's `create` payload: BUY `amount` USD at `MAIN`'s counter 36.25.
fn walk_in(branch: &str, name: &str, phone: &str) -> Value {
    json!({
        "branchId": branch,
        "requestId": request_id(),
        "name": name,
        "phone": phone,
        "type": "BUY",
        "currency": "USD",
        "amount": "1000",
        "expectedRate": if branch == MAIN { "36.25" } else { "36.1" },
    })
}

// ── the shop ─────────────────────────────────────────────────────────────────

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
}

impl Room {
    /// `MAIN` and `AIRPORT`, each with an OPEN shift; counter rates `MAIN` USD 36.25 / 36.75,
    /// `MAIN` EUR 39.5 / 40.1, `AIRPORT` USD 36.1 / 36.9; two LIVE customers of `MAIN`; the
    /// owner and one employee of every staff role in `MAIN`, all signed in. The route clock at
    /// the real time before anybody signs in.
    async fn open() -> Room {
        let desk = Desk::open().await;
        desk.clock.set(real_now());
        desk.db.seed_branch(AIRPORT, AIRPORT_NAME).await;
        let app = rpc_app!(desk);
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        let room = Room { desk, app, cookies };
        room.seed_shift("SH-MAIN", MAIN).await;
        room.seed_shift("SH-AIR", AIRPORT).await;
        room.set_rate(MAIN, "USD", "36.25", "36.75").await;
        room.set_rate(MAIN, "EUR", "39.5", "40.1").await;
        room.set_rate(AIRPORT, "USD", "36.1", "36.9").await;
        for (id, name) in [SOMCHAI, SUDA] {
            room.seed_customer(id, MAIN, name).await;
        }
        room
    }

    fn now(&self) -> DateTime<Utc> {
        self.desk.clock.now()
    }

    fn tick(&self) {
        self.desk.clock.advance(Duration::seconds(1));
    }

    async fn seed_shift(&self, id: &str, branch: &str) {
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, closed_at, closed_by, \
                 open_request_id, teller_id) \
             VALUES ($1, $2, 'OPEN', $3, 'owner', NULL, NULL, $4, (SELECT u.id FROM users u WHERE u.legacy_employee_id IS NULL AND EXISTS (SELECT 1 FROM user_roles r WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL)))",
        )
        .bind(id)
        .bind(branch)
        .bind(self.now() - Duration::hours(1))
        .bind(format!("open-{id}"))
        .execute(&self.desk.db.pool)
        .await
        .expect("insert into shifts");
    }

    async fn seed_customer(&self, id: &str, branch: &str, name: &str) {
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             SELECT $1, b.organization_id, $1, $3, 'LIVE', $4 FROM branches b WHERE b.id = $2",
        )
        .bind(id)
        .bind(branch)
        .bind(name)
        .bind(self.now() - Duration::hours(1))
        .execute(&self.desk.db.pool)
        .await
        .expect("insert into customers");
    }

    /// A counter rate behind the routes' back (test data, not a writer under test).
    ///
    /// Amended for #758 (RE-4d): also publishes the branch's current lines with this one (D-3:
    /// the counter is the current publication), behind the routes' back too.
    ///
    /// Re-expected for RE-7 #604 by the ticket author: only the publication; the `rates` row it
    /// also wrote is gone with the table (rates.md §8 migration 6).
    async fn set_rate(&self, branch: &str, code: &str, buy: &str, sell: &str) {
        let pool = self.desk.db.pool.clone();
        let mut lines = self.published(branch).await;
        lines.retain(|(c, _, _)| c != code);
        lines.push((code.to_owned(), buy.to_owned(), sell.to_owned()));
        let lines: Vec<(&str, &str, &str)> = lines
            .iter()
            .map(|(c, b, s)| (c.as_str(), b.as_str(), s.as_str()))
            .collect();
        rates_re4_common::publish(&pool, branch, &lines).await;
    }

    /// `(currency, buy, sell)` of the branch's current publication.
    async fn published(&self, branch: &str) -> Vec<(String, String, String)> {
        sqlx::query_as(
            "SELECT l.currency_code::text, l.buy_rate::text, l.sell_rate::text \
             FROM branches b JOIN published_rate_lines l \
               ON l.publication_id = b.current_publication_id \
             WHERE b.id = $1",
        )
        .bind(branch)
        .fetch_all(&self.desk.db.pool)
        .await
        .expect("the current lines")
    }

    /// The owner publishes the branch's current lines with `(code, buy, sell)` through the rate
    /// routes (`POST /api/rate-drafts`, `…/submit`, `…/publish`), as the rates screen does.
    async fn publish_through_routes(&self, branch: &str, code: &str, buy: &str, sell: &str) {
        let mut lines: Vec<Value> = self
            .published(branch)
            .await
            .into_iter()
            .filter(|(c, _, _)| c != code)
            .map(|(c, b, s)| manual_line(&c, &b, &s))
            .collect();
        lines.push(manual_line(code, buy, sell));
        let draft = self
            .command(
                "/api/rate-drafts",
                json!({ "branch": branch, "lines": lines }),
            )
            .await;
        let id = draft["id"].as_str().expect("the draft's id").to_owned();
        self.command(
            &format!("/api/rate-drafts/{id}/submit"),
            json!({ "branch": branch }),
        )
        .await;
        let version: Option<i32> = sqlx::query_scalar(
            "SELECT p.version FROM branches b JOIN rate_publications p \
               ON p.id = b.current_publication_id WHERE b.id = $1",
        )
        .bind(branch)
        .fetch_optional(&self.desk.db.pool)
        .await
        .expect("the current version");
        self.command(
            &format!("/api/rate-drafts/{id}/publish"),
            json!({ "branch": branch, "expectedVersion": version }),
        )
        .await;
    }

    /// A v3 command as the owner, after the clock moves one second; its 200 body.
    async fn command(&self, path: &str, body: Value) -> Value {
        self.tick();
        let call = Call::post(ORIGIN, path, body.to_string())
            .header("idempotency-key", &request_id())
            .cookie(&self.cookies["OWNER"]);
        let answer = self.app.send(call).await;
        assert_eq!(answer.status, 200, "{path} {body}: {:.400}", answer.text);
        answer.json()
    }

    /// A booking row behind the routes' back, for the states no route makes at a chosen time.
    #[allow(clippy::too_many_arguments)]
    async fn insert_booking(
        &self,
        id: &str,
        status: &str,
        code: &str,
        side: &str,
        expires_at: Option<DateTime<Utc>>,
        member_id: Option<&str>,
        created_at: DateTime<Utc>,
    ) {
        sqlx::query(
            "INSERT INTO bookings (id, branch_id, request_id, request_hash, name, phone, member_id, \
             type, code, amount, rate, status, created_at, created_by, expires_at) \
             VALUES ($1, 'MAIN', $1, $1, $1, '0812345678', $2, $3, $4, 500::numeric, \
             10::numeric, $5, $6, 'owner', $7)",
        )
        .bind(id)
        .bind(member_id)
        .bind(side)
        .bind(code)
        .bind(status)
        .bind(created_at)
        .bind(expires_at)
        .execute(&self.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("insert booking {id}: {e}"));
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

    /// A view as `who`, answered 200 with JSON, never cached.
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
    async fn data(&self, view: &str, query: &str) -> Value {
        let body = self.view("OWNER", view, query).await;
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

    /// The owner's page of `admin_bookings`.
    async fn rows(&self, query: &str) -> Vec<Value> {
        let data = self.data(BOOKINGS, query).await;
        data.as_array()
            .unwrap_or_else(|| panic!("{BOOKINGS}?{query}: a list of rows, got {data}"))
            .clone()
    }

    async fn settings(&self, branch: &str) -> Value {
        self.data(SETTINGS, &format!("branch={branch}")).await
    }

    /// `action` on `POST /api/bookings` as the owner, after the clock moves one second.
    async fn act(&self, action: &str, payload: Value) -> Value {
        self.tick();
        let call =
            Call::action(ORIGIN, ROUTE, action, payload.clone()).cookie(&self.cookies["OWNER"]);
        let answer = self.app.send(call).await;
        assert_eq!(
            answer.status, 200,
            "{action} {payload}: {:.400}",
            answer.text
        );
        answer.json()
    }

    /// `create`; the booking as `{saved, booking}` prints it.
    async fn create(&self, payload: Value) -> Value {
        let answer = self.act("create", payload.clone()).await;
        assert_eq!(answer["saved"], json!(true), "create {payload}: {answer}");
        answer["booking"].clone()
    }

    async fn create_id(&self, payload: Value) -> String {
        self.create(payload)
            .await
            .get("id")
            .and_then(Value::as_str)
            .expect("the new booking's id")
            .to_owned()
    }

    /// The owner issues a member code for `customer` of `MAIN`.
    async fn issue_member(&self, customer: &str, secret: &str) {
        self.act(
            "member",
            json!({ "branchId": MAIN, "customerId": customer, "identityConfirmed": true,
                    "phone": "081-234-5678", "memberSecret": secret }),
        )
        .await;
    }

    /// A PENDING member booking of `customer` (who must be a member): `side` `amount` `code`
    /// at the counter's `expected`.
    async fn pending(
        &self,
        customer: &str,
        side: &str,
        code: &str,
        amount: &str,
        expected: &str,
    ) -> String {
        let booking = self
            .create(json!({
                "branchId": MAIN, "requestId": request_id(), "name": "Member Customer",
                "phone": "081-234-5678", "type": side, "currency": code, "amount": amount,
                "expectedRate": expected, "customerId": customer,
            }))
            .await;
        assert_eq!(booking["status"], json!("PENDING"), "{booking}");
        booking["id"].as_str().expect("an id").to_owned()
    }

    /// The route's own `list` rows of `branch`, by id.
    async fn listed(&self, branch: &str) -> BTreeMap<String, Value> {
        let call = Call::action(ORIGIN, ROUTE, "list", json!({ "branchId": branch }))
            .cookie(&self.cookies["OWNER"]);
        let answer = self.app.send(call).await;
        assert_eq!(answer.status, 200, "list: {:.300}", answer.text);
        answer.json()["bookings"]
            .as_array()
            .expect("the list's bookings")
            .iter()
            .map(|b| (b["id"].as_str().expect("an id").to_owned(), b.clone()))
            .collect()
    }

    async fn version(&self, branch: &str) -> i64 {
        BranchVersions::new(self.desk.db.pool.clone())
            .current(&BranchId::new(branch))
            .await
            .expect("read the branch version")
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

/// Every caller kind gets what `Policy::allow(actor, OwnerRoute, "/api/bookings")` gives: 200
/// with data when it allows, its refusal (status and sentence) otherwise.
async fn answers_exactly_whom_the_route_allows(room: &Room, view: &str) -> (usize, usize) {
    let mut allowed = 0;
    let mut refused = 0;
    for (who, actor) in callers() {
        let policy = Policy::allow(actor.as_ref(), INHERITS_PATH, &Action::new(INHERITS_ACTION));
        let answer = room.get(&who, &view_path(view, "branch=MAIN")).await;
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

// ── who may read ─────────────────────────────────────────────────────────────

/// W8 (ticket §8 rows 1–2): the owner reads both views; every employee gets the owner route's
/// refusal, exactly as `POST /api/bookings` answers them (403 `เฉพาะเจ้าของระบบเท่านั้น`), and
/// nobody gets the sign-in refusal.
#[actix_web::test]
async fn both_views_answer_exactly_whom_the_bookings_route_allows() {
    let room = Room::open().await;
    for view in [BOOKINGS, SETTINGS] {
        let (allowed, refused) = answers_exactly_whom_the_route_allows(&room, view).await;
        assert_eq!(
            allowed, 10,
            "{view}: the owner, and the owner beside each of nine roles"
        );
        assert_eq!(
            refused, 10,
            "{view}: nobody, and each of the nine roles alone"
        );
    }
    // an employee is refused as the route refuses them, not answered for their own branch
    let answer = room.get("FRONT", &view_path(BOOKINGS, "")).await;
    assert_eq!(answer.status, 403, "{:.300}", answer.text);
    assert_eq!(
        answer.json()["error"]["message"],
        json!("เฉพาะเจ้าของระบบเท่านั้น")
    );
    room.finish().await;
}

// ── admin_bookings: the rows ─────────────────────────────────────────────────

/// Ticket §8 row 1 and §7: the branch's bookings, newest first, each one #413's `list` row with
/// money and rates as decimal strings and the two counter fields; another branch's bookings are
/// not in it, and the owner naming that branch gets only those.
#[actix_web::test]
async fn the_page_is_the_branchs_list_rows_newest_first_with_decimal_strings() {
    let room = Room::open().await;
    let first = room
        .create_id(walk_in(MAIN, "Somchai Jaidee", "081-234-5678"))
        .await;
    let second = room
        .create_id(walk_in(MAIN, "Suda Rakthai", "089-999-0000"))
        .await;
    let mut sell = walk_in(MAIN, "Anan Sell", "02 111 2222");
    sell["type"] = json!("SELL");
    sell["currency"] = json!("EUR");
    sell["amount"] = json!("250.5");
    sell["expectedRate"] = json!("40.1");
    let third = room.create_id(sell).await;
    let airport = room
        .create_id(walk_in(AIRPORT, "Airport Walk-in", "081-000-0000"))
        .await;

    let body = room.view("OWNER", BOOKINGS, "branch=MAIN").await;
    assert_eq!(body["version"], json!(room.version(MAIN).await), "{body}");
    assert_eq!(body["changed"], json!(true), "{body}");
    let rows = body["data"].as_array().expect("a list of rows").clone();
    assert_eq!(
        ids(&rows),
        [third.clone(), second.clone(), first.clone()],
        "newest first"
    );

    let listed = room.listed(MAIN).await;
    for r in &rows {
        let id = r["id"].as_str().expect("an id");
        assert_eq!(keys(r), set_of(&ROW_FIELDS), "{id}: the row's fields: {r}");
        let l = &listed[id];
        for same in [
            "id",
            "branchId",
            "name",
            "phone",
            "member",
            "type",
            "currency",
            "status",
            "createdAt",
            "expiresAt",
            "transactionRef",
        ] {
            assert_eq!(r[same], l[same], "{id}.{same}: as `list` prints it");
        }
        assert_eq!(
            r["counterRate"],
            Value::Null,
            "{id}: a CONFIRMED row has no counter rate"
        );
        assert_eq!(r["counterPrincipal"], Value::Null, "{id}");
        assert_eq!(
            r["status"],
            json!("CONFIRMED"),
            "{id}: a walk-in is CONFIRMED"
        );
    }
    // money and rates as `Money` / `Rate` print them, never JSON numbers
    assert_eq!(row(&rows, &first)["amount"], json!("1000.00"));
    assert_eq!(row(&rows, &first)["rate"], json!("36.2500"));
    assert_eq!(row(&rows, &third)["amount"], json!("250.50"));
    assert_eq!(row(&rows, &third)["rate"], json!("40.1000"));
    assert_eq!(row(&rows, &third)["type"], json!("SELL"));
    assert_eq!(row(&rows, &third)["currency"], json!("EUR"));
    assert_eq!(row(&rows, &first)["branchId"], json!(MAIN));
    assert!(
        !ids(&rows).contains(&airport),
        "AIRPORT's booking is not MAIN's"
    );

    let rows = room.rows("branch=AIRPORT").await;
    assert_eq!(
        ids(&rows),
        [airport.as_str()],
        "the owner naming AIRPORT reads AIRPORT's"
    );
    assert_eq!(row(&rows, &airport)["rate"], json!("36.1000"));
    room.finish().await;
}

/// Ticket §9 row 1: no booking → an empty page (the screen shows v2's empty text).
#[actix_web::test]
async fn a_branch_without_bookings_answers_an_empty_page() {
    let room = Room::open().await;
    assert_eq!(room.rows("branch=MAIN").await, Vec::<Value>::new());
    room.finish().await;
}

/// Design §4.2 (a list that grows is a page): `limit` rows newest first, `before` = the last
/// row's id continues with the next older ones, and the pages together are every row once.
#[actix_web::test]
async fn pages_by_before_and_limit_newest_first() {
    let room = Room::open().await;
    let mut made = Vec::new();
    for n in 1..=5 {
        made.push(
            room.create_id(walk_in(MAIN, &format!("Page {n}"), "081-234-5678"))
                .await,
        );
    }
    let newest_first: Vec<String> = made.iter().rev().cloned().collect();

    let page1 = room.rows("branch=MAIN&limit=2").await;
    assert_eq!(ids(&page1), newest_first[0..2]);
    let page2 = room
        .rows(&format!("branch=MAIN&limit=2&before={}", newest_first[1]))
        .await;
    assert_eq!(ids(&page2), newest_first[2..4]);
    let page3 = room
        .rows(&format!("branch=MAIN&limit=2&before={}", newest_first[3]))
        .await;
    assert_eq!(ids(&page3), newest_first[4..5]);
    let after_last = room
        .rows(&format!("branch=MAIN&limit=2&before={}", newest_first[4]))
        .await;
    assert!(
        after_last.is_empty(),
        "nothing older than the oldest: {after_last:?}"
    );
    room.finish().await;
}

/// Ticket K2 and design §12.1: `q` is a literal, case-folded substring of id, name or phone,
/// filtered by the server; `%`, `_` and `\` match only themselves; empty = no filter; paging
/// stays inside the filter.
#[actix_web::test]
async fn q_filters_id_name_and_phone_literally_and_case_folded() {
    let room = Room::open().await;
    // phones and queries with spaces or letters a booking id (`BK-<uuid>`, hex) never holds
    let somchai = room
        .create_id(walk_in(MAIN, "Somchai Jaidee", "081 234 5678"))
        .await;
    let suda = room
        .create_id(walk_in(MAIN, "Suda Rakthai", "089 999 0000"))
        .await;
    let percent = room
        .create_id(walk_in(MAIN, "50%_off Tom", "02 111 2222"))
        .await;
    let lookalike = room
        .create_id(walk_in(MAIN, "50xxoff Ann", "02 333 4444"))
        .await;
    let backslash = room
        .create_id(walk_in(MAIN, "Tom\\Jerry", "02 555 6666"))
        .await;
    let thai = room
        .create_id(walk_in(MAIN, SOMCHAI.1, "02 777 8888"))
        .await;
    let all = [
        thai.clone(),
        backslash.clone(),
        lookalike.clone(),
        percent.clone(),
        suda.clone(),
        somchai.clone(),
    ];

    let q = |text: &str| format!("branch=MAIN&q={}", enc(text));
    let cases: Vec<(String, Vec<String>)> = vec![
        ("somchai".into(), vec![somchai.clone()]),
        ("SUDA".into(), vec![suda.clone()]),
        ("234 5678".into(), vec![somchai.clone()]),
        ("999 0".into(), vec![suda.clone()]),
        (suda.to_uppercase(), vec![suda.clone()]),
        ("%".into(), vec![percent.clone()]),
        ("_".into(), vec![percent.clone()]),
        ("0%_O".into(), vec![percent.clone()]),
        ("m\\j".into(), vec![backslash.clone()]),
        ("\\".into(), vec![backslash.clone()]),
        ("สมชาย".into(), vec![thai.clone()]),
        ("nobody here".into(), vec![]),
    ];
    for (text, expected) in cases {
        let rows = room.rows(&q(&text)).await;
        assert_eq!(ids(&rows), expected, "q={text:?}: {:?}", names(&rows));
    }
    assert_eq!(
        ids(&room.rows("branch=MAIN&q=").await),
        all,
        "q empty = no filter"
    );
    assert_eq!(ids(&room.rows("branch=MAIN").await), all, "no q");

    // paging stays inside the filter: "tom" is in two names
    assert_eq!(
        ids(&room.rows(&q("tom")).await),
        [backslash.clone(), percent.clone()]
    );
    let first = room.rows(&format!("{}&limit=1", q("tom"))).await;
    assert_eq!(ids(&first), [backslash.as_str()]);
    let second = room
        .rows(&format!("{}&limit=1&before={backslash}", q("tom")))
        .await;
    assert_eq!(
        ids(&second),
        [percent.as_str()],
        "the next match, not the next row"
    );
    room.finish().await;
}

/// Design §12.1: `q` longer than 100 characters is `400 invalidRequest` naming `q`, before the
/// `since` shortcut; 100 is read.
#[actix_web::test]
async fn q_over_100_characters_is_refused_naming_q() {
    let room = Room::open().await;
    let v = room.version(MAIN).await;
    for (label, text) in [
        ("101 latin", "a".repeat(101)),
        ("101 thai", "ก".repeat(101)),
    ] {
        for query in [
            format!("branch=MAIN&q={}", enc(&text)),
            format!("branch=MAIN&since={v}&q={}", enc(&text)),
        ] {
            let answer = room.get("OWNER", &view_path(BOOKINGS, &query)).await;
            assert_eq!(answer.status, 400, "{label} {query}: {:.300}", answer.text);
            let body = answer.json();
            assert_eq!(
                body["error"]["code"],
                json!("invalidRequest"),
                "{label}: {body}"
            );
            assert_eq!(body["error"]["field"], json!("q"), "{label}: {body}");
            assert!(body.get("data").is_none(), "{label}: {body}");
        }
    }
    for text in ["a".repeat(100), "ก".repeat(100)] {
        let rows = room.rows(&format!("branch=MAIN&q={}", enc(&text))).await;
        assert!(rows.is_empty(), "100 characters are read: {rows:?}");
    }
    room.finish().await;
}

/// Design §12.14 (confirm figures): a PENDING row carries the branch's latest counter rate for
/// its currency and side and the amount at that rate; its own `rate` stays the one stored. Every
/// other row, and a PENDING row whose currency has no counter rate, carries `null`.
///
/// Amended for #758 (RE-4d, rates §4.9 R6): the counter is the current published line. This test
/// confirmed the PENDING booking at the moved counter's 36.5 and expected the row locked at
/// 36.5000. R6: confirm compares the line stored at create with the current one and never
/// re-locks, so after the move confirm is refused, at the new rate and at the old one, and the
/// row is unchanged: PENDING, 36.2500, on its create-time line.
#[actix_web::test]
async fn a_pending_row_carries_the_counter_rate_and_principal() {
    let room = Room::open().await;
    room.issue_member(SOMCHAI.0, SECRET_A).await;
    room.issue_member(SUDA.0, SECRET_B).await;
    let buy = room.pending(SOMCHAI.0, "BUY", "USD", "1000", "36.25").await;
    let sell = room.pending(SUDA.0, "SELL", "EUR", "200", "40.1").await;
    let walk = room
        .create_id(walk_in(MAIN, "Walk In", "081-234-5678"))
        .await;
    let create_time_usd = rates_re4_common::line(&room.desk.db.pool, MAIN, "USD")
        .await
        .id;

    let rows = room.rows("branch=MAIN").await;
    assert_eq!(row(&rows, &buy)["status"], json!("PENDING"));
    assert_eq!(row(&rows, &buy)["member"], json!(true));
    assert_eq!(
        row(&rows, &buy)["counterRate"],
        json!("36.2500"),
        "BUY reads `buy`"
    );
    assert_eq!(row(&rows, &buy)["counterPrincipal"], json!("36250.00"));
    assert_eq!(
        row(&rows, &sell)["counterRate"],
        json!("40.1000"),
        "SELL reads `sell`"
    );
    assert_eq!(row(&rows, &sell)["counterPrincipal"], json!("8020.00"));
    assert_eq!(row(&rows, &walk)["counterRate"], Value::Null);
    assert_eq!(row(&rows, &walk)["counterPrincipal"], Value::Null);

    // the latest counter row wins; the booking's own rate stays
    room.set_rate(MAIN, "USD", "36.5", "37").await;
    let rows = room.rows("branch=MAIN").await;
    assert_eq!(row(&rows, &buy)["counterRate"], json!("36.5000"));
    assert_eq!(row(&rows, &buy)["counterPrincipal"], json!("36500.00"));
    assert_eq!(
        row(&rows, &buy)["rate"],
        json!("36.2500"),
        "the stored rate"
    );

    // a PENDING booking in a currency the branch has no counter rate for
    let (member_id,): (String,) =
        sqlx::query_as("SELECT id FROM members WHERE branch_id = 'MAIN' AND customer_id = $1")
            .bind(SOMCHAI.0)
            .fetch_one(&room.desk.db.pool)
            .await
            .expect("Somchai's member");
    room.insert_booking(
        "BK-no-rate",
        "PENDING",
        "GBP",
        "BUY",
        None,
        Some(&member_id),
        room.now() + Duration::seconds(30),
    )
    .await;
    let rows = room.rows("branch=MAIN").await;
    assert_eq!(row(&rows, "BK-no-rate")["counterRate"], Value::Null);
    assert_eq!(row(&rows, "BK-no-rate")["counterPrincipal"], Value::Null);

    // RE-4d R6: the counter moved since create, so confirm is refused at the new rate and at
    // the old one, and nothing is written: the row keeps its rate and its create-time line
    for expected in ["36.5", "36.25"] {
        room.tick();
        let call = Call::action(
            ORIGIN,
            ROUTE,
            "confirm",
            json!({ "branchId": MAIN, "id": buy,
                    "expiresAt": stamp(room.now() + Duration::days(1)),
                    "expectedRate": expected }),
        )
        .cookie(&room.cookies["OWNER"]);
        room.app.send(call).await.assert_refused(
            400,
            RATE_MOVED,
            &format!("confirm at {expected}"),
        );
    }
    let rows = room.rows("branch=MAIN").await;
    assert_eq!(row(&rows, &buy)["status"], json!("PENDING"));
    assert_eq!(row(&rows, &buy)["rate"], json!("36.2500"));
    assert_eq!(row(&rows, &buy)["counterRate"], json!("36.5000"));
    let (stored_line,): (Option<String>,) =
        sqlx::query_as("SELECT published_rate_line_id::text FROM bookings WHERE id = $1")
            .bind(&buy)
            .fetch_one(&room.desk.db.pool)
            .await
            .expect("the booking's line");
    assert_eq!(
        stored_line,
        Some(create_time_usd),
        "the line of create (R1, R6)"
    );
    room.finish().await;
}

/// Added for #758 (RE-4d) by its ticket author (claude/e5ab6c), rates D-3, §3.2 and §4.9 R2/R3
/// (ticket author's ruling 2 on #758): both views' counter rates are the current publication's
/// lines per 1, exactly, and `counterPrincipal` is `ExchangeArithmetic::thb` at the line's rate
/// per its unit, HALF_UP once. The screen sends them back as `expectedRate` and create and confirm
/// accept them. JPY is per 100 (RU-1): 23.1 / 23.5 print `"0.2310"` / `"0.2350"`; SELL 3 JPY is
/// 3 × 23.5 ÷ 100 = 0.705 → `"0.71"` (banker's rounding gives 0.70). A currency with no line
/// (GBP) is not a counter rate. (RE-7 #604: before RE-7 GBP also had a direct-save `rates` row
/// here as a decoy; the table is dropped.)
#[actix_web::test]
async fn the_counter_rates_are_the_published_lines_per_1() {
    let room = Room::open().await;
    room.set_rate(MAIN, "JPY", "23.1", "23.5").await;

    let settings = room.settings(MAIN).await;
    assert_eq!(
        settings["counterRates"],
        json!([
            { "currency": "EUR", "buy": "39.5000", "sell": "40.1000" },
            { "currency": "JPY", "buy": "0.2310", "sell": "0.2350" },
            { "currency": "USD", "buy": "36.2500", "sell": "36.7500" },
        ]),
        "the published lines per 1; GBP has no line"
    );
    let jpy_buy = settings["counterRates"][1]["buy"]
        .as_str()
        .expect("JPY's buy")
        .to_owned();
    let jpy_sell = settings["counterRates"][1]["sell"]
        .as_str()
        .expect("JPY's sell")
        .to_owned();

    room.issue_member(SOMCHAI.0, SECRET_A).await;
    room.issue_member(SUDA.0, SECRET_B).await;
    let buy = room
        .pending(SOMCHAI.0, "BUY", "JPY", "1000", &jpy_buy)
        .await;
    let sell = room.pending(SUDA.0, "SELL", "JPY", "3", &jpy_sell).await;

    let rows = room.rows("branch=MAIN").await;
    assert_eq!(row(&rows, &buy)["counterRate"], json!("0.2310"));
    assert_eq!(
        row(&rows, &buy)["counterPrincipal"],
        json!("231.00"),
        "1000 × 23.1 ÷ 100"
    );
    assert_eq!(row(&rows, &sell)["counterRate"], json!("0.2350"));
    assert_eq!(
        row(&rows, &sell)["counterPrincipal"],
        json!("0.71"),
        "0.705 HALF_UP"
    );

    let counter = row(&rows, &buy)["counterRate"].clone();
    room.act(
        "confirm",
        json!({ "branchId": MAIN, "id": buy, "expiresAt": stamp(room.now() + Duration::days(1)),
                "expectedRate": counter }),
    )
    .await;
    let rows = room.rows("branch=MAIN").await;
    assert_eq!(row(&rows, &buy)["status"], json!("CONFIRMED"));
    room.finish().await;
}

/// Ticket §9 row 3: the status is the server's — a CONFIRMED booking past its expiry reads
/// `EXPIRED`, one before it `CONFIRMED`; a CANCELLED one stays `CANCELLED` whatever its expiry.
#[actix_web::test]
async fn a_confirmed_booking_past_its_expiry_reads_expired() {
    let room = Room::open().await;
    let now = real_now();
    room.insert_booking(
        "BK-past",
        "CONFIRMED",
        "USD",
        "BUY",
        Some(now - Duration::minutes(1)),
        None,
        now - Duration::minutes(40),
    )
    .await;
    room.insert_booking(
        "BK-future",
        "CONFIRMED",
        "USD",
        "SELL",
        Some(now + Duration::days(1)),
        None,
        now - Duration::minutes(30),
    )
    .await;
    room.insert_booking(
        "BK-cancelled",
        "CANCELLED",
        "USD",
        "BUY",
        Some(now - Duration::minutes(1)),
        None,
        now - Duration::minutes(20),
    )
    .await;
    let rows = room.rows("branch=MAIN").await;
    assert_eq!(ids(&rows), ["BK-cancelled", "BK-future", "BK-past"]);
    assert_eq!(row(&rows, "BK-past")["status"], json!("EXPIRED"));
    assert_eq!(row(&rows, "BK-future")["status"], json!("CONFIRMED"));
    assert_eq!(row(&rows, "BK-cancelled")["status"], json!("CANCELLED"));
    assert_eq!(
        row(&rows, "BK-past")["expiresAt"],
        json!(stamp(now - Duration::minutes(1))),
        "expiresAt as `list` prints it"
    );
    room.finish().await;
}

// ── admin_booking_settings ───────────────────────────────────────────────────

/// `bookings.js:8-9`, `:16` and design §12.17: the rules (v2's default when none is stored), the
/// branch's verified members counted, `publicUrl` always `""`, and the
/// latest counter rate of each currency the branch has one for, sorted by code.
#[actix_web::test]
async fn settings_answers_rules_members_public_url_and_counter_rates() {
    let room = Room::open().await;
    let data = room.settings(MAIN).await;
    assert_eq!(
        keys(&data),
        set_of(&[
            "rules",
            "memberCount",
            "publicUrl",
            "counterRates",
            "freshness"
        ]),
        "{data}"
    );
    assert_eq!(
        data["rules"],
        json!({ "enabled": true, "minutes": 30 }),
        "v2's default"
    );
    assert_eq!(data["memberCount"], json!(0));
    assert_eq!(data["publicUrl"], json!(""));
    assert_eq!(
        data["counterRates"],
        json!([
            { "currency": "EUR", "buy": "39.5000", "sell": "40.1000" },
            { "currency": "USD", "buy": "36.2500", "sell": "36.7500" },
        ])
    );

    room.act(
        "settings",
        json!({ "branchId": MAIN, "enabled": false, "minutes": 45 }),
    )
    .await;
    room.issue_member(SOMCHAI.0, SECRET_A).await;
    room.issue_member(SUDA.0, SECRET_B).await;
    room.set_rate(MAIN, "USD", "36.5", "37").await;
    let data = room.settings(MAIN).await;
    assert_eq!(data["rules"], json!({ "enabled": false, "minutes": 45 }));
    assert_eq!(data["memberCount"], json!(2));
    assert_eq!(
        data["counterRates"],
        json!([
            { "currency": "EUR", "buy": "39.5000", "sell": "40.1000" },
            { "currency": "USD", "buy": "36.5000", "sell": "37.0000" },
        ]),
        "the current publication's line of each currency (RE-4d)"
    );

    let airport = room.settings(AIRPORT).await;
    assert_eq!(airport["rules"], json!({ "enabled": true, "minutes": 30 }));
    assert_eq!(
        airport["memberCount"],
        json!(0),
        "MAIN's members are MAIN's"
    );
    assert_eq!(
        airport["counterRates"],
        json!([{ "currency": "USD", "buy": "36.1000", "sell": "36.9000" }])
    );
    room.finish().await;
}

// ── §4.2: every writer moves the version ─────────────────────────────────────

/// With `since` equal to the branch's version, both views answer `changed: false` and no data.
async fn unchanged(room: &Room, v: i64) {
    for view in [BOOKINGS, SETTINGS] {
        let body = room
            .view("OWNER", view, &format!("branch=MAIN&since={v}"))
            .await;
        assert_eq!(body, json!({ "version": v, "changed": false }), "{view}");
    }
}

/// After one write: both views, read with the `since` held before it, answer `changed: true`.
async fn changed(room: &Room, v: i64, writer: &str) -> (Vec<Value>, Value) {
    let now = room.version(MAIN).await;
    assert!(now > v, "{writer} moved MAIN's version ({v} → {now})");
    let rows = room.rows(&format!("branch=MAIN&since={v}")).await;
    let settings = room.data(SETTINGS, &format!("branch=MAIN&since={v}")).await;
    (rows, settings)
}

/// Design §4.2 and ticket §7: one write per writer of the views' tables, each through
/// `BranchTx`, and the same `since` then answers `changed: true` with the write in `data`.
#[actix_web::test]
async fn every_writer_of_the_views_tables_moves_the_version() {
    let room = Room::open().await;

    // create
    let v = room.version(MAIN).await;
    unchanged(&room, v).await;
    let walk = room
        .create_id(walk_in(MAIN, "Walk In", "081-234-5678"))
        .await;
    let (rows, _) = changed(&room, v, "create").await;
    assert_eq!(row(&rows, &walk)["status"], json!("CONFIRMED"));

    // member
    let v = room.version(MAIN).await;
    unchanged(&room, v).await;
    room.issue_member(SOMCHAI.0, SECRET_A).await;
    let (_, settings) = changed(&room, v, "member").await;
    assert_eq!(settings["memberCount"], json!(1));

    // confirm
    let pending = room.pending(SOMCHAI.0, "BUY", "USD", "1000", "36.25").await;
    let v = room.version(MAIN).await;
    unchanged(&room, v).await;
    room.act(
        "confirm",
        json!({ "branchId": MAIN, "id": pending, "expiresAt": stamp(room.now() + Duration::days(1)),
                "expectedRate": "36.25" }),
    )
    .await;
    let (rows, _) = changed(&room, v, "confirm").await;
    assert_eq!(row(&rows, &pending)["status"], json!("CONFIRMED"));

    // cancel
    let v = room.version(MAIN).await;
    unchanged(&room, v).await;
    room.act("cancel", json!({ "branchId": MAIN, "id": walk }))
        .await;
    let (rows, _) = changed(&room, v, "cancel").await;
    assert_eq!(row(&rows, &walk)["status"], json!("CANCELLED"));

    // settings
    let v = room.version(MAIN).await;
    unchanged(&room, v).await;
    room.act(
        "settings",
        json!({ "branchId": MAIN, "enabled": false, "minutes": 45 }),
    )
    .await;
    let (_, settings) = changed(&room, v, "settings").await;
    assert_eq!(
        settings["rules"],
        json!({ "enabled": false, "minutes": 45 })
    );

    // a publication (the counter rates). Amended for #758 (RE-4d, D-3): the counter rates are the
    // current publication's lines, so their writer is a publish through the rate routes, no
    // longer `adminSaveRate`
    let v = room.version(MAIN).await;
    unchanged(&room, v).await;
    room.publish_through_routes(MAIN, "GBP", "45.1", "46.2")
        .await;
    let (_, settings) = changed(&room, v, "a publication").await;
    let gbp = settings["counterRates"]
        .as_array()
        .expect("the counter rates")
        .iter()
        .find(|r| r["currency"] == json!("GBP"))
        .cloned()
        .unwrap_or_else(|| panic!("GBP in {settings}"));
    assert_eq!(
        gbp,
        json!({ "currency": "GBP", "buy": "45.1000", "sell": "46.2000" })
    );
    room.finish().await;
}

// ── §4.2: the desk's hand-off (#414) ─────────────────────────────────────────

const FILE: &str = "rpc/engine_actions.json";
/// `engine_actions.json` #120: a LIVE BUY of USD 100 at 32.2 by cash in `MAIN`.
const LIVE_BUY: usize = 120;

/// The day replayed through the routes up to and including step `last`.
async fn day_up_to(fixture: &Fixture, last: usize) -> Replay<'_> {
    let desk = Desk::open().await;
    let app = rpc_app!(desk);
    let mut replay = Replay::start(desk, app, fixture).await;
    for n in 1..=last {
        replay.step(fixture.step(n)).await;
    }
    replay
}

/// Design §4.2 and ticket §7 (writer 3, the desk hand-off of #414): a save naming a CONFIRMED
/// booking marks it USED inside the save's branch transaction, and the `since` held before it
/// then answers `changed: true` with the row `USED` and its document.
#[actix_web::test]
async fn the_desk_hand_off_moves_the_version_and_the_row_reads_used() {
    let fixture = Fixture::load(FILE);
    let replay = day_up_to(&fixture, LIVE_BUY - 1).await;
    let step = replay.fixture.step(LIVE_BUY);
    replay.desk.clock.set(time(&step.at));
    sqlx::query(
        "INSERT INTO bookings (id, branch_id, request_id, request_hash, name, phone, member_id, \
         type, code, amount, rate, status, created_at, created_by, expires_at) \
         VALUES ('BK-hand', 'MAIN', 'BK-hand', 'BK-hand', 'Booked Customer', '0812345678', NULL, \
         'BUY', 'USD', 100::numeric, 32.2::numeric, 'CONFIRMED', \
         '2026-10-06T06:00:00.000Z'::timestamptz, 'owner', '2026-10-06T07:30:00.000Z'::timestamptz)",
    )
    .execute(&replay.desk.db.pool)
    .await
    .expect("book BK-hand");

    let version = || async {
        BranchVersions::new(replay.desk.db.pool.clone())
            .current(&BranchId::new(MAIN))
            .await
            .expect("read MAIN's version")
    };
    let read = |query: String| {
        let cookie = replay.cookies["owner"].clone();
        let app = &replay.app;
        async move {
            let answer = app
                .send(get(&view_path(BOOKINGS, &query)).cookie(&cookie))
                .await;
            assert_eq!(answer.status, 200, "{query}: {:.300}", answer.text);
            answer.json()
        }
    };

    let v = version().await;
    assert_eq!(
        read(format!("branch=MAIN&since={v}")).await,
        json!({ "version": v, "changed": false })
    );

    let mut payload = replay
        .ids
        .to_actual(step.payload.as_ref().expect("a recorded payload"));
    payload["onlineBookingId"] = json!("BK-hand");
    payload["booking"] = json!("BK-hand");
    payload["onlineBookingCustomerConfirmed"] = json!(true);
    // OR-4b (#767, design §4.2 C2): ExchangeTransact is TELLER's; the day's FRONT, FR1, saves
    // with files of their own upload
    let payload = replay.uploads_as("FR1", &payload).await;
    let answer = replay
        .app
        .send(Call::action(ORIGIN, RPC, "saveTransaction", payload).cookie(&replay.cookies["FR1"]))
        .await;
    assert_eq!(answer.status, 200, "the save: {:.300}", answer.text);
    let document = answer.json()["documentRef"]
        .as_str()
        .expect("a document number")
        .to_owned();

    let now = version().await;
    assert!(now > v, "the save moved MAIN's version ({v} → {now})");
    let body = read(format!("branch=MAIN&since={v}")).await;
    assert_eq!(body["changed"], json!(true), "{body}");
    let rows = body["data"].as_array().expect("rows").clone();
    let hand = row(&rows, "BK-hand");
    assert_eq!(hand["status"], json!("USED"), "{hand}");
    assert_eq!(hand["transactionRef"], json!(document), "{hand}");
    replay.finish().await;
}

// ── the generated table and types ────────────────────────────────────────────

fn exported(view: &str) -> BTreeMap<String, String> {
    let entry = VIEWS
        .iter()
        .find(|v| v.name == view)
        .unwrap_or_else(|| panic!("the view file {view} is in the generated table"));
    let dir: PathBuf =
        std::env::temp_dir().join(format!("xc-463-{view}-{}-{}", std::process::id(), next()));
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

/// Every `export type Name = { … };` object of a generated file: name → field → type.
///
/// Doc comments are dropped; an optional field `name?: T` is recorded as `name` → `?T`.
fn object_types(ts: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut plain = String::new();
    let mut rest = ts;
    while let Some(start) = rest.find("/*") {
        plain.push_str(&rest[..start]);
        let end = rest[start..]
            .find("*/")
            .unwrap_or_else(|| panic!("an unclosed comment in:\n{ts}"));
        rest = &rest[start + end + 2..];
    }
    plain.push_str(rest);
    let plain: String = plain
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = BTreeMap::new();
    let mut rest = plain.as_str();
    while let Some(start) = rest.find("export type ") {
        rest = &rest[start + "export type ".len()..];
        let Some((name, after)) = rest.split_once(" = ") else {
            break;
        };
        let name = name.trim().to_owned();
        if !after.starts_with('{') {
            continue;
        }
        // the object's body, braces balanced
        let mut depth = 0usize;
        let mut end = 0usize;
        for (i, c) in after.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = &after[1..end];
        let mut fields = BTreeMap::new();
        let mut depth = 0i32;
        let mut part = String::new();
        let mut parts = Vec::new();
        for c in body.chars() {
            match c {
                '{' | '<' | '[' | '(' => depth += 1,
                '}' | '>' | ']' | ')' => depth -= 1,
                _ => {}
            }
            if c == ',' && depth == 0 {
                parts.push(std::mem::take(&mut part));
            } else {
                part.push(c);
            }
        }
        parts.push(part);
        for p in parts {
            let p = p.trim();
            if p.is_empty() {
                continue;
            }
            let (field, ty) = p
                .split_once(':')
                .unwrap_or_else(|| panic!("`name: type` in {p:?} of {name}"));
            let (field, ty) = match field.trim().strip_suffix('?') {
                Some(name) => (name.to_owned(), format!("?{}", ty.trim())),
                None => (field.trim().to_owned(), ty.trim().to_owned()),
            };
            fields.insert(field, ty);
        }
        out.insert(name, fields);
        rest = &after[end..];
    }
    out
}

fn the_type_with<'a>(
    types: &'a BTreeMap<String, BTreeMap<String, String>>,
    field: &str,
    ts: &str,
) -> &'a BTreeMap<String, String> {
    let found: Vec<_> = types.values().filter(|f| f.contains_key(field)).collect();
    assert_eq!(found.len(), 1, "exactly one type with `{field}` in:\n{ts}");
    found[0]
}

fn a_decimal_string(ty: &str) -> bool {
    ["string", "Money", "Rate"].contains(&ty)
}

#[test]
fn the_view_file_is_in_the_generated_table() {
    let names: Vec<&str> = VIEWS.iter().map(|v| v.name).collect();
    assert!(names.contains(&BOOKINGS), "{BOOKINGS} in {names:?}");
}

/// The row, the filter and the settings as `generated/admin_bookings.ts` declares them: the
/// row's 15 fields with money and rates decimal strings (W7), `| null` exactly on `expiresAt`,
/// `transactionRef` and the two counter fields; `q` an optional string; no `bigint` anywhere.
#[test]
fn the_generated_types_carry_the_tickets_fields() {
    let files = exported(BOOKINGS);
    let names: BTreeSet<String> = files.keys().cloned().collect();
    assert!(
        names.contains("admin_bookings.ts"),
        "admin_bookings.ts in {names:?}"
    );
    let ts = &files["admin_bookings.ts"];
    assert!(!ts.contains("bigint"), "every number is a `number`:\n{ts}");
    let types = object_types(ts);

    let row = the_type_with(&types, "counterPrincipal", ts);
    let fields: BTreeSet<String> = row.keys().cloned().collect();
    assert_eq!(fields, set_of(&ROW_FIELDS), "the row's fields:\n{ts}");
    for money in ["amount", "rate"] {
        assert!(
            a_decimal_string(&row[money]),
            "{money}: a decimal string, got `{}`",
            row[money]
        );
    }
    for counter in ["counterRate", "counterPrincipal"] {
        let ty = &row[counter];
        let base = ty
            .strip_suffix(" | null")
            .unwrap_or_else(|| panic!("{counter} | null: `{ty}`"));
        assert!(
            a_decimal_string(base),
            "{counter}: a decimal string or null, got `{ty}`"
        );
    }
    for nullable in ["expiresAt", "transactionRef"] {
        assert_eq!(row[nullable], "string | null", "{nullable}");
    }
    assert_eq!(row["member"], "boolean");
    for text in [
        "id",
        "branchId",
        "name",
        "phone",
        "currency",
        "status",
        "createdAt",
    ] {
        assert_eq!(row[text], "string", "{text}");
    }

    let filter = the_type_with(&types, "q", ts);
    assert!(
        ["?string", "string | null", "?string | null"].contains(&filter["q"].as_str()),
        "q is an optional string: {filter:?}"
    );

    let settings = the_type_with(&types, "publicUrl", ts);
    let fields: BTreeSet<String> = settings.keys().cloned().collect();
    assert_eq!(
        fields,
        set_of(&[
            "rules",
            "memberCount",
            "publicUrl",
            "counterRates",
            "freshness"
        ]),
        "{ts}"
    );
    assert_eq!(settings["memberCount"], "number");
    assert_eq!(settings["publicUrl"], "string");
    let rules = settings["rules"].as_str();
    let rules = types
        .get(rules)
        .cloned()
        .unwrap_or_else(|| object_types(&format!("export type R = {rules};"))["R"].clone());
    assert_eq!(
        rules.get("enabled").map(String::as_str),
        Some("boolean"),
        "{ts}"
    );
    assert_eq!(
        rules.get("minutes").map(String::as_str),
        Some("number"),
        "{ts}"
    );
    let rate = the_type_with(&types, "sell", ts);
    assert!(a_decimal_string(&rate["buy"]), "{ts}");
    assert!(a_decimal_string(&rate["sell"]), "{ts}");
    assert_eq!(
        rate.get("currency").map(String::as_str),
        Some("string"),
        "{ts}"
    );
}
