//! AES-256-GCM cipher for PII encryption and decryption.

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use rand::Rng;
use std::sync::Arc;

use crate::pii::{PiiError, PiiKeys};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub key_id: String,
    pub bytes: Vec<u8>,
}

pub struct PiiCipher {
    keys: Arc<PiiKeys>,
}

impl PiiCipher {
    pub fn new(keys: Arc<PiiKeys>) -> Self {
        Self { keys }
    }

    pub fn encrypt(&self, aad: &[u8], plain: &[u8]) -> Sealed {
        let (key_id, key_bytes) = self.keys.current_key();
        let cipher = Aes256Gcm::new(&(*key_bytes).into());

        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill(&mut nonce_bytes);
        let nonce = Nonce::from(nonce_bytes);

        let payload = Payload { msg: plain, aad };
        let ciphertext_and_tag = cipher
            .encrypt(&nonce, payload)
            .expect("AES-GCM encryption should not fail");

        let mut bytes = Vec::with_capacity(12 + ciphertext_and_tag.len());
        bytes.extend_from_slice(&nonce_bytes);
        bytes.extend_from_slice(&ciphertext_and_tag);

        Sealed {
            key_id: key_id.to_string(),
            bytes,
        }
    }

    pub fn decrypt(&self, aad: &[u8], sealed: &Sealed) -> Result<Vec<u8>, PiiError> {
        let key_bytes = self
            .keys
            .get_key(&sealed.key_id)
            .ok_or_else(|| PiiError::UnknownKey(sealed.key_id.clone()))?;

        // nonce (12) + tag (16) = minimum 28 bytes
        if sealed.bytes.len() < 12 + 16 {
            return Err(PiiError::Decrypt);
        }

        let (nonce_slice, ct_and_tag) = sealed.bytes.split_at(12);
        let nonce_arr: &[u8; 12] = nonce_slice.try_into().map_err(|_| PiiError::Decrypt)?;
        let nonce = Nonce::from(*nonce_arr);
        let cipher = Aes256Gcm::new(&(*key_bytes).into());

        let payload = Payload {
            msg: ct_and_tag,
            aad,
        };

        cipher
            .decrypt(&nonce, payload)
            .map_err(|_| PiiError::Decrypt)
    }
}
