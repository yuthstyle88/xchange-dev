//! The AUTO anchor (pricing-tiers.md §3.1, owner P-4: configurable per branch, overridable per currency).
//!
//! Written whole in the skeleton (PT-0): several tickets read it, so no ticket owns it.

/// Which reference line an AUTO price is anchored on. The stored text is also the line's `reference_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AnchorSource {
    /// Bank of Thailand (today's behaviour, R-12); `'START'` stays its fallback before BoT first quotes.
    #[default]
    Bot,
    /// TT Pattaya (`MARKET_TT` reference lines, §4).
    MarketTt,
    /// Super Rich (`MARKET_SR` reference lines, §4).
    MarketSr,
}

impl AnchorSource {
    /// The stored text: `rate_policy_versions.anchor_source`, `rate_policy_currency_settings.anchor_source`.
    pub fn as_str(self) -> &'static str {
        match self {
            AnchorSource::Bot => "BOT",
            AnchorSource::MarketTt => "MARKET_TT",
            AnchorSource::MarketSr => "MARKET_SR",
        }
    }

    /// The `rate_reference_lines.reference_type` this anchor reads. Equal to [`AnchorSource::as_str`] by design.
    pub fn reference_type(self) -> &'static str {
        self.as_str()
    }

    /// Parses the stored text; `None` for anything else.
    pub fn parse(text: &str) -> Option<AnchorSource> {
        match text {
            "BOT" => Some(AnchorSource::Bot),
            "MARKET_TT" => Some(AnchorSource::MarketTt),
            "MARKET_SR" => Some(AnchorSource::MarketSr),
            _ => None,
        }
    }
}
