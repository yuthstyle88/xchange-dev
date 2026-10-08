//! Tests for agent follow-ups (§5.4c, ticket #370):
//! 1. Abort when journal write fails: transport is never called.
//! 2. A torn line in the middle of the journal is skipped and later valid lines survive.
//! 3. On Linux, install fails with one line when xc-agent user is missing and writes no unit.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use xc_device_agent::client::{
    AgentApi, ApiError, ClaimRequest, Claimed, DrawerInfo, Outcome, PairAnswer, PairRequest, Report,
};
use xc_device_agent::clock::FakeClock;
use xc_device_agent::journal::{Journal, Pending};
use xc_device_agent::runner::{AgentConfig, AgentRunner, Step};
use xc_device_agent::transport::{DrawerTransport, SendReceipt, TransportError};

const KICK_B64: &str = "G0AbcAAZ+g==";
const JOB: &str = "0b0b0b0b-de71-4ce5-8000-000000000001";
const LEASE: &str = "1ea5e000-0000-4000-8000-000000000001";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch_file(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "xc-agent-followups-{}-{nanos}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir.join(label)
}

#[derive(Default)]
struct ServerState {
    claims: VecDeque<Result<Option<Claimed>, ApiError>>,
    report_answers: VecDeque<Result<(), ApiError>>,
    claimed: Vec<ClaimRequest>,
    reported: Vec<Report>,
}

#[derive(Clone, Default)]
struct FakeServer(Arc<Mutex<ServerState>>);

impl FakeServer {
    fn answer_claim(&self, answer: Result<Option<Claimed>, ApiError>) {
        self.0.lock().expect("server").claims.push_back(answer);
    }
    fn reported(&self) -> Vec<Report> {
        self.0.lock().expect("server").reported.clone()
    }
}

impl AgentApi for FakeServer {
    async fn pair(&self, _request: &PairRequest) -> Result<PairAnswer, ApiError> {
        Err(ApiError::Rejected {
            status: 400,
            error: "not in these tests".to_owned(),
        })
    }

    async fn claim(&self, request: &ClaimRequest) -> Result<Option<Claimed>, ApiError> {
        let mut s = self.0.lock().expect("server");
        s.claimed.push(request.clone());
        s.claims.pop_front().unwrap_or(Ok(None))
    }

    async fn report(&self, report: &Report) -> Result<(), ApiError> {
        let mut s = self.0.lock().expect("server");
        s.reported.push(report.clone());
        s.report_answers.pop_front().unwrap_or(Ok(()))
    }
}

#[derive(Clone)]
struct FakePrinter {
    sent: Arc<Mutex<Vec<Vec<u8>>>>,
    answer: Result<SendReceipt, TransportError>,
}

impl FakePrinter {
    fn new(answer: Result<SendReceipt, TransportError>) -> Self {
        Self {
            sent: Arc::new(Mutex::new(Vec::new())),
            answer,
        }
    }
    fn sent(&self) -> Vec<Vec<u8>> {
        self.sent.lock().expect("printer").clone()
    }
}

impl DrawerTransport for FakePrinter {
    async fn send(&self, bytes: &[u8]) -> Result<SendReceipt, TransportError> {
        self.sent.lock().expect("printer").push(bytes.to_vec());
        self.answer.clone()
    }
}

fn cups_receipt() -> Result<SendReceipt, TransportError> {
    Ok(SendReceipt {
        cups_job: Some("POS80_-12".to_owned()),
    })
}

fn config() -> AgentConfig {
    AgentConfig {
        version: "1.0.0".to_owned(),
        build: "abc1234".to_owned(),
        os: "linux-x86_64".to_owned(),
        host: "counter-1.local".to_owned(),
        drawer: DrawerInfo {
            transport: "CUPS".to_owned(),
            target: "POS80_".to_owned(),
        },
        wait_ms: 20_000,
    }
}

fn claimed(send_within_ms: u64) -> Claimed {
    Claimed {
        job_id: JOB.to_owned(),
        lease_id: LEASE.to_owned(),
        bytes: KICK_B64.to_owned(),
        send_within_ms,
    }
}

fn sending_pending(job: &str, lease: &str) -> Pending {
    Pending::Sending {
        job_id: job.to_owned(),
        lease_id: lease.to_owned(),
    }
}

struct Rig {
    server: FakeServer,
    printer: FakePrinter,
    runner: AgentRunner<FakeServer, FakePrinter, FakeClock>,
}

