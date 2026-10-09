//! Keyed lookup hasher using HKDF-SHA256 and HMAC-SHA256.

use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::pii::{IdentityType, PiiKeys};

type HmacSha256 = Hmac<Sha256>;

pub struct LookupHasher {
    derived_key: [u8; 32],
}

impl LookupHasher {
    pub fn new(keys: &PiiKeys) -> Self {
        let oldest = keys.oldest_key();
        let hk = Hkdf::<Sha256>::new(None, oldest);
        let mut derived_key = [0u8; 32];
        hk.expand(b"xc-pii-lookup-v1", &mut derived_key)
            .expect("32 bytes is valid expand length for HKDF-SHA256");

        Self { derived_key }
    }

    pub fn hash(&self, t: IdentityType, normalized: &str) -> [u8; 32] {
        let mut mac =
            HmacSha256::new_from_slice(&self.derived_key).expect("HMAC can take key of any size");
        mac.update(t.as_str().as_bytes());
        mac.update(&[0x1f]); // Unit separator
        mac.update(normalized.as_bytes());
        mac.finalize().into_bytes().into()
    }
}
