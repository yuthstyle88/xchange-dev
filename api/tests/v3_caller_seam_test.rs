//! Ticket #1373 (APP-18): the v3 endpoint seam admits a `Need::Public` `Rpc` row with no session and hands the
//! handler one caller (design `docs/design/blueprint/customer-app.md` §9 APP-18 row, §9.1 APP-18 test row; the lane
//! controller's rulings on #1373, issuecomment-6117475076 and issuecomment-6117531901).
//!
//! Written before the implementation by the test-writer claude/dell-claude-5 on claude/ad05bf8d's order. Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! What these tests pin ([required] by the rulings):
//! - the caller is `Caller { user: Option<access::Actor>, app_session: Option<AppSession> }` (`v3/caller.rs`, ruling 3);
//! - option B: each seam trait has a default `*_as` method that takes the caller (`load_as`, `page_as`, `run_as`,
//!   `compute_as`) beside today's `&access::Actor` method. The seam always calls the `*_as` method; by default it calls
//!   today's method with `caller.user`, and answers `signIn` when there is none;
//! - a `Need::Public` row on `EntryPath::Rpc` with no session reaches the handler's `*_as` method with no user, for a
//!   branch view, a paged view, a command and a compute action (ruling 1, option A: `Rpc` only);
//! - the app session is the `xc_app_sid` the request carries, as `AppSession { id, hash }`. The seam never issues one:
//!   no cookie means `app_session: None` and no `Set-Cookie` (ruling 4);
//! - a CUSTOMER-only user on a `Public` `Rpc` row reaches `load_as` with `access::Actor.customer_id` set (ruling 2);
//! - staff callers see no change: a view that does not override `load_as` is served through `load`;
//! - a non-`Public` row with no session, and a `Public` row on the Kiosk path with no session, still answer `signIn`
//!   (ruling 1; #430's `v3_view_no_session_test.rs` stays as it is);
//! - `notificationRead` and `notificationReadAll` write in the seam's transaction: when the seam's transaction rolls
//!   back after `run`, `read_at` stays unset (done criterion 2; ruling 5 puts `commands/notification_read.rs` in scope).
//!
//! Choices of this file:
//! - The test views and commands inherit the real `Public` `Rpc` rows of `customer_app.rs` (`my_planner_drafts`,
//!   `plannerDraftSave`, `planner_preview`), the non-`Public` row `myNotifications` (`SignedIn`) and the Kiosk `Public`
//!   row `rates`. No area file is touched (§6).
//! - The test views are `VERSIONED = false`, so `own_version` (which still takes `&access::Actor`) is not pinned here.
//! - The seam's rollback is forced by a test-only trigger that refuses `finish_in`'s `UPDATE idempotency_records …
//!   status = 'FINISHED'`, the last write the seam makes in its transaction after `run`.
//!
//! Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips. Each test works in a schema of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Utc};
use pos_error::AppResult;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use ts_rs::TS;
use uuid::Uuid;
use xc_app::domain::app_session::{APP_SESSION_COOKIE, AppSessionId};
use xc_auth::domain::access::{Actor, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::idempotency::store::IdempotencyKey;
use xchange::presentation::dto::v3::common::NoFilter;
use xchange::presentation::http::v3::action::{ComputeAction, ComputeEndpoint};
use xchange::presentation::http::v3::caller::Caller;
use xchange::presentation::http::v3::command::{CommandAction, CommandEndpoint};
use xchange::presentation::http::v3::error::V3Error;
use xchange::presentation::http::v3::view::{BranchView, PagedView, RowId, ViewEndpoint};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
/// The organisation the migrations seed.
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const B1: &str = "XC-B01";
/// Who the request is: `customer`, `teller`, or absent (no session). The recipient of a notification test is sent as
/// `customer:<user uuid>`.
const AS: &str = "x-test-as";
const CUSTOMER_ID: &str = "C-APP18";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

// ------------------------------------------------------------ database

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect(
            "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip",
        );
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("app18_caller");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, 'B01')")
            .bind(B1)
            .execute(&pool)
            .await
            .expect("a branch");
        Room { pool, schema }
    }

    async fn finish(self) {
        let Room { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ callers

fn since() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

/// A CUSTOMER-only user: no v2 owner or staff actor, `customer_id` set.
fn customer(user_id: &str) -> Actor {
    Actor::new(
        user_id,
        ORG,
        vec![ScopedRole::new(Role::Customer, None, since(), None)],
        Some(CUSTOMER_ID),
    )
}

/// A TELLER of B1.
fn teller() -> Actor {
    Actor::new(
        "teller-1",
        ORG,
        vec![ScopedRole::new(
            Role::Teller,
            Some(BranchId::new(B1)),
            since(),
            None,
        )],
        None,
    )
}

/// The user named by the `x-test-as` header: `customer` (or `customer:<id>`), `teller`, or none.
struct HeaderCallers;

fn user_of(req: &HttpRequest) -> Option<Actor> {
    let spec = req.headers().get(AS)?.to_str().ok()?.to_owned();
    match spec.as_str() {
        "customer" => Some(customer("customer-1")),
        "teller" => Some(teller()),
        other => other.strip_prefix("customer:").map(customer),
    }
}

impl SessionReader for HeaderCallers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        // No legacy v2 actor at all: the seam must decide with `Callers::user` alone.
        Box::pin(async move {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let user = user_of(req);
        Box::pin(async move {
            Callers {
                owner: None,
                staff: None,
                user,
                staff_user: None,
            }
        })
    }
}

// ------------------------------------------------------------ what a handler saw

/// Which method the seam called, and the caller it handed over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
struct Seen {
    /// `"load"`/`"page"`/`"run"`/`"compute"` (today's method) or the `*_as` name.
    path: String,
    user: Option<String>,
    customer: Option<String>,
    /// The app session's hash, as `AppSession::hash` stores it.
    session: Option<String>,
}

fn seen_as(path: &str, caller: &Caller) -> Seen {
    Seen {
        path: path.to_owned(),
        user: caller.user.as_ref().map(|u| u.user_id.clone()),
        customer: caller.user.as_ref().and_then(|u| u.customer_id.clone()),
        session: caller
            .app_session
            .as_ref()
            .map(|s| s.hash.as_str().to_owned()),
    }
}

fn seen_legacy(path: &str, who: &Actor) -> Seen {
    Seen {
        path: path.to_owned(),
        user: Some(who.user_id.clone()),
        customer: who.customer_id.clone(),
        session: None,
    }
}

// ------------------------------------------------------------ test routes

/// `Public` on `Rpc` (`customer_app.rs`: `my_planner_drafts`), overriding `load_as`.
struct PublicView;

impl BranchView for PublicView {
    const NAME: &'static str = "app18_public_view";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "my_planner_drafts");
    const VERSIONED: bool = false;
    type Data = Seen;
    type Filter = NoFilter;

    async fn load(&self, _branch: &BranchId, who: &Actor, _filter: &NoFilter) -> AppResult<Seen> {
        Ok(seen_legacy("load", who))
    }

    async fn load_as(
        &self,
        _branch: &BranchId,
        caller: &Caller,
        _filter: &NoFilter,
    ) -> AppResult<Seen> {
        Ok(seen_as("load_as", caller))
    }
}

/// The same `Public` `Rpc` row, but **not** overriding `load_as`: the default must serve it through `load`.
struct PublicDefaultView;

impl BranchView for PublicDefaultView {
    const NAME: &'static str = "app18_public_default_view";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "my_planner_drafts");
    const VERSIONED: bool = false;
    type Data = Seen;
    type Filter = NoFilter;

    async fn load(&self, _branch: &BranchId, who: &Actor, _filter: &NoFilter) -> AppResult<Seen> {
        Ok(seen_legacy("load", who))
    }
}

/// A `Public` `Rpc` paged view, overriding `page_as`.
struct PublicPagedView;

impl PagedView for PublicPagedView {
    const NAME: &'static str = "app18_public_paged";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "my_planner_drafts");
    type Row = Seen;
    type Filter = NoFilter;

    async fn page(
        &self,
        _branch: &BranchId,
        who: &Actor,
        _filter: &NoFilter,
        _before: Option<RowId>,
        _limit: u16,
    ) -> AppResult<Vec<Seen>> {
        Ok(vec![seen_legacy("page", who)])
    }

    async fn page_as(
        &self,
        _branch: &BranchId,
        caller: &Caller,
        _filter: &NoFilter,
        _before: Option<RowId>,
        _limit: u16,
    ) -> AppResult<Vec<Seen>> {
        Ok(vec![seen_as("page_as", caller)])
    }
}

/// A non-`Public` `Rpc` row (`notifications.rs`: `myNotifications`, `SignedIn`), overriding `load_as`.
struct SignedInView;

impl BranchView for SignedInView {
    const NAME: &'static str = "app18_signed_in_view";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "myNotifications");
    const VERSIONED: bool = false;
    type Data = Seen;
    type Filter = NoFilter;

    async fn load(&self, _branch: &BranchId, who: &Actor, _filter: &NoFilter) -> AppResult<Seen> {
        Ok(seen_legacy("load", who))
    }

    async fn load_as(
        &self,
        _branch: &BranchId,
        caller: &Caller,
        _filter: &NoFilter,
    ) -> AppResult<Seen> {
        Ok(seen_as("load_as", caller))
    }
}

/// A `Public` row on the **Kiosk** path (`policy.rs`: `(Kiosk, "rates")`), overriding `load_as` (ruling 1: not admitted).
struct KioskPublicView;

impl BranchView for KioskPublicView {
    const NAME: &'static str = "app18_kiosk_public_view";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Kiosk, "rates");
    const VERSIONED: bool = false;
    type Data = Seen;
    type Filter = NoFilter;

    async fn load(&self, _branch: &BranchId, who: &Actor, _filter: &NoFilter) -> AppResult<Seen> {
        Ok(seen_legacy("load", who))
    }

    async fn load_as(
        &self,
        _branch: &BranchId,
        caller: &Caller,
        _filter: &NoFilter,
    ) -> AppResult<Seen> {
        Ok(seen_as("load_as", caller))
    }
}

#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
struct NoteInput {
    note: String,
}

/// A `Public` `Rpc` command (`customer_app.rs`: `plannerDraftSave`), overriding `run_as`.
struct PublicCommand;

impl CommandAction for PublicCommand {
    const NAME: &'static str = "app18PublicCommand";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "plannerDraftSave");
    type Input = NoteInput;
    type Output = Seen;
    const ROUTE: &'static str = "/api/test-app18/command";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: NoteInput,
        _key: IdempotencyKey,
    ) -> Result<Seen, V3Error> {
        assert_eq!(input.note, "hello");
        Ok(seen_legacy("run", who))
    }

    async fn run_as(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        caller: &Caller,
        input: NoteInput,
        _key: IdempotencyKey,
    ) -> Result<Seen, V3Error> {
        assert_eq!(input.note, "hello");
        Ok(seen_as("run_as", caller))
    }
}

#[derive(Deserialize, TS)]
struct NoRequest {}

/// A `Public` `Rpc` compute action (`customer_app.rs`: `planner_preview`), overriding `compute_as`.
struct PublicCompute;

impl ComputeAction for PublicCompute {
    const NAME: &'static str = "app18_public_compute";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "planner_preview");
    type Request = NoRequest;
    type Answer = Seen;

    async fn compute(
        &self,
        _branch: &BranchId,
        who: &Actor,
        _request: NoRequest,
    ) -> Result<Seen, V3Error> {
        Ok(seen_legacy("compute", who))
    }

    async fn compute_as(
        &self,
        _branch: &BranchId,
        caller: &Caller,
        _request: NoRequest,
    ) -> Result<Seen, V3Error> {
        Ok(seen_as("compute_as", caller))
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    set_cookie: Vec<String>,
    body: Value,
}

/// What the client sends: who it is, and the `xc_app_sid` it carries.
#[derive(Default)]
struct From {
    who: Option<String>,
    cookie: Option<String>,
}

fn anonymous() -> From {
    From::default()
}

fn with_cookie(id: &AppSessionId) -> From {
    From {
        who: None,
        cookie: Some(id.as_str().to_owned()),
    }
}

fn as_user(who: &str) -> From {
    From {
        who: Some(who.to_owned()),
        cookie: None,
    }
}

async fn call(room: &Room, req: atest::TestRequest, from: From) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| {
                ViewEndpoint::register(cfg, Arc::new(PublicView));
                ViewEndpoint::register(cfg, Arc::new(PublicDefaultView));
                ViewEndpoint::register_paged(cfg, Arc::new(PublicPagedView));
                ViewEndpoint::register(cfg, Arc::new(SignedInView));
                ViewEndpoint::register(cfg, Arc::new(KioskPublicView));
                CommandEndpoint::register(cfg, Arc::new(PublicCommand));
                ComputeEndpoint::register(cfg, Arc::new(PublicCompute));
                bootstrap::configure(cfg, state);
            })
            // After `configure`, so this reader is the one the seam asks.
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = req.insert_header((header::HOST, HOST));
    if let Some(who) = from.who {
        req = req.insert_header((AS, who));
    }
    if let Some(id) = from.cookie {
        req = req.insert_header((header::COOKIE, format!("{APP_SESSION_COOKIE}={id}")));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let set_cookie = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .filter_map(|v| v.to_str().ok().map(str::to_owned))
        .collect();
    let bytes = atest::read_body(resp).await;
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    Answer {
        status,
        set_cookie,
        body,
    }
}

fn view(name: &str) -> atest::TestRequest {
    atest::TestRequest::get().uri(&format!("/api/v3/views/{name}?branch={B1}"))
}

fn command(route: &str, body: Value) -> atest::TestRequest {
    atest::TestRequest::post()
        .uri(route)
        .insert_header(("x-xchange-request", "1"))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .insert_header(("idempotency-key", unique("key")))
        .set_json(body)
}

fn compute(name: &str) -> atest::TestRequest {
    atest::TestRequest::post()
        .uri(&format!("/api/v3/compute/{name}"))
        .insert_header(("x-xchange-request", "1"))
        .insert_header((header::ORIGIN, SAME_ORIGIN))
        .set_json(json!({ "branch": B1 }))
}

/// The `Seen` of a branch view's or a command's answer (`data` for a view; the whole body for a command/compute).
fn seen(answer: &Answer) -> Seen {
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    let value = answer
        .body
        .get("data")
        .cloned()
        .unwrap_or(answer.body.clone());
    serde_json::from_value(value).unwrap_or_else(|e| panic!("a Seen: {e}: {}", answer.body))
}

fn no_app_cookie_issued(answer: &Answer) {
    assert!(
        answer
            .set_cookie
            .iter()
            .all(|c| !c.starts_with(&format!("{APP_SESSION_COOKIE}="))),
        "the seam never issues `{APP_SESSION_COOKIE}` (ruling 4): {:?}",
        answer.set_cookie
    );
}

fn assert_sign_in(answer: &Answer, why: &str) {
    assert_eq!(
        answer.status,
        StatusCode::UNAUTHORIZED,
        "{why}: {}",
        answer.body
    );
    assert_eq!(
        answer.body["error"]["code"], "signIn",
        "{why}: {}",
        answer.body
    );
}

// ------------------------------------------------------------ Public on Rpc, no session (§9.1 row 1)

#[actix_web::test]
async fn a_public_rpc_view_with_no_session_reaches_load_as_with_no_user_and_no_app_session() {
    let room = Room::start().await;
    let answer = call(&room, view(PublicView::NAME), anonymous()).await;
    assert_eq!(
        seen(&answer),
        Seen {
            path: "load_as".to_owned(),
            user: None,
            customer: None,
            session: None
        },
        "no session and no cookie: the handler gets an empty caller, not `signIn`"
    );
    no_app_cookie_issued(&answer);
    room.finish().await;
}

#[actix_web::test]
async fn a_public_rpc_view_with_no_session_is_handed_the_app_session_it_carries() {
    let room = Room::start().await;
    let id = AppSessionId::generate();
    let answer = call(&room, view(PublicView::NAME), with_cookie(&id)).await;
    assert_eq!(
        seen(&answer),
        Seen {
            path: "load_as".to_owned(),
            user: None,
            customer: None,
            session: Some(id.hash().as_str().to_owned())
        }
    );
    no_app_cookie_issued(&answer);
    room.finish().await;
}

#[actix_web::test]
async fn a_public_rpc_paged_view_with_no_session_reaches_page_as() {
    let room = Room::start().await;
    let id = AppSessionId::generate();
    let answer = call(&room, view(PublicPagedView::NAME), with_cookie(&id)).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    let rows: Vec<Seen> = serde_json::from_value(answer.body["data"]["rows"].clone())
        .or_else(|_| serde_json::from_value(answer.body["data"].clone()))
        .unwrap_or_else(|e| panic!("the page's rows: {e}: {}", answer.body));
    assert_eq!(
        rows,
        vec![Seen {
            path: "page_as".to_owned(),
            user: None,
            customer: None,
            session: Some(id.hash().as_str().to_owned())
        }]
    );
    room.finish().await;
}

// ------------------------------------------------------------ Public command / compute, no session (§9.1 row 2)

#[actix_web::test]
async fn a_public_rpc_command_with_no_session_reaches_run_as_with_the_app_session_only() {
    let room = Room::start().await;
    let id = AppSessionId::generate();
    let answer = call(
        &room,
        command(
            PublicCommand::ROUTE,
            json!({ "branch": B1, "note": "hello" }),
        ),
        with_cookie(&id),
    )
    .await;
    assert_eq!(
        seen(&answer),
        Seen {
            path: "run_as".to_owned(),
            user: None,
            customer: None,
            session: Some(id.hash().as_str().to_owned())
        }
    );
    no_app_cookie_issued(&answer);
    room.finish().await;
}

#[actix_web::test]
async fn a_public_rpc_command_without_a_cookie_gets_no_app_session_and_no_cookie() {
    let room = Room::start().await;
    let answer = call(
        &room,
        command(
            PublicCommand::ROUTE,
            json!({ "branch": B1, "note": "hello" }),
        ),
        anonymous(),
    )
    .await;
    assert_eq!(
        seen(&answer),
        Seen {
            path: "run_as".to_owned(),
            user: None,
            customer: None,
            session: None
        },
        "the seam reads an existing `xc_app_sid` only; issuing one is the first app command's (P7, ruling 4)"
    );
    no_app_cookie_issued(&answer);
    room.finish().await;
}

#[actix_web::test]
async fn a_public_rpc_compute_with_no_session_reaches_compute_as() {
    let room = Room::start().await;
    let id = AppSessionId::generate();
    let answer = call(&room, compute(PublicCompute::NAME), with_cookie(&id)).await;
    assert_eq!(
        seen(&answer),
        Seen {
            path: "compute_as".to_owned(),
            user: None,
            customer: None,
            session: Some(id.hash().as_str().to_owned())
        }
    );
    room.finish().await;
}

// ------------------------------------------------------------ signed-in callers (§9.1 rows 3 and 4)

#[actix_web::test]
async fn a_customer_only_user_reaches_load_as_with_customer_id_set() {
    let room = Room::start().await;
    let answer = call(&room, view(PublicView::NAME), as_user("customer")).await;
    assert_eq!(
        seen(&answer),
        Seen {
            path: "load_as".to_owned(),
            user: Some("customer-1".to_owned()),
            customer: Some(CUSTOMER_ID.to_owned()),
            session: None
        },
        "a CUSTOMER-only user is the caller's user, with its customer id (ruling 2)"
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_signed_in_caller_who_also_carries_an_app_session_gets_both() {
    let room = Room::start().await;
    let id = AppSessionId::generate();
    let from = From {
        who: Some("customer".to_owned()),
        cookie: Some(id.as_str().to_owned()),
    };
    let answer = call(&room, view(PublicView::NAME), from).await;
    assert_eq!(
        seen(&answer),
        Seen {
            path: "load_as".to_owned(),
            user: Some("customer-1".to_owned()),
            customer: Some(CUSTOMER_ID.to_owned()),
            session: Some(id.hash().as_str().to_owned())
        }
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_staff_user_on_a_view_that_overrides_load_as_is_the_callers_user() {
    let room = Room::start().await;
    let answer = call(&room, view(SignedInView::NAME), as_user("teller")).await;
    assert_eq!(
        seen(&answer),
        Seen {
            path: "load_as".to_owned(),
            user: Some("teller-1".to_owned()),
            customer: None,
            session: None
        }
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_view_that_does_not_override_load_as_serves_a_signed_in_caller_through_load() {
    let room = Room::start().await;
    let answer = call(&room, view(PublicDefaultView::NAME), as_user("teller")).await;
    assert_eq!(
        seen(&answer),
        Seen {
            path: "load".to_owned(),
            user: Some("teller-1".to_owned()),
            customer: None,
            session: None
        },
        "option B: the default `load_as` calls today's `load` with `caller.user`, so staff see no change"
    );
    room.finish().await;
}

// ------------------------------------------------------------ still refused (§9.1 row 5; ruling 1)

#[actix_web::test]
async fn a_public_rpc_view_that_does_not_override_load_as_answers_sign_in_to_no_session() {
    let room = Room::start().await;
    let id = AppSessionId::generate();
    for from in [anonymous(), with_cookie(&id)] {
        let answer = call(&room, view(PublicDefaultView::NAME), from).await;
        assert_sign_in(
            &answer,
            "the default `load_as` has no user to hand `load`, so it answers `signIn` (option B)",
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn a_non_public_rpc_view_with_no_session_still_answers_sign_in() {
    let room = Room::start().await;
    let id = AppSessionId::generate();
    for from in [anonymous(), with_cookie(&id)] {
        let answer = call(&room, view(SignedInView::NAME), from).await;
        assert_sign_in(
            &answer,
            "`myNotifications` needs `SignedIn`; an app session is not a sign-in",
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn a_kiosk_public_view_with_no_session_still_answers_sign_in() {
    let room = Room::start().await;
    let id = AppSessionId::generate();
    for from in [anonymous(), with_cookie(&id)] {
        let answer = call(&room, view(KioskPublicView::NAME), from).await;
        assert_sign_in(
            &answer,
            "a no-session `Public` row is admitted on `Rpc` only; the Kiosk path keeps `signIn` (ruling 1)",
        );
    }
    room.finish().await;
}

// ------------------------------------------------------------ notificationRead* in the seam's transaction (criterion 2)

/// A user of ORG and one unread notification for them. Answers (user id, notification id).
async fn unread_notification(room: &Room) -> (String, String) {
    let user: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, 'app18 customer', 'ACTIVE', 'app18-test') RETURNING id::text",
    )
    .bind(ORG)
    .fetch_one(&room.pool)
    .await
    .expect("a user");
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO notifications (id, organization_id, recipient_user_id, event_id, template_key, \
           template_version, locale) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, 'app18.test', 1, 'th')",
    )
    .bind(&id)
    .bind(ORG)
    .bind(&user)
    .bind(unique("event"))
    .execute(&room.pool)
    .await
    .expect("a notification");
    (user, id)
}

async fn read_at(room: &Room, id: &str) -> Option<DateTime<Utc>> {
    sqlx::query_scalar("SELECT read_at FROM notifications WHERE id = $1::uuid")
        .bind(id)
        .fetch_one(&room.pool)
        .await
        .expect("the notification")
}

/// Makes the seam's own transaction fail after `run`: `finish_in`'s `UPDATE … status = 'FINISHED'` is refused.
async fn refuse_finishing(room: &Room) {
    sqlx::query(
        "CREATE FUNCTION app18_refuse_finish() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RAISE EXCEPTION 'app18 test: the seam transaction fails after run'; END $$",
    )
    .execute(&room.pool)
    .await
    .expect("the refusing function");
    sqlx::query(
        "CREATE TRIGGER app18_refuse_finish BEFORE UPDATE ON idempotency_records \
         FOR EACH ROW WHEN (NEW.status = 'FINISHED') EXECUTE FUNCTION app18_refuse_finish()",
    )
    .execute(&room.pool)
    .await
    .expect("the refusing trigger");
}

#[actix_web::test]
async fn notification_read_marks_read_when_the_seam_commits() {
    let room = Room::start().await;
    let (user, id) = unread_notification(&room).await;
    let answer = call(
        &room,
        command(
            &format!("/api/app/notifications/{id}/read"),
            json!({ "branch": B1 }),
        ),
        as_user(&format!("customer:{user}")),
    )
    .await;
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    assert!(
        read_at(&room, &id).await.is_some(),
        "the notification is read"
    );
    room.finish().await;
}

#[actix_web::test]
async fn notification_read_writes_in_the_seams_transaction() {
    let room = Room::start().await;
    let (user, id) = unread_notification(&room).await;
    refuse_finishing(&room).await;
    let answer = call(
        &room,
        command(
            &format!("/api/app/notifications/{id}/read"),
            json!({ "branch": B1 }),
        ),
        as_user(&format!("customer:{user}")),
    )
    .await;
    assert_ne!(
        answer.status,
        StatusCode::OK,
        "the seam's transaction failed: {}",
        answer.body
    );
    assert_eq!(
        read_at(&room, &id).await,
        None,
        "notificationRead writes in the seam's transaction, so its rollback leaves read_at unset (criterion 2)"
    );
    room.finish().await;
}

#[actix_web::test]
async fn notification_read_all_writes_in_the_seams_transaction() {
    let room = Room::start().await;
    let (user, id) = unread_notification(&room).await;
    refuse_finishing(&room).await;
    let answer = call(
        &room,
        command("/api/app/notifications/read-all", json!({ "branch": B1 })),
        as_user(&format!("customer:{user}")),
    )
    .await;
    assert_ne!(
        answer.status,
        StatusCode::OK,
        "the seam's transaction failed: {}",
        answer.body
    );
    assert_eq!(
        read_at(&room, &id).await,
        None,
        "notificationReadAll writes in the seam's transaction, so its rollback leaves read_at unset (criterion 2)"
    );
    room.finish().await;
}
