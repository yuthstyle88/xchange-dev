//! What the device service asks about saved exchanges.

use std::future::Future;
use std::pin::Pin;

use pos_error::AppResult;
use xc_kernel::ids::BranchId;

/// How far an exchange the branch knows has come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExchangeState {
    /// Saved with all its documents.
    Completed,
    /// Known, but pending or cancelled.
    NotCompleted,
}

/// The exchanges of a branch, by document number. The device service owns no
/// exchange table; whoever builds it supplies the look-up.
///
/// Only live exchanges count (the old server looked in the live list only),
/// and the number is compared exactly.
pub trait SavedExchanges: Send + Sync {
    /// `None` when the branch has no exchange with that document number.
    fn find<'a>(
        &'a self,
        branch: &'a BranchId,
        reference: &'a str,
    ) -> Pin<Box<dyn Future<Output = AppResult<Option<ExchangeState>>> + Send + 'a>>;
}
