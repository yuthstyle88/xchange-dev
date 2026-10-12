//! `TierSource` source trait (design §6, §7).

use uuid::Uuid;

pub trait TierSource: Send + Sync {
    fn tier_of(&self, organization_id: Uuid, customer: &str) -> Option<String>;
}
