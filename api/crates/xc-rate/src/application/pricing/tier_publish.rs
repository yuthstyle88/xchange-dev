//! Publish one tier set at its branch (pricing-tiers.md §15.3 steps 4–6), through the one publication writer:
//! `RateDraftService::copy` the branch's current publication (an empty draft before the first one), replace the
//! currency's line with `MANUAL` tier 1 plus the 4 tier rows (`DraftLine::tiers`), `submit` (a line with tier rows is
//! in bound: B3), then `PublishService::publish` (4 tier lines, `source_snapshot.pricing = {tier_set_id, source}`).
//! Used by `pricingApprove` (§15.3), `BranchPush` (§15.4) and the Follow publisher (§15.5).
//!
//! SKELETON (XC-V9-PRICING): `TierSetPublisher::publish_in` is PV-b2's. Types and signatures stay.

use std::sync::Arc;

use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_auth::domain::access::Actor;
use xc_kernel::ids::BranchId;

use crate::application::draft::rate_draft_service::RateDraftService;
use crate::application::publication::publish_service::PublishService;
use crate::domain::draft::rate_draft::{DraftId, DraftLine, DraftStatus, LineSide};
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::tier_set::{PricingTierSet, TierSetStore};
use crate::domain::publication::published_rates::PublicationRef;
use crate::infrastructure::pricing::tier_set_store::PgTierSetStore;

/// What publishing a tier set wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedTierSet {
    pub draft_id: DraftId,
    pub publication: PublicationRef,
}

pub struct TierSetPublisher {
    drafts: Arc<RateDraftService>,
    publisher: Arc<PublishService>,
}

impl TierSetPublisher {
    pub fn new(drafts: Arc<RateDraftService>, publisher: Arc<PublishService>) -> Self {
        Self { drafts, publisher }
    }

    /// Steps 4–6 for `set` at `set.branch`, in the caller's transaction. `submit_key` is the draft submit's request
    /// key (the set's `request_id`). Any other line of the copied publication keeps today's evaluation; if one of
    /// them sends the draft to `PENDING_APPROVAL`, the answer is `RateEngineError::ApprovalRequired` and the caller
    /// rolls back.
    pub async fn publish_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        who: &Actor,
        set: &PricingTierSet,
        submit_key: &str,
    ) -> Result<PublishedTierSet, RateEngineError> {
        let branch = BranchId::new(&set.branch);
        let cur: Option<(Option<Uuid>, i32)> = sqlx::query_as(
            "SELECT p.draft_id, p.version \
             FROM branches b \
             JOIN rate_publications p ON p.id = b.current_publication_id \
             WHERE b.id = $1",
        )
        .bind(branch.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let mut tier_line = DraftLine::new(
            set.currency,
            LineSide::Manual(set.tiers[0].buy),
            LineSide::Manual(set.tiers[0].sell),
            Some("V.9 approved tiers".to_owned()),
        );
        tier_line.tiers = Some(set.tiers);

        let (expected_version, draft) = match cur {
            Some((Some(from_uuid), version)) => {
                let copied = self
                    .drafts
                    .copy(tx, &branch, who, &DraftId::new(from_uuid))
                    .await?;
                let mut lines = copied.lines;
                lines.retain(|l| l.currency != set.currency);
                lines.push(tier_line);
                let edited = self
                    .drafts
                    .edit(tx, &branch, who, &copied.id, copied.version, lines)
                    .await?;
                (Some(version), edited)
            }
            Some((None, _)) => {
                // A publication with no draft (only `FeedPublisher` writes one, and nothing wires it today) cannot be
                // copied; a draft of this line alone would publish a board with every other currency gone.
                return Err(RateEngineError::Internal(format!(
                    "branch {} publishes a publication with no draft; it cannot be copied",
                    branch.as_str()
                )));
            }
            None => {
                let created = self
                    .drafts
                    .create(tx, &branch, who, vec![tier_line])
                    .await?;
                (None, created)
            }
        };

        let draft_id = draft.id;
        PgTierSetStore
            .set_draft_in(tx, set.id, draft_id.as_uuid())
            .await?;

        let outcome = self
            .drafts
            .submit(tx, &branch, who, &draft_id, submit_key)
            .await?;
        if outcome.status != DraftStatus::Approved {
            return Err(RateEngineError::ApprovalRequired(format!(
                "draft is in {} status, must be APPROVED",
                outcome.status.as_str()
            )));
        }

        let publication = self
            .publisher
            .publish(tx, &branch, who, &draft_id, expected_version)
            .await?;

        Ok(PublishedTierSet {
            draft_id,
            publication,
        })
    }
}
