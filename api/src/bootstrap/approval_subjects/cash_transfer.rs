//! Approval subject: CASH_TRANSFER.

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;

pub const TYPE: &str = xc_cash::application::approval::transfer::CASH_TRANSFER;

pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    xc_cash::application::approval::transfer::register(registry, pool);
}
