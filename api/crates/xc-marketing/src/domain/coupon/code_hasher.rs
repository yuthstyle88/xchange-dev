//! `CouponCodeHasher` (design D-9): HMAC-SHA256 of the normalised code, keyed from the PII key file
//! (`xc-kernel::pii`, HKDF `info = "xc-coupon-v1"`), so `coupons` stores only `code_hash`.
//!
//! Up-front scaffolding (MK-3 #1395): signatures the provided tests pin, with `todo!()` bodies.

use sha2::{Digest, Sha256};
use xc_kernel::pii::PiiKeys;

pub struct CouponCodeHasher {
    key: [u8; 32],
}

impl CouponCodeHasher {
    /// The hasher for this PII key file (D-9: no second key file).
    pub fn new(keys: &PiiKeys) -> Self {
        Self {
            key: keys.derive_key(b"xc-coupon-v1"),
        }
    }

    /// D-9: upper-case, no spaces.
    pub fn normalize(code: &str) -> String {
        code.chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| c.to_ascii_uppercase())
            .collect()
    }

    /// HMAC-SHA256 of [`CouponCodeHasher::normalize`]`(code)`.
    pub fn hash(&self, code: &str) -> [u8; 32] {
        let normalized = Self::normalize(code);
        let mut k_ipad = [0x36u8; 64];
        let mut k_opad = [0x5cu8; 64];
        for i in 0..32 {
            k_ipad[i] ^= self.key[i];
            k_opad[i] ^= self.key[i];
        }

        let mut inner = Sha256::new();
        inner.update(k_ipad);
        inner.update(normalized.as_bytes());
        let inner_hash = inner.finalize();

        let mut outer = Sha256::new();
        outer.update(k_opad);
        outer.update(inner_hash);
        outer.finalize().into()
    }
}
