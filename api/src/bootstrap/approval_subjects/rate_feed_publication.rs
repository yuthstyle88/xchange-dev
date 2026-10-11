//! Approval subject: RATE_FEED_PUBLICATION.

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;

pub const TYPE: &str = "RATE_FEED_PUBLICATION";

pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    xc_rate::application::approval::rate_feed_publication::register(registry, pool);
}
