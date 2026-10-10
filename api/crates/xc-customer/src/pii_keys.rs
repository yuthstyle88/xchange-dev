//! PII keys registry and installation.

use pos_error::{AppError, AppResult};
use std::sync::{Arc, RwLock};
use xc_kernel::pii::{IdentityType, LookupHasher, PiiCipher, PiiKeys};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("PII keys already installed with different keys")]
pub struct PiiKeysConflict;

static INSTALLED_KEYS: RwLock<Option<Arc<PiiKeys>>> = RwLock::new(None);

fn are_keys_equal(a: &Arc<PiiKeys>, b: &Arc<PiiKeys>) -> bool {
    if Arc::ptr_eq(a, b) {
        return true;
    }
    if format!("{a:?}") != format!("{b:?}") {
        return false;
    }
    let hasher_a = LookupHasher::new(a);
    let hasher_b = LookupHasher::new(b);
    if hasher_a.hash(IdentityType::Passport, "equality-check-probe")
        != hasher_b.hash(IdentityType::Passport, "equality-check-probe")
    {
        return false;
    }
    let cipher_a = PiiCipher::new(a.clone());
    let cipher_b = PiiCipher::new(b.clone());
    let probe = b"equality-check-probe";
    let aad = b"probe-aad";
    let sealed_a = cipher_a.encrypt(aad, probe);
    if cipher_b.decrypt(aad, &sealed_a).as_deref() != Ok(probe.as_slice()) {
        return false;
    }
    let sealed_b = cipher_b.encrypt(aad, probe);
    if cipher_a.decrypt(aad, &sealed_b).as_deref() != Ok(probe.as_slice()) {
        return false;
    }
    true
}

pub fn install_pii_keys(keys: Arc<PiiKeys>) -> Result<(), PiiKeysConflict> {
    let mut lock = INSTALLED_KEYS.write().expect("lock not poisoned");
    if let Some(existing) = &*lock {
        if are_keys_equal(existing, &keys) {
            Ok(())
        } else {
            Err(PiiKeysConflict)
        }
    } else {
        *lock = Some(keys);
        Ok(())
    }
}

pub fn get_pii_keys() -> AppResult<Arc<PiiKeys>> {
    let lock = INSTALLED_KEYS.read().expect("lock not poisoned");
    lock.as_ref()
        .cloned()
        .ok_or_else(|| AppError::Internal("PII keys not installed".to_string()))
}
