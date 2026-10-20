//! Approval subject: NEGATIVE_STOCK (#3405, design `ledger-inventory.md` §5.1).

use std::sync::Arc;

use sqlx::PgPool;
use xc_approval::application::registry::ApprovalRegistry;
use xc_cash::application::ledger_approval::negative_stock::{
    NEGATIVE_STOCK, NegativeStockPolicy, NegativeStockSubject,
};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;

pub const TYPE: &str = NEGATIVE_STOCK;

pub fn register(registry: &mut ApprovalRegistry, _pool: PgPool) {
    registry.register(
        Arc::new(NegativeStockSubject::new(Arc::new(
            LedgerCashPostings::new(),
        ))),
        Arc::new(NegativeStockPolicy),
    );
}
