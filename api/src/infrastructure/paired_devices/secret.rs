//! `DeviceSecret` (design `kiosk-shared-phone.md` §7.5): the device's two secrets, the one-time
//! pairing code and the token, and the one hash both are stored as. `AgentToken`
//! (`device_agents/agent_token.rs`) generalised by kind; BT-2 turns `AgentToken` into thin calls
//! into it, so `AgentToken`'s callers and tests do not change.
//!
//! Filled by BT-1. `api/tests/paired_device_core_test.rs` pins it.

use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::kind::DeviceKind;

/// Crockford base32: `0-9 A-Z` without `I L O U`.
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Characters in a pairing code, without its dash.
const CODE_LEN: usize = 10;

/// Builds and hashes the paired devices' secrets. Holds nothing.
pub struct DeviceSecret;

impl DeviceSecret {
    /// `kind.token_prefix()` and the 64 lower-case hex digits of two UUID v4 values
    /// (`agent_token.rs:17-24` with the prefix of `kind`).
    pub fn new_token(kind: DeviceKind) -> String {
        format!(
            "{}{}{}",
            kind.token_prefix(),
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        )
    }

    /// 10 characters of Crockford base32 (`0-9 A-Z` without `I L O U`, 50 random bits), shown as
    /// `XXXXX-XXXXX` (`agent_token.rs:28-40`).
    pub fn new_pairing_code() -> String {
        // The low 62 bits of a v4 UUID are random; take 50 of them.
        let bits = Uuid::new_v4().as_u128() & ((1u128 << 50) - 1);
        let mut code = String::with_capacity(CODE_LEN + 1);
        for i in 0..CODE_LEN {
            if i == CODE_LEN / 2 {
                code.push('-');
            }
            let shift = 5 * (CODE_LEN - 1 - i);
            code.push(CROCKFORD[((bits >> shift) & 31) as usize] as char);
        }
        code
    }

    /// The hash a pairing code is stored and looked up by: [`DeviceSecret::hash`] of its 10
    /// characters with `-` and white space removed and letters upper-cased. `None` when what is
    /// left is not 10 characters of Crockford base32 (`agent_token.rs:53-62`).
    pub fn code_hash(code: &str) -> Option<String> {
        let canonical: String = code
            .chars()
            .filter(|c| *c != '-' && !c.is_whitespace())
            .map(|c| c.to_ascii_uppercase())
            .collect();
        let valid =
            canonical.len() == CODE_LEN && canonical.bytes().all(|b| CROCKFORD.contains(&b));
        valid.then(|| Self::hash(&canonical))
    }

    /// The lower-case hex SHA-256 of `text`'s UTF-8 bytes (`agent_token.rs:43-48`).
    pub fn hash(text: &str) -> String {
        Sha256::digest(text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}
