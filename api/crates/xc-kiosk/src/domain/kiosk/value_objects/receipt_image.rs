//! The receipt picture the desk sends to the customer's screen.

/// A PNG data URL, as the desk screen draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptImage(String);

impl ReceiptImage {
    /// The longest text accepted, prefix included.
    pub const MAX_LENGTH: usize = 1_800_000;
    const PREFIX: &'static str = "data:image/png;base64,";

    /// `None` unless `text` is the prefix followed by at least one character of
    /// the base64 alphabet (`A-Z a-z 0-9 + / =`) and nothing else, and no longer
    /// than [`Self::MAX_LENGTH`]. The data is not decoded.
    pub fn parse(text: &str) -> Option<Self> {
        if text.len() > Self::MAX_LENGTH {
            return None;
        }
        let data = text.strip_prefix(Self::PREFIX)?;
        let alphabet = |b: u8| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=';
        if data.is_empty() || !data.bytes().all(alphabet) {
            return None;
        }
        Some(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
