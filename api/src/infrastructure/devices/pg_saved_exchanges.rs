//! The exchanges the device service asks about, read from the transactions.
//!
//! `find` answers from `TransactionService::find`, no SQL of its own: a live
//! exchange of the branch with that document number is `Completed`, a `DEMO`
//! one or none at all is `None` — only live exchanges count, and v3 stores no
//! pending or cancelled exchange, so `NotCompleted` is never answered
//! (`docs/design/xchange-v3.md` §5.4).

use std::future::Future;
use std::pin::Pin;

use pos_error::AppResult;
use xc_exchange::application::exchange::service::TransactionService;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_kernel::ids::BranchId;

use crate::infrastructure::devices::saved_exchanges::{ExchangeState, SavedExchanges};

/// `SavedExchanges` over `TransactionService::find`: no SQL of its own.
#[derive(Clone)]
pub struct PgSavedExchanges {
    transactions: TransactionService,
}

impl PgSavedExchanges {
    pub fn new(transactions: TransactionService) -> Self {
        Self { transactions }
    }
}

impl SavedExchanges for PgSavedExchanges {
    fn find<'a>(
        &'a self,
        branch: &'a BranchId,
        reference: &'a str,
    ) -> Pin<Box<dyn Future<Output = AppResult<Option<ExchangeState>>> + Send + 'a>> {
        Box::pin(async move {
            match self.transactions.find(branch, reference).await? {
                Some(t) if t.mode == Mode::Live => Ok(Some(ExchangeState::Completed)),
                _ => Ok(None),
            }
        })
    }
}
