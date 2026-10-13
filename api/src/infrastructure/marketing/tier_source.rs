//! Marketing tier source adapter (MK-5, design §6, §6.1).

use uuid::Uuid;
use xc_marketing::domain::engine::tier_source::TierSource;

#[derive(Clone, Copy, Debug)]
pub struct MarketingTierSource;

impl MarketingTierSource {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MarketingTierSource {
    fn default() -> Self {
        Self::new()
    }
}

impl TierSource for MarketingTierSource {
    fn tier_of(&self, organization_id: Uuid, customer: &str) -> Option<String> {
        let _ = (organization_id, customer);
        None
    }
}
