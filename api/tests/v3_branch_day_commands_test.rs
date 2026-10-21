//! CA-8b (#1274; design `docs/design/blueprint/cash-shift-transfer.md` §4.4 `:234-235`, §3.2, §9 CA-8b,
//! §9.1; `docs/design/xchange-v3-wave3-staff.md` C1–C4, C10–C11): the seven branch-day commands on the
//! CM-1 seam, each wrapping CA-7b's `BranchDayService` (#1273).
//! Written before the implementation by the test-writer claude/5d45b017, on the BE lane controller
//! claude/ad05bf8d's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Routes and bodies (camelCase) this file assumes ([required] by §4.4 and SD unless marked):
//! - `dayStartService` `POST /api/branch-day/start` `{equipmentReady, ratesVerified, rateVersion}`;
//!   `rateVersion` is the ISO-8601 `published_at` of the branch's current rate publication
//!   (`custody_ports.rs:40`: milliseconds and `Z`);
//! - `dayCutoff` `POST /api/branch-day/cutoff` `{}`;
//! - `dayTarget` `POST /api/branch-day/target` `{amounts: {CUR: "…"}}`;
//! - `clock` `POST /api/attendance` `{direction: "IN" | "OUT"}`;
//! - `holdCreate` `POST /api/customer-holds` `{customerId, reason}`, answering the hold's `id`
//!   ([proposed]: the id is in the answer text, any shape);
//! - `holdRelease` `POST /api/customer-holds/{id}/release` `{note}`;
//! - `draftBegin` `POST /api/tx-drafts` `{side: "BUY" | "SELL", currency, amount}` ([proposed]: CA-7b's
//!   `BeginDraft` in camelCase; SD has no row for it);
//! - each body also carries `branch`, as every CM-1 command does.
//!
//! Permissions (§3.2): start/cut-off `BranchDayOperate` (BRANCH_MANAGER); target `CashTargetSet`
//! (FINANCE); clock `AttendanceClock` (every staff role); hold `CustomerHoldPlace` (TELLER,
//! BRANCH_MANAGER); release `VerificationReview` (VERIFICATION_REVIEWER); draft `ShiftOperate`
//! (TELLER, BRANCH_MANAGER) on the caller's own OPEN shift (owner, §10 Q11).
//!
//! Refusals are pinned the way today's rules say: a client-error status plus the code
//! (`signIn`, `roleNotAllowed` / `roleNotAllowedHere`, `outOfScope`, `idempotencyConflict`), never a
//! fixed 403. A refusal by the branch day itself (CA-7b / `BranchDay`) has no code in the design, so it is
//! pinned by its client-error status, `CustodyError`'s own sentence (`custody_error.rs`) and nothing
//! changed. Every effect is read back through CA-7b's tables with `PgCustodyRepository::load_state`.
//!
//! Callers: `x-test-as: <ROLE>@<branch>`. Each is a Blueprint `access::Actor` of the seeded
//! organisation with that one role.
//!
//! Needs `TEST_DATABASE_URL`; nothing skips.

mod common;
mod rates_re4c_common;

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
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Side};
use xc_shift::domain::custody::entities::customer_hold::HoldStatus;
use xc_shift::domain::custody::repositories::custody_repository::CustodyRepository;
use xc_shift::domain::custody::state::CustodyState;
use xc_shift::domain::custody::value_objects::phase::Phase;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const B01: &str = "XC-B01";
const B09: &str = "XC-B09";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const MANAGER: &str = "BRANCH_MANAGER@XC-B01";
const TELLER: &str = "TELLER@XC-B01";
const BOB: &str = "TELLER2@XC-B01";
const FINANCE: &str = "FINANCE@XC-B01";
const REVIEWER: &str = "VERIFICATION_REVIEWER@XC-B01";
const REVIEWER_B09: &str = "VERIFICATION_REVIEWER@XC-B09";
const CUSTOMER: &str = "CU-LIVE-1";
/// The `published_at` of the branch's current publication, as `custody_ports.rs` prints it.
const RATE_VERSION: &str = "2026-10-11T01:02:03.456Z";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

