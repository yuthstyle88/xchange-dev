//! Shared branch-organisation scope check for marketing campaign commands (#2031,
//! design `docs/design/blueprint/marketing-campaigns.md` §6).

use uuid::Uuid;

pub struct CampaignBranchScope;

impl CampaignBranchScope {
    /// The first id of `branch_ids`, in input order, that is not a branch of `organization_id`
    /// (an unknown id and another organisation's branch alike). `None` when every id is the organisation's,
    /// including an empty slice.
    pub async fn first_outside_org_in(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        organization_id: Uuid,
        branch_ids: &[String],
    ) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT t.b \
             FROM unnest($1::text[]) WITH ORDINALITY AS t(b, n) \
             WHERE NOT EXISTS ( \
                 SELECT 1 FROM branches WHERE id = t.b AND organization_id = $2 \
             ) \
             ORDER BY t.n \
             LIMIT 1",
        )
        .bind(branch_ids)
        .bind(organization_id)
        .fetch_optional(&mut **tx)
        .await
    }
}
