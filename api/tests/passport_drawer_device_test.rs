//! Row AG-PD (`docs/design/xchange-v3.md` §5.4c *PD-1 becomes an agent job*, §5.5.5 rows 7-12,
//! §10): the passport-step drawer, for the kiosk and the desk, switched on per branch by
//! `devices.passportDrawerBranches` and opened by the branch's counter drawer agent. Written
//! before the implementation. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Everything goes through the server's whole route table (`bootstrap::configure`) with real
//! sessions made at the real sign-in doors (`rpc_common`), with settings loaded from the
//! committed `base.yaml` plus a `development.yaml` naming the switched-on branches. The agent
//! is played by the test: it is written straight into `device_agents` (row AG-1's table), and
//! a task of the test moves the passport job the way `AgentService` would (row AG-2), so
//! nothing here depends on AG-2.
//!
//! Nothing here names a type AG-PD adds: the only entry point used directly is S0's builder
//! `passport_drawer_device(&Settings, &PgPool)`. The route `POST /api/drawer-passport` is
//! row PD-0's (#327); until it lands every route test answers 404.
//!
//! Mutations that must turn this red:
//! - the builder still answering `None`                         -> the_builder_builds_the_device_…
//! - the builder answering `Some` with an empty switch          -> the_builder_builds_nothing_…
//! - the switch not checked per branch (any listed branch on)   -> the_switch_decides_row_7_…
//! - the order id checked before the switch                     -> the_switch_decides_row_7_…
//! - a branch shape not checked when the configuration loads    -> a_bad_branch_in_the_switch_…
//! - the kiosk holder not checked, or checked for desk orders   -> row_9_…
//! - ACCEPTED or ID_CHECKED left out of the holder rule         -> row_9_…
//! - the answer given before the agent reports (no reading)     -> row_12_…, the_answer_maps_…
//! - the job read inside the queueing transaction (never seen)  -> row_12_…, the_answer_maps_…
//! - the reading stopped at LEASED (the report not waited for) -> the_answer_maps_…
//! - a wait longer than 5 s, or no limit                        -> the_answer_maps_…
//! - a mapped word other than §5.5.5's                          -> the_answer_maps_…
//! - a repeat by another employee answered instead of refused   -> row_10_…
//! - `duplicate` missing on a repeat, or a second job queued    -> row_11_…, two_asks_at_once_…
//! - a repeat waiting 5 s instead of answering the job's state  -> row_11_…
//! - KIOSK and DESK of the same id treated as one order         -> row_11_…
//! - `NoAgent` answered as anything but `SIMULATED`             -> no_active_agent_…
//! - `NoDrawer` answered as anything but `UNCERTAIN`            -> a_branch_without_a_drawer_…
//! - the built-in printer models used, not the configured ones  -> the_branchs_printer_model_…
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use compat_common::Person;
use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use sqlx::PgPool;
use xchange::bootstrap::AppState;
use xchange::config::Settings;
use xchange::infrastructure::devices::passport_drawer_device::passport_drawer_device;

const ORIGIN: &str = "http://desk.test";
const ROUTE: &str = "/api/drawer-passport";

const NOT_HOLDER: &str = "ต้องเป็นผู้รับเรื่องของคิวนี้ก่อนเปิดลิ้นชัก";
const OTHER_EMPLOYEE: &str = "ออเดอร์นี้เป็นของพนักงานคนอื่น";
const BAD_ORDER: &str = "รหัสออเดอร์ไม่ถูกต้อง";

const AGENT_MAIN: &str = "a0000000-0000-4000-8000-0000000000a1";
const AGENT_B2: &str = "a0000000-0000-4000-8000-0000000000b2";

/// The window of the "reported inside the window" rows: long enough that an agent's report lands
/// inside it on a loaded test machine, while bounding `took < LONG_WINDOW`.
const LONG_WINDOW_MS: u64 = 8_000;
const LONG_WINDOW: Duration = Duration::from_millis(LONG_WINDOW_MS);
/// The short window the "never reported" rows run with, plus room for a loaded test machine while
/// keeping `SHORT_WINDOW + SLACK` under 5 s.
const SHORT_WINDOW_MS: u64 = 400;
const SHORT_WINDOW: Duration = Duration::from_millis(SHORT_WINDOW_MS);
const SLACK: Duration = Duration::from_millis(4_400);

fn order(n: u32) -> String {
    format!("0a000000-0000-4000-8000-{n:012x}")
}

fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join(format!("xc-agpd-{}", uuid::Uuid::new_v4()))
}

