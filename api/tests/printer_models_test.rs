//! Specification tests for DRV-4 (design `docs/design/xchange-v3.md` §5.4b
//! *Printer models and the drawer switch*, issue #274): printer models from
//! configuration, the per-branch drawer switch, `DeviceDriver::run_with`,
//! `EscposDriver::with_models`, and `queue` refusing a drawer request where
//! the branch has said it has no drawer.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! Two kinds of test:
//! - configuration, the registry and the drivers alone, with no database: a
//!   transport that keeps every call stands in for the printer;
//! - the drawer settings and the service, on a real PostgreSQL
//!   (`TEST_DATABASE_URL`, as the other device tests). Without it those FAIL;
//!   none skips.

mod common;
mod devices_common;
mod routes_common;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use devices_common::*;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xchange::config::Settings;
use xchange::infrastructure::devices::device_service::BAD_PRINTER;
use xchange::infrastructure::devices::escpos_driver::{EscposDriver, PrinterTransport};
use xchange::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use xchange::infrastructure::devices::printer_models::{
    DrawerPulse, DrawerSettings, PrinterModel, PrinterModels,
};
use xchange::infrastructure::devices::printer_settings::{PrinterKind, PrinterSettings};
use xchange::infrastructure::devices::simulated_driver::{DeviceDriver, SimulatedDriver};

/// The first model of the committed configuration (owner 2026-10-07, #274).
const NEOCAL: &str = "NEOCAL_NTP_8220BT";

/// `ESC @` then `ESC p 0 25 250`: what DRV-1 sends today, and the NEOCAL
/// entry's pulse (pos108's kick, `pos-printer/src/escpos.rs:77-78` @ `8b53cee`).
const NEOCAL_KICK: [u8; 7] = [0x1B, 0x40, 0x1B, 0x70, 0x00, 0x19, 0xFA];

/// `ESC @` then `ESC p 1 40 120`: a model that pulses the other pin.
const OTHER_KICK: [u8; 7] = [0x1B, 0x40, 0x1B, 0x70, 0x01, 0x28, 0x78];

/// `queue`'s answer to a drawer request where the branch has no drawer.
const NO_DRAWER: &str = "สาขานี้ไม่มีลิ้นชัก";

/// The two settings keys of the drawer switch (design §5.4b).
const PRINTER_MODEL_KEY: &str = "printerModel";
const HAS_DRAWER_KEY: &str = "hasDrawer";

fn neocal() -> PrinterModel {
    PrinterModel {
        id: NEOCAL.to_owned(),
        name: "NEOCAL NTP-8220BT".to_owned(),
        paper_mm: 80,
        drawer_pulse: Some(DrawerPulse {
            pin: 0,
            on: 25,
            off: 250,
        }),
    }
}

fn model(id: &str, pulse: Option<(u8, u8, u8)>) -> PrinterModel {
    PrinterModel {
        id: id.to_owned(),
        name: format!("{id} name"),
        paper_mm: 58,
        drawer_pulse: pulse.map(|(pin, on, off)| DrawerPulse { pin, on, off }),
    }
}

/// A registry the tests build: `PIN5` pulses pin 1 for 40/120, `NO_PULSE`
/// has no drawer port.
fn test_models() -> Arc<PrinterModels> {
    Arc::new(
        PrinterModels::new(vec![
            model("PIN5", Some((1, 40, 120))),
            model("NO_PULSE", None),
            neocal(),
        ])
        .expect("a registry the tests build"),
    )
}

fn drawer(model: &str, has_drawer: Option<bool>) -> DrawerSettings {
    DrawerSettings {
        model: model.to_owned(),
        has_drawer,
    }
}

fn printer(online: bool, kind: PrinterKind) -> PrinterSettings {
    PrinterSettings { online, kind }
}

fn network(host: &str, port: u16) -> PrinterKind {
    PrinterKind::Network(host.to_owned(), port)
}

fn error_text<T, E: std::fmt::Display>(answer: Result<T, E>, what: &str) -> String {
    match answer {
        Ok(_) => panic!("{what}: expected the registry to refuse, it accepted"),
        Err(e) => e.to_string(),
    }
}

// ───────────────────────────── configuration ────────────────────────────────

