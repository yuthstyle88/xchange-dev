//! TransactionStore port and status transition types.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use uuid::Uuid;

use super::receipt_record::Receipt;
use super::transaction::{ExchangeTransaction, TransactionId, TransactionStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusMove {
    pub from: TransactionStatus,
    pub to: TransactionStatus,
    pub reason: Option<String>,
    pub approval_request_id: Option<Uuid>,
}

pub trait TransactionStore: Send + Sync {
    fn insert_draft_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        transaction: &'a ExchangeTransaction,
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;

    fn find_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: TransactionId,
    ) -> impl std::future::Future<Output = AppResult<Option<ExchangeTransaction>>> + Send + 'a;

    fn set_status_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: TransactionId,
        r#move: StatusMove,
        now: DateTime<Utc>,
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;

    fn complete_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: TransactionId,
        receipt_file_key: &'a str,
        now: DateTime<Utc>,
    ) -> impl std::future::Future<Output = AppResult<(String, Receipt)>> + Send + 'a;
}
