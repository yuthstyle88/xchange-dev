//! Value tiers for rate publications (BP §02, design `docs/design/rate-feed-connector.md` §2, §3.4, §3.8).

use xc_kernel::money::Money;

/// Code identifying a value tier (e.g. "RETAIL", "STANDARD", "LARGE", "SPECIAL", "ALL").
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TierCode(String);

impl TierCode {
    /// Creates a new `TierCode`, refusing empty or whitespace-only strings.
    pub fn new(text: impl Into<String>) -> Result<Self, String> {
        let s = text.into();
        if s.trim().is_empty() {
            return Err("tier code cannot be empty or blank".to_string());
        }
        Ok(Self(s))
    }

    /// Returns the tier code as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TierCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::ops::Deref for TierCode {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A value band with lower (exclusive) and upper (inclusive) bounds in THB.
///
/// Missing bounds represent open intervals (e.g. `None` lower bound means unbounded below;
/// `None` upper bound means unbounded above).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierBand {
    pub code: TierCode,
    pub lower_exclusive_thb: Option<Money>,
    pub upper_inclusive_thb: Option<Money>,
}

impl TierBand {
    /// Checks whether `v` falls within this tier band (`lower < v <= upper`).
    pub fn contains(&self, v: Money) -> bool {
        let lower_ok = match self.lower_exclusive_thb {
            Some(lower) => v > lower,
            None => true,
        };
        let upper_ok = match self.upper_inclusive_thb {
            Some(upper) => v <= upper,
            None => true,
        };
        lower_ok && upper_ok
    }
}

/// A collection of tier bands used to pick a tier for a base transaction amount in THB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TierTable {
    pub bands: Vec<TierBand>,
}

impl TierTable {
    /// Creates a new `TierTable` from a list of bands.
    pub fn new(bands: Vec<TierBand>) -> Self {
        Self { bands }
    }

    /// Picks the single tier code that contains `value`.
    ///
    /// Returns `None` if no band contains the value (gap) or if more than one band
    /// contains the value (overlap).
    pub fn pick(&self, value: Money) -> Option<TierCode> {
        let mut matched = None;
        for band in &self.bands {
            if band.contains(value) {
                if matched.is_some() {
                    // Overlap: more than one band holds the value
                    return None;
                }
                matched = Some(&band.code);
            }
        }
        matched.cloned()
    }
}
