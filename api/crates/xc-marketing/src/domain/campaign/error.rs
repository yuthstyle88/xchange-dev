//! `CampaignError` (ticket #1390 §8, design §4.5 errors).

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CampaignError {
    /// An edit of a version that is no longer DRAFT or SIMULATED (D-3, design §7).
    #[error("staleVersion")]
    StaleVersion,
}
