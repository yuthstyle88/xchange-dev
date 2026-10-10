//! Specification tests for DRV-1 (design `docs/design/xchange-v3.md` §5.4b,
//! issue #274): the async `DeviceDriver` with `PrinterSettings`, the
//! `EscposDriver` over a `PrinterTransport`, the branch's printer settings, and
//! `DeviceService::set_printer`.
//!
//! Written before the implementation. Do not edit, delete or weaken.
//!
//! Two kinds of test:
//! - the drivers alone, with no database: a fake printer is a TCP listener on
//!   127.0.0.1 that keeps every byte it is sent, or a transport that keeps
//!   every call;
//! - the settings and the service, on a real PostgreSQL (`TEST_DATABASE_URL`,
//!   as the other device tests). Without it those FAIL; none skips.

mod common;
mod devices_common;
mod routes_common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use devices_common::*;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xchange::infrastructure::devices::escpos_driver::{
    EscposDriver, PrinterTransport, TcpTransport,
};
use xchange::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use xchange::infrastructure::devices::printer_settings::{
    DEFAULT_PORT, PRINTER_HOST, PRINTER_KIND, PRINTER_PORT, PrinterKind, PrinterSettings,
};
use xchange::infrastructure::devices::simulated_driver::{DeviceDriver, SimulatedDriver};

/// `ESC @` then `ESC p 0 25 250`: initialise, then pulse pin 2 for 50 ms on and
/// 500 ms off — pos108's kick (`pos-printer/src/escpos.rs:77-78` @ `8b53cee`).
const KICK: [u8; 7] = [0x1B, 0x40, 0x1B, 0x70, 0x00, 0x19, 0xFA];

/// `network_printer::send` gives up after 5 s to connect and 10 s to write.
const SEND_BOUND: Duration = Duration::from_secs(20);

fn printer(online: bool, kind: PrinterKind) -> PrinterSettings {
    PrinterSettings { online, kind }
}

fn network(host: &str, port: u16) -> PrinterKind {
    PrinterKind::Network(host.to_owned(), port)
}

// ───────────────────────────── a printer on the network ─────────────────────

/// A printer that listens on 127.0.0.1: it accepts one connection and keeps
/// what it is sent until the sender closes it.
struct FakePrinter {
    port: u16,
    received: tokio::task::JoinHandle<Vec<u8>>,
}

impl FakePrinter {
    async fn listen() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a port on 127.0.0.1");
        let port = listener.local_addr().expect("the bound address").port();
        let received = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept the driver");
            let mut bytes = Vec::new();
            socket
                .read_to_end(&mut bytes)
                .await
                .expect("read until the driver closes");
            bytes
        });
        Self { port, received }
    }

    async fn bytes(self) -> Vec<u8> {
        within(SEND_BOUND, "the printer reads to the end", self.received)
            .await
            .expect("the printer task")
    }
}

/// A port on 127.0.0.1 where nothing listens: connecting is refused.
async fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a port on 127.0.0.1");
    let port = listener.local_addr().expect("the bound address").port();
    drop(listener);
    port
}

fn tcp_driver() -> EscposDriver {
    EscposDriver::new(Arc::new(TcpTransport))
}

#[tokio::test]
async fn a_drawer_request_to_a_network_printer_sends_the_kick_and_opens_the_drawer() {
    let fake = FakePrinter::listen().await;
    let settings = printer(true, network("127.0.0.1", fake.port));

    let status = within(
        SEND_BOUND,
        "the driver runs a drawer request",
        tcp_driver().run(JobKind::DrawerRequest, &settings),
    )
    .await;

    assert_eq!(
        status.as_str(),
        "DRAWER_OPENED",
        "a kick the printer took is DRAWER_OPENED"
    );
    assert_eq!(
        fake.bytes().await,
        KICK.to_vec(),
        "the printer is sent exactly ESC @ ESC p 0 25 250"
    );
}

