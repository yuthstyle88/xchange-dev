//! The ledger's two approval types (LG-3 #925; design `docs/design/blueprint/ledger-inventory.md` §5):
//! `NEGATIVE_STOCK` (N-1) and `COUNT_DIFFERENCE` (V-1), on AP-1's core.
//!
//! Scaffold written by the ticket author; the bodies of the two files are `todo!()`.

pub mod count_difference;
pub mod negative_stock;

use std::sync::Arc;

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;

/// Registers both types. `make_approval_registry` (`api/src/presentation/http/v3/commands/mod.rs`)
/// calls this with one line; `negative_stock` is LG-2 #924's source of the hash.
pub fn register(
    registry: &mut ApprovalRegistry,
    pool: PgPool,
    negative_stock: Arc<dyn negative_stock::NegativeStockSource>,
) {
    registry.register(
        Arc::new(negative_stock::NegativeStockSubject::new(negative_stock)),
        Arc::new(negative_stock::NegativeStockPolicy),
    );
    count_difference::register(registry, pool);
}
