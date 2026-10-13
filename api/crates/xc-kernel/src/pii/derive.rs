//! Subkey derivation from PII keys using HKDF-SHA256.

use hkdf::Hkdf;
use sha2::Sha256;

use crate::pii::PiiKeys;

/// Derives a 32-byte subkey using HKDF-SHA256 over `keys.oldest_key()` and the provided info.
pub fn derive_subkey(keys: &PiiKeys, info: &[u8]) -> [u8; 32] {
    let oldest = keys.oldest_key();
    let hk = Hkdf::<Sha256>::new(None, oldest);
    let mut subkey = [0u8; 32];
    hk.expand(info, &mut subkey)
        .expect("32 bytes is valid expand length for HKDF-SHA256");
    subkey
}