#[tokio::test]
async fn a_drawer_request_to_a_printer_that_refuses_the_connection_fails_in_bounded_time() {
    let port = closed_port().await;
    let settings = printer(true, network("127.0.0.1", port));

    let status = within(
        SEND_BOUND,
        "the driver gives up on a refused connection",
        tcp_driver().run(JobKind::DrawerRequest, &settings),
    )
    .await;

    assert_eq!(status, JobStatus::Failed);
}

// ───────────────────────────── a transport that keeps every call ────────────

#[derive(Default)]
struct RecordingTransport {
    calls: Mutex<Vec<(String, u16, Vec<u8>)>>,
    fail: bool,
}

impl RecordingTransport {
    fn failing() -> Self {
        Self {
            calls: Mutex::default(),
            fail: true,
        }
    }

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
        if self.fail {
            anyhow::bail!("the printer is unplugged");
        }
        Ok(())
    }
}

#[tokio::test]
async fn a_network_drawer_request_goes_to_the_branch_host_and_port_once() {
    let transport = Arc::new(RecordingTransport::default());
    let driver = EscposDriver::new(transport.clone());
    let settings = printer(true, network("printer.branch.local", 9101));

    let status = driver.run(JobKind::DrawerRequest, &settings).await;

    assert_eq!(status.as_str(), "DRAWER_OPENED");
    assert_eq!(
        transport.calls(),
        vec![("printer.branch.local".to_owned(), 9101, KICK.to_vec())],
        "one send, to the branch's host and port, with the kick"
    );
}

#[tokio::test]
async fn a_send_error_is_failed() {
    let transport = Arc::new(RecordingTransport::failing());
    let driver = EscposDriver::new(transport.clone());
    let settings = printer(true, network("printer.branch.local", DEFAULT_PORT));

    let status = driver.run(JobKind::DrawerRequest, &settings).await;

    assert_eq!(status, JobStatus::Failed);
    assert_eq!(transport.calls().len(), 1, "it was tried once, not again");
}

#[tokio::test]
async fn a_receipt_on_a_network_printer_is_still_simulated_and_sends_nothing() {
    let transport = Arc::new(RecordingTransport::default());
    let driver = EscposDriver::new(transport.clone());
    let settings = printer(true, network("printer.branch.local", DEFAULT_PORT));

    let status = driver.run(JobKind::PrintReceipt, &settings).await;

    assert_eq!(
        status,
        JobStatus::SimulatedPrinted,
        "the receipt renderer is T13b's; until then a receipt is simulated"
    );
    assert!(transport.calls().is_empty(), "no byte reaches the printer");
}

/// What `SimulatedDriver` answered before DRV-1, written out: the switch off
/// fails every job, otherwise a receipt is printed and a drawer commanded, in
/// simulation.
fn answered_before(kind: JobKind, online: bool) -> JobStatus {
    match (online, kind) {
        (false, _) => JobStatus::Failed,
        (true, JobKind::PrintReceipt) => JobStatus::SimulatedPrinted,
        (true, JobKind::DrawerRequest) => JobStatus::CommandSimulated,
    }
}

const BOTH_KINDS: [JobKind; 2] = [JobKind::PrintReceipt, JobKind::DrawerRequest];

#[tokio::test]
async fn the_simulated_driver_answers_as_before() {
    for kind in BOTH_KINDS {
        for online in [true, false] {
            let status = SimulatedDriver
                .run(kind, &printer(online, PrinterKind::Simulated))
                .await;
            assert_eq!(
                status,
                answered_before(kind, online),
                "{kind:?} online={online}"
            );
        }
    }
    assert_eq!(SimulatedDriver.mode(), "SIMULATION");
}

#[tokio::test]
async fn a_simulated_printer_on_the_escpos_driver_answers_as_the_simulated_driver_and_sends_nothing()
 {
    let transport = Arc::new(RecordingTransport::default());
    let driver = EscposDriver::new(transport.clone());
    for kind in BOTH_KINDS {
        for online in [true, false] {
            let settings = printer(online, PrinterKind::Simulated);
            let status = driver.run(kind, &settings).await;
            assert_eq!(
                status,
                SimulatedDriver.run(kind, &settings).await,
                "{kind:?} online={online}"
            );
            assert_eq!(
                status,
                answered_before(kind, online),
                "{kind:?} online={online}"
            );
        }
    }
    assert!(transport.calls().is_empty(), "no byte reaches any printer");
}

