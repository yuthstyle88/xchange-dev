//! `WaitEstimator` (design §5, Q9, owner Q-A5 and Q-A6): the wait a customer joining now can
//! expect, with its range. Computed, never typed; unknown when there is nothing to compute from.

/// `estimate_seconds` with `estimate_low_seconds` and `estimate_high_seconds`; `low ≤ seconds ≤ high`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaitEstimate {
    pub seconds: i32,
    pub low_seconds: i32,
    pub high_seconds: i32,
}

pub struct WaitEstimator;

impl WaitEstimator {
    /// Today's sample is used once it has at least this many completed tickets (owner Q-A5).
    pub const TODAY_MINIMUM: usize = 3;
    /// Before that, the last this-many completed tickets of earlier days (owner Q-A5).
    pub const EARLIER_SAMPLE: usize = 30;

    /// The sample of service times (`served_at → completed_at`, in seconds) the estimate uses:
    /// `today` once it holds at least [`Self::TODAY_MINIMUM`], else `earlier` (the caller passes
    /// the last [`Self::EARLIER_SAMPLE`] of earlier days).
    pub fn sample<'a>(today: &'a [i64], earlier: &'a [i64]) -> &'a [i64] {
        if today.len() >= Self::TODAY_MINIMUM {
            today
        } else if !earlier.is_empty() {
            earlier
        } else {
            &[]
        }
    }

    /// PostgreSQL `percentile_cont(p)` over `sample` (linear interpolation; order does not
    /// matter). `None` for an empty sample.
    pub fn percentile_cont(sample: &[i64], p: f64) -> Option<f64> {
        if sample.is_empty() {
            return None;
        }
        let mut sorted = sample.to_vec();
        sorted.sort_unstable();
        let n = sorted.len();
        if n == 1 {
            return Some(sorted[0] as f64);
        }
        let idx = (n - 1) as f64 * p;
        let k = idx.floor() as usize;
        let f = idx - k as f64;
        if k + 1 < n {
            Some(sorted[k] as f64 + f * (sorted[k + 1] as f64 - sorted[k] as f64))
        } else {
            Some(sorted[k] as f64)
        }
    }

    /// `ahead ÷ active_counters × percentile` for the 0.5 (estimate), 0.25 (low) and 0.75 (high)
    /// percentiles of the chosen sample. `None` when the sample is empty or `active_counters` is 0.
    pub fn estimate(
        ahead: i64,
        active_counters: i32,
        today: &[i64],
        earlier: &[i64],
    ) -> Option<WaitEstimate> {
        if active_counters <= 0 {
            return None;
        }
        let s = Self::sample(today, earlier);
        if s.is_empty() {
            return None;
        }
        if ahead == 0 {
            return Some(WaitEstimate {
                seconds: 0,
                low_seconds: 0,
                high_seconds: 0,
            });
        }
        let p50 = Self::percentile_cont(s, 0.5)?;
        let p25 = Self::percentile_cont(s, 0.25)?;
        let p75 = Self::percentile_cont(s, 0.75)?;
        let ratio = ahead as f64 / active_counters as f64;
        Some(WaitEstimate {
            seconds: (ratio * p50).round() as i32,
            low_seconds: (ratio * p25).round() as i32,
            high_seconds: (ratio * p75).round() as i32,
        })
    }
}
