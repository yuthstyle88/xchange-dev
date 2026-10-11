//! TransactionStore port and status transition types.

use chrono::{DateTime, Utc};
use pos_error::AppResult;
use uuid::Uuid;
use xc_kernel::money::{Money, MoneyDelta};

use super::receipt_record::Receipt;
use super::transaction::{ExchangeTransaction, TransactionId, TransactionStatus};

/// The costs a posting answers for one line (TX-5 #873, ruling Q2), written on `transaction_lines`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineCosts {
    pub line_id: Uuid,
    pub cost_basis_thb: Money,
    pub realized_margin_thb: Option<MoneyDelta>,
}

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

    /// `next_document_ref_in` then `complete_numbered_in`, on one transaction.
    fn complete_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: TransactionId,
        receipt_file_key: &'a str,
        now: DateTime<Utc>,
    ) -> impl std::future::Future<Output = AppResult<(String, Receipt)>> + Send + 'a;

    /// The next number of the exchange's organisation and mode (TX-5 #873, ruling Q8): it takes the counter, so
    /// a caller that renders the receipt with the number can store the receipt file before completing.
    fn next_document_ref_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: TransactionId,
        now: DateTime<Utc>,
    ) -> impl std::future::Future<Output = AppResult<String>> + Send + 'a;

    /// A CONFIRMED exchange becomes COMPLETED under `document_ref`, with receipt revision 1 naming
    /// `receipt_file_key` (TX-5 #873, ruling Q8).
    fn complete_numbered_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: TransactionId,
        document_ref: &'a str,
        receipt_file_key: &'a str,
        now: DateTime<Utc>,
    ) -> impl std::future::Future<Output = AppResult<Receipt>> + Send + 'a;

    /// Writes the costs a posting answered on the exchange's lines (TX-5 #873, ruling Q2).
    fn record_line_costs_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        id: TransactionId,
        costs: &'a [LineCosts],
    ) -> impl std::future::Future<Output = AppResult<()>> + Send + 'a;
}
