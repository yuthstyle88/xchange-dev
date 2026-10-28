//! Approval subject: LEDGER_ADJUSTMENT (design `ledger-inventory.md` §13.5).

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;

pub const TYPE: &str = xc_cash::application::ledger_approval::ledger_adjustment::LEDGER_ADJUSTMENT;

pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    xc_cash::application::ledger_approval::ledger_adjustment::register(registry, pool);
}
