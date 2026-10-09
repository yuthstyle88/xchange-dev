use ring::aead::{
    AES_256_GCM, BoundKey, NONCE_LEN, Nonce, NonceSequence, OpeningKey, SealingKey, UnboundKey,
};
use ring::hkdf;
use ring::rand::{SecureRandom, SystemRandom};

struct OneNonce(Option<Nonce>);
impl NonceSequence for OneNonce {
    fn advance(&mut self) -> Result<Nonce, ring::error::Unspecified> {
        self.0.take().ok_or(ring::error::Unspecified)
    }
}

pub fn derive_refresh_token_key(client_secret: &str) -> [u8; 32] {
    let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, &[]);
    let prk = salt.extract(client_secret.as_bytes());
    struct Key;
    impl hkdf::KeyType for Key {
        fn len(&self) -> usize {
            32
        }
    }
    let mut okm = [0u8; 32];
    let info = [b"xc-v3 refresh-token v1".as_slice()];
    let okm_out = prk.expand(&info, Key).expect("hkdf expand");
    okm_out.fill(&mut okm).expect("hkdf fill");
    okm
}

pub fn encrypt_refresh_token(
    client_secret: &str,
    plaintext: &str,
) -> Result<Vec<u8>, ring::error::Unspecified> {
    let key_bytes = derive_refresh_token_key(client_secret);
    let unbound_key = UnboundKey::new(&AES_256_GCM, &key_bytes)?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    let rng = SystemRandom::new();
    rng.fill(&mut nonce_bytes)?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);
    let mut sealing_key = SealingKey::new(unbound_key, OneNonce(Some(nonce)));

    let mut in_out = plaintext.as_bytes().to_vec();
    sealing_key.seal_in_place_append_tag(ring::aead::Aad::empty(), &mut in_out)?;

    let mut result = Vec::with_capacity(NONCE_LEN + in_out.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&in_out);
    Ok(result)
}

pub fn decrypt_refresh_token(
    client_secret: &str,
    ciphertext_with_nonce: &[u8],
) -> Result<String, ring::error::Unspecified> {
    if ciphertext_with_nonce.len() < NONCE_LEN + AES_256_GCM.tag_len() {
        return Err(ring::error::Unspecified);
    }
    let (nonce_bytes, ciphertext_and_tag) = ciphertext_with_nonce.split_at(NONCE_LEN);
    let key_bytes = derive_refresh_token_key(client_secret);
    let unbound_key = UnboundKey::new(&AES_256_GCM, &key_bytes)?;
    let mut nonce_arr = [0u8; NONCE_LEN];
    nonce_arr.copy_from_slice(nonce_bytes);
    let nonce = Nonce::assume_unique_for_key(nonce_arr);
    let mut opening_key = OpeningKey::new(unbound_key, OneNonce(Some(nonce)));

    let mut in_out = ciphertext_and_tag.to_vec();
    let decrypted = opening_key.open_in_place(ring::aead::Aad::empty(), &mut in_out)?;
    String::from_utf8(decrypted.to_vec()).map_err(|_| ring::error::Unspecified)
}
