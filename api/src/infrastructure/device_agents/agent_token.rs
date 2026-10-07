//! The agent's two secrets: the one-time pairing code and the bearer token,
//! and the one hash both are stored as (design §5.4c *Identity*).

use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Crockford base32: `0-9 A-Z` without `I L O U`.
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Characters in a pairing code, without its dash.
const CODE_LEN: usize = 10;

/// Builds and hashes the agent's secrets. Holds nothing.
pub struct AgentToken;

impl AgentToken {
    /// `xcda_` and the 64 lower-case hex digits of two UUID v4 values.
    pub fn new_token() -> String {
        format!(
            "xcda_{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        )
    }

    /// 10 characters of Crockford base32 (50 bits of a UUID v4), shown as
    /// `XXXXX-XXXXX`.
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

    /// The lower-case hex SHA-256 of `text`'s UTF-8 bytes.
    pub fn hash(text: &str) -> String {
        Sha256::digest(text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// The hash a pairing code is stored and looked up by: `hash` of its 10
    /// characters with `-` and white space removed and letters upper-cased.
    /// `None` when what is left is not 10 characters of Crockford base32.
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
}