/// The committed `base.yaml`, plus a `development.yaml` holding `devices:` `extra` when given.
fn load_settings(extra: Option<&str>) -> Result<Settings, config::ConfigError> {
    let dir = scratch_dir();
    std::fs::create_dir_all(&dir).expect("create a configuration directory");
    std::fs::copy(
        common::config_dir().join("base.yaml"),
        dir.join("base.yaml"),
    )
    .expect("copy base.yaml");
    let development = match extra {
        None => "{}\n".to_string(),
        Some(text) => format!("devices:\n  {text}\n"),
    };
    std::fs::write(dir.join("development.yaml"), development).expect("write development.yaml");
    let uploads = dir.join("uploads");
    let uploads = uploads
        .to_str()
        .expect("a scratch path in UTF-8")
        .to_string();
    let loaded = Settings::load_from(&dir, common::env(&[("APP_FILES__DIR", uploads.as_str())]));
    let _ = std::fs::remove_dir_all(&dir);
    loaded
}

/// Settings whose switch lists exactly `branches`.
fn switched_on(branches: &[&str]) -> Settings {
    let list = serde_json::to_string(branches).expect("a JSON list is YAML");
    load_settings(Some(&format!("passportDrawerBranches: {list}")))
        .expect("the configuration loads with the switch")
}

/// Settings whose switch lists `MAIN` and sets `passportDrawerReadWindowMs` to `window_ms`.
fn switched_on_with_window(window_ms: u64) -> Settings {
    load_settings(Some(&format!(
        "passportDrawerBranches: [MAIN]\n  passportDrawerReadWindowMs: {window_ms}"
    )))
    .expect("the configuration loads with the switch and the window")
}

fn ask_body(branch: &str, order: &str, kiosk: bool, mode: &str) -> String {
    json!({ "branchId": branch, "orderId": order, "kiosk": kiosk, "mode": mode }).to_string()
}

fn state_body(state: &str) -> Value {
    json!({ "state": state, "physicalResultConfirmed": false })
}

fn duplicate_body(state: &str) -> Value {
    json!({ "state": state, "duplicate": true, "physicalResultConfirmed": false })
}

/// Fails unless this is a 200 answering exactly `body`, never cached.
fn assert_answer(answer: &Answer, body: &Value, label: &str) {
    assert_eq!(answer.status, 200, "{label}: {:.300}", answer.text);
    assert_eq!(&answer.json(), body, "{label}");
    assert_eq!(answer.header("cache-control"), Some("no-store"), "{label}");
}

