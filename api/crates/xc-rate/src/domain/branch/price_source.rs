//! Per-branch price source domain model and port (§3.5, R-7).

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_kernel::ids::BranchId;

use crate::domain::draft::rate_draft::DraftId;
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::publication::published_rates::PublicationRef;

/// The price source feeding a branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PriceSource {
    /// Rates are published from Bank of Thailand draft rates.
    BotDraft,
    /// Rates are fed directly from the central rate board.
    RateBoard,
}

impl PriceSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BotDraft => "BOT_DRAFT",
            Self::RateBoard => "RATE_BOARD",
        }
    }
}

impl std::fmt::Display for PriceSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for PriceSource {
    type Err = RateEngineError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "BOT_DRAFT" => Ok(Self::BotDraft),
            "RATE_BOARD" => Ok(Self::RateBoard),
            other => Err(RateEngineError::ValidationFailed(format!(
                "invalid price source: {other}"
            ))),
        }
    }
}

/// Port for querying and switching branch price source.
pub trait BranchPriceSource: Send + Sync {
    /// Queries the current price source of a branch.
    fn source_of<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, Result<PriceSource, RateEngineError>>;

    /// Switches the branch price source.
    ///
    /// OWNER only. A switch from `RATE_BOARD` to `BOT_DRAFT` is refused here;
    /// `switch_back` must be used instead so that a draft is published simultaneously.
    fn set<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        who: &'a Actor,
        branch: &'a BranchId,
        to: PriceSource,
    ) -> BoxFuture<'a, Result<(), RateEngineError>>;

    /// Switches the branch price source back to `BOT_DRAFT` and publishes an approved BoT draft
    /// in the caller's transaction.
    ///
    /// OWNER only. If the draft publish fails, the caller's rollback leaves the branch on `RATE_BOARD`.
    fn switch_back<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        who: &'a Actor,
        branch: &'a BranchId,
        draft: &'a DraftId,
        expected_version: Option<i32>,
    ) -> BoxFuture<'a, Result<PublicationRef, RateEngineError>>;
}
