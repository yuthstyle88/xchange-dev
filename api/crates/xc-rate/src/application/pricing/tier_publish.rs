//! Publish one tier set at its branch (pricing-tiers.md §15.3 steps 4–6), through the one publication writer:
//! `RateDraftService::copy` the branch's current publication (an empty draft before the first one), replace the
//! currency's line with `MANUAL` tier 1 plus the 4 tier rows (`DraftLine::tiers`), `submit` (a line with tier rows is
//! in bound: B3), then `PublishService::publish` (4 tier lines, `source_snapshot.pricing = {tier_set_id, source}`).
//! Used by `pricingApprove` (§15.3), `BranchPush` (§15.4) and the Follow publisher (§15.5).
//!
//! SKELETON (XC-V9-PRICING): `TierSetPublisher::publish_in` is PV-b2's. Types and signatures stay.

use std::sync::Arc;

use sqlx::{Postgres, Transaction};
use xc_auth::domain::access::Actor;

use crate::application::draft::rate_draft_service::RateDraftService;
use crate::application::publication::publish_service::PublishService;
use crate::domain::draft::rate_draft::DraftId;
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::tier_set::PricingTierSet;
use crate::domain::publication::published_rates::PublicationRef;

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
        let _ = (&self.drafts, &self.publisher, tx, who, set, submit_key);
        todo!("PV-b2: TierSetPublisher::publish_in")
    }
}
