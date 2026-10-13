//! Approval subject: CAMPAIGN_VERSION (MK-4, design `marketing-campaigns.md` §4.4, §6, D-5).

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;

pub const TYPE: &str = "CAMPAIGN_VERSION";

pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    xc_marketing::application::approval::register(registry, pool);
}
