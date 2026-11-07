//! CA-4a. The scanner in tests: scripted answers, and busy while a scan is in flight.
//! CA-4a gives these types their fields.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use super::{ProbeFuture, ScanFailure, ScanFuture, ScanRequest, Scanner};

struct FakeScannerInner {
    script: Vec<Result<Vec<u8>, ScanFailure>>,
    cursor: usize,
    in_flight: bool,
    requests: Vec<ScanRequest>,
    holds: VecDeque<Arc<Notify>>,
}

/// Answers each scan with the next scripted answer. Past the end of the script: `NoScanner`.
pub struct FakeScanner {
    inner: Mutex<FakeScannerInner>,
}

impl FakeScanner {
    pub fn new(script: Vec<Result<Vec<u8>, ScanFailure>>) -> Self {
        Self {
            inner: Mutex::new(FakeScannerInner {
                script,
                cursor: 0,
                in_flight: false,
                requests: Vec::new(),
                holds: VecDeque::new(),
            }),
        }
    }

    /// The next scan stays in flight until the answer is released.
    pub fn hold_next(&self) -> ScanHold {
        let notify = Arc::new(Notify::new());
        self.inner
            .lock()
            .unwrap()
            .holds
            .push_back(Arc::clone(&notify));
        ScanHold { notify }
    }

    /// Whether a scan is in flight now.
    pub fn in_flight(&self) -> bool {
        self.inner.lock().unwrap().in_flight
    }

    /// The requests scanned so far, a `Busy` refusal not counted.
    pub fn requests(&self) -> Vec<ScanRequest> {
        self.inner.lock().unwrap().requests.clone()
    }
}

/// Holds a scan in flight until [`ScanHold::release`].
pub struct ScanHold {
    notify: Arc<Notify>,
}

impl ScanHold {
    pub fn release(self) {
        self.notify.notify_one();
    }
}

struct InFlightGuard<'a>(&'a Mutex<FakeScannerInner>);

impl<'a> Drop for InFlightGuard<'a> {
    fn drop(&mut self) {
        self.0.lock().unwrap().in_flight = false;
    }
}

impl Scanner for FakeScanner {
    fn probe(&self) -> ProbeFuture<'_> {
        Box::pin(std::future::ready(Ok(())))
    }

    fn scan(&self, request: ScanRequest) -> ScanFuture<'_> {
        Box::pin(async move {
            let (result, hold) = {
                let mut inner = self.inner.lock().unwrap();
                if inner.in_flight {
                    return Err(ScanFailure::Busy);
                }
                inner.in_flight = true;
                inner.requests.push(request);
                let result = if inner.cursor < inner.script.len() {
                    let r = inner.script[inner.cursor].clone();
                    inner.cursor += 1;
                    r
                } else {
                    Err(ScanFailure::NoScanner)
                };
                (result, inner.holds.pop_front())
            };

            let _guard = InFlightGuard(&self.inner);
            if let Some(notify) = hold {
                notify.notified().await;
            }
            result
        })
    }
}
