use std::sync::Arc;

use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;
use xc_rate::application::draft::rate_draft_service::RateDraftService;
use xc_rate::application::publication::publish_service::PublishService;
use xc_rate::domain::draft::rate_draft::{DraftId, DraftStatus};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;

use super::feed_loop::{BoxFuture, BranchRefresh, RefreshOutcome};

pub struct DraftBranchRefresh {
    pub(crate) drafts: Arc<RateDraftService>,
    pub(crate) publish: Arc<PublishService>,
}

impl DraftBranchRefresh {
    pub fn new(drafts: Arc<RateDraftService>, publish: Arc<PublishService>) -> Self {
        Self { drafts, publish }
    }

    async fn do_refresh(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        batch: Uuid,
    ) -> Result<RefreshOutcome, RateEngineError> {
        // 1. Find the draft to copy.
        // Read branches.current_publication_id and that publication's draft_id
        let row: Option<(Option<Uuid>, i32)> = sqlx::query_as(
            "SELECT p.draft_id, p.version \
             FROM branches b \
             JOIN rate_publications p ON p.id = b.current_publication_id \
             WHERE b.id = $1",
        )
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let (published_draft_id, expected_version) = match row {
            Some((Some(draft_uuid), version)) => (DraftId::new(draft_uuid), Some(version)),
            _ => return Ok(RefreshOutcome::NothingToCopy),
        };

        // 2. Skip when up to date.
        // If a draft of this branch created by the SYSTEM actor already has a line
        // whose reference_line_id belongs to batch, answer UpToDate.
        let already_handled: bool = sqlx::query_scalar(
            "SELECT EXISTS ( \
                 SELECT 1 \
                 FROM rate_drafts d \
                 JOIN rate_draft_lines l ON l.draft_id = d.id \
                 JOIN rate_reference_lines r ON r.id = l.reference_line_id \
                 WHERE d.branch_id = $1 \
                   AND d.created_by = $2 \
                   AND r.batch_id = $3 \
             )",
        )
        .bind(branch.as_str())
        .bind("SYSTEM:rate-feed:BOT")
        .bind(batch)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if already_handled {
            return Ok(RefreshOutcome::UpToDate);
        }

        // 3. Copy.
        let system_actor = Actor::staff("SYSTEM:rate-feed:BOT", Role::System, branch.clone())
            .ok_or_else(|| {
                RateEngineError::Internal("failed to create system actor".to_string())
            })?;

        let draft = self
            .drafts
            .copy(tx, branch, &system_actor, &published_draft_id)
            .await?;

        // 4. Submit with key = "rate-feed:{branch}:{batch}"
        let submit_key = format!("rate-feed:{}:{batch}", branch.as_str());
        let submit_outcome = self
            .drafts
            .submit(tx, branch, &system_actor, &draft.id, &submit_key)
            .await?;

        match submit_outcome.status {
            DraftStatus::Approved => {
                // 5. Publish
                let pub_ref = self
                    .publish
                    .publish(tx, branch, &system_actor, &draft.id, expected_version)
                    .await?;
                Ok(RefreshOutcome::Published(pub_ref))
            }
            DraftStatus::PendingApproval => Ok(RefreshOutcome::AwaitingOwner(draft.id)),
            other => Err(RateEngineError::Internal(format!(
                "unexpected draft status after submit: {other:?}"
            ))),
        }
    }
}

impl BranchRefresh for DraftBranchRefresh {
    fn refresh<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        batch: Uuid,
    ) -> BoxFuture<'a, Result<RefreshOutcome, RateEngineError>> {
        Box::pin(self.do_refresh(tx, branch, batch))
    }
}
