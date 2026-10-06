//! Customer repository trait.

use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use crate::domain::customer::entities::customer::Customer;

pub trait CustomerRepository: Send + Sync {
    fn branch_exists(
        &self,
        branch_id: &BranchId,
    ) -> impl std::future::Future<Output = AppResult<bool>> + Send;

    fn find(
        &self,
        branch_id: &BranchId,
        id: &str,
    ) -> impl std::future::Future<Output = AppResult<Option<Customer>>> + Send;

    fn list_live(
        &self,
        branch_id: &BranchId,
    ) -> impl std::future::Future<Output = AppResult<Vec<Customer>>> + Send;
}
