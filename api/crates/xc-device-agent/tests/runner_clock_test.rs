//! Clock seam tests (§5.4c): verifies freshness window boundary conditions,
//! mutant elimination for >= vs > and non-zero window expiration, and PR 356 review follow-ups.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use xc_device_agent::client::{
    AgentApi, ApiError, ClaimRequest, Claimed, DrawerInfo, Outcome, PairAnswer, PairRequest, Report,
};
use xc_device_agent::clock::FakeClock;
use xc_device_agent::journal::Journal;
use xc_device_agent::runner::{AgentConfig, AgentRunner, Step};
use xc_device_agent::transport::{DrawerTransport, SendReceipt, TransportError};

const KICK_B64: &str = "G0AbcAAZ+g==";
const KICK: [u8; 7] = [0x1B, 0x40, 0x1B, 0x70, 0x00, 0x19, 0xFA];
const JOB: &str = "0b0b0b0b-de71-4ce5-8000-000000000001";
const LEASE: &str = "1ea5e000-0000-4000-8000-000000000001";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch_file(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("xc-agent-clock-{}-{nanos}-{n}", std::process::id()));
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
    fn answer_report(&self, answer: Result<(), ApiError>) {
        self.0
            .lock()
            .expect("server")
            .report_answers
            .push_back(answer);
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
        os: "macos-14.6-arm64".to_owned(),
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

struct Rig {
    server: FakeServer,
    printer: FakePrinter,
    clock: FakeClock,
    journal_path: PathBuf,
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
        Journal::open(journal_path.clone()).expect("open journal"),
        config(),
        clock.clone(),
    );
    Rig {
        server,
        printer,
        clock,
        journal_path,
        runner,
    }
}

/// A job claimed at t and sent at t + window − 1 ms is sent.
#[tokio::test]
async fn a_job_claimed_at_t_and_sent_at_t_plus_window_minus_1ms_is_sent() {
    let t = Instant::now();
    let window_ms = 10_000;
    let window = Duration::from_millis(window_ms);

    let mut r = rig(cups_receipt());
    // First call to now() is arrival (t), second is elapsed check (t + window - 1ms)
    r.clock
        .set_sequence([t, t + window - Duration::from_millis(1)]);
    r.server.answer_claim(Ok(Some(claimed(window_ms))));

    assert_eq!(r.runner.step().await, Step::Done(Outcome::DrawerOpened));
    assert_eq!(r.printer.sent().len(), 1, "sent to printer");
    assert_eq!(r.printer.sent()[0], KICK, "kick bytes sent");
    let reports = r.server.reported();
    assert_eq!(reports.len(), 1);
    assert_eq!(
        (
            reports[0].outcome,
            reports[0].job_id.as_str(),
            reports[0].lease_id.as_str()
        ),
        (Outcome::DrawerOpened, JOB, LEASE)
    );
}

/// A job at exactly t + window is expired and never sent (the boundary follows §5.4c's "older than").
#[tokio::test]
async fn a_job_at_exactly_t_plus_window_is_expired_and_never_sent() {
    let t = Instant::now();
    let window_ms = 10_000;
    let window = Duration::from_millis(window_ms);

    let mut r = rig(cups_receipt());
    // First call to now() is arrival (t), second is elapsed check (exactly t + window)
    r.clock.set_sequence([t, t + window]);
    r.server.answer_claim(Ok(Some(claimed(window_ms))));

    assert_eq!(r.runner.step().await, Step::Done(Outcome::Expired));
    assert!(r.printer.sent().is_empty(), "never sent to printer");
    let reports = r.server.reported();
    assert_eq!(reports.len(), 1);
    assert_eq!(
        (
            reports[0].outcome,
            reports[0].reason.as_str(),
            reports[0].job_id.as_str(),
            reports[0].lease_id.as_str()
        ),
        (Outcome::Expired, "SEND_WINDOW_PASSED", JOB, LEASE)
    );
}