// ───────────────────────────── the branch's printer settings ────────────────

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

/// The branch's printer as a transaction of its own reads it.
async fn read(db: &Devices, branch_id: &str) -> pos_error::AppResult<PrinterSettings> {
    let repo = PgDeviceRepository::new(db.pool.clone());
    let mut tx = db.pool.begin().await.expect("begin");
    let settings = PrinterSettings::read_in(&mut tx, &repo, &branch(branch_id)).await;
    tx.rollback().await.expect("rollback");
    settings
}

#[tokio::test]
async fn a_branch_with_no_printer_settings_has_a_simulated_printer_switched_on() {
    let db = Devices::start().await;

    assert_eq!(
        read(&db, MAIN).await.expect("read"),
        printer(true, PrinterKind::Simulated)
    );

    db.finish().await;
}

#[tokio::test]
async fn the_switch_is_read_as_before() {
    let db = Devices::start().await;
    db.printer(MAIN, false).await;
    db.printer(B9, true).await;

    assert_eq!(
        read(&db, MAIN).await.expect("read MAIN"),
        printer(false, PrinterKind::Simulated),
        "\"0\" is off"
    );
    assert_eq!(
        read(&db, B9).await.expect("read B9"),
        printer(true, PrinterKind::Simulated),
        "anything but \"0\" is on"
    );

    db.finish().await;
}

#[tokio::test]
async fn a_network_printer_is_read_with_its_host_and_port() {
    let db = Devices::start().await;
    put(&db, MAIN, PRINTER_KIND, "NETWORK").await;
    put(&db, MAIN, PRINTER_HOST, "192.168.1.50").await;
    put(&db, MAIN, PRINTER_PORT, "9101").await;
    put(&db, B9, PRINTER_KIND, "NETWORK").await;
    put(&db, B9, PRINTER_HOST, "printer-b9.local").await;

    assert_eq!(
        read(&db, MAIN).await.expect("read MAIN"),
        printer(true, network("192.168.1.50", 9101))
    );
    assert_eq!(
        read(&db, B9).await.expect("read B9"),
        printer(true, network("printer-b9.local", 9100)),
        "no port is 9100"
    );
    assert_eq!(DEFAULT_PORT, 9100);

    db.finish().await;
}

#[tokio::test]
async fn simulated_ignores_a_stored_host_and_port() {
    let db = Devices::start().await;
    put(&db, MAIN, PRINTER_KIND, "SIMULATED").await;
    put(&db, MAIN, PRINTER_HOST, "192.168.1.50").await;
    put(&db, MAIN, PRINTER_PORT, "not a port").await;

    assert_eq!(
        read(&db, MAIN).await.expect("read"),
        printer(true, PrinterKind::Simulated)
    );

    db.finish().await;
}

#[tokio::test]
async fn a_network_printer_whose_settings_cannot_be_used_is_an_error_not_a_guess() {
    // (kind, host, port) as stored; `None` = not stored.
    let broken: [(&str, Option<&str>, Option<&str>); 8] = [
        ("NETWORK", Some("192.168.1.50"), Some("abc")),
        ("NETWORK", Some("192.168.1.50"), Some("0")),
        ("NETWORK", Some("192.168.1.50"), Some("65536")),
        ("NETWORK", Some("192.168.1.50"), Some("-1")),
        ("NETWORK", Some("192.168.1.50"), Some("")),
        ("NETWORK", None, Some("9100")),
        ("NETWORK", Some("   "), None),
        ("USB", Some("192.168.1.50"), None),
    ];
    let db = Devices::start().await;
    for (kind, host, port) in broken {
        sqlx::query("DELETE FROM device_settings WHERE branch_id = $1")
            .bind(MAIN)
            .execute(&db.pool)
            .await
            .expect("clear MAIN's settings");
        put(&db, MAIN, PRINTER_KIND, kind).await;
        if let Some(host) = host {
            put(&db, MAIN, PRINTER_HOST, host).await;
        }
        if let Some(port) = port {
            put(&db, MAIN, PRINTER_PORT, port).await;
        }
        let answer = read(&db, MAIN).await;
        assert!(
            matches!(answer, Err(pos_error::AppError::Internal(_))),
            "kind={kind} host={host:?} port={port:?}: expected Err(Internal), got {answer:?}"
        );
    }
    db.finish().await;
}