/// The users row of a spec's person: one uuid per role name (and branch, for the B09 reviewer).
fn user_id(spec: &str) -> &'static str {
    match spec {
        MANAGER => "00000000-0000-4000-8000-0000000000b1",
        TELLER => "00000000-0000-4000-8000-0000000000b2",
        BOB => "00000000-0000-4000-8000-0000000000b3",
        FINANCE => "00000000-0000-4000-8000-0000000000b4",
        REVIEWER => "00000000-0000-4000-8000-0000000000b5",
        REVIEWER_B09 => "00000000-0000-4000-8000-0000000000b6",
        _ => "00000000-0000-4000-8000-0000000000b7",
    }
}

// ------------------------------------------------------------ database

struct TestDb {
    pool: PgPool,
    schema: String,
}

impl TestDb {
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
        let schema = unique("v3branchday");
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
            .max_connections(8)
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
        let db = Self { pool, schema };
        db.exec("INSERT INTO branches (id, name) VALUES ('XC-B01', 'B01'), ('XC-B09', 'B09')")
            .await;
        for spec in [MANAGER, TELLER, BOB, FINANCE, REVIEWER, REVIEWER_B09] {
            db.exec(&format!(
                "INSERT INTO users (id, organization_id, name, status, created_by) \
                 VALUES ('{}', '{ORG}', '{spec}', 'ACTIVE', 'test')",
                user_id(spec)
            ))
            .await;
        }
        db.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('{CUSTOMER}', '{ORG}', '{CUSTOMER}', 'Somchai', 'LIVE', now())"
        ))
        .await;
        db
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// The branch's day in `phase` (ops_state).
    async fn phase_is(&self, branch: &str, phase: &str) {
        self.exec(&format!(
            "INSERT INTO ops_state (branch_id, phase) VALUES ('{branch}', '{phase}') \
             ON CONFLICT (branch_id) DO UPDATE SET phase = EXCLUDED.phase"
        ))
        .await;
    }

    /// `branch` open every day from an hour before now to two hours after, in its own timezone, so a draft can
    /// begin (CA-7b `begin_draft` checks the service window; no hours set refuses it).
    async fn open_around_now(&self, branch: &str) {
        self.exec(&format!(
            "UPDATE branches b SET opening_hours = (SELECT jsonb_object_agg(d, jsonb_build_array(jsonb_build_object( \
               'open', to_char((now() AT TIME ZONE b.timezone) - interval '1 hour', 'HH24:MI'), \
               'close', to_char((now() AT TIME ZONE b.timezone) + interval '2 hours', 'HH24:MI')))) \
             FROM unnest(ARRAY['mon','tue','wed','thu','fri','sat','sun']) d) WHERE b.id = '{branch}'"
        ))
        .await;
    }

    /// An OPEN shift of `spec`'s person at `branch`; answers its id.
    async fn open_shift(&self, branch: &str, spec: &str) -> String {
        let id = unique("SH");
        self.exec(&format!(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             VALUES ('{id}', '{branch}', 'OPEN', now(), 'test', '{id}-open', '{}')",
            user_id(spec)
        ))
        .await;
        id
    }

    /// A current rate publication for `branch` whose `published_at` is `RATE_VERSION`.
    async fn rates_published(&self, branch: &str) {
        rates_re4c_common::set_counter_rate(&self.pool, branch, "USD", "35.0", "36.0").await;
        self.exec(&format!(
            "UPDATE rate_publications SET published_at = '{RATE_VERSION}' \
             WHERE id = (SELECT current_publication_id FROM branches WHERE id = '{branch}')"
        ))
        .await;
    }

    async fn state(&self, branch: &str) -> CustodyState {
        PgCustodyRepository::new(self.pool.clone())
            .load_state(&BranchId::new(branch))
            .await
            .expect("load the branch's custody state")
    }

    async fn finish(self) {
        let TestDb { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ callers

struct HeaderUsers;

fn user_of(spec: &str) -> Option<User> {
    let (role, branch) = spec.split_once('@')?;
    let role = match role {
        "BRANCH_MANAGER" => Role::BranchManager,
        "TELLER" | "TELLER2" => Role::Teller,
        "FINANCE" => Role::Finance,
        "VERIFICATION_REVIEWER" => Role::VerificationReviewer,
        "CUSTOMER" => Role::Customer,
        other => panic!("unknown role {other}"),
    };
    Some(User::new(
        user_id(spec),
        ORG,
        vec![ScopedRole::new(
            role,
            Some(BranchId::new(branch)),
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ requests

#[derive(Debug)]
struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }
}

/// `POST uri` as `who` (empty = signed out) with `body` and the Idempotency-Key `key`.
async fn post(db: &TestDb, who: &str, uri: &str, body: Value, key: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(Method::POST)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header(("x-xchange-request", "1"))
        .insert_header(("origin", SAME_ORIGIN))
        .insert_header(("content-type", "application/json"))
        .insert_header(("idempotency-key", key))
        .set_payload(body.to_string());
    if !who.is_empty() {
        req = req.insert_header((AS, who));
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Answer { status, text, body }
}

async fn fresh(db: &TestDb, who: &str, uri: &str, body: Value) -> Answer {
    post(db, who, uri, body, &unique("k")).await
}

fn ok(a: &Answer, what: &str) {
    assert!(
        a.status.is_success(),
        "{what}: expected success; got {} {}",
        a.status,
        a.text
    );
}

/// A refusal with one of `codes` and a client-error status (never a fixed 403).
fn refused(a: &Answer, codes: &[&str], what: &str) {
    assert!(
        a.status.is_client_error() && codes.contains(&a.code()),
        "{what}: expected a client-error {codes:?} refusal; got {} {}",
        a.status,
        a.text
    );
}

/// A refusal by the branch day itself: a client-error status carrying `CustodyError`'s sentence.
fn refused_saying(a: &Answer, sentence: &str, what: &str) {
    assert!(
        a.status.is_client_error() && a.text.contains(sentence),
        "{what}: expected a client-error refusal saying {sentence:?}; got {} {}",
        a.status,
        a.text
    );
}

const POLICY: &[&str] = &["roleNotAllowed", "roleNotAllowedHere"];

/// (what, uri, body for a branch, a role that holds the permission, a role that lacks it)
type Route = (
    &'static str,
    String,
    fn(&str) -> Value,
    &'static str,
    &'static str,
);

fn start_body(branch: &str) -> Value {
    json!({ "branch": branch, "equipmentReady": true, "ratesVerified": true, "rateVersion": RATE_VERSION })
}

fn draft_body(branch: &str) -> Value {
    json!({ "branch": branch, "side": "BUY", "currency": "USD", "amount": "150.00" })
}

// ------------------------------------------------------------ tests

/// Every route: signed out ⇒ `signIn`; a role without the command's §3.2 permission ⇒ a policy
/// refusal; the permitted role naming a branch it holds no row at ⇒ `outOfScope`. None of them writes.
#[actix_web::test]
async fn every_route_refuses_signed_out_a_role_without_its_permission_and_another_branch() {
    let db = TestDb::start().await;
    db.phase_is(B01, "READY").await;
    db.phase_is(B09, "READY").await;
    let hold = "00000000-0000-0000-0000-00000000ffff";
    let release = format!("/api/customer-holds/{hold}/release");
    let routes: Vec<Route> = vec![
        (
            "start",
            "/api/branch-day/start".into(),
            start_body,
            MANAGER,
            TELLER,
        ),
        (
            "cutoff",
            "/api/branch-day/cutoff".into(),
            |b| json!({ "branch": b }),
            MANAGER,
            TELLER,
        ),
        (
            "target",
            "/api/branch-day/target".into(),
            |b| json!({ "branch": b, "amounts": { "USD": "1000.00" } }),
            FINANCE,
            MANAGER,
        ),
        (
            "clock",
            "/api/attendance".into(),
            |b| json!({ "branch": b, "direction": "IN" }),
            TELLER,
            "CUSTOMER@XC-B01",
        ),
        (
            "hold",
            "/api/customer-holds".into(),
            |b| json!({ "branch": b, "customerId": CUSTOMER, "reason": "passport check" }),
            TELLER,
            FINANCE,
        ),
        (
            "release",
            release.clone(),
            |b| json!({ "branch": b, "note": "checked" }),
            REVIEWER,
            TELLER,
        ),
        (
            "draft",
            "/api/tx-drafts".into(),
            draft_body,
            TELLER,
            FINANCE,
        ),
    ];
    let before = db.state(B01).await;
    for (what, uri, body, allowed, lacking) in &routes {
        let a = fresh(&db, "", uri, body(B01)).await;
        assert_eq!(
            a.status,
            StatusCode::UNAUTHORIZED,
            "{what} signed out: {}",
            a.text
        );
        assert_eq!(a.code(), "signIn", "{what} signed out: {}", a.text);

        let a = fresh(&db, lacking, uri, body(B01)).await;
        refused(&a, POLICY, &format!("{what} as {lacking}"));

        let a = fresh(&db, allowed, uri, body(B09)).await;
        refused(
            &a,
            &["outOfScope"],
            &format!("{what} as {allowed} naming {B09}"),
        );
    }
    let after = db.state(B01).await;
    assert_eq!(after.phase, before.phase, "no refusal moved the day");
    assert_eq!(
        (
            after.holds.len(),
            after.drafts.len(),
            after.attendance.len(),
            after.target.is_some()
        ),
        (0, 0, 0, false),
        "no refusal wrote a hold, a draft, an attendance row or a target"
    );
    assert_eq!(db.state(B09).await.phase, Phase::Ready);
    db.finish().await;
}

/// `dayStartService` and `dayCutoff` move the day READY → OPEN → CUT_OFF through `BranchDay`;
/// cut-off before start and start with a stale `rateVersion` are refused and change nothing.
#[actix_web::test]
async fn start_service_then_cut_off_move_the_day_and_refuse_out_of_order() {
    let db = TestDb::start().await;
    db.phase_is(B01, "READY").await;
    db.open_shift(B01, TELLER).await;
    db.rates_published(B01).await;

    let a = fresh(
        &db,
        MANAGER,
        "/api/branch-day/cutoff",
        json!({ "branch": B01 }),
    )
    .await;
    refused_saying(
        &a,
        "ตัดรายการได้เฉพาะเมื่อสาขาเปิดบริการอยู่",
        "cut-off before start (DayNotOpen)",
    );
    assert_eq!(
        db.state(B01).await.phase,
        Phase::Ready,
        "cut-off before start changes nothing"
    );

    let mut stale = start_body(B01);
    stale["rateVersion"] = json!("2026-10-10T00:00:00.000Z");
    let a = fresh(&db, MANAGER, "/api/branch-day/start", stale).await;
    refused_saying(
        &a,
        "ยืนยันอุปกรณ์และเรทที่แสดงให้ตรงกับชุดเรทล่าสุด",
        "a stale rateVersion (ServiceChecksIncomplete)",
    );
    assert_eq!(
        db.state(B01).await.phase,
        Phase::Ready,
        "a refused start changes nothing"
    );

    let a = fresh(&db, MANAGER, "/api/branch-day/start", start_body(B01)).await;
    ok(&a, "start");
    let s = db.state(B01).await;
    assert_eq!(s.phase, Phase::Open, "start: READY → OPEN");
    assert!(
        s.service_opened_at.is_some(),
        "start records when service opened"
    );

    let a = fresh(&db, MANAGER, "/api/branch-day/start", start_body(B01)).await;
    assert!(
        a.status.is_client_error(),
        "a second start (not READY): {}",
        a.text
    );
    assert_eq!(db.state(B01).await.phase, Phase::Open);

    let a = fresh(
        &db,
        MANAGER,
        "/api/branch-day/cutoff",
        json!({ "branch": B01 }),
    )
    .await;
    ok(&a, "cut-off");
    let s = db.state(B01).await;
    assert_eq!(s.phase, Phase::CutOff, "cut-off: OPEN → CUT_OFF");
    let cut = s.cutoff.expect("cut-off records who and when");
    assert_eq!(cut.by, user_id(MANAGER));
    db.finish().await;
}

/// `dayTarget` writes the cash target with its setter; `clock` writes IN then OUT; OUT with no IN is
/// refused.
#[actix_web::test]
async fn target_and_clock_write_their_rows() {
    let db = TestDb::start().await;
    db.phase_is(B01, "OPEN").await;

    let a = fresh(
        &db,
        FINANCE,
        "/api/branch-day/target",
        json!({ "branch": B01, "amounts": { "USD": "1000.00", "EUR": "250.50" } }),
    )
    .await;
    ok(&a, "target");
    let target = db.state(B01).await.target.expect("the target is stored");
    assert_eq!(target.by, user_id(FINANCE));
    let amounts: Vec<(String, String)> = target
        .amounts
        .non_zero()
        .into_iter()
        .map(|(c, m)| (c.as_str().to_owned(), m.to_string()))
        .collect();
    assert!(
        amounts.contains(&("USD".into(), "1000.00".into()))
            && amounts.contains(&("EUR".into(), "250.50".into())),
        "the target's amounts: {amounts:?}"
    );

    let a = fresh(
        &db,
        TELLER,
        "/api/attendance",
        json!({ "branch": B01, "direction": "OUT" }),
    )
    .await;
    refused_saying(
        &a,
        "ยังไม่ได้ลงเวลาเข้า",
        "clock OUT with no clock IN (NoClockInToClose)",
    );
    assert!(db.state(B01).await.attendance.is_empty());

    let a = fresh(
        &db,
        TELLER,
        "/api/attendance",
        json!({ "branch": B01, "direction": "IN" }),
    )
    .await;
    ok(&a, "clock IN");
    let att = db.state(B01).await.attendance;
    assert_eq!(att.len(), 1);
    assert_eq!(att[0].employee_id, user_id(TELLER));
    assert!(att[0].clock_out.is_none());

    let a = fresh(
        &db,
        TELLER,
        "/api/attendance",
        json!({ "branch": B01, "direction": "OUT" }),
    )
    .await;
    ok(&a, "clock OUT");
    let att = db.state(B01).await.attendance;
    assert_eq!(att.len(), 1);
    assert!(att[0].clock_out.is_some(), "clock OUT closes the clock IN");
    db.finish().await;
}

/// `holdCreate` places an OPEN hold answered by its id; `holdRelease` needs that hold OPEN at the
/// named branch and a note; a reviewer of another branch cannot release it.
#[actix_web::test]
async fn hold_then_release_at_its_own_branch_only() {
    let db = TestDb::start().await;
    db.phase_is(B01, "OPEN").await;
    db.phase_is(B09, "OPEN").await;

    let a = fresh(
        &db,
        TELLER,
        "/api/customer-holds",
        json!({ "branch": B01, "customerId": CUSTOMER, "reason": "passport check" }),
    )
    .await;
    ok(&a, "hold");
    let holds = db.state(B01).await.holds;
    assert_eq!(holds.len(), 1);
    let hold = holds[0].clone();
    assert_eq!(
        (
            hold.customer_id.as_str(),
            hold.reason.as_str(),
            hold.by.as_str(),
            hold.status
        ),
        (
            CUSTOMER,
            "passport check",
            user_id(TELLER),
            HoldStatus::Open
        )
    );
    assert!(
        a.text.contains(&hold.id),
        "the answer names the hold: {}",
        a.text
    );
    let release = format!("/api/customer-holds/{}/release", hold.id);

    // A reviewer of XC-B09: naming B01 is out of scope; naming its own branch finds no such hold.
    let a = fresh(
        &db,
        REVIEWER_B09,
        &release,
        json!({ "branch": B01, "note": "checked" }),
    )
    .await;
    refused(&a, &["outOfScope"], "a B09 reviewer naming B01");
    let a = fresh(
        &db,
        REVIEWER_B09,
        &release,
        json!({ "branch": B09, "note": "checked" }),
    )
    .await;
    refused_saying(
        &a,
        "ระบุผลตรวจรายการที่พัก",
        "a B09 reviewer releasing B01's hold at B09 (ReleaseInvalid)",
    );
    assert_eq!(
        db.state(B01).await.holds[0].status,
        HoldStatus::Open,
        "still OPEN"
    );

    let a = fresh(
        &db,
        REVIEWER,
        &release,
        json!({ "branch": B01, "note": "   " }),
    )
    .await;
    refused_saying(
        &a,
        "ระบุผลตรวจรายการที่พัก",
        "a release with no note (ReleaseInvalid)",
    );
    assert_eq!(db.state(B01).await.holds[0].status, HoldStatus::Open);

    let a = fresh(
        &db,
        REVIEWER,
        &release,
        json!({ "branch": B01, "note": "documents verified" }),
    )
    .await;
    ok(&a, "release");
    let h = db.state(B01).await.holds[0].clone();
    assert_eq!(h.status, HoldStatus::Released);
    assert_eq!(h.reviewed_by.as_deref(), Some(user_id(REVIEWER)));
    assert_eq!(h.note.as_deref(), Some("documents verified"));

    let a = fresh(
        &db,
        REVIEWER,
        &release,
        json!({ "branch": B01, "note": "again" }),
    )
    .await;
    refused_saying(
        &a,
        "ระบุผลตรวจรายการที่พัก",
        "releasing a released hold (ReleaseInvalid)",
    );
    db.finish().await;
}

/// `draftBegin` (owner §10 Q11, `ShiftOperate`): only on the caller's own OPEN shift, while the day is OPEN.
/// v2's kept rule (`BranchDay::v2_begin_transaction`, CA-7b's `begin_draft`): a draft is begun during service and
/// used by a save after cut-off; a draft begun at CUT_OFF is refused (`NotCustodianInOpenService`). Lane
/// controller's ruling A on #1274.
#[actix_web::test]
async fn draft_begin_only_on_the_callers_own_open_shift() {
    let db = TestDb::start().await;
    db.phase_is(B01, "OPEN").await;
    db.open_around_now(B01).await;
    let bob_shift = db.open_shift(B01, BOB).await;

    let a = fresh(&db, TELLER, "/api/tx-drafts", draft_body(B01)).await;
    refused_saying(
        &a,
        "ยังไม่มีกะเปิด",
        "a teller with no shift of her own, while Bob's is OPEN (NoOpenShift)",
    );
    assert!(
        db.state(B01).await.drafts.is_empty(),
        "nothing written on Bob's shift"
    );

    let a = fresh(&db, BOB, "/api/tx-drafts", draft_body(B01)).await;
    ok(&a, "Bob's draft on his own shift");
    let drafts = db.state(B01).await.drafts;
    assert_eq!(drafts.len(), 1);
    assert_eq!(
        (drafts[0].shift_id.as_str(), drafts[0].employee_id.as_str()),
        (bob_shift.as_str(), user_id(BOB))
    );
    assert_eq!(drafts[0].amount.to_string(), "150.00");
    assert!(
        a.text.contains(&drafts[0].id),
        "the answer names the draft: {}",
        a.text
    );

    db.phase_is(B01, "CUT_OFF").await;
    let a = fresh(&db, BOB, "/api/tx-drafts", draft_body(B01)).await;
    refused_saying(
        &a,
        "ต้องเป็น Front ผู้ถือครองเงินในกะที่เปิด",
        "a draft begun after cut-off (NotCustodianInOpenService)",
    );
    assert_eq!(
        db.state(B01).await.drafts.len(),
        1,
        "nothing written at CUT_OFF"
    );
    db.finish().await;
}

/// An `Idempotency-Key` replay answers the first answer and writes nothing more; the key with
/// another body is `idempotencyConflict`.
#[actix_web::test]
async fn a_replayed_key_writes_once_and_a_reused_key_conflicts() {
    let db = TestDb::start().await;
    db.phase_is(B01, "OPEN").await;
    let body = json!({ "branch": B01, "customerId": CUSTOMER, "reason": "passport check" });
    let key = unique("hold");
    let first = post(&db, TELLER, "/api/customer-holds", body.clone(), &key).await;
    ok(&first, "first hold");
    let again = post(&db, TELLER, "/api/customer-holds", body, &key).await;
    ok(&again, "replay");
    assert_eq!(
        again.body, first.body,
        "the replay answers the first answer"
    );
    assert_eq!(db.state(B01).await.holds.len(), 1, "one hold, not two");

    let other = json!({ "branch": B01, "customerId": CUSTOMER, "reason": "another reason" });
    let a = post(&db, TELLER, "/api/customer-holds", other, &key).await;
    refused(&a, &["idempotencyConflict"], "the key with another body");
    assert_eq!(db.state(B01).await.holds.len(), 1);
    db.finish().await;
}

/// Every command runs under the seam's branch lock (CM-1, §4.9): while another transaction holds
/// `branch_versions … FOR UPDATE` on the branch, the command waits, and it finishes once that commits.
#[actix_web::test]
async fn a_command_waits_for_the_branch_lock() {
    let db = TestDb::start().await;
    db.phase_is(B01, "OPEN").await;
    db.exec(&format!(
        "INSERT INTO branch_versions (branch_id, version) VALUES ('{B01}', 0) ON CONFLICT DO NOTHING"
    ))
    .await;
    for (uri, who, body) in [
        (
            "/api/attendance",
            TELLER,
            json!({ "branch": B01, "direction": "IN" }),
        ),
        (
            "/api/customer-holds",
            TELLER,
            json!({ "branch": B01, "customerId": CUSTOMER, "reason": "lock check" }),
        ),
        (
            "/api/branch-day/target",
            FINANCE,
            json!({ "branch": B01, "amounts": { "USD": "10.00" } }),
        ),
    ] {
        let mut holder = db.pool.begin().await.expect("begin");
        sqlx::query("SELECT version FROM branch_versions WHERE branch_id = $1 FOR UPDATE")
            .bind(B01)
            .fetch_one(&mut *holder)
            .await
            .expect("hold the branch lock");
        let call = fresh(&db, who, uri, body);
        tokio::pin!(call);
        let early = tokio::time::timeout(Duration::from_millis(700), &mut call).await;
        assert!(
            early.is_err(),
            "{uri} finished while another transaction held the branch lock: {:?}",
            early.ok()
        );
        holder.commit().await.expect("release the lock");
        let a = tokio::time::timeout(MUST_FINISH, call)
            .await
            .unwrap_or_else(|_| panic!("{uri} did not finish after the lock was released"));
        ok(&a, uri);
    }
    db.finish().await;
}

// ------------------------------------------------------------ added by claude/07d3765f (review gaps M6, M7 on #1274)

/// Review gap M6 (#1274): a draft stores the side and the currency it was sent, because the later save must match
/// them (`BranchDay::complete_draft`).
#[actix_web::test]
async fn draft_begin_stores_the_side_and_currency_it_was_sent() {
    let db = TestDb::start().await;
    // begun while the day is OPEN (lane controller's ruling A on #1274)
    db.phase_is(B01, "OPEN").await;
    db.open_around_now(B01).await;
    db.open_shift(B01, BOB).await;

    for (side, currency) in [("BUY", "USD"), ("SELL", "EUR")] {
        let a = fresh(
            &db,
            BOB,
            "/api/tx-drafts",
            json!({ "branch": B01, "side": side, "currency": currency, "amount": "150.00" }),
        )
        .await;
        ok(&a, &format!("Bob's {side} {currency} draft"));
    }
    let mut got: Vec<(Side, CurrencyCode)> = db
        .state(B01)
        .await
        .drafts
        .iter()
        .map(|d| (d.side, d.currency))
        .collect();
    got.sort_by_key(|(side, _)| *side == Side::Sell);
    assert_eq!(
        got,
        vec![
            (Side::Buy, CurrencyCode::parse("USD").expect("USD")),
            (Side::Sell, CurrencyCode::parse("EUR").expect("EUR")),
        ],
        "each draft keeps the side and currency it was sent"
    );
    db.finish().await;
}

/// Review gap M7 (#1274): each command's policy row is under §4.4's command name and needs §3.2's permission, so
/// the shared row table names them as the design does (the shape of TX-10's `transactionView` pin).
#[test]
fn each_commands_row_is_under_its_design_name_with_its_permission() {
    for (action, permission) in [
        ("dayStartService", Permission::BranchDayOperate),
        ("dayCutoff", Permission::BranchDayOperate),
        ("dayTarget", Permission::CashTargetSet),
        ("clock", Permission::AttendanceClock),
        ("holdCreate", Permission::CustomerHoldPlace),
        ("holdRelease", Permission::VerificationReview),
        ("draftBegin", Permission::ShiftOperate),
    ] {
        assert_eq!(
            Policy::need(EntryPath::Rpc, &Action::new(action)),
            Some(Need::Permission(permission)),
            "{action}: Rpc row with its §3.2 permission"
        );
    }
}
