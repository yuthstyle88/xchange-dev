//! Transaction repository trait.

use pos_error::AppResult;
use xc_kernel::ids::BranchId;

use crate::domain::customer::value_objects::mode::Mode;
use crate::domain::exchange::entities::transaction::Transaction;

pub trait TransactionRepository: Send + Sync + 'static {
    fn find(
        &self,
        branch: &BranchId,
        document_ref: &str,
    ) -> impl std::future::Future<Output = AppResult<Option<Transaction>>> + Send;

    fn list(
        &self,
        branch: &BranchId,
        mode: Mode,
    ) -> impl std::future::Future<Output = AppResult<Vec<Transaction>>> + Send;

    fn find_submission_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        mode: Mode,
        submission_id: &'a str,
    ) -> impl std::future::Future<Output = AppResult<Option<(Transaction, String)>>> + Send + 'a;

    fn next_number_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        counter_key: &'a str,
    ) -> impl std::future::Future<Output = AppResult<i64>> + Send + 'a;

    fn insert_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        transaction: &'a Transaction,
        payload_hash: &'a str,
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;
}