/// A job at t + window + 1 ms is expired.
#[tokio::test]
async fn a_job_at_t_plus_window_plus_1ms_is_expired() {
    let t = Instant::now();
    let window_ms = 10_000;
    let window = Duration::from_millis(window_ms);

    let mut r = rig(cups_receipt());
    // First call to now() is arrival (t), second is elapsed check (t + window + 1ms)
    r.clock
        .set_sequence([t, t + window + Duration::from_millis(1)]);
    r.server.answer_claim(Ok(Some(claimed(window_ms))));

    assert_eq!(r.runner.step().await, Step::Done(Outcome::Expired));
    assert!(r.printer.sent().is_empty(), "never sent to printer");
    let reports = r.server.reported();
    assert_eq!(reports.len(), 1);
    assert_eq!(
        (
            reports[0].outcome,
            reports[0].reason.as_str(),
            reports[0].job_id.as_str(),
            reports[0].lease_id.as_str()
        ),
        (Outcome::Expired, "SEND_WINDOW_PASSED", JOB, LEASE)
    );
}

/// Follow-up from PR 356: A rejected report is acknowledged (not owed) and its status and reason
/// are journalled.
#[tokio::test]
async fn a_rejected_report_journals_status_and_reason_and_is_not_owed() {
    let mut r = rig(cups_receipt());
    r.server.answer_claim(Ok(Some(claimed(10_000))));
    r.server.answer_report(Err(ApiError::Rejected {
        status: 409,
        error: "already processed on another node".to_owned(),
    }));

    assert_eq!(r.runner.step().await, Step::Done(Outcome::DrawerOpened));

    // Assert it is no longer owed in journal
    let journal = Journal::open(&r.journal_path).expect("open journal");
    let owed = journal.pending(SystemTime::now());
    assert!(owed.is_empty(), "rejected report must not be owed");

    // Assert journal file on disk contains a 'rejected' line with status and reason
    let content = std::fs::read_to_string(&r.journal_path).expect("read journal");
    let rejected_line = content
        .lines()
        .find(|line| line.contains("\"kind\":\"rejected\""))
        .expect("journal contains rejected entry");

    let parsed: serde_json::Value = serde_json::from_str(rejected_line).expect("valid json");
    assert_eq!(parsed["kind"], "rejected");
    assert_eq!(parsed["job_id"], JOB);
    assert_eq!(parsed["lease_id"], LEASE);
    assert_eq!(parsed["status"], 409);
    assert_eq!(parsed["reason"], "already processed on another node");
}

/// Follow-up from PR 356: HttpAgentApi redacts tokens from rejected responses.
#[tokio::test]
async fn a_rejected_report_redacts_token_from_error() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_url = format!("http://127.0.0.1:{port}");
    let secret_token = "super-secret-token-xyz-12345";

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let body = format!("conflict with token {secret_token}");
            let response = format!(
                "HTTP/1.1 409 Conflict\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });

    let api = xc_device_agent::client::HttpAgentApi::new(&server_url, secret_token).unwrap();
    let report = Report {
        job_id: JOB.to_owned(),
        lease_id: LEASE.to_owned(),
        outcome: Outcome::DrawerOpened,
        reason: String::new(),
        detail: serde_json::json!({}),
    };

    let err = api.report(&report).await.unwrap_err();
    match err {
        ApiError::Rejected { status, error } => {
            assert_eq!(status, 409);
            assert!(
                !error.contains(secret_token),
                "error must not contain secret token, got: {error}"
            );
            assert!(
                error.contains("<redacted>"),
                "token must be redacted in error"
            );
        }
        other => panic!("expected ApiError::Rejected, got: {other:?}"),
    }
}

/// Follow-up from PR 356: reqwest client has Policy::none() so it does not follow redirects.
#[tokio::test]
async fn reqwest_client_does_not_follow_redirects() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_url = format!("http://127.0.0.1:{port}");

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            // 302 Found redirecting to a port where nothing is listening
            let response = "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:59999/other\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });

    let api = xc_device_agent::client::HttpAgentApi::new(&server_url, "test-token").unwrap();
    let report = Report {
        job_id: JOB.to_owned(),
        lease_id: LEASE.to_owned(),
        outcome: Outcome::DrawerOpened,
        reason: String::new(),
        detail: serde_json::json!({}),
    };

    let err = api.report(&report).await.unwrap_err();
    match err {
        ApiError::Rejected { status, .. } => {
            assert_eq!(
                status, 302,
                "must receive 302 directly without following redirect"
            );
        }
        other => panic!("expected 302 Rejected, got: {other:?}"),
    }
}
