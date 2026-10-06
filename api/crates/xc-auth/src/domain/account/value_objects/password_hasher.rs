use argon2::password_hash::{PasswordHash, PasswordHasher as _, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use pos_error::{AppError, AppResult};

const MIN_UTF16: usize = 12;
const MAX_UTF16: usize = 128;
const DEFAULT_MEMORY_KIB: u32 = 19_456;
const DEFAULT_ITERATIONS: u32 = 2;
const DEFAULT_PARALLELISM: u32 = 1;
const PASSWORD_LENGTH_MSG: &str = "รหัสผ่านต้องมี 12–128 ตัวอักษร";

#[derive(Debug, Clone)]
pub struct PasswordHasher {
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
}

impl PasswordHasher {
    pub fn new() -> Self {
        Self {
            memory_kib: DEFAULT_MEMORY_KIB,
            iterations: DEFAULT_ITERATIONS,
            parallelism: DEFAULT_PARALLELISM,
        }
    }

    pub fn with_cost(memory_kib: u32, iterations: u32, parallelism: u32) -> Self {
        Self {
            memory_kib,
            iterations,
            parallelism,
        }
    }

    pub fn hash(&self, password: &str) -> AppResult<String> {
        let utf16_len = password.encode_utf16().count();
        if !(MIN_UTF16..=MAX_UTF16).contains(&utf16_len) {
            return Err(AppError::Domain(PASSWORD_LENGTH_MSG.to_string()));
        }

        let params = Params::new(self.memory_kib, self.iterations, self.parallelism, None)
            .map_err(|e| AppError::Domain(e.to_string()))?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);

        let hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| AppError::Domain(e.to_string()))?
            .to_string();

        Ok(hash)
    }

    pub fn verify(&self, password: &str, stored: &str) -> bool {
        if password.encode_utf16().count() > MAX_UTF16 {
            return false;
        }

        let parsed_hash = match PasswordHash::new(stored) {
            Ok(h) => h,
            Err(_) => return false,
        };

        if parsed_hash.algorithm != Algorithm::Argon2id.ident() {
            return false;
        }

        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok()
    }
}

impl Default for PasswordHasher {
    fn default() -> Self {
        Self::new()
    }
}
