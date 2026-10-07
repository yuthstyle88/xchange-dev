//! Monotonic clock seam (§5.4c): allows the agent's runner to observe time
//! and permits tests to control time across claim, send, and report boundaries.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A monotonic clock reading for the runner.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Instant;
}

impl<C: Clock> Clock for Arc<C> {
    fn now(&self) -> Instant {
        (**self).now()
    }
}

/// The production clock reading `Instant::now()`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl SystemClock {
    pub fn new() -> Self {
        Self
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A fake, controllable clock for testing runner boundaries.
#[derive(Debug, Clone)]
pub struct FakeClock {
    state: Arc<Mutex<FakeClockState>>,
}

#[derive(Debug)]
struct FakeClockState {
    current: Instant,
    sequence: VecDeque<Instant>,
    step_by: Option<Duration>,
}

impl FakeClock {
    pub fn new(initial: Instant) -> Self {
        Self {
            state: Arc::new(Mutex::new(FakeClockState {
                current: initial,
                sequence: VecDeque::new(),
                step_by: None,
            })),
        }
    }

    /// Sets the current instant.
    pub fn set(&self, instant: Instant) {
        let mut s = self.state.lock().expect("fake clock lock");
        s.current = instant;
    }

    /// Advances the current instant by `duration`.
    pub fn advance(&self, duration: Duration) {
        let mut s = self.state.lock().expect("fake clock lock");
        s.current += duration;
    }

    /// Sets a sequence of instants to return on subsequent calls to `now()`.
    /// When the sequence is exhausted, subsequent calls return the last instant.
    pub fn set_sequence(&self, instants: impl IntoIterator<Item = Instant>) {
        let mut s = self.state.lock().expect("fake clock lock");
        s.sequence = instants.into_iter().collect();
    }

    /// Automatically advances `current` by `step` on each call to `now()` after returning.
    pub fn set_step_by(&self, step: Duration) {
        let mut s = self.state.lock().expect("fake clock lock");
        s.step_by = Some(step);
    }

    /// Reads the current instant without modifying the sequence or advancing.
    pub fn current(&self) -> Instant {
        self.state.lock().expect("fake clock lock").current
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        let mut s = self.state.lock().expect("fake clock lock");
        if let Some(next) = s.sequence.pop_front() {
            s.current = next;
            next
        } else if let Some(step) = s.step_by {
            let res = s.current;
            s.current += step;
            res
        } else {
            s.current
        }
    }
}

pub type ManualClock = FakeClock;
