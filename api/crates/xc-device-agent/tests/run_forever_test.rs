//! CA-3e (XC Counter §6: the agent crate as a library): `run_forever` claims until `stop` fires,
//! then returns. Written before the implementation. Do not edit, delete or weaken; add tests in
//! a file of your own.
//!
//! | Case | Required |
//! |---|---|
//! | `stop` is already `true` at the start | returns with no claim |
//! | every claim answers 204 | keeps claiming; `stop` fires at the third; no fourth claim |
//! | the claim answers 401 (a 10-minute wait follows) | `stop` cuts the wait short: returns within 5 s |

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use xc_device_agent::client::{
    AgentApi, ApiError, ClaimRequest, Claimed, PairAnswer, PairRequest, Report,
};
use xc_device_agent::journal::Journal;
use xc_device_agent::runner::{AgentConfig, AgentRunner, run_forever};
use xc_device_agent::transport::{DrawerTransport, SendReceipt, TransportError};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch_journal() -> Journal {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "xc-agent-run-forever-{}-{nanos}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    Journal::open(dir.join("journal")).expect("open the journal")
}

fn config() -> AgentConfig {
    AgentConfig {
        version: "1.0.0".to_owned(),
        build: "abc1234".to_owned(),
        os: "macos".to_owned(),
        host: "counter-1".to_owned(),
        drawer: None,
        wait_ms: 0,
    }
}

/// Answers every claim the same, counts them, and fires `stop` at the claim numbered `stop_at`.
#[derive(Clone)]
struct FakeServer {
    claims: Arc<Mutex<u32>>,
    answer: Result<Option<Claimed>, ApiError>,
    stop_at: u32,
    stop: Arc<watch::Sender<bool>>,
}

impl FakeServer {
    fn claims(&self) -> u32 {
        *self.claims.lock().expect("server")
    }
}

impl AgentApi for FakeServer {
    async fn pair(&self, _request: &PairRequest) -> Result<PairAnswer, ApiError> {
        Err(ApiError::Rejected {
            status: 400,
            error: "not in these tests".to_owned(),
        })
    }

    async fn claim(&self, _request: &ClaimRequest) -> Result<Option<Claimed>, ApiError> {
        let count = {
            let mut claims = self.claims.lock().expect("server");
            *claims += 1;
            *claims
        };
        if count >= self.stop_at {
            self.stop.send_replace(true);
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
        self.answer.clone()
    }

    async fn report(&self, _report: &Report) -> Result<(), ApiError> {
        Ok(())
    }
}

struct NoPrinter;

impl DrawerTransport for NoPrinter {
    async fn send(&self, _bytes: &[u8]) -> Result<SendReceipt, TransportError> {
        Err(TransportError::Failed("no job in these tests".to_owned()))
    }
}

fn rig(
    answer: Result<Option<Claimed>, ApiError>,
    stop_at: u32,
    stopped: bool,
) -> (FakeServer, watch::Receiver<bool>) {
    let (tx, rx) = watch::channel(stopped);
    let server = FakeServer {
        claims: Arc::new(Mutex::new(0)),
        answer,
        stop_at,
        stop: Arc::new(tx),
    };
    (server, rx)
}

const LONG_ENOUGH: Duration = Duration::from_secs(5);

#[tokio::test]
async fn run_forever_claims_until_stop_fires_and_not_after() {
    let (server, rx) = rig(Ok(None), u32::MAX, true);
    let runner = AgentRunner::new(server.clone(), NoPrinter, scratch_journal(), config());
    tokio::time::timeout(LONG_ENOUGH, run_forever(runner, rx))
        .await
        .expect("stop already true: returns at once");
    assert_eq!(server.claims(), 0, "stop already true: nothing is claimed");

    let (server, rx) = rig(Ok(None), 3, false);
    let runner = AgentRunner::new(server.clone(), NoPrinter, scratch_journal(), config());
    tokio::time::timeout(LONG_ENOUGH, run_forever(runner, rx))
        .await
        .expect("returns once stop fires");
    assert_eq!(server.claims(), 3, "claims until stop fires, and not after");
}

#[tokio::test]
async fn run_forever_stop_cuts_the_re_pair_wait_short() {
    let (server, rx) = rig(Err(ApiError::Unauthorized), 1, false);
    let runner = AgentRunner::new(server.clone(), NoPrinter, scratch_journal(), config());
    tokio::time::timeout(LONG_ENOUGH, run_forever(runner, rx))
        .await
        .expect("stop ends the 10-minute re-pair wait");
    assert_eq!(server.claims(), 1);
}
