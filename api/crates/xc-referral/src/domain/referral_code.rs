//! `ReferralCode` (design §4.1, §4.2): a Broker referral code as v3 keeps it.
//!
//! Broker's format (Broker-Platform `broker/src/domain/broker.rs:80-89`): `[A-Z0-9]`, 6 to 12
//! characters. Broker looks a code up unchanged (`attributions.rs:76`), so a typed code is trimmed
//! and upper-cased here, or a correct lowercase code would be `404 unknown code` at Broker.

use crate::application::capture_service::CaptureError;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReferralCode(String);

impl ReferralCode {
    /// Trim, upper-case, then `^[A-Z0-9]{6,12}$`; anything else is `CaptureError::BadCode`.
    pub fn parse(text: &str) -> Result<Self, CaptureError> {
        let trimmed = text.trim();
        let upper = trimmed.to_uppercase();
        if upper.len() < 6 || upper.len() > 12 {
            return Err(CaptureError::BadCode);
        }
        if !upper
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            return Err(CaptureError::BadCode);
        }
        Ok(Self(upper))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ReferralCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