#[test]
fn the_committed_configuration_seeds_the_neocal_ntp_8220bt_first() {
    let s = common::settings(&[]);

    assert!(
        !s.devices.printer_models.is_empty(),
        "base.yaml names at least one printer model"
    );
    assert_eq!(
        s.devices.printer_models[0],
        neocal(),
        "the first entry is the NEOCAL NTP-8220BT, 80 mm, pulse pin 0 on 25 off 250"
    );
    let registry = PrinterModels::new(s.devices.printer_models.clone())
        .expect("the committed models make a registry");
    assert_eq!(registry.default_id(), NEOCAL);
}

#[test]
fn the_built_in_registry_is_the_neocal_entry_of_the_configuration() {
    let configured = common::settings(&[]).devices.printer_models[0].clone();
    let built_in = PrinterModels::built_in();

    assert_eq!(built_in.default_id(), NEOCAL);
    assert_eq!(built_in.get(NEOCAL), Some(&configured));
}

/// A scratch configuration directory: the committed `base.yaml` with its
/// `devices:` section replaced by `devices`.
fn configuration_with(devices: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let base = std::fs::read_to_string(common::config_dir().join("base.yaml")).expect("base.yaml");
    let mut kept = Vec::new();
    let mut skipping = false;
    for line in base.lines() {
        if line.starts_with("devices:") {
            skipping = true;
            continue;
        }
        if skipping && (line.is_empty() || line.starts_with(' ') || line.starts_with('#')) {
            continue;
        }
        skipping = false;
        kept.push(line);
    }
    let mut text = kept.join("\n");
    assert!(!text.contains("printerModels"), "{text}");
    text.push_str("\n\n");
    text.push_str(devices);
    text.push('\n');
    let dir = std::env::temp_dir().join(format!(
        "xchange-printer-models-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("a scratch configuration directory");
    std::fs::write(dir.join("base.yaml"), text).expect("write base.yaml");
    dir
}

fn load(devices: &str) -> Result<Settings, String> {
    let dir = configuration_with(devices);
    let loaded = Settings::load_from(&dir, common::env(&[]));
    std::fs::remove_dir_all(&dir).ok();
    loaded.map_err(|e| e.to_string())
}

#[test]
fn a_configuration_whose_models_the_registry_refuses_does_not_load() {
    // The harness itself: a good list loads, a model without a pulse included.
    let good = "devices:
  printerModels:
    - id: GOOD
      name: Good
      paperMm: 80
      drawerPulse: { pin: 1, on: 40, off: 120 }
    - id: BARE
      name: Bare
      paperMm: 58
      drawerPulse: null";
    let loaded = load(good).expect("a configuration with good models loads");
    assert_eq!(
        loaded.devices.printer_models,
        vec![
            model_named("GOOD", "Good", 80, Some((1, 40, 120))),
            model_named("BARE", "Bare", 58, None)
        ]
    );

    let pulse = |pin: &str, on: &str, off: &str| {
        format!(
            "devices:
  printerModels:
    - id: BAD
      name: Bad
      paperMm: 80
      drawerPulse: {{ pin: {pin}, on: {on}, off: {off} }}"
        )
    };
    let duplicate = "devices:
  printerModels:
    - id: TWICE
      name: One
      paperMm: 80
      drawerPulse: null
    - id: TWICE
      name: Two
      paperMm: 58
      drawerPulse: null"
        .to_owned();
    let refused = [
        ("a duplicate id", duplicate),
        ("pin 2", pulse("2", "25", "250")),
        ("on 0", pulse("0", "0", "250")),
        ("off 0", pulse("0", "25", "0")),
        ("on 256", pulse("0", "256", "250")),
        ("off 256", pulse("0", "25", "256")),
    ];
    for (what, devices) in refused {
        assert!(
            load(&devices).is_err(),
            "{what}: the server must not start with this configuration"
        );
    }
}

fn model_named(id: &str, name: &str, paper_mm: u16, pulse: Option<(u8, u8, u8)>) -> PrinterModel {
    PrinterModel {
        name: name.to_owned(),
        paper_mm,
        ..model(id, pulse)
    }
}

// ───────────────────────────── the registry ─────────────────────────────────

#[test]
fn the_default_is_the_first_model_and_every_model_is_found_by_its_id() {
    let models = test_models();

    assert_eq!(models.default_id(), "PIN5");
    assert_eq!(models.get("PIN5"), Some(&model("PIN5", Some((1, 40, 120)))));
    assert_eq!(models.get(NEOCAL), Some(&neocal()));
    assert_eq!(models.get("EPSON_TM_T82"), None);
}

#[test]
fn a_model_without_a_pulse_is_accepted() {
    let models = PrinterModels::new(vec![model("NO_PULSE", None)]).expect("accepted");

    assert_eq!(models.get("NO_PULSE"), Some(&model("NO_PULSE", None)));
    assert_eq!(models.default_id(), "NO_PULSE");
}

#[test]
fn the_registry_refuses_a_duplicate_id_naming_it() {
    let answer = PrinterModels::new(vec![
        model("TWICE", None),
        neocal(),
        model("TWICE", Some((0, 25, 250))),
    ]);

    let text = error_text(answer, "a duplicate id");
    assert!(text.contains("TWICE"), "the refusal names the id: {text}");
}

#[test]
fn the_registry_refuses_a_pulse_outside_pin_0_to_1_and_on_off_1_to_255() {
    for (what, pulse) in [
        ("pin 2", (2, 25, 250)),
        ("pin 255", (255, 25, 250)),
        ("on 0", (0, 0, 250)),
        ("off 0", (0, 25, 0)),
        ("on 0 and off 0", (1, 0, 0)),
    ] {
        let answer = PrinterModels::new(vec![neocal(), model("BAD", Some(pulse))]);
        let text = error_text(answer, what);
        assert!(
            text.contains("BAD"),
            "{what}: the refusal names the id: {text}"
        );
    }
}

#[test]
fn the_registry_accepts_the_edges_of_the_pulse_range() {
    let models = PrinterModels::new(vec![
        model("LOW", Some((0, 1, 1))),
        model("HIGH", Some((1, 255, 255))),
    ])
    .expect("pin 0..=1 and on/off 1..=255 are accepted");

    assert_eq!(
        models.get("HIGH"),
        Some(&model("HIGH", Some((1, 255, 255))))
    );
}

#[test]
fn the_registry_refuses_an_empty_list() {
    // With no model there is no first one to default to.
    error_text(PrinterModels::new(Vec::new()), "an empty list");
}

#[test]
fn the_kick_is_esc_at_then_esc_p_pin_on_off() {
    let pulse = DrawerPulse {
        pin: 1,
        on: 40,
        off: 120,
    };
    assert_eq!(pulse.kick_bytes(), OTHER_KICK.to_vec());

    let pulse = neocal().drawer_pulse.expect("the NEOCAL entry has a pulse");
    assert_eq!(pulse.kick_bytes(), NEOCAL_KICK.to_vec());
}

// ───────────────────────────── the drivers ──────────────────────────────────

#[derive(Default)]
struct RecordingTransport {
    calls: Mutex<Vec<(String, u16, Vec<u8>)>>,
}

impl RecordingTransport {
    fn calls(&self) -> Vec<(String, u16, Vec<u8>)> {
        self.calls.lock().expect("calls").clone()
    }
}

#[async_trait]
impl PrinterTransport for RecordingTransport {
    async fn send(&self, host: &str, port: u16, bytes: &[u8]) -> anyhow::Result<()> {
        self.calls
            .lock()
            .expect("calls")
            .push((host.to_owned(), port, bytes.to_vec()));
        Ok(())
    }
}

fn on_the_network() -> PrinterSettings {
    printer(true, network("printer.branch.local", 9101))
}

#[tokio::test]
async fn with_models_sends_the_kick_of_the_branch_model() {
    for has_drawer in [Some(true), None] {
        let transport = Arc::new(RecordingTransport::default());
        let driver = EscposDriver::with_models(transport.clone(), test_models());

        let status = driver
            .run_with(
                JobKind::DrawerRequest,
                &on_the_network(),
                &drawer("PIN5", has_drawer),
            )
            .await;

        assert_eq!(status, JobStatus::DrawerOpened, "has_drawer={has_drawer:?}");
        assert_eq!(
            transport.calls(),
            vec![("printer.branch.local".to_owned(), 9101, OTHER_KICK.to_vec())],
            "has_drawer={has_drawer:?}: one send of ESC @ ESC p 1 40 120"
        );
    }
}

#[tokio::test]
async fn a_branch_that_said_it_has_no_drawer_fails_without_sending() {
    let transport = Arc::new(RecordingTransport::default());
    let driver = EscposDriver::with_models(transport.clone(), test_models());

    let status = driver
        .run_with(
            JobKind::DrawerRequest,
            &on_the_network(),
            &drawer("PIN5", Some(false)),
        )
        .await;

    assert_eq!(status, JobStatus::Failed);
    assert!(transport.calls().is_empty(), "no byte reaches the printer");
}

#[tokio::test]
async fn a_model_without_a_pulse_fails_without_sending() {
    let transport = Arc::new(RecordingTransport::default());
    let driver = EscposDriver::with_models(transport.clone(), test_models());

    for has_drawer in [Some(true), None] {
        let status = driver
            .run_with(
                JobKind::DrawerRequest,
                &on_the_network(),
                &drawer("NO_PULSE", has_drawer),
            )
            .await;
        assert_eq!(status, JobStatus::Failed, "has_drawer={has_drawer:?}");
    }
    assert!(transport.calls().is_empty(), "no byte reaches the printer");
}

#[tokio::test]
async fn a_model_the_registry_does_not_know_fails_without_sending() {
    // A stored model that a later configuration no longer lists.
    let transport = Arc::new(RecordingTransport::default());
    let driver = EscposDriver::with_models(transport.clone(), test_models());

    let status = driver
        .run_with(
            JobKind::DrawerRequest,
            &on_the_network(),
            &drawer("REMOVED_MODEL", Some(true)),
        )
        .await;

    assert_eq!(status, JobStatus::Failed);
    assert!(transport.calls().is_empty(), "no byte reaches the printer");
}

#[tokio::test]
async fn new_still_sends_the_neocal_kick() {
    let transport = Arc::new(RecordingTransport::default());
    let driver = EscposDriver::new(transport.clone());

    let through_run = driver.run(JobKind::DrawerRequest, &on_the_network()).await;
    let through_run_with = driver
        .run_with(
            JobKind::DrawerRequest,
            &on_the_network(),
            &drawer(NEOCAL, None),
        )
        .await;

    assert_eq!(through_run, JobStatus::DrawerOpened);
    assert_eq!(through_run_with, JobStatus::DrawerOpened);
    assert_eq!(
        transport.calls(),
        vec![
            (
                "printer.branch.local".to_owned(),
                9101,
                NEOCAL_KICK.to_vec()
            ),
            (
                "printer.branch.local".to_owned(),
                9101,
                NEOCAL_KICK.to_vec()
            ),
        ],
        "both send exactly ESC @ ESC p 0 25 250"
    );
}

#[tokio::test]
async fn the_drawer_switch_does_not_touch_receipts_or_the_printer_switch() {
    let transport = Arc::new(RecordingTransport::default());
    let driver = EscposDriver::with_models(transport.clone(), test_models());

    for has_drawer in [Some(false), Some(true), None] {
        let settings = drawer("PIN5", has_drawer);
        assert_eq!(
            driver
                .run_with(JobKind::PrintReceipt, &on_the_network(), &settings)
                .await,
            JobStatus::SimulatedPrinted,
            "a receipt, has_drawer={has_drawer:?}"
        );
        assert_eq!(
            driver
                .run_with(
                    JobKind::DrawerRequest,
                    &printer(false, network("printer.branch.local", 9101)),
                    &settings
                )
                .await,
            JobStatus::Failed,
            "the printer switched off, has_drawer={has_drawer:?}"
        );
    }
    assert!(transport.calls().is_empty(), "no byte reaches the printer");
}

/// A driver that implements only `run` and keeps what it was handed.
#[derive(Default)]
struct RunOnlyDriver {
    seen: Mutex<Vec<(JobKind, PrinterSettings)>>,
}

#[async_trait]
impl DeviceDriver for RunOnlyDriver {
    fn mode(&self) -> &'static str {
        "SIMULATION"
    }

    async fn run(&self, kind: JobKind, printer: &PrinterSettings) -> JobStatus {
        self.seen
            .lock()
            .expect("seen")
            .push((kind, printer.clone()));
        JobStatus::SimulatedPrinted
    }
}

#[tokio::test]
async fn the_default_run_with_calls_run() {
    let driver = RunOnlyDriver::default();
    let settings = on_the_network();

    let status = driver
        .run_with(
            JobKind::DrawerRequest,
            &settings,
            &drawer(NEOCAL, Some(false)),
        )
        .await;

    assert_eq!(status, JobStatus::SimulatedPrinted, "run's answer");
    assert_eq!(
        *driver.seen.lock().expect("seen"),
        vec![(JobKind::DrawerRequest, settings)],
        "run is called once, with the kind and the printer"
    );
}

#[tokio::test]
async fn the_simulated_driver_answers_run_with_as_run() {
    for kind in [JobKind::PrintReceipt, JobKind::DrawerRequest] {
        for online in [true, false] {
            for has_drawer in [Some(false), Some(true), None] {
                let settings = printer(online, PrinterKind::Simulated);
                assert_eq!(
                    SimulatedDriver
                        .run_with(kind, &settings, &drawer(NEOCAL, has_drawer))
                        .await,
                    SimulatedDriver.run(kind, &settings).await,
                    "{kind:?} online={online} has_drawer={has_drawer:?}"
                );
            }
        }
    }
}

// ───────────────────────────── the branch's drawer settings ─────────────────

/// Store a setting of a branch behind the service's back.
async fn put(db: &Devices, branch_id: &str, key: &str, value: &str) {
    sqlx::query(
        "INSERT INTO device_settings (branch_id, key, value) VALUES ($1, $2, $3) \
         ON CONFLICT (branch_id, key) DO UPDATE SET value = excluded.value",
    )
    .bind(branch_id)
    .bind(key)
    .bind(value)
    .execute(&db.pool)
    .await
    .expect("store a setting");
}

/// The branch's drawer settings as a transaction of its own reads them.
async fn read(db: &Devices, branch_id: &str, models: &PrinterModels) -> DrawerSettings {
    let repo = PgDeviceRepository::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let settings = DrawerSettings::read_in(&mut tx, &repo, &branch(branch_id), models)
        .await
        .expect("read the drawer settings");
    tx.rollback().await.expect("rollback");
    settings
}

fn owner() -> Actor {
    Actor::owner("owner")
}

fn staff(role: Role) -> Actor {
    Actor::staff("be1", role, branch(MAIN)).expect("a staff actor")
}

fn key(branch_id: &str, key: &str) -> (String, String) {
    (branch_id.to_owned(), key.to_owned())
}

#[tokio::test]
async fn a_branch_with_no_drawer_settings_has_the_first_model_and_has_said_nothing() {
    let db = Devices::start().await;

    assert_eq!(
        read(&db, MAIN, &PrinterModels::built_in()).await,
        drawer(NEOCAL, None)
    );
    assert_eq!(
        read(&db, MAIN, &test_models()).await,
        drawer("PIN5", None),
        "the default is the first model of the registry it is handed"
    );

    db.finish().await;
}

#[tokio::test]
async fn what_the_owner_sets_is_what_the_branch_reads() {
    let db = Devices::start().await;

    db.service
        .set_drawer(&branch(MAIN), NEOCAL, false, &owner(), at(1))
        .await
        .expect("the owner says MAIN has no drawer");
    assert_eq!(
        read(&db, MAIN, &PrinterModels::built_in()).await,
        drawer(NEOCAL, Some(false))
    );
    let settings = db.settings().await;
    assert_eq!(
        settings
            .get(&key(MAIN, PRINTER_MODEL_KEY))
            .map(String::as_str),
        Some(NEOCAL)
    );
    assert_eq!(
        settings.get(&key(MAIN, HAS_DRAWER_KEY)).map(String::as_str),
        Some("0")
    );

    db.service
        .set_drawer(&branch(MAIN), NEOCAL, true, &owner(), at(2))
        .await
        .expect("the owner says MAIN has a drawer");
    assert_eq!(
        read(&db, MAIN, &PrinterModels::built_in()).await,
        drawer(NEOCAL, Some(true))
    );
    assert_eq!(
        db.settings()
            .await
            .get(&key(MAIN, HAS_DRAWER_KEY))
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        read(&db, B9, &PrinterModels::built_in()).await,
        drawer(NEOCAL, None),
        "another branch is not touched"
    );

    db.finish().await;
}

#[tokio::test]
async fn a_stored_zero_is_no_drawer_and_a_stored_one_is_a_drawer() {
    let db = Devices::start().await;
    put(&db, MAIN, HAS_DRAWER_KEY, "0").await;
    put(&db, B9, HAS_DRAWER_KEY, "1").await;
    put(&db, B9, PRINTER_MODEL_KEY, "PIN5").await;

    assert_eq!(
        read(&db, MAIN, &test_models()).await,
        drawer("PIN5", Some(false))
    );
    assert_eq!(
        read(&db, B9, &PrinterModels::built_in()).await,
        drawer("PIN5", Some(true)),
        "the stored model is read as stored"
    );

    db.finish().await;
}

#[tokio::test]
async fn an_unknown_model_is_refused_and_nothing_is_stored() {
    let db = Devices::start().await;
    let before = db.stored().await;

    for unknown in ["EPSON_TM_T82", "", "neocal_ntp_8220bt"] {
        let answer = db
            .service
            .set_drawer(&branch(MAIN), unknown, true, &owner(), at(1))
            .await;
        assert_eq!(refusal(answer), BAD_PRINTER, "{unknown:?}");
    }
    assert_eq!(BAD_PRINTER, "ตั้งค่าเครื่องพิมพ์ไม่ถูกต้อง");
    assert!(
        db.stored().await.same_as(&before),
        "a refusal stores nothing"
    );

    db.finish().await;
}

#[tokio::test]
async fn only_the_owner_sets_the_drawer() {
    // OR-4b (#767, design §4.2 C5): who may set the drawer is the route table's alone
    // (`Policy::need(Devices, setDrawer)` = SystemSettings: EXECUTIVE and ADMIN), asked by the
    // route (`devices_set_printer_test.rs`); the service no longer judges the caller's role, so
    // each role's call reaches the setting
    let db = Devices::start().await;

    for role in [
        Role::Front,
        Role::Backend,
        Role::Manager,
        Role::Rates,
        Role::Monitor,
        Role::Finance,
        Role::System,
    ] {
        let answer = db
            .service
            .set_drawer(&branch(MAIN), NEOCAL, false, &staff(role), at(1))
            .await;
        assert!(answer.is_ok(), "{role:?}: {answer:?}");
    }

    db.finish().await;
}

#[tokio::test]
async fn set_printer_never_writes_the_drawer_switch() {
    let db = Devices::start().await;

    db.service
        .set_printer(
            &branch(MAIN),
            network("192.168.1.50", 9101),
            &owner(),
            at(1),
        )
        .await
        .expect("set MAIN to a network printer");
    db.service
        .set_printer(&branch(MAIN), PrinterKind::Simulated, &owner(), at(2))
        .await
        .expect("set MAIN back to simulated");

    let settings = db.settings().await;
    assert_eq!(settings.get(&key(MAIN, HAS_DRAWER_KEY)), None);
    assert_eq!(settings.get(&key(MAIN, PRINTER_MODEL_KEY)), None);
    assert_eq!(
        read(&db, MAIN, &PrinterModels::built_in()).await,
        drawer(NEOCAL, None)
    );

    db.finish().await;
}

// ───────────────────────────── queue ────────────────────────────────────────

fn drawer_request(n: u32) -> QueueJob {
    QueueJob {
        kind: "DRAWER_REQUEST".to_owned(),
        reference: String::new(),
        request_id: request_id(n),
        reason: "change".to_owned(),
    }
}

#[tokio::test]
async fn a_drawer_request_is_refused_where_the_branch_has_said_it_has_no_drawer() {
    let db = Devices::start().await;
    db.service
        .set_drawer(&branch(MAIN), NEOCAL, false, &owner(), at(1))
        .await
        .expect("the owner says MAIN has no drawer");
    let before = db.stored().await;
    db.ids.give_one(&job_id(1));

    let answer = db
        .service
        .queue(&branch(MAIN), drawer_request(1), ACTOR, at(2))
        .await;

    assert_eq!(refusal(answer), NO_DRAWER);
    assert!(
        db.stored().await.same_as(&before),
        "a refusal stores no job and no line"
    );

    db.finish().await;
}

#[tokio::test]
async fn a_has_drawer_of_0_stored_behind_the_service_is_refused_too() {
    let db = Devices::start().await;
    put(&db, MAIN, HAS_DRAWER_KEY, "0").await;
    db.ids.give_one(&job_id(1));

    let answer = db
        .service
        .queue(&branch(MAIN), drawer_request(1), ACTOR, at(2))
        .await;

    assert_eq!(refusal(answer), NO_DRAWER);

    db.finish().await;
}

#[tokio::test]
async fn a_drawer_request_is_queued_as_today_where_the_branch_has_said_nothing_or_has_a_drawer() {
    let db = Devices::start().await;
    db.service
        .set_drawer(&branch(B9), NEOCAL, true, &owner(), at(1))
        .await
        .expect("the owner says B9 has a drawer");

    // MAIN has no hasDrawer row: today's answer.
    db.ids.give_one(&job_id(1));
    let main = db
        .service
        .queue(&branch(MAIN), drawer_request(1), ACTOR, at(2))
        .await
        .expect("MAIN queues the drawer request");
    assert_eq!(main.status, JobStatus::Queued);
    assert_eq!(main.id, job_id(1));

    db.ids.give_one(&job_id(2));
    let b9 = db
        .service
        .queue(&branch(B9), drawer_request(2), ACTOR, at(3))
        .await
        .expect("B9 queues the drawer request");
    assert_eq!(b9.status, JobStatus::Queued);
    assert_eq!(b9.id, job_id(2));

    db.finish().await;
}

#[tokio::test]
async fn a_receipt_is_queued_where_the_branch_has_no_drawer() {
    let db = Devices::start().await;
    db.service
        .set_drawer(&branch(MAIN), NEOCAL, false, &owner(), at(1))
        .await
        .expect("the owner says MAIN has no drawer");

    let id = db.queued_print(1).await;

    assert_eq!(
        db.jobs().await[&id]["status"],
        "QUEUED",
        "the drawer switch says nothing about receipts"
    );

    db.finish().await;
}

// ───────────────────────────── process ──────────────────────────────────────

/// A driver that overrides `run_with` and keeps what it was handed; `run`
/// is kept apart so a `process` that skips `run_with` shows.
#[derive(Default)]
struct DrawerSeeingDriver {
    seen: Mutex<Vec<(PrinterSettings, DrawerSettings)>>,
    run_calls: Mutex<u32>,
}

#[async_trait]
impl DeviceDriver for DrawerSeeingDriver {
    fn mode(&self) -> &'static str {
        "SIMULATION"
    }

    async fn run(&self, _kind: JobKind, _printer: &PrinterSettings) -> JobStatus {
        *self.run_calls.lock().expect("run calls") += 1;
        JobStatus::Failed
    }

    async fn run_with(
        &self,
        _kind: JobKind,
        printer: &PrinterSettings,
        drawer: &DrawerSettings,
    ) -> JobStatus {
        self.seen
            .lock()
            .expect("seen")
            .push((printer.clone(), drawer.clone()));
        JobStatus::CommandSimulated
    }
}

