//! Test proving CouponCodeHasher is rotation-stable across PII key rotations (ticket #1593, design D-9).
//!
//! A coupon code hashed with key file v1 matches the same code hashed with key file v1 plus a newer
//! rotated key v2, because the derivation uses `xc_kernel::pii::derive_subkey` (anchored on the
//! oldest key in the file, like `lookup_hash`), not the newest key.

use xc_kernel::pii::PiiKeys;
use xc_marketing::domain::coupon::code_hasher::CouponCodeHasher;

const KEYS_V1: &str = "v1:BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";
const KEYS_V2_ROTATED: &str = "\
v1:BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=
v2:CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk=";

#[test]
fn coupon_hash_survives_pii_key_rotation() {
    let pii_v1 = PiiKeys::parse(KEYS_V1).expect("parse keys v1");
    let pii_v2 = PiiKeys::parse(KEYS_V2_ROTATED).expect("parse keys v2 rotated");

    let hasher_v1 = CouponCodeHasher::new(&pii_v1);
    let hasher_v2 = CouponCodeHasher::new(&pii_v2);

    let code = "SAVE20";

    // 1. Same code, same key file => same hash (deterministic)
    assert_eq!(
        hasher_v1.hash(code),
        hasher_v1.hash(code),
        "same code with same key file produces same hash"
    );

    // 2. Same code, key file rotated (newer key appended) => same hash (rotation-stable)
    let hash_v1 = hasher_v1.hash(code);
    let hash_v2 = hasher_v2.hash(code);
    assert_eq!(
        hash_v1, hash_v2,
        "a coupon hashed with key file v1 must match the same code hashed after rotation (v1 + v2)"
    );

    // 3. Different code => different hash
    assert_ne!(
        hasher_v2.hash("SAVE20"),
        hasher_v2.hash("SAVE30"),
        "different codes produce different hashes"
    );

    // 4. Normalization (spaces, case) => unchanged
    assert_eq!(
        hasher_v2.hash(" save 20 "),
        hasher_v2.hash("SAVE20"),
        "normalization ignores spaces and case"
    );
    assert_eq!(CouponCodeHasher::normalize(" save 20 "), "SAVE20");
}
