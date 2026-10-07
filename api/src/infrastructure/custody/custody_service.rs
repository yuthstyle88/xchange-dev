//! Building `CustodyService` on PostgreSQL.

use std::sync::Arc;

use sqlx::PgPool;
use xc_kernel::branch_tx::BranchTx;
use xc_shift::application::custody::service::CustodyService;
use xc_shift::application::shift::service::ShiftService;
use xc_shift::infrastructure::custody::custody_repository::PgCustodyRepository;

use crate::infrastructure::custody::custody_ports::PgCustodyPorts;

/// Builds `CustodyService` with `PgCustodyRepository` and `PgCustodyPorts` on `pool`.
pub fn pg_custody_service(
    pool: &PgPool,
    shifts: &ShiftService,
    branch_tx: &BranchTx,
) -> CustodyService {
    CustodyService::new(
        Arc::new(PgCustodyRepository::new(pool.clone())),
        shifts.clone(),
        Arc::new(PgCustodyPorts),
        branch_tx.clone(),
    )
}
