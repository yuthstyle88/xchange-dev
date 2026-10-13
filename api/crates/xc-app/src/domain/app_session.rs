//! The anonymous app session (design P7): an opaque id, either random or, when a command
//! mints it, derived with a server key from that command's organisation, name and
//! `Idempotency-Key`, in the `HttpOnly; Secure; SameSite=Lax` cookie `xc_app_sid`.
//! Only its SHA-256 is stored (`planner_drafts.session_hash`).

/// The cookie that carries the app session id.
pub const APP_SESSION_COOKIE: &str = "xc_app_sid";

/// Random bytes in an id: 256 bits, written as 64 lower-case hex characters.
const ID_BYTES: usize = 32;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The raw session id, as the cookie carries it. Never stored and never logged.
#[derive(Clone, PartialEq, Eq)]
pub struct AppSessionId(String);

impl AppSessionId {
    /// A constructor from 32 raw bytes (e.g. derived HMAC-SHA256).
    pub fn from_bytes(bytes: [u8; ID_BYTES]) -> Self {
        Self(hex(&bytes))
    }

    /// A new random id (at least 128 bits of randomness).
    pub fn generate() -> Self {
        let bytes: [u8; ID_BYTES] = rand::random();
        Self::from_bytes(bytes)
    }

    /// Reads a cookie value. `None` when it is not an id `generate` could have made.
    pub fn parse(text: &str) -> Option<Self> {
        let well_formed = text.len() == ID_BYTES * 2
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        well_formed.then(|| Self(text.to_owned()))
    }

    /// The cookie value.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The stored form: lower-case hex SHA-256 of the id's bytes.
    pub fn hash(&self) -> SessionHash {
        use sha2::{Digest, Sha256};
        SessionHash(hex(&Sha256::digest(self.0.as_bytes())))
    }
}

impl std::fmt::Debug for AppSessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AppSessionId(…)")
    }
}

/// The SHA-256 of an app session id, as `planner_drafts.session_hash` stores it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionHash(String);

impl SessionHash {
    /// A stored hash read back from the database.
    pub fn from_stored(hex: impl Into<String>) -> Self {
        Self(hex.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The session of one request: its id (for the cookie) and its hash (for the rows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSession {
    pub id: AppSessionId,
    pub hash: SessionHash,
}
