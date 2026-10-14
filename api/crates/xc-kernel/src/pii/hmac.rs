//! HMAC-SHA-256 over concatenated byte slices.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC can take key of any size");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}
