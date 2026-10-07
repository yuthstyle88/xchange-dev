//! Row AG-5 (`docs/design/xchange-v3.md` §5.4c *Claim → send → record*, *Restart and
//! offline*): the agent's runner against a fake server and a fake printer. Written before the
//! implementation. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The names are the ticket's (stubbed in this branch's first commit so that this file
//! compiles; the stubs do nothing).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;
use xc_device_agent::client::{
    AgentApi, ApiError, ClaimRequest, Claimed, DrawerInfo, Outcome, PairAnswer, PairRequest, Report,
};
use xc_device_agent::journal::{Journal, Pending};
use xc_device_agent::runner::{AgentConfig, AgentRunner, Step, backoff};
use xc_device_agent::transport::{DrawerTransport, SendReceipt, TransportError};

/// `ESC @` + `ESC p 0 25 250`: the NEOCAL model's kick (§5.4b), base64 as the server sends it.
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
    let dir = std::env::temp_dir().join(format!(
        "xc-agent-runner-{}-{nanos}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir.join(label)
}

// ── the fake server ──────────────────────────────────────────────────────────

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

// ── the fake printer ─────────────────────────────────────────────────────────

struct PrinterState {
    answer: Result<SendReceipt, TransportError>,
    sent: Vec<Vec<u8>>,
    /// What the journal owed at the moment each send began.
    journal_at_send: Vec<Vec<Pending>>,
    journal: PathBuf,
}

#[derive(Clone)]
struct FakePrinter(Arc<Mutex<PrinterState>>);

impl FakePrinter {
    fn new(journal: &Path, answer: Result<SendReceipt, TransportError>) -> Self {
        FakePrinter(Arc::new(Mutex::new(PrinterState {
            answer,
            sent: Vec::new(),
            journal_at_send: Vec::new(),
            journal: journal.to_path_buf(),
        })))
    }
    fn sent(&self) -> Vec<Vec<u8>> {
        self.0.lock().expect("printer").sent.clone()
    }
}

impl DrawerTransport for FakePrinter {
    async fn send(&self, bytes: &[u8]) -> Result<SendReceipt, TransportError> {
        let mut s = self.0.lock().expect("printer");
        let owed = Journal::open(s.journal.clone())
            .expect("the journal opens while the runner holds it")
            .pending(SystemTime::now());
        s.journal_at_send.push(owed);
        s.sent.push(bytes.to_vec());
        s.answer.clone()
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
    journal: PathBuf,
    runner: AgentRunner<FakeServer, FakePrinter>,
}

fn rig(answer: Result<SendReceipt, TransportError>) -> Rig {
    let journal = scratch_file("journal.jsonl");
    let server = FakeServer::default();
    let printer = FakePrinter::new(&journal, answer);
    let runner = AgentRunner::new(
        server.clone(),
        printer.clone(),
        Journal::open(journal.clone()).expect("open the journal"),
        config(),
    );
    Rig {
        server,
        printer,
        journal,
        runner,
    }
}

fn owed(path: &Path) -> Vec<Pending> {
    Journal::open(path.to_path_buf())
        .expect("open the journal")
        .pending(SystemTime::now())
}

// ── the tests ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_claim_says_what_the_agent_is() {
    let mut r = rig(cups_receipt());
    assert_eq!(r.runner.step().await, Step::Idle, "a 204 is idle");
    let sent = r.server.claimed();
    assert_eq!(sent.len(), 1, "one claim per step");
    assert_eq!(sent[0], r.runner.claim_request());
    let expected = ClaimRequest {
        wait_ms: 20_000,
        version: "1.0.0".to_owned(),
        build: "abc1234".to_owned(),
        os: "macos-14.6-arm64".to_owned(),
        host: "counter-1.local".to_owned(),
        drawer: DrawerInfo {
            transport: "CUPS".to_owned(),
            target: "POS80_".to_owned(),
        },
    };
    assert_eq!(sent[0], expected);
    assert_eq!(
        serde_json::to_value(&sent[0]).expect("JSON"),
        json!({ "waitMs": 20000, "version": "1.0.0", "build": "abc1234",
                "os": "macos-14.6-arm64", "host": "counter-1.local",
                "drawer": { "transport": "CUPS", "target": "POS80_" } }),
        "the claim body of §5.4c, camelCase, nothing else"
    );
    assert!(r.printer.sent().is_empty(), "nothing to send");
}

#[tokio::test]
async fn a_claimed_job_is_sent_as_given_and_reported_opened() {
    let mut r = rig(cups_receipt());
    r.server.answer_claim(Ok(Some(claimed(5_000))));
    assert_eq!(r.runner.step().await, Step::Done(Outcome::DrawerOpened));
    assert_eq!(
        r.printer.sent(),
        vec![KICK.to_vec()],
        "the seven bytes it was given, once"
    );
    assert_eq!(
        r.server.reported(),
        vec![Report {
            job_id: JOB.to_owned(),
            lease_id: LEASE.to_owned(),
            outcome: Outcome::DrawerOpened,
            reason: String::new(),
            detail: json!({ "cupsJob": "POS80_-12" }),
        }]
    );
    assert_eq!(
        serde_json::to_value(&r.server.reported()[0]).expect("JSON"),
        json!({ "jobId": JOB, "leaseId": LEASE, "outcome": "DRAWER_OPENED", "reason": "",
                "detail": { "cupsJob": "POS80_-12" } })
    );
    assert!(
        owed(&r.journal).is_empty(),
        "the server took it: nothing owed"
    );
}

/// The journal says `sending` before the bytes go out (a crash during the send must never
/// lead to a second kick).
#[tokio::test]
async fn the_journal_says_sending_before_the_bytes_go_out() {
    let mut r = rig(cups_receipt());
    r.server.answer_claim(Ok(Some(claimed(5_000))));
    r.runner.step().await;
    let at_send = r.printer.0.lock().expect("printer").journal_at_send.clone();
    assert_eq!(
        at_send,
        vec![vec![Pending::Sending {
            job_id: JOB.to_owned(),
            lease_id: LEASE.to_owned(),
        }]]
    );
}

/// A claim whose send window has passed — on the agent's monotonic clock, from when the
/// answer arrived — is reported `EXPIRED` and never sent.
#[tokio::test]
async fn a_claim_past_its_send_window_is_expired_and_never_sent() {
    let mut r = rig(cups_receipt());
    r.server.answer_claim(Ok(Some(claimed(0))));
    assert_eq!(r.runner.step().await, Step::Done(Outcome::Expired));
    assert!(r.printer.sent().is_empty(), "never sent");
    let reports = r.server.reported();
    assert_eq!(reports.len(), 1);
    assert_eq!(
        (
            reports[0].outcome,
            reports[0].job_id.as_str(),
            reports[0].lease_id.as_str()
        ),
        (Outcome::Expired, JOB, LEASE)
    );
}

#[tokio::test]
async fn a_busy_queue_or_a_failed_send_is_reported_failed_with_its_reason() {
    for (answer, reason) in [
        (Err(TransportError::QueueBusy), "QUEUE_BUSY"),
        (
            Err(TransportError::Failed("connection refused".to_owned())),
            "SEND_FAILED",
        ),
    ] {
        let mut r = rig(answer);
        r.server.answer_claim(Ok(Some(claimed(5_000))));
        assert_eq!(r.runner.step().await, Step::Done(Outcome::Failed));
        let reports = r.server.reported();
        assert_eq!(reports.len(), 1);
        assert_eq!(
            (reports[0].outcome, reports[0].reason.as_str()),
            (Outcome::Failed, reason)
        );
    }
    assert_eq!(TransportError::QueueBusy.reason(), "QUEUE_BUSY");
    assert_eq!(
        TransportError::Failed(String::new()).reason(),
        "SEND_FAILED"
    );
}

/// The report did not get through: the outcome stays owed and goes again at the next start
/// (or step); the kick is not sent again.
#[tokio::test]
async fn an_unacknowledged_outcome_is_reported_again_and_never_resent() {
    let mut r = rig(cups_receipt());
    r.server.answer_claim(Ok(Some(claimed(5_000))));
    r.server
        .answer_report(Err(ApiError::Connection("reset".to_owned())));
    assert_eq!(r.runner.step().await, Step::Done(Outcome::DrawerOpened));
    let owed_now = owed(&r.journal);
    assert_eq!(owed_now.len(), 1, "{owed_now:?}");
    assert!(
        matches!(&owed_now[0], Pending::Unacknowledged(rep)
            if rep.outcome == Outcome::DrawerOpened && rep.lease_id == LEASE),
        "{owed_now:?}"
    );

    r.runner.recover().await;
    let reports = r.server.reported();
    assert_eq!(reports.len(), 2, "reported again");
    assert_eq!(reports[1], reports[0]);
    assert_eq!(r.printer.sent().len(), 1, "sent once only");
    assert!(owed(&r.journal).is_empty());
}

/// A send that began and has no outcome (the agent died mid-send) is reported `FAILED`
/// `AGENT_RESTARTED` at the next start; the kick may have gone out, so it is never sent again.
#[tokio::test]
async fn a_restart_reports_a_send_without_outcome_and_never_resends() {
    let journal = scratch_file("journal.jsonl");
    Journal::open(journal.clone())
        .expect("open")
        .sending(JOB, LEASE, SystemTime::now())
        .expect("write sending");
    let server = FakeServer::default();
    let printer = FakePrinter::new(&journal, cups_receipt());
    let mut runner = AgentRunner::new(
        server.clone(),
        printer.clone(),
        Journal::open(journal.clone()).expect("reopen"),
        config(),
    );
    runner.recover().await;
    let reports = server.reported();
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(
        (
            reports[0].job_id.as_str(),
            reports[0].lease_id.as_str(),
            reports[0].outcome,
            reports[0].reason.as_str()
        ),
        (JOB, LEASE, Outcome::Failed, "AGENT_RESTARTED")
    );
    assert!(printer.sent().is_empty(), "never sent again");
    assert!(
        server.claimed().is_empty(),
        "recover reports; it does not claim"
    );
    assert!(owed(&journal).is_empty());
}

/// 401 stops claiming (re-pair needed); a connection error or 429 waits and counts.
#[tokio::test]
async fn a_401_stops_and_errors_back_off() {
    let mut r = rig(cups_receipt());
    r.server.answer_claim(Err(ApiError::Unauthorized));
    assert_eq!(r.runner.step().await, Step::RePairNeeded);

    let mut r = rig(cups_receipt());
    r.server
        .answer_claim(Err(ApiError::Connection("refused".to_owned())));
    match r.runner.step().await {
        Step::Wait(d) => assert!(
            d >= Duration::from_millis(800) && d <= Duration::from_millis(1200),
            "the first wait is 1 s ± 20 %: {d:?}"
        ),
        other => panic!("a connection error waits: {other:?}"),
    }
    assert_eq!(r.runner.failures, 1);
    r.server.answer_claim(Err(ApiError::RateLimited {
        retry_after_secs: 7,
    }));
    match r.runner.step().await {
        Step::Wait(d) => assert!(d >= Duration::from_secs(7), "429 waits Retry-After: {d:?}"),
        other => panic!("a 429 waits: {other:?}"),
    }
    assert_eq!(r.runner.step().await, Step::Idle);
    assert_eq!(r.runner.failures, 0, "an answer resets the count");
}

#[test]
fn backoff_doubles_from_one_second_to_sixty_with_twenty_percent_jitter() {
    let secs = |a, j| backoff(a, j).as_secs_f64();
    assert_eq!(secs(0, 0.0), 1.0);
    assert_eq!(secs(1, 0.0), 2.0);
    assert_eq!(secs(5, 0.0), 32.0);
    assert_eq!(secs(6, 0.0), 60.0);
    assert_eq!(secs(40, 0.0), 60.0, "no overflow");
    assert!((secs(0, 1.0) - 1.2).abs() < 1e-9);
    assert!((secs(0, -1.0) - 0.8).abs() < 1e-9);
    assert!((secs(6, 1.0) - 72.0).abs() < 1e-9);
    assert!((secs(6, -1.0) - 48.0).abs() < 1e-9);
}
