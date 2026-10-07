//! The save family: `saveTransaction`, the save of an exchange.
//!
//! **Not wired yet (ticket P2-S).** [`SaveAction::handles`] says `false` for
//! every name, so the dispatcher answers `saveTransaction` as a name nobody
//! knows, as it did before this file existed. P2-S changes this file only —
//! `handles` (both routes) and `run` — and sets `WIRED` in
//! `api/tests/rpc_common/gap_save.rs`.
//!
//! What the arm will be made of: the payload as `SaveTransaction`, the save
//! workflow with the step custody adds around a save, and v2's nine keys from
//! what was saved.

use pos_error::AppError;
use xc_shift::application::custody::service::CustodyService;

use crate::application::compat::actions::{ActionFuture, RpcAction, RpcCall};
use crate::application::compat::envelope::RpcRoute;
use crate::application::compat::rpc::RpcFailure;
use crate::application::workflows::save_transaction::SaveTransactionWorkflow;

/// The save family: `saveTransaction`, the save of an exchange.
#[derive(Clone)]
pub struct SaveAction {
    #[expect(dead_code, reason = "read by the arm ticket P2-S adds")]
    workflow: SaveTransactionWorkflow,
    #[expect(dead_code, reason = "read by the arm ticket P2-S adds")]
    custody: CustodyService,
}

impl SaveAction {
    /// `workflow` saves an exchange; `custody` is for the step custody adds
    /// around the save (its check before, its draft completed after).
    pub fn new(workflow: SaveTransactionWorkflow, custody: CustodyService) -> Self {
        Self { workflow, custody }
    }
}

impl RpcAction for SaveAction {
    fn handles(&self, _route: RpcRoute, _action: &str) -> bool {
        false
    }

    fn run<'a>(&'a self, call: RpcCall<'a>) -> ActionFuture<'a> {
        // never reached while `handles` says no
        Box::pin(async move {
            Err(RpcFailure::Failed(AppError::Internal(format!(
                "the save family is not wired: {}",
                call.action
            ))))
        })
    }
}
