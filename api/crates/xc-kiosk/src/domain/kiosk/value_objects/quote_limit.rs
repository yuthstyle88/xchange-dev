//! How many quotes one address may make in a branch.

/// More than [`QuoteLimit::MAX`] quotes from one address in one branch within
/// [`QuoteLimit::WINDOW_MS`] are refused. v2 has no such limit (design §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuoteLimit;

impl QuoteLimit {
    pub const MAX: i64 = 20;
    pub const WINDOW_MS: i64 = 60_000;

    /// Whether one more quote is refused when `recent` were made in the window.
    pub fn reached(recent: i64) -> bool {
        recent >= Self::MAX
    }
}
