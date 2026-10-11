//! Approval subject: EXCHANGE_TRANSACTION (TX-4 #672, design `quote-transaction.md` §5.1).

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;

pub const TYPE: &str = "EXCHANGE_TRANSACTION";

pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    crate::application::workflows::exchange::approval::register(registry, pool);
}
