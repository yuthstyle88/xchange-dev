//! The action families behind the RPC dispatcher: one file per family.
//!
//! `CompatRpc` (`../rpc.rs`) is the shared core of both RPC routes: may this
//! caller ask for this name here, which branch, and how the answer goes out
//! (the staff view of it on `/api/staff/rpc`). A family is what differs — which
//! names it answers and what it does for each — and lives in one file of this
//! directory behind [`RpcAction`]:
//!
//! | File | Family | Names |
//! |---|---|---|
//! | `documents.rs` | the documents of one exchange | `documents` |
//! | `custody.rs` | custody of the cash | every name starting with `ops` |
//! | `save.rs` | the save of an exchange | `saveTransaction` |
//!
//! Wiring a family is an edit of its own file only: `handles` says which names
//! on which route, `run` answers them. The dispatcher and the route layer do
//! not change.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use xc_kernel::ids::BranchId;

use crate::application::compat::envelope::RpcRoute;
use crate::application::compat::rpc::{RpcCaller, RpcFailure};
use crate::application::compat::v2_json::V2Json;

pub mod custody;
pub mod documents;
pub mod save;

/// One request, as the dispatcher hands it to the family that answers it.
///
/// By the time a family sees it the caller's role was judged for this name on
/// this route (`Policy::allow`) and `branch` was resolved and found to exist.
#[derive(Debug)]
pub struct RpcCall<'a> {
    /// The route the request came in on.
    pub route: RpcRoute,
    /// The action's name, as sent.
    pub action: &'a str,
    /// The payload, as sent (an object; `{}` when none was sent).
    pub payload: Map<String, Value>,
    /// Who is calling: the actor, the username writes are recorded under, and
    /// the display name.
    pub caller: &'a RpcCaller,
    /// The branch the request is answered for: an employee's own, the owner's
    /// `payload.branchId` or `MAIN`.
    pub branch: &'a BranchId,
    /// The server clock at the request.
    pub now: DateTime<Utc>,
}

/// What [`RpcAction::run`] gives back: the answer as `/api/rpc` prints it, or
/// why there is none. An `AppError::Domain` sentence becomes a refusal through
/// `?` (`From<AppError> for RpcFailure`).
pub type ActionFuture<'a> = Pin<Box<dyn Future<Output = Result<V2Json, RpcFailure>> + Send + 'a>>;

/// The families registered with a dispatcher, in the order they are asked.
pub type RpcActions = Vec<Arc<dyn RpcAction>>;

/// One family of actions behind the RPC dispatcher.
///
/// The dispatcher asks the registered families only for a name that is not one
/// of its own ten, and only after the caller's role was judged. It asks them
/// in the order they were registered and the **first** whose
/// [`handles`](Self::handles) says `true` answers; no other family is asked.
/// When none does, the name is answered as a name nobody knows.
pub trait RpcAction: Send + Sync {
    /// Whether this family answers `action` on `route`. No side effect, no I/O:
    /// it is asked for every request that is not one of the dispatcher's own.
    fn handles(&self, route: RpcRoute, action: &str) -> bool;

    /// Answers `call`. The answer is what `/api/rpc` prints; on
    /// `/api/staff/rpc` the dispatcher turns its links to the staff paths
    /// afterwards, so a family writes `/files/…` and `/documents?…` as the
    /// owner sees them.
    fn run<'a>(&'a self, call: RpcCall<'a>) -> ActionFuture<'a>;
}
