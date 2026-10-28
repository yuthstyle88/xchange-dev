//! `PgVersionLabels` (design `marketing-campaigns.md` §6.4.4, BE-3): the customer-facing label and end of a
//! campaign version, for the indicative estimate.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// A campaign version's label (the campaign's `name`, as `my_benefits`) and its `ends_at`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionLabel {
    pub name: String,
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PgVersionLabels;

impl PgVersionLabels {
    pub fn new() -> Self {
        Self
    }

    /// The label of each of `version_ids` that belongs to `organization_id`; a version of another
    /// organisation is absent from the map.
    pub async fn labels_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        version_ids: &[Uuid],
    ) -> AppResult<HashMap<Uuid, VersionLabel>> {
        if version_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let rows: Vec<(Uuid, String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT v.id, c.name, v.ends_at \
             FROM campaign_versions v \
             JOIN campaigns c ON c.id = v.campaign_id \
             WHERE v.organization_id = $1 AND v.id = ANY($2)",
        )
        .bind(organization_id)
        .bind(version_ids)
        .fetch_all(&mut **tx)
        .await
        .map_err(AppError::Database)?;

        Ok(rows
            .into_iter()
            .map(|(id, name, ends_at)| (id, VersionLabel { name, ends_at }))
            .collect())
    }
}
