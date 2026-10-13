//! `QrToken` (design §2 B7, §5): HMAC-derived token and its SHA-256 storage hash.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use xc_kernel::pii::PiiKeys;

type HmacSha256 = Hmac<Sha256>;

pub struct QrToken;

impl QrToken {
    /// Derive key from PII keys with HKDF (info = "xc-booking-qr-v1").
    pub fn key_from_pii(keys: &PiiKeys) -> [u8; 32] {
        xc_kernel::pii::derive_subkey(keys, b"xc-booking-qr-v1")
    }

    /// Derive: base64url(HMAC-SHA256(qr_key, booking_id.as_bytes() || qr_version.to_be_bytes())).
    pub fn derive(key: &[u8], id: &Uuid, version: u32) -> String {
        let mut mac = HmacSha256::new_from_slice(key).expect("HMAC can take key of any length");
        mac.update(id.as_bytes());
        mac.update(&version.to_be_bytes());
        let code = mac.finalize().into_bytes();
        URL_SAFE_NO_PAD.encode(code)
    }

    /// Hash: SHA-256(token). Only this hash is stored.
    pub fn hash(token: &str) -> Vec<u8> {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hasher.finalize().to_vec()
    }
}
