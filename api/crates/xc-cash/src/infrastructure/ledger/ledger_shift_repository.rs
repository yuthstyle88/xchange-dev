//! Ledger-backed `PgShiftRepository` factory (LG-6.r4 #1043, design §4.2.1).

use std::sync::Arc;

use sqlx::PgPool;
use xc_shift::infrastructure::shift::shift_repository::PgShiftRepository;

use crate::infrastructure::ledger::shift_balances::PgLedgerShiftBalances;

pub fn ledger_shift_repository(pool: PgPool) -> PgShiftRepository {
    PgShiftRepository::with_balances(pool.clone(), Arc::new(PgLedgerShiftBalances::new(pool)))
}
