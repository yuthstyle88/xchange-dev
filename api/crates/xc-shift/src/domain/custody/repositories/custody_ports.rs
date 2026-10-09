//! External context ports for custody.

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;

use crate::domain::custody::value_objects::custody_role::CustodyRole;
use crate::domain::custody::value_objects::custody_time::CustodyTime;
use crate::domain::custody::value_objects::service_window::ServiceWindow;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutsideFacts {
    pub has_live_rate: bool,
    pub rates_updated: String,
    pub pending_transactions: Vec<String>,
    pub last_transaction: Option<String>,
}

pub trait CustodyPorts: Send + Sync + 'static {
    fn outside_facts<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        open_shift_id: Option<&'a str>,
    ) -> BoxFuture<'a, AppResult<OutsideFacts>>;

    fn customer_known<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<bool>>;

    fn evidence_owned<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        owner_id: &'a str,
        keys: &'a [String],
    ) -> BoxFuture<'a, AppResult<bool>>;

    fn employee_role<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        employee_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<CustodyRole>>>;

    fn service_window<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        now: CustodyTime,
    ) -> BoxFuture<'a, AppResult<ServiceWindow>>;
}
