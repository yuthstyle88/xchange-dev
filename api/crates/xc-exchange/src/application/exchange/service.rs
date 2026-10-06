//! Exchange service.

use std::sync::Arc;

use pos_error::AppResult;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::domain::customer::value_objects::mode::Mode;
use crate::domain::exchange::entities::transaction::Transaction;
use crate::domain::exchange::repositories::transaction_repository::TransactionRepository;
use crate::infrastructure::exchange::transaction_repository::PgTransactionRepository;

pub struct TransactionService<R: TransactionRepository = PgTransactionRepository> {
    repo: Arc<R>,
    branch_tx: BranchTx,
}

impl<R: TransactionRepository> Clone for TransactionService<R> {
    fn clone(&self) -> Self {
        Self {
            repo: Arc::clone(&self.repo),
            branch_tx: self.branch_tx.clone(),
        }
    }
}

impl<R: TransactionRepository> TransactionService<R> {
    pub fn new(repo: Arc<R>, branch_tx: BranchTx) -> Self {
        Self { repo, branch_tx }
    }

    pub fn branch_tx(&self) -> &BranchTx {
        &self.branch_tx
    }

    pub async fn find(
        &self,
        branch: &BranchId,
        document_ref: &str,
    ) -> AppResult<Option<Transaction>> {
        self.repo.find(branch, document_ref).await
    }

    pub async fn list(&self, branch: &BranchId, mode: Mode) -> AppResult<Vec<Transaction>> {
        self.repo.list(branch, mode).await
    }

    pub async fn find_submission_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        mode: Mode,
        submission_id: &str,
    ) -> AppResult<Option<(Transaction, String)>> {
        self.repo
            .find_submission_in(tx, branch, mode, submission_id)
            .await
    }

    pub async fn next_number_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        counter_key: &str,
    ) -> AppResult<i64> {
        self.repo.next_number_in(tx, counter_key).await
    }

    pub async fn record_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        transaction: &Transaction,
        payload_hash: &str,
    ) -> AppResult<()> {
        self.repo.insert_in(tx, transaction, payload_hash).await
    }
}