fn rig(answer: Result<SendReceipt, TransportError>) -> Rig {
    let journal_path = scratch_file("journal.jsonl");
    let server = FakeServer::default();
    let printer = FakePrinter::new(answer);
    let clock = FakeClock::default();
    let runner = AgentRunner::with_clock(
        server.clone(),
        printer.clone(),
        Journal::open(journal_path).expect("open journal"),
        config(),
        clock,
    );
    Rig {
        server,
        printer,
        runner,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Item 1: Writing the journal fails -> kick aborted, transport never called
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_failed_sending_journal_write_aborts_and_never_calls_transport() {
    let mut r = rig(cups_receipt());
    r.server.answer_claim(Ok(Some(claimed(10_000))));

    // Simulate journal write failure
    r.runner.journal.set_fail_writes(true);

    let step = r.runner.step().await;
    assert_eq!(step, Step::Done(Outcome::Failed));

    // The transport must NEVER be called when journal write fails
    assert!(
        r.printer.sent().is_empty(),
        "transport must never be called when journal write fails"
    );

    // Failed outcome reported with JOURNAL_WRITE_FAILED reason
    let reports = r.server.reported();
    assert_eq!(reports.len(), 1);
    assert_eq!(
        (
            reports[0].outcome,
            reports[0].reason.as_str(),
            reports[0].job_id.as_str(),
            reports[0].lease_id.as_str()
        ),
        (Outcome::Failed, "JOURNAL_WRITE_FAILED", JOB, LEASE)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Item 2: Torn line in the middle is skipped, later valid lines survive
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn a_torn_line_in_the_middle_is_skipped_and_later_valid_lines_survive() {
    let path = scratch_file("journal_mid_torn.jsonl");
    let now = SystemTime::now();

    // 1. Write valid entry j1
    {
        let mut j = Journal::open(path.clone()).expect("open journal");
        j.sending("j1", "l1", now).expect("sending j1");
    }

    // 2. Append a torn/corrupt line in the middle
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open file for append");
        f.write_all(b"{\"kind\":\"outc\n")
            .expect("write torn middle line");
        f.flush().expect("flush");
    }

    // 3. Append valid entry j2 after the torn line
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open file for append");
        let j2_entry = serde_json::json!({
            "kind": "sending",
            "job_id": "j2",
            "lease_id": "l2",
            "at": now
        });
        f.write_all(serde_json::to_string(&j2_entry).unwrap().as_bytes())
            .expect("write j2 entry");
        f.write_all(b"\n").expect("write newline");
        f.flush().expect("flush");
    }

    // 4. Open journal: repair should skip the torn line in the middle
    // and both j1 and j2 must survive
    let j = Journal::open(path).expect("open journal with torn line in middle");
    let pending = j.pending(now);
    assert_eq!(
        pending,
        vec![sending_pending("j1", "l1"), sending_pending("j2", "l2"),],
        "both j1 and j2 must survive despite torn line in the middle"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Item 3: On Linux, install fails when xc-agent user is missing, writes no unit
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;
    use std::thread;
    use std::time::Duration;

    fn scratch_home_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos();
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "xc-agent-followups-home-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create scratch home dir");
        dir
    }

    fn assert_no_service_units(dir: &Path) {
        if !dir.exists() {
            return;
        }
        for entry in std::fs::read_dir(dir).expect("read dir") {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                assert_no_service_units(&path);
            } else {
                let name = path.file_name().unwrap().to_string_lossy();
                assert!(
                    !name.ends_with(".service"),
                    "found unexpected service file: {}",
                    path.display()
                );
                assert!(
                    !name.ends_with(".plist"),
                    "found unexpected plist file: {}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn on_linux_missing_service_user_fails_install_and_writes_no_unit() {
        // Assert that the test machine does not have xc-agent user
        assert!(
            !xc_device_agent::config::service_user_exists("xc-agent"),
            "test requires xc-agent user to be absent"
        );

        // Direct library check: install_service must fail with NotFound and instruction line
        let dummy_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/bin/true"));
        let err = xc_device_agent::config::install_service(&dummy_exe).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        let msg = err.to_string();
        assert!(
            msg.contains("service user 'xc-agent' does not exist; create it with: useradd -r -s /usr/sbin/nologin xc-agent"),
            "error must tell operator how to create the user, got: {msg}"
        );

        // CLI check: xc-device-agent install without --no-service fails before pairing
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let port = listener.local_addr().expect("local addr").port();
        let hit = Arc::new(AtomicBool::new(false));
        let hit_clone = Arc::clone(&hit);

        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let server_thread = thread::spawn(move || {
            listener.set_nonblocking(true).expect("set nonblocking");
            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        hit_clone.store(true, Ordering::SeqCst);
                        let mut buf = [0u8; 1024];
                        let _ = stream.read(&mut buf);
                        let _ = stream.write_all(b"HTTP/1.1 500 Internal Server Error\r\n\r\n");
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        let temp_home = scratch_home_dir();
        let server_url = format!("http://127.0.0.1:{port}");

        let bin = env!("CARGO_BIN_EXE_xc-device-agent");
        let output = std::process::Command::new(bin)
            .arg("install")
            .arg("--server")
            .arg(&server_url)
            .arg("--code")
            .arg("PAIR-CODE-88888")
            .arg("--cups-queue")
            .arg("POS80_RECEIPT")
            .arg("--channel")
            .arg("stable")
            .env("HOME", &temp_home)
            .env_remove("XC_AGENT_CONFIG_DIR")
            .output()
            .expect("run xc-device-agent install");

        let _ = stop_tx.send(());
        server_thread.join().expect("server thread finished");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // 1. Process must fail (exit code non-zero)
        assert!(
            !output.status.success(),
            "install must fail when xc-agent user is missing\nstdout: {stdout}\nstderr: {stderr}"
        );

        // 2. Fails with one line telling the operator how to create it
        assert!(
            stderr.contains("service user 'xc-agent' does not exist; create it with: useradd -r -s /usr/sbin/nologin xc-agent"),
            "stderr missing expected instruction line, got: {stderr}"
        );
        assert_eq!(
            stderr.trim().lines().count(),
            1,
            "stderr must be exactly one line, got: {stderr}"
        );

        // 3. The pairing endpoint was NEVER contacted (pairing code not spent)
        assert!(
            !hit.load(Ordering::SeqCst),
            "pair endpoint must never be hit when service user is missing"
        );

        // 4. No config or token was saved
        assert!(!temp_home.join(".config").join("xc-device-agent").exists());

        // 5. Writes no unit file under temp HOME or systemd
        assert_no_service_units(&temp_home);
        assert!(!temp_home.join(".config").join("systemd").exists());
    }
}