/// What the test's agent does with the passport job once it is queued.
#[derive(Clone, Copy)]
enum Agent {
    /// Claims it and reports `outcome`, as `AgentService::claim` then `report` would.
    Reports(&'static str),
    /// Claims it and never reports.
    Leases,
    /// Claims it, and reports `outcome` 600 ms later.
    LeasesThenReports(&'static str),
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
    people: Vec<Person>,
}

impl Room {
    async fn open(settings: Settings) -> Room {
        let desk = Desk::open().await;
        let state = AppState::new(Arc::new(settings), desk.db.pool.clone());
        let clock: Arc<dyn xchange::presentation::http::rpc::RpcClock> = desk.clock.clone();
        let bank: Arc<dyn xc_rate::domain::rate::entities::rate_source::RateSource> =
            desk.bank.clone();
        let app = App::new(
            actix_web::test::init_service(
                actix_web::App::new()
                    .configure(move |cfg| xchange::bootstrap::configure(cfg, state))
                    .app_data(actix_web::web::Data::from(clock))
                    .app_data(actix_web::web::Data::new(bank)),
            )
            .await,
            |t| t.to_request(),
        );
        let people = one_of_each_role();
        desk.employ(&people).await;
        let cookies = sign_in(&app, ORIGIN, &people).await;
        Room {
            desk,
            app,
            cookies,
            people,
        }
    }

    fn pool(&self) -> PgPool {
        self.desk.db.pool.clone()
    }

    /// The id the caller acts under: `owner`, or the employee's id (`Actor::id`).
    fn id_of(&self, who: &str) -> String {
        self.people
            .iter()
            .find(|p| p.key == who)
            .unwrap_or_else(|| panic!("nobody is {who}"))
            .id
            .clone()
    }

    /// Every ask is made a millisecond after the one before it, on the clock the routes read,
    /// so the jobs it queues are made at strictly later times, as they are in production.
    async fn ask(&self, who: &str, body: String) -> Answer {
        self.desk.clock.advance(chrono::Duration::milliseconds(1));
        let call = Call::post(ORIGIN, ROUTE, body).cookie(&self.cookies[who]);
        self.app.send(call).await
    }

    /// Asks while the test's agent does `agent` to the job `request_id`; gives the answer and
    /// how long it took.
    async fn ask_with(
        &self,
        who: &str,
        body: String,
        request_id: &str,
        agent: Agent,
    ) -> (Answer, Duration) {
        let played = tokio::spawn(play(self.pool(), request_id.to_string(), agent));
        let started = Instant::now();
        let answer = self.ask(who, body).await;
        let took = started.elapsed();
        assert!(
            played.await.expect("the test's agent ran"),
            "{request_id}: the test's agent found no queued job: {:.300}",
            answer.text
        );
        (answer, took)
    }

    async fn branch(&self, id: &str) {
        sqlx::query("INSERT INTO branches (id, name) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("Branch {id}"))
            .execute(&self.desk.db.pool)
            .await
            .unwrap_or_else(|e| panic!("insert branch {id}: {e}"));
    }

    /// An active agent of the branch, as the owner's `add` leaves it (row AG-1's table).
    async fn agent(&self, branch: &str, id: &str) {
        sqlx::query(
            "INSERT INTO device_agents (id, branch_id, name, created_at, created_by) \
             VALUES ($1::uuid, $2, 'Mac เคาน์เตอร์ 1', now(), 'owner')",
        )
        .bind(id)
        .bind(branch)
        .execute(&self.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("insert an agent of {branch}: {e}"));
    }

    async fn revoke(&self, id: &str) {
        sqlx::query(
            "UPDATE device_agents SET revoked_at = now(), revoked_by = 'owner' \
             WHERE id = $1::uuid",
        )
        .bind(id)
        .execute(&self.desk.db.pool)
        .await
        .expect("revoke the agent");
    }

    async fn printer_model(&self, branch: &str, model: &str) {
        sqlx::query(
            "INSERT INTO device_settings (branch_id, key, value) VALUES ($1, 'printerModel', $2) \
             ON CONFLICT (branch_id, key) DO UPDATE SET value = excluded.value",
        )
        .bind(branch)
        .bind(model)
        .execute(&self.desk.db.pool)
        .await
        .expect("set printerModel");
    }

    async fn no_drawer(&self, branch: &str) {
        sqlx::query(
            "INSERT INTO device_settings (branch_id, key, value) VALUES ($1, 'hasDrawer', '0') \
             ON CONFLICT (branch_id, key) DO UPDATE SET value = excluded.value",
        )
        .bind(branch)
        .execute(&self.desk.db.pool)
        .await
        .expect("set hasDrawer = 0");
    }

    /// A kiosk request of `branch` in `status`, held by `staff` (an `Actor::id`).
    async fn kiosk_request(&self, branch: &str, id: &str, status: &str, staff: Option<&str>) {
        sqlx::query(
            "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, \
             currency, rate, foreign_amount, thb, input_amount, payment_method) \
             VALUES ($1, $2, $3, now(), $4, 'BUY', 'USD', 32.5, 100, 3250, 100, 'Cash')",
        )
        .bind(branch)
        .bind(id)
        .bind(status)
        .bind(staff)
        .execute(&self.desk.db.pool)
        .await
        .unwrap_or_else(|e| panic!("insert kiosk request {id}: {e}"));
    }

    /// Every job: request id, kind, status, actor, ref — in the order they were made. A job is
    /// made at the request's clock (#379 §7) and `ask` moves the test's clock on before every
    /// ask, so `created_at` alone orders them; `id` only breaks a tie no test here makes.
    async fn jobs(&self) -> Vec<(String, String, String, String, String)> {
        sqlx::query_as(
            "SELECT request_id, kind, status, actor, ref FROM device_jobs \
             ORDER BY created_at, id",
        )
        .fetch_all(&self.desk.db.pool)
        .await
        .expect("read device_jobs")
    }

    /// The details of every device line of `kind`.
    async fn lines(&self, kind: &str) -> Vec<Value> {
        let texts: Vec<String> = sqlx::query_scalar(
            "SELECT detail::text FROM device_events WHERE kind = $1 ORDER BY seq",
        )
        .bind(kind)
        .fetch_all(&self.desk.db.pool)
        .await
        .expect("read device_events");
        texts
            .iter()
            .map(|t| serde_json::from_str(t).expect("a line's detail is JSON"))
            .collect()
    }

    async fn line_count(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM device_events")
            .fetch_one(&self.desk.db.pool)
            .await
            .expect("count device_events")
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// The test's agent: waits up to 5 s for the job `request_id` to be queued (committed), then
/// moves it as `agent` says. True when it found the job.
async fn play(pool: PgPool, request_id: String, agent: Agent) -> bool {
    for _ in 0..100 {
        let moved = match agent {
            Agent::Leases | Agent::LeasesThenReports(_) => sqlx::query(
                "UPDATE device_jobs SET status = 'LEASED', lease_id = $2::uuid, \
                 leased_by = (SELECT id FROM device_agents d \
                              WHERE d.branch_id = device_jobs.branch_id \
                                AND d.revoked_at IS NULL), \
                 lease_until = now() + interval '10 seconds', updated_at = now() \
                 WHERE request_id = $1 AND status = 'QUEUED'",
            )
            .bind(&request_id)
            .bind(uuid::Uuid::new_v4().to_string())
            .execute(&pool)
            .await
            .expect("lease the job"),
            Agent::Reports(outcome) => sqlx::query(
                "UPDATE device_jobs SET status = $2, outcome_reason = 'TEST_AGENT', \
                 updated_at = now() WHERE request_id = $1 AND status = 'QUEUED'",
            )
            .bind(&request_id)
            .bind(outcome)
            .execute(&pool)
            .await
            .expect("report on the job"),
        };
        if moved.rows_affected() == 1 {
            if let Agent::LeasesThenReports(outcome) = agent {
                tokio::time::sleep(Duration::from_millis(600)).await;
                let reported = sqlx::query(
                    "UPDATE device_jobs SET status = $2, outcome_reason = 'TEST_AGENT', \
                     updated_at = now() WHERE request_id = $1 AND status = 'LEASED'",
                )
                .bind(&request_id)
                .bind(outcome)
                .execute(&pool)
                .await
                .expect("report on the leased job");
                return reported.rows_affected() == 1;
            }
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

fn passport_id(branch: &str, source: &str, order: &str) -> String {
    format!("auto:{branch}:passport:{source}:{order}")
}

// ── the switch and the builder ───────────────────────────────────────────────

/// The committed configuration switches no branch on, and an empty list is the same: the
/// builder gives no device, so PD-0's workflow answers `DISABLED` everywhere.
#[actix_web::test]
async fn the_builder_builds_nothing_while_no_branch_is_switched_on() {
    let pool = common::unreachable_pool();
    let committed = load_settings(None).expect("the committed configuration loads");
    assert!(
        passport_drawer_device(&committed, &pool).is_none(),
        "the committed configuration switches no branch on"
    );
    assert!(
        passport_drawer_device(&switched_on(&[]), &pool).is_none(),
        "an empty switch builds no device"
    );
}

/// A switch listing a branch builds the device; building it touches no database.
#[actix_web::test]
async fn the_builder_builds_the_device_once_a_branch_is_switched_on() {
    let pool = common::unreachable_pool();
    assert!(
        passport_drawer_device(&switched_on(&["MAIN"]), &pool).is_some(),
        "passportDrawerBranches: [MAIN] builds the passport drawer"
    );
    assert!(
        passport_drawer_device(&switched_on(&["B2", "MAIN"]), &pool).is_some(),
        "two branches build one device"
    );
}

/// An entry that is not a branch id (1-70 of `A-Za-z0-9-`, `passport-drawer.mjs:17`) stops
/// the configuration from loading, as a bad printer model does.
#[actix_web::test]
async fn a_bad_branch_in_the_switch_stops_the_configuration_from_loading() {
    for list in [r#"["bad branch"]"#, r#"[""]"#, r#"["MAIN", "B/2"]"#] {
        assert!(
            load_settings(Some(&format!("passportDrawerBranches: {list}"))).is_err(),
            "passportDrawerBranches: {list} must be refused"
        );
    }
    let long = "A".repeat(71);
    assert!(
        load_settings(Some(&format!("passportDrawerBranches: [{long}]"))).is_err(),
        "a 71-character branch must be refused"
    );
    let longest = "A".repeat(70);
    assert!(
        load_settings(Some(&format!(
            "passportDrawerBranches: [{longest}, MAIN, b-2]"
        )))
        .is_ok(),
        "70 characters, upper and lower case and '-' are a branch id"
    );
}

/// Row 7 per branch: only a listed branch reaches the drawer; row 6 still comes first, the
/// order id is not looked at on a branch that is off, and staff are on their own branch.
#[actix_web::test]
async fn the_switch_decides_row_7_per_branch() {
    let room = Room::open(switched_on(&["B2"])).await;
    room.branch("B2").await;
    room.agent("MAIN", AGENT_MAIN).await;
    room.agent("B2", AGENT_B2).await;

    let off = room
        .ask("OWNER", ask_body("MAIN", &order(1), false, "LIVE"))
        .await;
    assert_answer(&off, &state_body("DISABLED"), "MAIN is not listed");

    let off_bad_order = room
        .ask("OWNER", ask_body("MAIN", "not-an-order", false, "LIVE"))
        .await;
    assert_answer(
        &off_bad_order,
        &state_body("DISABLED"),
        "row 7 comes before row 8",
    );

    // OR-4b (#767, D-4, §02 "Server ตรวจ … branch scope"): a MAIN manager naming B2 is refused as
    // out of scope; naming MAIN, they ask for MAIN
    let staff_elsewhere = room
        .ask("MANAGER", ask_body("B2", &order(2), false, "LIVE"))
        .await;
    assert_eq!(staff_elsewhere.status, 403, "{:.300}", staff_elsewhere.text);
    assert_eq!(
        staff_elsewhere.json(),
        json!({ "error": "สาขานี้อยู่นอกขอบเขตของบัญชีนี้" }),
        "a MAIN manager naming B2"
    );
    let staff_home = room
        .ask("MANAGER", ask_body("MAIN", &order(2), false, "LIVE"))
        .await;
    assert_answer(
        &staff_home,
        &state_body("DISABLED"),
        "a MAIN manager naming MAIN asks for MAIN",
    );

    let test_mode = room
        .ask("OWNER", ask_body("B2", &order(3), false, "TEST"))
        .await;
    assert_answer(
        &test_mode,
        &state_body("SIMULATED"),
        "row 6 comes before the switch",
    );
    assert!(room.jobs().await.is_empty(), "nothing queued so far");

    let on_id = passport_id("B2", "DESK", &order(4));
    let (on, _) = room
        .ask_with(
            "OWNER",
            ask_body("B2", &order(4), false, "LIVE"),
            &on_id,
            Agent::Reports("DRAWER_OPENED"),
        )
        .await;
    assert_answer(&on, &state_body("SUBMITTED_TO_CUPS"), "B2 is listed");
    let jobs = room.jobs().await;
    assert_eq!(jobs.len(), 1, "one job: {jobs:?}");
    assert_eq!(jobs[0].0, on_id);
    room.finish().await;
}

// ── rows 8 and 9 ─────────────────────────────────────────────────────────────

/// Row 8 on a switched-on branch: the order id must be 36 of `[a-f0-9-]`; nothing is queued.
#[actix_web::test]
async fn row_8_a_malformed_order_id_is_refused_on_a_switched_on_branch() {
    let room = Room::open(switched_on(&["MAIN"])).await;
    room.agent("MAIN", AGENT_MAIN).await;

    let upper = order(1).to_uppercase();
    let short = &order(1)[..35];
    let long = format!("{}0", order(1));
    let bodies = [
        ask_body("MAIN", "", false, "LIVE"),
        ask_body("MAIN", &upper, false, "LIVE"),
        ask_body("MAIN", short, false, "LIVE"),
        ask_body("MAIN", &long, true, "LIVE"),
        json!({ "branchId": "MAIN", "kiosk": false, "mode": "LIVE" }).to_string(),
        json!({ "branchId": "MAIN", "orderId": 7, "kiosk": false, "mode": "LIVE" }).to_string(),
    ];
    for body in bodies {
        let answer = room.ask("FRONT", body.clone()).await;
        answer.assert_refused(400, BAD_ORDER, &body);
        assert_eq!(answer.header("cache-control"), Some("no-store"), "{body}");
    }
    assert!(room.jobs().await.is_empty(), "nothing queued");
    assert_eq!(room.line_count().await, 0, "nothing logged");
    room.finish().await;
}

/// Row 9: a kiosk order opens the drawer only for its holder, while it is ACCEPTED or
/// ID_CHECKED in the caller's branch. A desk order is never looked up among kiosk requests.
#[actix_web::test]
async fn row_9_only_the_holder_of_a_kiosk_order_in_accepted_or_id_checked_opens_the_drawer() {
    let room = Room::open(switched_on(&["MAIN", "B2"])).await;
    room.branch("B2").await;
    room.agent("MAIN", AGENT_MAIN).await;
    let front = room.id_of("FRONT");
    let backend = room.id_of("BACKEND");

    room.kiosk_request("MAIN", &order(2), "WAITING", None).await;
    room.kiosk_request("MAIN", &order(3), "ACCEPTED", Some(&backend))
        .await;
    room.kiosk_request("MAIN", &order(4), "COMPLETED", Some(&front))
        .await;
    room.kiosk_request("B2", &order(5), "ACCEPTED", Some(&front))
        .await;
    room.kiosk_request("MAIN", &order(6), "QUOTED", None).await;

    for (n, why) in [
        (1, "no such kiosk request"),
        (2, "WAITING, nobody holds it"),
        (3, "held by somebody else"),
        (4, "COMPLETED"),
        (5, "a request of another branch"),
        (6, "QUOTED"),
    ] {
        let answer = room
            .ask("FRONT", ask_body("MAIN", &order(n), true, "LIVE"))
            .await;
        answer.assert_refused(400, NOT_HOLDER, why);
    }
    assert!(room.jobs().await.is_empty(), "no refusal queues a job");
    assert_eq!(room.line_count().await, 0, "no refusal logs a line");

    room.kiosk_request("MAIN", &order(7), "ACCEPTED", Some(&front))
        .await;
    room.kiosk_request("MAIN", &order(8), "ID_CHECKED", Some(&front))
        .await;
    for n in [7, 8] {
        let id = passport_id("MAIN", "KIOSK", &order(n));
        let (answer, _) = room
            .ask_with(
                "FRONT",
                ask_body("MAIN", &order(n), true, "LIVE"),
                &id,
                Agent::Reports("DRAWER_OPENED"),
            )
            .await;
        assert_answer(&answer, &state_body("SUBMITTED_TO_CUPS"), &id);
    }

    // The id of a kiosk request somebody else holds, asked as a desk order: not a kiosk order.
    let desk_id = passport_id("MAIN", "DESK", &order(3));
    let (desk, _) = room
        .ask_with(
            "FRONT",
            ask_body("MAIN", &order(3), false, "LIVE"),
            &desk_id,
            Agent::Reports("DRAWER_OPENED"),
        )
        .await;
    assert_answer(&desk, &state_body("SUBMITTED_TO_CUPS"), "a desk order");

    let ids: Vec<String> = room.jobs().await.into_iter().map(|j| j.0).collect();
    assert_eq!(
        ids,
        vec![
            passport_id("MAIN", "KIOSK", &order(7)),
            passport_id("MAIN", "KIOSK", &order(8)),
            desk_id,
        ]
    );
    room.finish().await;
}

// ── row 12 and the states ────────────────────────────────────────────────────

/// Row 12: the first ask queues one passport job for the agent (AG-3's id, reason and line)
/// and answers what the agent reported, as soon as it reported.
#[actix_web::test]
async fn row_12_a_first_ask_queues_one_passport_job_and_answers_what_the_agent_reported() {
    let room = Room::open(switched_on_with_window(LONG_WINDOW_MS)).await;
    room.agent("MAIN", AGENT_MAIN).await;

    let id = passport_id("MAIN", "DESK", &order(1));
    let (answer, took) = room
        .ask_with(
            "OWNER",
            ask_body("MAIN", &order(1), false, "LIVE"),
            &id,
            Agent::Reports("DRAWER_OPENED"),
        )
        .await;
    assert_answer(&answer, &state_body("SUBMITTED_TO_CUPS"), "the first ask");
    assert!(
        took < LONG_WINDOW,
        "answered {took:?} after asking, not inside the {LONG_WINDOW:?} window"
    );

    let owner_id: String = sqlx::query_scalar(
        "SELECT u.id::text FROM users u \
         WHERE EXISTS (SELECT 1 FROM user_roles r \
                       WHERE r.user_id = u.id AND r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
         ORDER BY u.created_at, u.id LIMIT 1",
    )
    .fetch_one(&room.desk.db.pool)
    .await
    .expect("read owner users.id");
    let jobs = room.jobs().await;
    // OR-7.m G2: rule 2 (#3700)
    assert_eq!(
        jobs,
        vec![(
            id.clone(),
            "DRAWER_REQUEST".to_string(),
            "DRAWER_OPENED".to_string(),
            owner_id,
            String::new(),
        )],
        "one passport job, queued by the caller"
    );
    let lines = room.lines("DRAWER_REQUEST").await;
    assert_eq!(lines.len(), 1, "one line for the job: {lines:?}");
    assert_eq!(lines[0]["reason"], "PASSPORT_STEP");
    room.finish().await;
}

/// The answer is the job's state after at most the configured read window: QUEUED and LEASED →
/// `REQUESTED`; DRAWER_OPENED → `SUBMITTED_TO_CUPS`; FAILED, EXPIRED, and the two simulated
/// states no agent job should reach → `UNCERTAIN`.
#[actix_web::test]
async fn the_answer_maps_every_state_the_job_can_reach() {
    let room = Room::open(switched_on_with_window(LONG_WINDOW_MS)).await;
    room.agent("MAIN", AGENT_MAIN).await;

    let reported = [
        ("DRAWER_OPENED", "SUBMITTED_TO_CUPS"),
        ("FAILED", "UNCERTAIN"),
        ("EXPIRED", "UNCERTAIN"),
        ("COMMAND_SIMULATED", "UNCERTAIN"),
        ("SIMULATED_PRINTED", "UNCERTAIN"),
    ];
    for (n, (status, word)) in reported.into_iter().enumerate() {
        let o = order(10 + n as u32);
        let id = passport_id("MAIN", "DESK", &o);
        let (answer, took) = room
            .ask_with(
                "BACKEND",
                ask_body("MAIN", &o, false, "LIVE"),
                &id,
                Agent::Reports(status),
            )
            .await;
        assert_answer(&answer, &state_body(word), status);
        assert!(took < LONG_WINDOW, "{status}: answered after {took:?}");
    }

    let late = passport_id("MAIN", "DESK", &order(19));
    let (answer, took) = room
        .ask_with(
            "BACKEND",
            ask_body("MAIN", &order(19), false, "LIVE"),
            &late,
            Agent::LeasesThenReports("DRAWER_OPENED"),
        )
        .await;
    assert_answer(
        &answer,
        &state_body("SUBMITTED_TO_CUPS"),
        "LEASED, then reported: the reading goes on past LEASED",
    );
    assert!(
        took < LONG_WINDOW,
        "LEASED then DRAWER_OPENED: answered after {took:?}"
    );

    let short_room = Room::open(switched_on_with_window(SHORT_WINDOW_MS)).await;
    short_room.agent("MAIN", AGENT_MAIN).await;

    let leased = passport_id("MAIN", "DESK", &order(20));
    let (answer, took) = short_room
        .ask_with(
            "BACKEND",
            ask_body("MAIN", &order(20), false, "LIVE"),
            &leased,
            Agent::Leases,
        )
        .await;
    assert_answer(&answer, &state_body("REQUESTED"), "LEASED, never reported");
    assert!(
        took >= SHORT_WINDOW && took < SHORT_WINDOW + SLACK,
        "LEASED: answered after {took:?}"
    );

    let started = Instant::now();
    let answer = short_room
        .ask("BACKEND", ask_body("MAIN", &order(21), false, "LIVE"))
        .await;
    let took = started.elapsed();
    assert_answer(&answer, &state_body("REQUESTED"), "QUEUED, never claimed");
    assert!(
        took >= SHORT_WINDOW && took < SHORT_WINDOW + SLACK,
        "QUEUED: answered after {took:?}"
    );

    let statuses: Vec<String> = room
        .jobs()
        .await
        .into_iter()
        .chain(short_room.jobs().await)
        .map(|j| j.2)
        .collect();
    assert_eq!(
        statuses,
        [
            "DRAWER_OPENED",
            "FAILED",
            "EXPIRED",
            "COMMAND_SIMULATED",
            "SIMULATED_PRINTED",
            "DRAWER_OPENED",
            "LEASED",
            "QUEUED",
        ],
        "the device never changes a job's status"
    );
    room.finish().await;
    short_room.finish().await;
}

// ── rows 10 and 11 ───────────────────────────────────────────────────────────

/// Row 10: an order somebody else asked for is refused, whoever asks; nothing more is queued.
#[actix_web::test]
async fn row_10_an_order_already_asked_by_another_employee_is_refused() {
    let room = Room::open(switched_on(&["MAIN"])).await;
    room.agent("MAIN", AGENT_MAIN).await;

    let id = passport_id("MAIN", "DESK", &order(1));
    let (first, _) = room
        .ask_with(
            "OWNER",
            ask_body("MAIN", &order(1), false, "LIVE"),
            &id,
            Agent::Reports("DRAWER_OPENED"),
        )
        .await;
    assert_answer(&first, &state_body("SUBMITTED_TO_CUPS"), "the owner asks");
    let lines = room.line_count().await;

    for who in ["FRONT", "MANAGER", "BACKEND"] {
        let answer = room
            .ask(who, ask_body("MAIN", &order(1), false, "LIVE"))
            .await;
        answer.assert_refused(400, OTHER_EMPLOYEE, who);
    }
    assert_eq!(room.jobs().await.len(), 1, "still one job");
    assert_eq!(room.line_count().await, lines, "a refusal logs nothing");
    room.finish().await;
}

/// Row 11: the caller asking again gets the job's state now, at once, with `duplicate: true`;
/// KIOSK and DESK of the same id are two orders.
#[actix_web::test]
async fn row_11_the_same_caller_asking_again_gets_the_jobs_state_with_duplicate() {
    let room = Room::open(switched_on(&["MAIN"])).await;
    room.agent("MAIN", AGENT_MAIN).await;
    let body = ask_body("MAIN", &order(1), false, "LIVE");
    let id = passport_id("MAIN", "DESK", &order(1));

    let first = room.ask("FRONT", body.clone()).await;
    assert_answer(&first, &state_body("REQUESTED"), "nobody claimed it");
    let lines = room.line_count().await;

    let started = Instant::now();
    let again = room.ask("FRONT", body.clone()).await;
    let took = started.elapsed();
    assert_answer(
        &again,
        &duplicate_body("REQUESTED"),
        "asked again, still QUEUED",
    );
    assert!(took < Duration::from_secs(2), "a repeat waited {took:?}");

    assert!(
        play(room.pool(), id.clone(), Agent::Reports("DRAWER_OPENED")).await,
        "the job is still queued"
    );
    let opened = room.ask("FRONT", body.clone()).await;
    assert_answer(
        &opened,
        &duplicate_body("SUBMITTED_TO_CUPS"),
        "after the agent",
    );

    sqlx::query("UPDATE device_jobs SET status = 'FAILED' WHERE request_id = $1")
        .bind(&id)
        .execute(&room.desk.db.pool)
        .await
        .expect("fail the job");
    let failed = room.ask("FRONT", body).await;
    assert_answer(&failed, &duplicate_body("UNCERTAIN"), "after a failure");

    assert_eq!(room.jobs().await.len(), 1, "a repeat queues nothing");
    assert_eq!(room.line_count().await, lines, "a repeat logs nothing");

    let front = room.id_of("FRONT");
    room.kiosk_request("MAIN", &order(1), "ACCEPTED", Some(&front))
        .await;
    let kiosk_id = passport_id("MAIN", "KIOSK", &order(1));
    let (kiosk, _) = room
        .ask_with(
            "FRONT",
            ask_body("MAIN", &order(1), true, "LIVE"),
            &kiosk_id,
            Agent::Reports("DRAWER_OPENED"),
        )
        .await;
    assert_answer(
        &kiosk,
        &state_body("SUBMITTED_TO_CUPS"),
        "the same id as a kiosk order is a first ask",
    );
    let ids: Vec<String> = room.jobs().await.into_iter().map(|j| j.0).collect();
    assert_eq!(ids, vec![id, kiosk_id]);
    room.finish().await;
}

/// Two asks for one order at once by the same caller: one job, one first answer, one repeat.
#[actix_web::test]
async fn two_asks_at_once_by_the_same_caller_queue_one_job() {
    let room = Room::open(switched_on(&["MAIN"])).await;
    room.agent("MAIN", AGENT_MAIN).await;
    let id = passport_id("MAIN", "DESK", &order(1));
    let played = tokio::spawn(play(
        room.pool(),
        id.clone(),
        Agent::Reports("DRAWER_OPENED"),
    ));

    let body = ask_body("MAIN", &order(1), false, "LIVE");
    let (a, b) = tokio::join!(room.ask("MANAGER", body.clone()), room.ask("MANAGER", body));
    assert!(played.await.expect("the test's agent ran"));
    let mut firsts = 0;
    let mut repeats = 0;
    for answer in [&a, &b] {
        assert_eq!(answer.status, 200, "{:.300}", answer.text);
        let json = answer.json();
        match json.get("duplicate") {
            None => firsts += 1,
            Some(Value::Bool(true)) => repeats += 1,
            Some(other) => panic!("duplicate: {other}"),
        }
        assert!(
            ["REQUESTED", "SUBMITTED_TO_CUPS"].contains(&json["state"].as_str().unwrap_or("")),
            "{json}"
        );
    }
    assert_eq!((firsts, repeats), (1, 1), "{} / {}", a.text, b.text);
    assert_eq!(room.jobs().await.len(), 1, "one job");
    room.finish().await;
}

// ── the outcomes that queue nothing ──────────────────────────────────────────

/// A switched-on branch with no active agent (none paired, or revoked): `SIMULATED`, nothing
/// queued, nothing logged — the device never sends by itself.
#[actix_web::test]
async fn no_active_agent_answers_simulated_and_queues_nothing() {
    let room = Room::open(switched_on(&["MAIN"])).await;

    let none = room
        .ask("OWNER", ask_body("MAIN", &order(1), false, "LIVE"))
        .await;
    assert_answer(&none, &state_body("SIMULATED"), "no agent");

    room.agent("MAIN", AGENT_MAIN).await;
    room.revoke(AGENT_MAIN).await;
    let revoked = room
        .ask("OWNER", ask_body("MAIN", &order(2), false, "LIVE"))
        .await;
    assert_answer(&revoked, &state_body("SIMULATED"), "a revoked agent");

    assert!(room.jobs().await.is_empty(), "nothing queued");
    assert_eq!(room.line_count().await, 0, "nothing logged");
    room.finish().await;
}

/// A branch whose drawer is off (`hasDrawer = 0`) with an agent: `UNCERTAIN`, nothing
/// queued, AG-3's one `DRAWER_SKIPPED` line.
#[actix_web::test]
async fn a_branch_without_a_drawer_answers_uncertain_and_queues_nothing() {
    let room = Room::open(switched_on(&["MAIN"])).await;
    room.agent("MAIN", AGENT_MAIN).await;
    room.no_drawer("MAIN").await;

    let answer = room
        .ask("OWNER", ask_body("MAIN", &order(1), false, "LIVE"))
        .await;
    assert_answer(&answer, &state_body("UNCERTAIN"), "hasDrawer = 0");
    assert!(room.jobs().await.is_empty(), "nothing queued");
    assert_eq!(
        room.lines("DRAWER_SKIPPED").await.len(),
        1,
        "one skipped line"
    );
    room.finish().await;
}

/// The device reads the branch's printer model in the configured registry
/// (`devices.printerModels`), as the agent's claim does (§5.4c *The bytes*): a model only the
/// configuration knows still has a drawer.
#[actix_web::test]
async fn the_branchs_printer_model_comes_from_the_configuration() {
    let settings = load_settings(Some(
        "passportDrawerBranches: [MAIN]\n  printerModels:\n    \
         - { id: NEOCAL_NTP_8220BT, name: NEOCAL, paperMm: 80, drawerPulse: { pin: 0, on: 25, off: 250 } }\n    \
         - { id: SHOP_POS80, name: POS80, paperMm: 80, drawerPulse: { pin: 0, on: 25, off: 125 } }",
    ))
    .expect("the configuration loads with a second printer model");
    let room = Room::open(settings).await;
    room.agent("MAIN", AGENT_MAIN).await;
    room.printer_model("MAIN", "SHOP_POS80").await;

    let id = passport_id("MAIN", "DESK", &order(1));
    let (answer, _) = room
        .ask_with(
            "OWNER",
            ask_body("MAIN", &order(1), false, "LIVE"),
            &id,
            Agent::Reports("DRAWER_OPENED"),
        )
        .await;
    assert_answer(
        &answer,
        &state_body("SUBMITTED_TO_CUPS"),
        "a configured model with a pulse",
    );
    assert!(room.lines("DRAWER_SKIPPED").await.is_empty(), "not skipped");
    room.finish().await;
}
