//! `DeviceSecret` (design `kiosk-shared-phone.md` §7.5): the device's two secrets, the one-time
//! pairing code and the token, and the one hash both are stored as. `AgentToken`
//! (`device_agents/agent_token.rs`) generalised by kind; BT-2 turns `AgentToken` into thin calls
//! into it, so `AgentToken`'s callers and tests do not change.
//!
//! Filled by BT-1. `api/tests/paired_device_core_test.rs` pins it.

use super::kind::DeviceKind;

/// Builds and hashes the paired devices' secrets. Holds nothing.
pub struct DeviceSecret;

impl DeviceSecret {
    /// `kind.token_prefix()` and the 64 lower-case hex digits of two UUID v4 values
    /// (`agent_token.rs:17-24` with the prefix of `kind`).
    pub fn new_token(kind: DeviceKind) -> String {
        let _ = kind;
        todo!("BT-1: DeviceSecret::new_token")
    }

    /// 10 characters of Crockford base32 (`0-9 A-Z` without `I L O U`, 50 random bits), shown as
    /// `XXXXX-XXXXX` (`agent_token.rs:28-40`).
    pub fn new_pairing_code() -> String {
        todo!("BT-1: DeviceSecret::new_pairing_code")
    }

    /// The hash a pairing code is stored and looked up by: [`DeviceSecret::hash`] of its 10
    /// characters with `-` and white space removed and letters upper-cased. `None` when what is
    /// left is not 10 characters of Crockford base32 (`agent_token.rs:53-62`).
    pub fn code_hash(code: &str) -> Option<String> {
        let _ = code;
        todo!("BT-1: DeviceSecret::code_hash")
    }

    /// The lower-case hex SHA-256 of `text`'s UTF-8 bytes (`agent_token.rs:43-48`).
    pub fn hash(text: &str) -> String {
        let _ = text;
        todo!("BT-1: DeviceSecret::hash")
    }
}
