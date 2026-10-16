//! An owed report whose retry fails (no answer, a 5xx, or a 429) is retried with backoff, not
//! on every step (design §5.4c: "reported again, with backoff, for 24 hours"). A 5xx keeps a
//! report owed (#2205), so without backoff a report the server keeps failing would be re-sent
//! on every claim loop for a day. The claim loop itself is never held back.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use xc_device_agent::client::{
    AgentApi, ApiError, ClaimRequest, Claimed, DrawerInfo, Outcome, PairAnswer, PairRequest, Report,
};
use xc_device_agent::journal::{Journal, Pending};
use xc_device_agent::runner::{AgentConfig, AgentRunner, FakeClock, Step};
use xc_device_agent::transport::{DrawerTransport, SendReceipt, TransportError};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch_file(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("xc-agent-test-{}-{nanos}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir.join(name)
}

const JOB: &str = "job-42";
const LEASE: &str = "lease-42";
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
    fn claimed(&self) -> Vec<ClaimRequest> {
        self.0.lock().expect("server").claimed.clone()
    }
}

impl AgentApi for FakeServer {
    async fn pair(&self, _request: &PairRequest) -> Result<PairAnswer, ApiError> {
        Err(ApiError::Rejected {
            status: 400,
            error: "not in tests".to_owned(),
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

#[derive(Default)]
struct PrinterState {
    sent: Vec<Vec<u8>>,
}

#[derive(Clone, Default)]
struct FakePrinter(Arc<Mutex<PrinterState>>);

impl DrawerTransport for FakePrinter {
    async fn send(&self, bytes: &[u8]) -> Result<SendReceipt, TransportError> {
        let mut s = self.0.lock().expect("printer");
        s.sent.push(bytes.to_vec());
        Ok(SendReceipt {
            cups_job: Some("POS80_-1".to_owned()),
        })
    }
}

fn config() -> AgentConfig {
    AgentConfig {
        version: "1.0.0".to_owned(),
        build: "abc".to_owned(),
        os: "linux".to_owned(),
        host: "counter".to_owned(),
        drawer: DrawerInfo {
            transport: "CUPS".to_owned(),
            target: "POS80_".to_owned(),
        },
        wait_ms: 20_000,
    }
}

fn owed(path: &Path) -> Vec<Pending> {
    Journal::open(path.to_path_buf())
        .expect("open journal")
        .pending(SystemTime::now())
}

struct Rig {
    server: FakeServer,
    clock: FakeClock,
    runner: AgentRunner<FakeServer, FakePrinter, FakeClock>,
    journal: PathBuf,
}

/// A runner on a fake clock with one outcome owed in its journal.
fn rig() -> Rig {
    let journal = scratch_file("journal.jsonl");
    let server = FakeServer::default();
    let clock = FakeClock::new(Instant::now());
    let mut runner = AgentRunner::with_clock(
        server.clone(),
        FakePrinter::default(),
        Journal::open(journal.clone()).expect("open journal"),
        config(),
        clock.clone(),
    );
    let report = Report {
        job_id: JOB.to_owned(),
        lease_id: LEASE.to_owned(),
        outcome: Outcome::DrawerOpened,
        reason: String::new(),
        detail: serde_json::json!({}),
    };
    runner
        .journal
        .outcome(&report, SystemTime::now())
        .expect("write outcome");
    Rig {
        server,
        clock,
        runner,
        journal,
    }
}

fn server_error() -> ApiError {
    ApiError::Connection("server error 500".to_owned())
}

#[tokio::test]
async fn a_failing_owed_report_waits_its_backoff_between_retries_and_claims_go_on() {
    let mut r = rig();
    // The server fails every report.
    for _ in 0..10 {
        r.server.answer_report(Err(server_error()));
    }

    // First retry is not delayed.
    assert_eq!(r.runner.step().await, Step::Idle);
    assert_eq!(r.server.reported().len(), 1);

    // Within the first backoff (1 s ± 20 %): not retried, but the agent still claims.
    assert_eq!(r.runner.step().await, Step::Idle);
    r.clock.advance(Duration::from_millis(700));
    assert_eq!(r.runner.step().await, Step::Idle);
    assert_eq!(r.server.reported().len(), 1, "no retry inside the backoff");
    assert_eq!(r.server.claimed().len(), 3, "claims are not held back");

    // Past it: retried once; the next wait doubles (2 s ± 20 %).
    r.clock.advance(Duration::from_millis(600));
    r.runner.step().await;
    assert_eq!(r.server.reported().len(), 2);
    r.clock.advance(Duration::from_millis(1_500));
    r.runner.step().await;
    assert_eq!(r.server.reported().len(), 2, "second backoff is longer");
    r.clock.advance(Duration::from_millis(1_000));
    r.runner.step().await;
    assert_eq!(r.server.reported().len(), 3);

    assert_eq!(owed(&r.journal).len(), 1, "a 5xx keeps the report owed");
}

#[tokio::test]
async fn an_owed_report_taken_after_its_backoff_is_acknowledged_and_not_sent_again() {
    let mut r = rig();
    r.server.answer_report(Err(server_error()));
    r.runner.step().await;
    r.clock.advance(Duration::from_secs(2));
    r.server.answer_report(Ok(()));
    r.runner.step().await;
    assert_eq!(r.server.reported().len(), 2);
    assert!(owed(&r.journal).is_empty());

    r.clock.advance(Duration::from_secs(120));
    r.runner.step().await;
    assert_eq!(
        r.server.reported().len(),
        2,
        "an acknowledged report is not sent again"
    );
}

#[tokio::test]
async fn an_owed_report_answered_429_waits_at_least_retry_after() {
    let mut r = rig();
    r.server.answer_report(Err(ApiError::RateLimited {
        retry_after_secs: 30,
    }));
    r.runner.step().await;
    assert_eq!(r.server.reported().len(), 1);

    r.clock.advance(Duration::from_secs(29));
    r.runner.step().await;
    assert_eq!(r.server.reported().len(), 1, "Retry-After is honoured");

    r.clock.advance(Duration::from_secs(2));
    r.runner.step().await;
    assert_eq!(r.server.reported().len(), 2);
    assert!(owed(&r.journal).is_empty());
}
