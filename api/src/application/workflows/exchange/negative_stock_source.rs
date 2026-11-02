//! `NegativeStockExchangeSource` (#5938, design `ledger-inventory.md` §5.1.1): wraps
//! `LedgerCashPostings` so a rejected `NEGATIVE_STOCK` request on a `CONFIRMED` exchange cancels
//! that exchange inside the decision's transaction.

use std::future::Future;
use std::pin::Pin;

use chrono::Utc;
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use xc_auth::domain::access::{Role, StandingKind};
use xc_cash::application::ledger_approval::negative_stock::NegativeStockSource;
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_exchange::domain::transaction::transaction::TransactionId;

use crate::application::workflows::exchange::cancel::CancelTransaction;
use crate::application::workflows::save_step::SaveActor;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub struct NegativeStockExchangeSource {
    postings: LedgerCashPostings,
    cancel: CancelTransaction,
}

impl NegativeStockExchangeSource {
    pub fn new(pool: PgPool) -> Self {
        Self {
            postings: LedgerCashPostings::new(),
            cancel: CancelTransaction::new(pool),
        }
    }
}

impl NegativeStockSource for NegativeStockExchangeSource {
    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        self.postings.version_hash(tx, object_id)
    }

    fn on_rejected<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
        reason: &'a str,
        approver_id: &'a str,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let Ok(id) = Uuid::parse_str(object_id) else {
                return Ok(());
            };
            let status: Option<String> =
                sqlx::query_scalar("SELECT status FROM transactions WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;
            if status.as_deref() != Some("CONFIRMED") {
                return Ok(());
            }
            let actor = SaveActor {
                id: approver_id.to_owned(),
                username: approver_id.to_owned(),
                employee_id: Some(approver_id.to_owned()),
                standing: StandingKind::Staff(Role::BranchManager),
            };
            self.cancel
                .run_in(tx, TransactionId(id), reason, &actor, Utc::now())
                .await?;
            Ok(())
        })
    }
}
