//! Approval subject: COUNT_DIFFERENCE.

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;

pub const TYPE: &str = xc_cash::application::ledger_approval::count_difference::COUNT_DIFFERENCE;

pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    xc_cash::application::ledger_approval::count_difference::register(registry, pool);
}
