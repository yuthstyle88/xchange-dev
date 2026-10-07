//! The custody family: every name starting with `ops`.
//!
//! **Not wired yet (ticket P2-C).** [`CustodyAction::handles`] says `false`
//! for every name, so the dispatcher answers every `ops…` name as a name
//! nobody knows, as it did before this file existed. P2-C changes this file
//! only — `handles` and `run` — and sets `WIRED` in
//! `api/tests/rpc_common/gap_custody.rs`.
//!
//! Decided, to keep when the arm is written:
//!
//! - the family handles the **staff route only**: v2 knows no `ops*` name on
//!   `/api/rpc`, and `Policy` already refuses them there;
//! - known difference from v2: an `ops…` name custody does not know is
//!   answered `คำขอไม่ถูกต้อง` whatever the state of the branch; v2 answers it
//!   by that state (`rpc/transport.json` #84, `rpc/ops_actions.json` #2, #23,
//!   #40).
//!
//! What the arm will be made of: `CustodyService::command(branch, action,
//! payload, actor, now)`, the actor from `RpcCall::caller`.

use pos_error::AppError;
use xc_shift::application::custody::service::CustodyService;

use crate::application::compat::actions::{ActionFuture, RpcAction, RpcCall};
use crate::application::compat::envelope::RpcRoute;
use crate::application::compat::rpc::RpcFailure;

/// The custody family: every name starting with `ops`.
#[derive(Clone)]
pub struct CustodyAction {
    #[expect(dead_code, reason = "read by the arm ticket P2-C adds")]
    custody: CustodyService,
}

impl CustodyAction {
    /// `custody` runs a custody command of a branch (`CustodyService::command`).
    pub fn new(custody: CustodyService) -> Self {
        Self { custody }
    }
}

impl RpcAction for CustodyAction {
    fn handles(&self, _route: RpcRoute, _action: &str) -> bool {
        false
    }

    fn run<'a>(&'a self, call: RpcCall<'a>) -> ActionFuture<'a> {
        // never reached while `handles` says no
        Box::pin(async move {
            Err(RpcFailure::Failed(AppError::Internal(format!(
                "the custody family is not wired: {}",
                call.action
            ))))
        })
    }
}
