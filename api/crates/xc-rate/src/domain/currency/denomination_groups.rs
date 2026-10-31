//! Denomination groups and board mappings domain port (§2, §3.2).

use std::fmt;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use xc_kernel::money::CurrencyCode;

use crate::domain::draft::rate_engine_error::RateEngineError;

/// A banknote denomination group code (1-32 uppercase ASCII letters, digits, and underscores).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GroupCode(String);

impl GroupCode {
    pub fn new(text: impl Into<String>) -> Result<Self, String> {
        let s = text.into();
        if s.is_empty() || s.len() > 32 {
            return Err("GroupCode must be between 1 and 32 characters".to_string());
        }
        if !s
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(
                "GroupCode must contain only uppercase ASCII letters, digits, and underscores"
                    .to_string(),
            );
        }
        Ok(Self(s))
    }

    pub fn all() -> Self {
        Self("ALL".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GroupCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub trait DenominationGroups: Send + Sync {
    /// Distinct group codes for a currency, sorted ascending.
    /// Every organisation's rows; production code uses groups_in / source_in (#5760).
    fn groups_of<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        code: CurrencyCode,
    ) -> BoxFuture<'a, Result<Vec<GroupCode>, RateEngineError>>;

    /// Distinct group codes for a currency in `organization_id`, sorted ascending.
    fn groups_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: &'a str,
        code: CurrencyCode,
    ) -> BoxFuture<'a, Result<Vec<GroupCode>, RateEngineError>>;

    /// The board row mapping (instrument_key) for a group and provider, or None if unmapped.
    /// Every organisation's rows; production code uses groups_in / source_in (#5760).
    fn source_of<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        code: CurrencyCode,
        group: &'a GroupCode,
        provider: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, RateEngineError>>;

    /// The board row mapping (instrument_key) for a group and provider in `organization_id`, or None if unmapped.
    fn source_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: &'a str,
        code: CurrencyCode,
        group: &'a GroupCode,
        provider: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, RateEngineError>>;
}
