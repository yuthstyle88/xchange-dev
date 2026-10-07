//! Tests for owed report retries while running and undecodable base64 handling.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use xc_device_agent::client::{
    AgentApi, ApiError, ClaimRequest, Claimed, DrawerInfo, Outcome, PairAnswer, PairRequest, Report,
};
use xc_device_agent::journal::{Journal, Pending};
use xc_device_agent::runner::{AgentConfig, AgentRunner, Step};
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
const KICK_B64: &str = "G0AbcAAZ+g==";
const KICK: [u8; 7] = [0x1B, 0x40, 0x1B, 0x70, 0x00, 0x19, 0xFA];

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

impl FakePrinter {
    fn sent(&self) -> Vec<Vec<u8>> {
        self.0.lock().expect("printer").sent.clone()
    }
}

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

#[tokio::test]
async fn an_undecodable_base64_is_reported_failed_and_never_sent() {
    let journal_path = scratch_file("journal.jsonl");
    let server = FakeServer::default();
    let printer = FakePrinter::default();
    let mut runner = AgentRunner::new(
        server.clone(),
        printer.clone(),
        Journal::open(journal_path.clone()).expect("open journal"),
        config(),
    );

    // Provide invalid base64 in claimed job
    server.answer_claim(Ok(Some(Claimed {
        job_id: JOB.to_owned(),
        lease_id: LEASE.to_owned(),
        bytes: "!!!not-valid-base64!!!".to_owned(),
        send_within_ms: 5000,
    })));

    let step_result = runner.step().await;
    assert_eq!(step_result, Step::Done(Outcome::Failed));

    // Must never send anything to printer
    assert!(
        printer.sent().is_empty(),
        "printer must not receive any bytes for invalid base64"
    );

    // Must report FAILED with reason BAD_BYTES
    let reports = server.reported();
    assert_eq!(reports.len(), 1, "exactly one report sent");
    assert_eq!(reports[0].outcome, Outcome::Failed);
    assert_eq!(reports[0].reason, "BAD_BYTES");
    assert_eq!(reports[0].job_id, JOB);
    assert_eq!(reports[0].lease_id, LEASE);

    // Report was acknowledged so nothing stays owed
    assert!(owed(&journal_path).is_empty());
}

#[tokio::test]
async fn an_owed_report_is_retried_on_next_step_and_never_resends() {
    let journal_path = scratch_file("journal.jsonl");
    let server = FakeServer::default();
    let printer = FakePrinter::default();
    let mut runner = AgentRunner::new(
        server.clone(),
        printer.clone(),
        Journal::open(journal_path.clone()).expect("open journal"),
        config(),
    );

    // Step 1: Claim succeeds, kick bytes sent, but report fails due to connection error
    server.answer_claim(Ok(Some(Claimed {
        job_id: JOB.to_owned(),
        lease_id: LEASE.to_owned(),
        bytes: KICK_B64.to_owned(),
        send_within_ms: 5000,
    })));
    server.answer_report(Err(ApiError::Connection("network offline".to_owned())));

    let step1 = runner.step().await;
    assert_eq!(step1, Step::Done(Outcome::DrawerOpened));

    // Sent once to printer
    assert_eq!(printer.sent(), vec![KICK.to_vec()]);

    // Outcome stays owed in journal
    let owed_items = owed(&journal_path);
    assert_eq!(owed_items.len(), 1);
    assert!(
        matches!(&owed_items[0], Pending::Unacknowledged(r) if r.job_id == JOB && r.lease_id == LEASE)
    );

    // Step 2: Next step runs while server is now reachable. No new jobs (204).
    server.answer_report(Ok(())); // Report will succeed
    server.answer_claim(Ok(None)); // No new job

    let step2 = runner.step().await;
    assert_eq!(step2, Step::Idle);

    // Printer was NOT sent to again (still length 1)
    assert_eq!(printer.sent().len(), 1, "kick must never be resent");

    // Server received second report attempt
    let reports = server.reported();
    assert_eq!(reports.len(), 2, "report was retried");
    assert_eq!(reports[1].job_id, JOB);
    assert_eq!(reports[1].lease_id, LEASE);
    assert_eq!(reports[1].outcome, Outcome::DrawerOpened);

    // Journal now acknowledges it: nothing owed
    assert!(
        owed(&journal_path).is_empty(),
        "owed outcome must be acknowledged"
    );
}

#[tokio::test]
async fn a_409_rejected_owed_report_is_not_retried_and_the_next_step_claims() {
    let journal_path = scratch_file("journal.jsonl");
    let server = FakeServer::default();
    let printer = FakePrinter::default();
    let mut runner = AgentRunner::new(
        server.clone(),
        printer.clone(),
        Journal::open(journal_path.clone()).expect("open journal"),
        config(),
    );

    // Create an unacknowledged report in journal
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

    // Server rejects report with 409 Conflict (e.g. lease already resolved)
    server.answer_report(Err(ApiError::Rejected {
        status: 409,
        error: "Conflict: lease already resolved".to_owned(),
    }));
    // Claim will succeed with no new job (204)
    server.answer_claim(Ok(None));

    // Step 1: Retries owed report -> gets 409 -> acknowledged as no longer owed -> claims!
    let step1 = runner.step().await;
    assert_eq!(step1, Step::Idle);

    // 409'd report must not remain owed in the journal
    assert!(
        owed(&journal_path).is_empty(),
        "409'd report must not remain owed"
    );

    // Exactly 1 report attempt was made
    assert_eq!(server.reported().len(), 1);
    assert_eq!(server.reported()[0].job_id, JOB);

    // Exactly 1 claim was made
    assert_eq!(server.claimed().len(), 1);

    // Step 2: Next step claims again; 409'd report is NOT retried
    server.answer_claim(Ok(None));
    let step2 = runner.step().await;
    assert_eq!(step2, Step::Idle);

    // Still exactly 1 report was sent (not retried)
    assert_eq!(
        server.reported().len(),
        1,
        "409'd report must not be retried"
    );
    // And claim was made again
    assert_eq!(server.claimed().len(), 2);
}

#[tokio::test]
async fn a_still_owed_report_does_not_stop_the_agent_from_claiming_new_jobs() {
    let journal_path = scratch_file("journal.jsonl");
    let server = FakeServer::default();
    let printer = FakePrinter::default();
    let mut runner = AgentRunner::new(
        server.clone(),
        printer.clone(),
        Journal::open(journal_path.clone()).expect("open journal"),
        config(),
    );

    // Create an unacknowledged report in journal
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

    // Server fails report with connection error (keeps it owed)
    server.answer_report(Err(ApiError::Connection("temporary timeout".to_owned())));
    // But claim succeeds!
    server.answer_claim(Ok(None));

    let step = runner.step().await;
    assert_eq!(
        step,
        Step::Idle,
        "still-owed report must not stop agent from claiming"
    );

    // The report stays owed in journal
    assert_eq!(
        owed(&journal_path).len(),
        1,
        "report should stay owed on connection error"
    );

    // The claim request WAS sent
    assert_eq!(server.claimed().len(), 1, "claim must be sent");
}
