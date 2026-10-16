//! The passport drawer's read window is configuration, not a constant (design §5.5.8: "It reads
//! the job every 250 ms for at most 5 s [proposed]"). Written before the implementation. Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Why: `DevicePassportDrawer::request` waited a hard-coded `Duration::from_millis(5000)` for a
//! job nobody claims, so `passport_drawer_device_test.rs`'s QUEUED row could only check the
//! answer against wall time over 5 s, and a loaded CI runner turned it red (PR 2049, run
//! 38797160366: "QUEUED: answered after 7.200270119s"). With the window in `Settings` a test
//! sets it short and checks the device really waited the window it was given.
//!
//! [required] `devices.passportDrawerReadWindowMs` in the configuration, field
//! `Settings::devices.passport_drawer_read_window_ms: u64`, 5000 when absent.
//!
//! Everything goes through the server's whole route table, as `passport_drawer_device_test.rs`
//! does; the harness below is a copy of its `Room` (that file is not edited).
//!
//! Mutations that must turn this red:
//! - the window still hard-coded at 5000 ms                     -> a_job_nobody_claims_…, a_job_leased_…
//! - no wait at all (answering at the first read)                -> a_job_nobody_claims_…, a_job_leased_…
//! - the device waiting out the window after the agent reported  -> a_report_inside_the_window_…
//! - the reading stopped before the report                       -> a_report_inside_the_window_…
//! - a default other than 5000 ms, or no default                 -> the_window_is_5000_ms_…
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod common;
mod compat_common;
mod rpc_common;
mod snapshot_common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rpc_common::{Answer, App, Call, Cookies, Desk, one_of_each_role, sign_in};
use serde_json::{Value, json};
use sqlx::PgPool;
use xchange::bootstrap::AppState;
use xchange::config::{DevicesSettings, Settings};

const ORIGIN: &str = "http://desk.test";
const ROUTE: &str = "/api/drawer-passport";
const AGENT_MAIN: &str = "a0000000-0000-4000-8000-0000000000a1";

/// The short window the "never reported" rows run with.
const SHORT_WINDOW_MS: u64 = 400;
/// Room for a loaded test machine on top of the window. A device still waiting the old fixed
/// 5 s cannot answer before 5 s, so window + slack must stay under 5 s for that to be caught:
/// 400 ms + 4.4 s = 4.8 s. A loaded machine was measured adding ~2.2 s to one ask (PR 2049).
const SLACK: Duration = Duration::from_millis(4_400);
/// The window of the "reported inside the window" row: long enough that the agent's report
/// (600 ms after its claim) lands well inside it on a loaded machine, where an ask that should
/// take ~0.9 s was measured at 3.1 s.
const LONG_WINDOW_MS: u64 = 8_000;

fn order(n: u32) -> String {
    format!("0b000000-0000-4000-8000-{n:012x}")
}

fn passport_id(branch: &str, source: &str, order: &str) -> String {
    format!("auto:{branch}:passport:{source}:{order}")
}

fn scratch_dir() -> PathBuf {
    std::env::temp_dir().join(format!("xc-pdrw-{}", uuid::Uuid::new_v4()))
}

