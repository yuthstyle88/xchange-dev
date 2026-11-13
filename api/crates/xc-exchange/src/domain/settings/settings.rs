//! Exchange settings domain entity.

use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangeSettings {
    pub organization_id: Uuid,
    pub quote_ttl_seconds: i32,
}
