//! The documents family: `documents`, the links to the papers of one exchange.
//!
//! **Not wired yet (ticket P2-D).** [`DocumentsAction::handles`] says `false`
//! for every name, so the dispatcher answers `documents` as a name nobody
//! knows, as it did before this file existed. P2-D changes this file only —
//! `handles` (the staff route: `Policy` lets no one ask it on `/api/rpc`) and
//! `run` — and sets `WIRED` in `api/tests/rpc_common/gap_documents.rs`.
//!
//! What the arm will be made of: `DocumentsLookup::find(branch, payload.ref)`
//! and `Documents::to_json`; the staff links are the dispatcher's to rewrite.

use pos_error::AppError;

use crate::application::compat::actions::{ActionFuture, RpcAction, RpcCall};
use crate::application::compat::envelope::RpcRoute;
use crate::application::compat::rpc::RpcFailure;
use crate::application::workflows::documents::DocumentsLookup;

/// The documents family: `documents`, the links to the papers of one exchange.
#[derive(Clone)]
pub struct DocumentsAction {
    #[expect(dead_code, reason = "read by the arm ticket P2-D adds")]
    lookup: DocumentsLookup,
}

impl DocumentsAction {
    /// `lookup` finds the documents of an exchange of a branch (`DocumentsLookup::find`).
    pub fn new(lookup: DocumentsLookup) -> Self {
        Self { lookup }
    }
}

impl RpcAction for DocumentsAction {
    fn handles(&self, _route: RpcRoute, _action: &str) -> bool {
        false
    }

    fn run<'a>(&'a self, call: RpcCall<'a>) -> ActionFuture<'a> {
        // never reached while `handles` says no
        Box::pin(async move {
            Err(RpcFailure::Failed(AppError::Internal(format!(
                "the documents family is not wired: {}",
                call.action
            ))))
        })
    }
}
