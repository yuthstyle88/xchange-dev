//! `RetryPolicy` (design §5.2): a destination's retry timing. The default is the fleet's
//! (Broker §6a, jh-api): 5 s doubling to a 10-minute cap; a row expires 7 days after it was
//! created. A destination that needs other timing overrides `OutboxHandler::retry_policy`.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// The wait after the first failed attempt.
    pub first: Duration,
    /// No wait is longer than this.
    pub cap: Duration,
    /// A row older than this (from `created_at`) turns `EXPIRED` instead of being sent again.
    pub expire_after: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            first: Duration::from_secs(5),
            cap: Duration::from_secs(600),
            expire_after: Duration::from_secs(7 * 24 * 3600),
        }
    }
}

impl RetryPolicy {
    /// The wait before the next attempt, after `attempts` attempts have failed (`attempts >= 1`):
    /// `first`, `2·first`, `4·first`, … capped at `cap`. Never overflows, whatever `attempts` is.
    pub fn backoff(&self, attempts: i32) -> Duration {
        if attempts <= 1 {
            return self.first.min(self.cap);
        }
        let shift = (attempts - 1) as u32;
        if shift >= 63 {
            return self.cap;
        }
        let factor = 1u64 << shift;
        let first_millis = self.first.as_millis();
        if let Some(total_millis) = first_millis.checked_mul(factor as u128) {
            let dur = Duration::from_millis(total_millis.min(u64::MAX as u128) as u64);
            dur.min(self.cap)
        } else {
            self.cap
        }
    }
}
