//! Ticket #1022 (OR-7.3): the payment settings read decides who may edit from `access::Actor` and `Permission`, not
//! from the old `Actor`'s kind. Design `docs/design/blueprint/org-roles-branches.md` §3.3 (v3 roles → Blueprint roles),
//! §4.3 (provided tests move 1:1), §10 row OR-7.
//!
//! Written before the implementation by the test-writer claude/709ec749, on the BE lane controller claude/ad05bf8d's
//! order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! **The rule pinned here (#1022 §3, §7–9):** `PaymentSettingsService::view` answers `canEdit` and the setting's
//! history to exactly the callers who hold `Permission::SystemSettings` for the branch (ADMIN and EXECUTIVE on
//! `main`), and to nobody else; everyone signed in may list. The owner of today maps to EXECUTIVE + ADMIN
//! organisation-wide (§3.3), so it edits as it does today; every staff role maps to a role without
//! `SystemSettings`, so staff list without editing, as today. Who may edit is computed from `RolePermissions`, never
//! from a role name.
//!
//! **Through both doors that call the service** (#1022 §5, "its callers change only the argument they pass"):
//! - the v3 view `admin_payments` (`views/admin_payments.rs`), and
//! - the v2 route `POST /api/payment-methods` `list` (`http/payment_methods.rs`).
//!
//! **Why these are red on `main` by result.** The callers here are Blueprint `access::Actor`s only, handed to the
//! seams through `SessionReader::callers` with empty legacy sessions:
//! - the view still converts with `legacy_actor` (`// OR-7 bridge, removed by #1022`), which maps an ADMIN-only
//!   caller to a v2 staff role (not editing) and refuses an EXECUTIVE-only caller;
//! - the route falls back to a `dummy_owner` when no legacy session is present, so every caller edits.
//!
//! Outcome changes this ticket brings, named as #1022 asks (old v2 role → mapped role → old → new):
//! - a Blueprint ADMIN without EXECUTIVE (v2 SYSTEM → ADMIN): `canEdit` false → **true**;
//! - a Blueprint EXECUTIVE without ADMIN: refused by the bridge (view) → **lists with `canEdit` true**;
//! - on the v2 route, every Blueprint-only caller: `canEdit` true (the dummy owner) → **true only with
//!   `SystemSettings`**.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test` (never the queue's port 6381); nothing
//! skips. Each test works in a schema of its own.

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
use chrono::{DateTime, TimeZone, Utc};
use pos_infra::transactions::Transactor;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::{Actor as User, Permission, Role, RolePermissions, ScopedRole};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::payment::PaymentRules;
use xc_rate::infrastructure::branch::branch_repository::PgBranchRepository;
use xchange::application::settings::PaymentSettingsService;
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::settings::branch_setting_store::BranchSettingStore;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const MAIN: &str = "MAIN";

/// The Blueprint staff roles, used only to *make* actors (no expectation names one).
const STAFF_ROLES: [Role; 8] = [
    Role::Executive,
    Role::Admin,
    Role::RateManager,
    Role::BranchManager,
    Role::Teller,
    Role::Finance,
    Role::Marketing,
    Role::VerificationReviewer,
];

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

/// Whether `role` edits payment settings: it holds `SystemSettings` (#1022 §3).
fn edits(role: Role) -> bool {
    RolePermissions::of(role).contains(&Permission::SystemSettings)
}

/// Organisation-wide for the roles that are organisation-wide on `main` (EXECUTIVE, ADMIN); at MAIN otherwise.
fn scope_of(role: Role) -> Option<&'static str> {
    match role {
        Role::Executive | Role::Admin => None,
        _ => Some(MAIN),
    }
}

// ------------------------------------------------------------ database

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
    /// A schema with MAIN and one saved payment setting (version 1), so the owner's history is not empty.
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("or73pay");
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
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $1) ON CONFLICT (id) DO NOTHING")
            .bind(MAIN)
            .execute(&pool)
            .await
            .expect("the MAIN branch");
        let service = PaymentSettingsService::new(
            BranchTx::new(Transactor::new(pool.clone())),
            BranchSettingStore::new(pool.clone()),
            PgBranchRepository::new(pool.clone()),
        );
        service
            .save(
                &BranchId::new(MAIN),
                0,
                PaymentRules::defaults(),
                "owner",
                Utc::now(),
            )
            .await
            .unwrap_or_else(|e| panic!("save the first payment setting: {e}"));
        Self { pool, schema }
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

/// One scoped role of a caller: the role, its branch (`None` = organisation-wide) and when it ends.
#[derive(Clone, Copy)]
struct Held {
    role: Role,
    branch: Option<&'static str>,
    ended: bool,
}

fn held(role: Role) -> Held {
    Held {
        role,
        branch: scope_of(role),
        ended: false,
    }
}

/// The spec of a caller: `<id>;<ROLE>,<branch or *>,<0|1 ended>;…`.
fn spec(id: &str, roles: &[Held]) -> String {
    let mut s = id.to_owned();
    for h in roles {
        s.push_str(&format!(
            ";{},{},{}",
            h.role.as_str(),
            h.branch.unwrap_or("*"),
            u8::from(h.ended)
        ));
    }
    s
}

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.split(';');
    let id = parts.next().filter(|s| !s.is_empty())?;
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let ended: DateTime<Utc> = Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();
    let roles = parts
        .map(|p| {
            let mut f = p.split(',');
            let role = Role::parse(f.next().expect("a role")).expect("a known role");
            let branch = f.next().filter(|b| *b != "*").map(BranchId::new);
            let to = (f.next() == Some("1")).then_some(ended);
            ScopedRole::new(role, branch, from, to)
        })
        .collect();
    Some(User::new(id, ORG, roles, None))
}

