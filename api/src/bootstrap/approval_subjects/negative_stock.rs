//! Approval subject: NEGATIVE_STOCK (#3405, design `ledger-inventory.md` §5.1).

use std::sync::Arc;

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;
use xc_cash::application::ledger_approval::negative_stock::{
    NEGATIVE_STOCK, NegativeStockPolicy, NegativeStockSubject,
};

use crate::application::workflows::exchange::negative_stock_source::NegativeStockExchangeSource;

pub const TYPE: &str = NEGATIVE_STOCK;

pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    registry.register(
        Arc::new(NegativeStockSubject::new(Arc::new(
            NegativeStockExchangeSource::new(pool),
        ))),
        Arc::new(NegativeStockPolicy),
    );
}
