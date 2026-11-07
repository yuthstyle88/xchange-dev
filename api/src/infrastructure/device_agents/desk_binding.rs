//! The desk nonce (xc-counter-app.md §5.4, §8.3): the server's proof, handed to the app's Rust side under the
//! agent token, that a desk page runs on counter `agent_id` of branch `branch_id`. The desk passes it to
//! `deskBindCounter` with its staff session; the server binds that session to the counter.
//!
//! Nothing is stored, so there is no table and it works with several API instances. A replay inside the 60 s only
//! binds the same counter again, which is harmless.
//!
//! Wire form `[required]`: `v1.<agentId>.<b64url(branchId)>.<exp unix seconds>.<b64url(mac)>`, the MAC
//! `HMAC-SHA256(derive_subkey(pii_keys, "xc-desk-binding-v1"), "v1" ‖ 0 ‖ agentId ‖ 0 ‖ branchId ‖ 0 ‖ exp)`,
//! compared in constant time (as `application::otp_challenge_seal`).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use xc_kernel::pii::PiiKeys;

const VERSION: &str = "v1";

/// The subkey's HKDF info `[required]`.
pub const SUBKEY_INFO: &[u8] = b"xc-desk-binding-v1";
/// How long a nonce is good for, in seconds `[required]`.
pub const TTL_SECS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeskNonce {
    /// The counter (a `device_agents.id`, lower case with dashes).
    pub agent_id: String,
    pub branch_id: String,
    /// Unix seconds; the nonce is good while `now < exp`.
    pub exp: i64,
}

/// Why a nonce was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonceRefusal {
    /// Not a value `seal` produced with these keys: bad shape, version, base64 or MAC.
    Invalid,
    /// Well formed and signed, but `now >= exp`.
    Expired,
}

impl NonceRefusal {
    /// The refusal code `deskBindCounter` answers (§8.4).
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "NONCE_INVALID",
            Self::Expired => "NONCE_EXPIRED",
        }
    }
}

impl DeskNonce {
    fn mac(keys: &PiiKeys, agent_id: &str, branch_id: &str, exp: i64) -> [u8; 32] {
        let subkey = xc_kernel::pii::derive_subkey(keys, SUBKEY_INFO);
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&subkey).expect("HMAC can take key of any length");
        mac.update(VERSION.as_bytes());
        mac.update(&[0]);
        mac.update(agent_id.as_bytes());
        mac.update(&[0]);
        mac.update(branch_id.as_bytes());
        mac.update(&[0]);
        mac.update(exp.to_string().as_bytes());
        mac.finalize().into_bytes().into()
    }

    /// A nonce for `agent_id` of `branch_id`, good for [`TTL_SECS`] from `now`.
    pub fn issue(agent_id: &str, branch_id: &str, now: DateTime<Utc>) -> Self {
        Self {
            agent_id: agent_id.to_owned(),
            branch_id: branch_id.to_owned(),
            exp: now.timestamp() + TTL_SECS,
        }
    }

    /// The wire form above. Deterministic for the same nonce and keys.
    pub fn seal(&self, keys: &PiiKeys) -> String {
        let branch_b64 = URL_SAFE_NO_PAD.encode(self.branch_id.as_bytes());
        let mac = Self::mac(keys, &self.agent_id, &self.branch_id, self.exp);
        let mac_b64 = URL_SAFE_NO_PAD.encode(mac);
        format!(
            "{VERSION}.{}.{branch_b64}.{}.{mac_b64}",
            self.agent_id, self.exp
        )
    }

    /// The nonce back, checked against these keys and `now`.
    pub fn open(value: &str, keys: &PiiKeys, now: DateTime<Utc>) -> Result<Self, NonceRefusal> {
        let mut parts = value.split('.');
        let (Some(version), Some(agent_id), Some(branch_b64), Some(exp_str), Some(mac_b64), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return Err(NonceRefusal::Invalid);
        };
        if version != VERSION || agent_id.is_empty() {
            return Err(NonceRefusal::Invalid);
        }
        let branch_bytes = URL_SAFE_NO_PAD
            .decode(branch_b64)
            .map_err(|_| NonceRefusal::Invalid)?;
        if URL_SAFE_NO_PAD.encode(&branch_bytes) != branch_b64 {
            return Err(NonceRefusal::Invalid);
        }
        let branch_id = String::from_utf8(branch_bytes).map_err(|_| NonceRefusal::Invalid)?;
        if branch_id.is_empty() {
            return Err(NonceRefusal::Invalid);
        }
        let exp: i64 = exp_str.parse().map_err(|_| NonceRefusal::Invalid)?;
        // The MAC covers `exp`'s canonical text, so `+123` or `0123` must not open as `123`.
        if exp.to_string() != exp_str {
            return Err(NonceRefusal::Invalid);
        }
        let mac_bytes = URL_SAFE_NO_PAD
            .decode(mac_b64)
            .map_err(|_| NonceRefusal::Invalid)?;
        if URL_SAFE_NO_PAD.encode(&mac_bytes) != mac_b64 {
            return Err(NonceRefusal::Invalid);
        }
        let expected = Self::mac(keys, agent_id, &branch_id, exp);
        if !bool::from(mac_bytes.as_slice().ct_eq(&expected)) {
            return Err(NonceRefusal::Invalid);
        }
        if now.timestamp() >= exp {
            return Err(NonceRefusal::Expired);
        }
        Ok(Self {
            agent_id: agent_id.to_owned(),
            branch_id,
            exp,
        })
    }
}
