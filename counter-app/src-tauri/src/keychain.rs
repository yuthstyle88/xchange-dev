//! CA-3c. Where the agent token lives: the macOS Keychain, never a file (design §2).

use std::sync::Mutex;

use security_framework::passwords::{
    delete_generic_password, get_generic_password, set_generic_password,
};

/// The Keychain service name of the agent token.
pub const SERVICE: &str = "cloud.spacing.xc-counter";

const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

/// Why the store did not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenError(pub String);

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "token store: {}", self.0)
    }
}

impl std::error::Error for TokenError {}

/// Keeps one agent token.
pub trait TokenStore: Send + Sync {
    /// `Ok(None)` when no token is stored.
    fn load(&self) -> Result<Option<String>, TokenError>;
    /// Replaces any stored token.
    fn save(&self, token: &str) -> Result<(), TokenError>;
    /// Clearing an empty store is fine.
    fn clear(&self) -> Result<(), TokenError>;
}

/// The real store: service [`SERVICE`], account the agent id or `"token"`.
pub struct KeychainTokenStore {
    pub account: String,
}

impl KeychainTokenStore {
    pub fn new(account: impl Into<String>) -> Self {
        Self {
            account: account.into(),
        }
    }
}

fn os_status_error(err: security_framework::base::Error) -> TokenError {
    TokenError(format!("OSStatus {}: {err}", err.code()))
}

impl TokenStore for KeychainTokenStore {
    fn load(&self) -> Result<Option<String>, TokenError> {
        match get_generic_password(SERVICE, &self.account) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|e| TokenError(e.to_string())),
            Err(err) if err.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(err) => Err(os_status_error(err)),
        }
    }

    fn save(&self, token: &str) -> Result<(), TokenError> {
        set_generic_password(SERVICE, &self.account, token.as_bytes()).map_err(os_status_error)
    }

    fn clear(&self) -> Result<(), TokenError> {
        match delete_generic_password(SERVICE, &self.account) {
            Ok(()) => Ok(()),
            Err(err) if err.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(err) => Err(os_status_error(err)),
        }
    }
}

/// A store in memory, for tests. CA-3c gives it its fields.
#[derive(Default)]
pub struct MemoryTokenStore {
    token: Mutex<Option<String>>,
}

impl TokenStore for MemoryTokenStore {
    fn load(&self) -> Result<Option<String>, TokenError> {
        self.token
            .lock()
            .map(|guard| guard.clone())
            .map_err(|e| TokenError(e.to_string()))
    }

    fn save(&self, token: &str) -> Result<(), TokenError> {
        self.token
            .lock()
            .map(|mut guard| {
                *guard = Some(token.to_owned());
            })
            .map_err(|e| TokenError(e.to_string()))
    }

    fn clear(&self) -> Result<(), TokenError> {
        self.token
            .lock()
            .map(|mut guard| {
                *guard = None;
            })
            .map_err(|e| TokenError(e.to_string()))
    }
}