struct HeaderUsers;

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                owner: None,
                staff: None,
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(room: &Room, who: &str, method: Method, uri: &str, body: Option<Value>) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who.to_owned()));
    if let Some(body) = body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// The v3 view `admin_payments` at MAIN: `(canEdit, history entries)`.
async fn through_the_view(room: &Room, who: &str) -> (bool, usize) {
    let a = call(
        room,
        who,
        Method::GET,
        &format!("/api/v3/views/admin_payments?branch={MAIN}"),
        None,
    )
    .await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "the view answers {who}: {}",
        a.text
    );
    let data = &a.body["data"];
    (
        data["canEdit"]
            .as_bool()
            .unwrap_or_else(|| panic!("canEdit is a bool: {}", a.text)),
        data["history"]
            .as_array()
            .unwrap_or_else(|| panic!("history is a list: {}", a.text))
            .len(),
    )
}

/// The v2 route `POST /api/payment-methods` `list` at MAIN: `(canEdit, history entries)`.
async fn through_the_route(room: &Room, who: &str) -> (bool, usize) {
    let a = call(
        room,
        who,
        Method::POST,
        "/api/payment-methods",
        Some(json!({ "action": "list", "payload": { "branchId": MAIN } })),
    )
    .await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "the route answers {who}: {}",
        a.text
    );
    let view = if a.body.get("canEdit").is_some() || a.body.get("can_edit").is_some() {
        a.body.clone()
    } else {
        a.body["data"].clone()
    };
    let can_edit = view
        .get("canEdit")
        .or_else(|| view.get("can_edit"))
        .and_then(Value::as_bool)
        .unwrap_or_else(|| panic!("the list answers canEdit: {}", a.text));
    let history = view["history"]
        .as_array()
        .unwrap_or_else(|| panic!("the list answers its history: {}", a.text))
        .len();
    (can_edit, history)
}

/// What every door must answer a caller with `edit` = holds `SystemSettings`: edit and see the history, or neither.
fn expected(edit: bool) -> (bool, bool) {
    (edit, edit)
}

fn seen(answer: (bool, usize)) -> (bool, bool) {
    (answer.0, answer.1 > 0)
}

// ------------------------------------------------------------ every Blueprint role, both doors

/// Each staff role alone edits through the view iff it holds `SystemSettings`; every other role lists only.
#[actix_web::test]
async fn through_the_view_each_role_edits_exactly_when_it_holds_system_settings() {
    let room = Room::start().await;
    for role in STAFF_ROLES {
        let who = spec(&unique("u"), &[held(role)]);
        assert_eq!(
            seen(through_the_view(&room, &who).await),
            expected(edits(role)),
            "{}: (canEdit, has history)",
            role.as_str()
        );
    }
    room.finish().await;
}

/// The same through the v2 route's `list`.
#[actix_web::test]
async fn through_the_route_each_role_edits_exactly_when_it_holds_system_settings() {
    let room = Room::start().await;
    for role in STAFF_ROLES {
        let who = spec(&unique("u"), &[held(role)]);
        assert_eq!(
            seen(through_the_route(&room, &who).await),
            expected(edits(role)),
            "{}: (canEdit, has history)",
            role.as_str()
        );
    }
    room.finish().await;
}

// ------------------------------------------------------------ the old outcomes, on the new path

/// Today's owner maps to EXECUTIVE + ADMIN organisation-wide (§3.3): it edits and sees the history, as today.
#[actix_web::test]
async fn the_owner_mapped_to_executive_and_admin_edits_and_sees_the_history_as_today() {
    let room = Room::start().await;
    let owner = spec(
        &unique("owner"),
        &[held(Role::Executive), held(Role::Admin)],
    );
    assert_eq!(seen(through_the_view(&room, &owner).await), (true, true));
    assert_eq!(seen(through_the_route(&room, &owner).await), (true, true));
    room.finish().await;
}

/// Every v2 staff role maps to a branch role without `SystemSettings` (§3.3): it lists without editing, as today.
#[actix_web::test]
async fn staff_mapped_to_branch_roles_list_without_editing_as_today() {
    let room = Room::start().await;
    for role in [
        Role::Teller,
        Role::BranchManager,
        Role::RateManager,
        Role::Finance,
        Role::VerificationReviewer,
    ] {
        assert!(
            !edits(role),
            "{} is a mapped staff role without SystemSettings",
            role.as_str()
        );
        let who = spec(&unique("staff"), &[held(role)]);
        assert_eq!(
            seen(through_the_view(&room, &who).await),
            (false, false),
            "{} through the view",
            role.as_str()
        );
        assert_eq!(
            seen(through_the_route(&room, &who).await),
            (false, false),
            "{} through the route",
            role.as_str()
        );
    }
    room.finish().await;
}

// ------------------------------------------------------------ the permission is read valid-now

/// An ADMIN role that has ended does not edit, even beside a role that is still valid.
#[actix_web::test]
async fn an_admin_role_that_has_ended_does_not_edit() {
    let room = Room::start().await;
    let who = spec(
        &unique("ex-admin"),
        &[
            Held {
                role: Role::Admin,
                branch: None,
                ended: true,
            },
            held(Role::Teller),
        ],
    );
    assert_eq!(seen(through_the_view(&room, &who).await), (false, false));
    assert_eq!(seen(through_the_route(&room, &who).await), (false, false));
    room.finish().await;
}
