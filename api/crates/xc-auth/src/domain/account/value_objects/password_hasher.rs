use argon2::password_hash::{PasswordHash, PasswordHasher as _, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use pos_error::{AppError, AppResult};

const MIN_UTF16: usize = 12;
const MAX_UTF16: usize = 128;
const DEFAULT_MEMORY_KIB: u32 = 19_456;
const DEFAULT_ITERATIONS: u32 = 2;
const DEFAULT_PARALLELISM: u32 = 1;
/// What a spare verification "tries".
const SPARE_ATTEMPT: &str = "spare verification";
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

    /// False for a wrong password, a password longer than 128 units, and any stored value that is
    /// not an Argon2id PHC string. The cost is read from `stored`, not from `self`. Never panics.
    pub fn verify(&self, password: &str, stored: &str) -> bool {
        Self::verifiable(password, stored).is_some_and(|hash| Self::run(password, &hash))
    }

    /// `verify` for a sign-in: the same answer, after exactly one verification
    /// whatever was sent. Where `verify` would answer without doing the work —
    /// no password, one longer than 128 units, no account (`stored` is `None`),
    /// a stored value that is no Argon2id hash — the work is done against
    /// `spare` instead and the answer is false. `spare` can never grant
    /// anything: its result is thrown away.
    pub fn verify_at_full_cost(
        &self,
        password: Option<&str>,
        stored: Option<&str>,
        spare: &str,
    ) -> bool {
        if let (Some(password), Some(stored)) = (password, stored)
            && let Some(hash) = Self::verifiable(password, stored)
        {
            return Self::run(password, &hash);
        }
        if let Some(hash) = Self::verifiable(SPARE_ATTEMPT, spare) {
            let _ = Self::run(SPARE_ATTEMPT, &hash);
        }
        false
    }

    /// The parsed hash when `password` and `stored` are something Argon2id can be run on.
    fn verifiable<'a>(password: &str, stored: &'a str) -> Option<PasswordHash<'a>> {
        if password.encode_utf16().count() > MAX_UTF16 {
            return None;
        }
        PasswordHash::new(stored)
            .ok()
            .filter(|hash| hash.algorithm == Algorithm::Argon2id.ident())
    }

    /// The Argon2id computation itself, at the cost written in `hash`.
    fn run(password: &str, hash: &PasswordHash<'_>) -> bool {
        Argon2::default()
            .verify_password(password.as_bytes(), hash)
            .is_ok()
    }
}

impl Default for PasswordHasher {
    fn default() -> Self {
        Self::new()
    }
}