#[tokio::test]
async fn process_hands_run_with_the_branch_drawer_settings() {
    let db = Devices::start().await;
    let driver = Arc::new(DrawerSeeingDriver::default());
    let service = db.service.clone().with_driver(driver.clone());
    service
        .set_drawer(&branch(MAIN), NEOCAL, true, &owner(), at(1))
        .await
        .expect("the owner says MAIN has a drawer");
    let main = db.queued(MAIN, "DRAWER_REQUEST", "", "change", 1).await;
    let b9 = db.queued(B9, "DRAWER_REQUEST", "", "change", 2).await;

    let main_job = service
        .process(&branch(MAIN), &main, ACTOR, at(2))
        .await
        .expect("process MAIN's job");
    let b9_job = service
        .process(&branch(B9), &b9, ACTOR, at(3))
        .await
        .expect("process B9's job");

    assert_eq!(
        main_job.status,
        JobStatus::CommandSimulated,
        "run_with's answer"
    );
    assert_eq!(
        b9_job.status,
        JobStatus::CommandSimulated,
        "run_with's answer"
    );
    assert_eq!(
        *driver.seen.lock().expect("seen"),
        vec![
            (
                printer(true, PrinterKind::Simulated),
                drawer(NEOCAL, Some(true))
            ),
            (printer(true, PrinterKind::Simulated), drawer(NEOCAL, None)),
        ],
        "run_with runs once per job, with the branch's printer and drawer settings"
    );
    assert_eq!(
        *driver.run_calls.lock().expect("run calls"),
        0,
        "process calls run_with, not run"
    );

    db.finish().await;
}
