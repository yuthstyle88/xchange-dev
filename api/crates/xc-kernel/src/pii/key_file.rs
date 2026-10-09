//! Key file parsing and loading for PII encryption and lookup hash.

use base64::prelude::*;
use std::collections::HashSet;
use std::path::Path;

use crate::pii::PiiError;

pub struct PiiKeys {
    /// Ordered list of (key_id, 32-byte key)
    keys: Vec<(String, [u8; 32])>,
}

impl PiiKeys {
    /// Parses key file contents.
    /// Format: lines of `key_id:base64(32 bytes)`. Blank lines and `#` comments ignored.
    /// The last valid line is the newest key (used for encrypting).
    /// The first valid line is the oldest key (used for lookup hash derivation).
    pub fn parse(input: &str) -> Result<Self, PiiError> {
        let mut keys = Vec::new();
        let mut seen_ids = HashSet::new();

        for (line_idx, line) in input.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let (id, b64) = trimmed.split_once(':').ok_or_else(|| {
                PiiError::KeyFile(format!("missing ':' on line {}", line_idx + 1))
            })?;

            let id = id.trim();
            if id.is_empty() {
                return Err(PiiError::KeyFile(format!(
                    "empty key id on line {}",
                    line_idx + 1
                )));
            }

            if !seen_ids.insert(id.to_string()) {
                return Err(PiiError::KeyFile(format!(
                    "duplicate key id '{id}' on line {}",
                    line_idx + 1
                )));
            }

            let decoded = BASE64_STANDARD.decode(b64.trim()).map_err(|e| {
                PiiError::KeyFile(format!("invalid base64 on line {}: {e}", line_idx + 1))
            })?;

            let key_bytes: [u8; 32] = decoded.try_into().map_err(|_| {
                PiiError::KeyFile(format!("key on line {} must be 32 bytes", line_idx + 1))
            })?;

            keys.push((id.to_string(), key_bytes));
        }

        if keys.is_empty() {
            return Err(PiiError::KeyFile("no keys found in key file".to_string()));
        }

        Ok(Self { keys })
    }

    /// Loads and parses keys from a file path.
    pub fn load(path: &Path) -> Result<Self, PiiError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            PiiError::KeyFile(format!("failed to read key file {}: {e}", path.display()))
        })?;
        Self::parse(&content)
    }

    /// Returns the current (newest, last) key ID used for encryption.
    pub fn current_id(&self) -> &str {
        &self.keys.last().expect("keys is non-empty").0
    }

    /// Returns the current (newest, last) key ID and 32-byte key.
    pub(crate) fn current_key(&self) -> (&str, &[u8; 32]) {
        let last = self.keys.last().expect("keys is non-empty");
        (&last.0, &last.1)
    }

    /// Returns the oldest (first) 32-byte key used for hash derivation.
    pub(crate) fn oldest_key(&self) -> &[u8; 32] {
        &self.keys.first().expect("keys is non-empty").1
    }

    /// Returns the key bytes for the given key ID, if present.
    pub(crate) fn get_key(&self, key_id: &str) -> Option<&[u8; 32]> {
        self.keys
            .iter()
            .find(|(id, _)| id == key_id)
            .map(|(_, k)| k)
    }
}

impl std::fmt::Debug for PiiKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let key_ids: Vec<&str> = self.keys.iter().map(|(id, _)| id.as_str()).collect();
        f.debug_struct("PiiKeys")
            .field("key_ids", &key_ids)
            .field("current_id", &self.current_id())
            .finish()
    }
}