/// The committed `base.yaml`, plus a `development.yaml` whose `devices:` holds `lines` (each
/// one `key: value`).
fn load_settings(lines: &[&str]) -> Result<Settings, config::ConfigError> {
    let dir = scratch_dir();
    std::fs::create_dir_all(&dir).expect("create a configuration directory");
    std::fs::copy(
        common::config_dir().join("base.yaml"),
        dir.join("base.yaml"),
    )
    .expect("copy base.yaml");
    let development = if lines.is_empty() {
        "{}\n".to_string()
    } else {
        let mut text = "devices:\n".to_string();
        for line in lines {
            text.push_str("  ");
            text.push_str(line);
            text.push('\n');
        }
        text
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

/// MAIN switched on, reading for `window_ms`.
fn main_with_window(window_ms: u64) -> Settings {
    let window = format!("passportDrawerReadWindowMs: {window_ms}");
    load_settings(&["passportDrawerBranches: [MAIN]", &window])
        .expect("the configuration loads with the switch and the window")
}

fn ask_body(order: &str) -> String {
    json!({ "branchId": "MAIN", "orderId": order, "kiosk": false, "mode": "LIVE" }).to_string()
}

fn state_body(state: &str) -> Value {
    json!({ "state": state, "physicalResultConfirmed": false })
}

fn assert_answer(answer: &Answer, body: &Value, label: &str) {
    assert_eq!(answer.status, 200, "{label}: {:.300}", answer.text);
    assert_eq!(&answer.json(), body, "{label}");
}

/// What the test's agent does with the passport job once it is queued.
#[derive(Clone, Copy)]
enum Agent {
    /// Claims it and never reports.
    Leases,
    /// Claims it, and reports `outcome` 600 ms later.
    LeasesThenReports(&'static str),
}

struct Room {
    desk: Desk,
    app: App,
    cookies: Cookies,
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
        let room = Room { desk, app, cookies };
        sqlx::query(
            "INSERT INTO device_agents (id, branch_id, name, created_at, created_by) \
             VALUES ($1::uuid, 'MAIN', 'Mac เคาน์เตอร์ 1', now(), 'owner')",
        )
        .bind(AGENT_MAIN)
        .execute(&room.desk.db.pool)
        .await
        .expect("insert MAIN's agent");
        room
    }

    fn pool(&self) -> PgPool {
        self.desk.db.pool.clone()
    }

    /// Asks as the backend employee; gives the answer and how long it took.
    async fn ask(&self, body: String) -> (Answer, Duration) {
        self.desk.clock.advance(chrono::Duration::milliseconds(1));
        let call = Call::post(ORIGIN, ROUTE, body).cookie(&self.cookies["BACKEND"]);
        let started = Instant::now();
        let answer = self.app.send(call).await;
        (answer, started.elapsed())
    }

    /// Asks while the test's agent does `agent` to the job `request_id`.
    async fn ask_with(&self, body: String, request_id: &str, agent: Agent) -> (Answer, Duration) {
        let played = tokio::spawn(play(self.pool(), request_id.to_string(), agent));
        let (answer, took) = self.ask(body).await;
        assert!(
            played.await.expect("the test's agent ran"),
            "{request_id}: the test's agent found no queued job: {:.300}",
            answer.text
        );
        (answer, took)
    }

    async fn statuses(&self) -> Vec<String> {
        sqlx::query_scalar("SELECT status FROM device_jobs ORDER BY created_at, id")
            .fetch_all(&self.desk.db.pool)
            .await
            .expect("read device_jobs")
    }

    async fn finish(self) {
        self.desk.finish().await;
    }
}

/// The test's agent: waits up to 5 s for the job `request_id` to be queued (committed), then
/// moves it as `agent` says. True when it found the job.
async fn play(pool: PgPool, request_id: String, agent: Agent) -> bool {
    for _ in 0..100 {
        let moved = sqlx::query(
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
        .expect("lease the job");
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

fn window_bounds(took: Duration, window_ms: u64, label: &str) {
    let window = Duration::from_millis(window_ms);
    assert!(
        took >= window,
        "{label}: answered after {took:?}, before the {window:?} window ended"
    );
    assert!(
        took < window + SLACK,
        "{label}: answered after {took:?}; the window is {window:?} (plus {SLACK:?} of room)"
    );
}

/// A job nobody claims stays QUEUED: the answer is `REQUESTED`, given once the configured
/// window has passed, not the old fixed 5 s.
#[actix_web::test]
async fn a_job_nobody_claims_is_answered_requested_after_the_configured_window() {
    let room = Room::open(main_with_window(SHORT_WINDOW_MS)).await;

    let (answer, took) = room.ask(ask_body(&order(1))).await;
    assert_answer(&answer, &state_body("REQUESTED"), "QUEUED, never claimed");
    window_bounds(took, SHORT_WINDOW_MS, "QUEUED");

    assert_eq!(room.statuses().await, ["QUEUED"], "one job, left QUEUED");
    room.finish().await;
}

/// A job the agent claims and never reports stays LEASED: the same window, the same answer.
#[actix_web::test]
async fn a_job_leased_and_never_reported_is_answered_requested_after_the_configured_window() {
    let room = Room::open(main_with_window(SHORT_WINDOW_MS)).await;

    let id = passport_id("MAIN", "DESK", &order(2));
    let (answer, took) = room.ask_with(ask_body(&order(2)), &id, Agent::Leases).await;
    assert_answer(&answer, &state_body("REQUESTED"), "LEASED, never reported");
    window_bounds(took, SHORT_WINDOW_MS, "LEASED");

    assert_eq!(room.statuses().await, ["LEASED"], "one job, left LEASED");
    room.finish().await;
}

/// The window is an upper limit: an agent reporting inside it is answered as soon as the next
/// read sees the report, not when the window ends.
#[actix_web::test]
async fn a_report_inside_the_window_is_answered_before_the_window_ends() {
    let room = Room::open(main_with_window(LONG_WINDOW_MS)).await;

    let id = passport_id("MAIN", "DESK", &order(3));
    let (answer, took) = room
        .ask_with(
            ask_body(&order(3)),
            &id,
            Agent::LeasesThenReports("DRAWER_OPENED"),
        )
        .await;
    assert_answer(
        &answer,
        &state_body("SUBMITTED_TO_CUPS"),
        "LEASED, then reported 600 ms later",
    );
    let window = Duration::from_millis(LONG_WINDOW_MS);
    assert!(
        took < window,
        "reported inside the {window:?} window, but answered after {took:?}"
    );

    assert_eq!(room.statuses().await, ["DRAWER_OPENED"]);
    room.finish().await;
}

/// Absent from the configuration, the window is 5000 ms (§5.5.8 "at most 5 s"); the committed
/// `base.yaml` gives the same, and a deployment may set its own.
#[actix_web::test]
async fn the_window_is_5000_ms_unless_the_configuration_sets_it() {
    assert_eq!(
        DevicesSettings::default().passport_drawer_read_window_ms,
        5_000,
        "DevicesSettings::default()"
    );

    let without_key: DevicesSettings = serde_json::from_value(json!({
        "printerModels": [{
            "id": "NEOCAL_NTP_8220BT",
            "name": "NEOCAL NTP-8220BT",
            "paperMm": 80,
            "drawerPulse": { "pin": 0, "on": 25, "off": 250 }
        }]
    }))
    .expect("a devices section without the window deserializes");
    assert_eq!(
        without_key.passport_drawer_read_window_ms, 5_000,
        "a devices section that does not name the window"
    );

    let committed = load_settings(&[]).expect("the committed configuration loads");
    assert_eq!(
        committed.devices.passport_drawer_read_window_ms, 5_000,
        "the committed configuration"
    );

    let set = load_settings(&["passportDrawerReadWindowMs: 400"]).expect("a window of 400 ms");
    assert_eq!(set.devices.passport_drawer_read_window_ms, 400);
}
