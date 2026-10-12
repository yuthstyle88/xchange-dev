//! `CampaignRepository` (design §6). Every method runs in the caller's transaction.

use futures::future::BoxFuture;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::campaign::Campaign;
use super::version::CampaignVersion;

/// Repository errors are database errors only (ticket #1390 §8).
#[derive(Debug, thiserror::Error)]
pub enum CampaignRepositoryError {
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
}

pub trait CampaignRepository: Send + Sync {
    /// Inserts a `campaigns` row.
    fn insert_campaign<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        campaign: &'a Campaign,
    ) -> BoxFuture<'a, Result<(), CampaignRepositoryError>>;

    /// Inserts a new `campaign_versions` row.
    fn insert_version<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        version: &'a CampaignVersion,
    ) -> BoxFuture<'a, Result<(), CampaignRepositoryError>>;

    /// Writes an existing version row in place (same id, same version number).
    fn save_version<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        version: &'a CampaignVersion,
    ) -> BoxFuture<'a, Result<(), CampaignRepositoryError>>;

    fn find_campaign<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        id: Uuid,
    ) -> BoxFuture<'a, Result<Option<Campaign>, CampaignRepositoryError>>;

    fn find_version<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        id: Uuid,
    ) -> BoxFuture<'a, Result<Option<CampaignVersion>, CampaignRepositoryError>>;

    /// Approves `version_id`: it becomes APPROVED, the campaign's previous APPROVED or PAUSED version
    /// becomes SUPERSEDED, and `campaigns.current_version_id` moves to it — all in `tx` (D-3).
    fn approve<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        version_id: Uuid,
    ) -> BoxFuture<'a, Result<(), CampaignRepositoryError>>;
}
