//! Approval subject: BINDING_QUOTE (design `docs/design/blueprint/binding-quote.md` D-15, D-19).

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;

pub const TYPE: &str = "BINDING_QUOTE";

pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    crate::application::workflows::exchange::binding_approval::register(registry, pool);
}
