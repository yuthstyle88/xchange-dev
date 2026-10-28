//! `QuoteType` (design `docs/design/blueprint/binding-quote.md` D-2): `INDICATIVE` or `BINDING`, parsed at the
//! edge like `QuoteChannel`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteType {
    /// Every quote until phase 6: valid while its publication is current (D-3).
    Indicative,
    /// The rate lock (phase 6): valid until `expires_at`, whatever is published since (D-3, O-4 A).
    Binding,
}

impl QuoteType {
    /// The stored `quotes.type` text.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Indicative => "INDICATIVE",
            Self::Binding => "BINDING",
        }
    }

    /// The stored text back; anything else is `None`.
    pub fn parse_db(s: &str) -> Option<Self> {
        match s {
            "INDICATIVE" => Some(Self::Indicative),
            "BINDING" => Some(Self::Binding),
            _ => None,
        }
    }
}
