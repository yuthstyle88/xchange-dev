//! Approval subject: TRANSACTION_REVERSAL (RV-2 #682, design `reversal.md` §5).

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;

pub const TYPE: &str = crate::application::workflows::exchange::reversal_approval::TRANSACTION_REVERSAL;

pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    crate::application::workflows::exchange::reversal_approval::register(registry, pool);
}