// ───────────────────────────── DeviceService ────────────────────────────────

fn owner() -> Actor {
    Actor::owner("owner")
}

fn staff(role: Role) -> Actor {
    Actor::staff("be1", role, branch(MAIN)).expect("a staff actor")
}

#[tokio::test]
async fn only_the_owner_sets_the_printer() {
    // OR-4b (#767, design §4.2 C5): who may set the printer is the route table's alone
    // (`Policy::need(Devices, setPrinter)` = SystemSettings: EXECUTIVE and ADMIN), asked by the
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
            .set_printer(
                &branch(MAIN),
                network("192.168.1.50", 9100),
                &staff(role),
                at(1),
            )
            .await;
        assert!(answer.is_ok(), "{role:?}: {answer:?}");
    }

    db.finish().await;
}

#[tokio::test]
async fn what_the_owner_sets_is_what_the_branch_reads() {
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
    assert_eq!(
        read(&db, MAIN).await.expect("read MAIN"),
        printer(true, network("192.168.1.50", 9101))
    );
    assert_eq!(
        read(&db, B9).await.expect("read B9"),
        printer(true, PrinterKind::Simulated),
        "another branch is not touched"
    );

    db.service
        .set_printer(&branch(MAIN), PrinterKind::Simulated, &owner(), at(2))
        .await
        .expect("set MAIN back to simulated");
    assert_eq!(
        read(&db, MAIN).await.expect("read MAIN again"),
        printer(true, PrinterKind::Simulated)
    );

    db.finish().await;
}

#[tokio::test]
async fn the_owner_cannot_store_a_network_printer_that_cannot_be_reached() {
    let db = Devices::start().await;
    let before = db.settings().await;

    for kind in [
        network("", 9100),
        network("   ", 9100),
        network("192.168.1.50", 0),
    ] {
        let answer = db
            .service
            .set_printer(&branch(MAIN), kind.clone(), &owner(), at(1))
            .await;
        assert_eq!(
            refusal(answer),
            xchange::infrastructure::devices::device_service::BAD_PRINTER,
            "{kind:?}"
        );
    }
    assert_eq!(db.settings().await, before, "a refusal stores nothing");

    db.finish().await;
}

/// A driver that keeps the printer settings it was handed and answers a
/// status the table already holds.
#[derive(Default)]
struct SeeingDriver {
    seen: Mutex<Vec<PrinterSettings>>,
}

#[async_trait]
impl DeviceDriver for SeeingDriver {
    fn mode(&self) -> &'static str {
        "SIMULATION"
    }

    async fn run(&self, _kind: JobKind, printer: &PrinterSettings) -> JobStatus {
        self.seen.lock().expect("seen").push(printer.clone());
        JobStatus::CommandSimulated
    }
}

#[tokio::test]
async fn process_hands_the_driver_the_branch_printer() {
    let db = Devices::start().await;
    let driver = Arc::new(SeeingDriver::default());
    let service = db.service.clone().with_driver(driver.clone());
    service
        .set_printer(
            &branch(MAIN),
            network("192.168.1.50", 9101),
            &owner(),
            at(1),
        )
        .await
        .expect("set MAIN to a network printer");
    let id = db.queued(MAIN, "DRAWER_REQUEST", "", "change", 1).await;

    let job = service
        .process(&branch(MAIN), &id, ACTOR, at(2))
        .await
        .expect("process");

    assert_eq!(job.status, JobStatus::CommandSimulated);
    assert_eq!(
        *driver.seen.lock().expect("seen"),
        vec![printer(true, network("192.168.1.50", 9101))],
        "the driver runs once, with what the branch's settings say"
    );

    db.finish().await;
}
