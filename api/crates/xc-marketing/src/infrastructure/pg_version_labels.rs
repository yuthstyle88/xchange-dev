//! `PgVersionLabels` (design `marketing-campaigns.md` §6.4.4, BE-3): the customer-facing label and end of a
//! campaign version, for the indicative estimate.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use pos_error::AppResult;
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
        let _ = (tx, organization_id, version_ids);
        todo!("BE-3: PgVersionLabels::labels_in")
    }
}
