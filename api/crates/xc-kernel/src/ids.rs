//! Identifiers used across xchange.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BranchId(pub String);

impl BranchId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether `text` has the shape of a branch name: 1 to 70 characters of `A-Za-z0-9-`
    /// (v2 `server.mjs:6999`). It does not say whether such a branch exists.
    pub fn is_well_formed(text: &str) -> bool {
        (1..=70).contains(&text.len())
            && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    }
}

impl std::fmt::Display for BranchId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for BranchId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
