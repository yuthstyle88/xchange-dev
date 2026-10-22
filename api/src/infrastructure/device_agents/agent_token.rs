//! The agent's two secrets: the one-time pairing code and the bearer token,
//! and the one hash both are stored as (design §5.4c *Identity*).

use crate::infrastructure::paired_devices::kind::DeviceKind;
use crate::infrastructure::paired_devices::secret::DeviceSecret;

/// Builds and hashes the agent's secrets. Holds nothing.
pub struct AgentToken;

impl AgentToken {
    /// `xcda_` and the 64 lower-case hex digits of two UUID v4 values.
    pub fn new_token() -> String {
        DeviceSecret::new_token(DeviceKind::DrawerAgent)
    }

    /// 10 characters of Crockford base32 (50 bits of a UUID v4), shown as
    /// `XXXXX-XXXXX`.
    pub fn new_pairing_code() -> String {
        DeviceSecret::new_pairing_code()
    }

    /// The lower-case hex SHA-256 of `text`'s UTF-8 bytes.
    pub fn hash(text: &str) -> String {
        DeviceSecret::hash(text)
    }

    /// The hash a pairing code is stored and looked up by: `hash` of its 10
    /// characters with `-` and white space removed and letters upper-cased.
    /// `None` when what is left is not 10 characters of Crockford base32.
    pub fn code_hash(code: &str) -> Option<String> {
        DeviceSecret::code_hash(code)
    }
}
