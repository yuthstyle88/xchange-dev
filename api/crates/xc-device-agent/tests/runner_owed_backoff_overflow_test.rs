//! Regression test for #2330 rework: an owed report answered with `RateLimited { retry_after_secs: u64::MAX }`
//! must never panic in `owe_later`, and subsequent `step()` calls with the clock advanced still claim.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use xc_device_agent::client::{
    AgentApi, ApiError, ClaimRequest, Claimed, DrawerInfo, Outcome, PairAnswer, PairRequest, Report,
};
use xc_device_agent::journal::Journal;
use xc_device_agent::runner::{AgentConfig, AgentRunner, FakeClock, Step};
use xc_device_agent::transport::{DrawerTransport, SendReceipt, TransportError};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch_file(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "xc-agent-overflow-test-{}-{nanos}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir.join(name)
}

#[derive(Default)]
struct ServerState {
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

    fn claimed(&self) -> Vec<ClaimRequest> {
        self.0.lock().expect("server").claimed.clone()
    }

    fn reported(&self) -> Vec<Report> {
        self.0.lock().expect("server").reported.clone()
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
        Ok(None)
    }

    async fn report(&self, report: &Report) -> Result<(), ApiError> {
        let mut s = self.0.lock().expect("server");
        s.reported.push(report.clone());
        s.report_answers.pop_front().unwrap_or(Ok(()))
    }
}

#[derive(Clone, Default)]
struct FakePrinter;

impl DrawerTransport for FakePrinter {
    async fn send(&self, _bytes: &[u8]) -> Result<SendReceipt, TransportError> {
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

#[tokio::test]
async fn owed_report_rate_limited_with_u64_max_does_not_panic_and_both_claims_happen() {
    let journal = scratch_file("journal.jsonl");
    let server = FakeServer::default();
    let clock = FakeClock::new(Instant::now());
    let mut runner = AgentRunner::with_clock(
        server.clone(),
        FakePrinter,
        Journal::open(journal).expect("open journal"),
        config(),
        clock.clone(),
    );
    let report = Report {
        job_id: "job-overflow".to_owned(),
        lease_id: "lease-overflow".to_owned(),
        outcome: Outcome::DrawerOpened,
        reason: String::new(),
        detail: serde_json::json!({}),
    };
    runner
        .journal
        .outcome(&report, SystemTime::now())
        .expect("write outcome");

    server.answer_report(Err(ApiError::RateLimited {
        retry_after_secs: u64::MAX,
    }));

    assert_eq!(runner.step().await, Step::Idle);
    assert_eq!(server.reported().len(), 1);
    assert_eq!(server.claimed().len(), 1);

    clock.advance(Duration::from_secs(60));
    assert_eq!(runner.step().await, Step::Idle);
    assert_eq!(
        server.reported().len(),
        1,
        "still inside the capped Retry-After window"
    );
    assert_eq!(server.claimed().len(), 2, "both claims happen");
}
