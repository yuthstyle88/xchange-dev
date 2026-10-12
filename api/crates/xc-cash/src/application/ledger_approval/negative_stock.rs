//! `NEGATIVE_STOCK` (N-1; design §5.1).

use std::sync::Arc;

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{Postgres, Transaction};
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::{Actor, Role};

/// The type name callers send.
pub const NEGATIVE_STOCK: &str = "NEGATIVE_STOCK";

/// Where a `NEGATIVE_STOCK` request's version hash comes from: the source whose posting went short
/// (an exchange's lines and status, or a transfer's). Lane controller's ruling on #925 (2026-10-10):
/// this ticket takes the hash as an input; LG-2 #924, the ticket that posts negative stock, implements
/// this port over the source lines.
pub trait NegativeStockSource: Send + Sync {
    /// The source's current version hash for `object_id` (e.g. `transaction:<id>`, `transfer:<id>`).
    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>>;
}

pub struct NegativeStockSubject {
    source: Arc<dyn NegativeStockSource>,
}

impl NegativeStockSubject {
    pub fn new(source: Arc<dyn NegativeStockSource>) -> Self {
        Self { source }
    }
}

impl ApprovalSubject for NegativeStockSubject {
    fn type_name(&self) -> &'static str {
        NEGATIVE_STOCK
    }

    /// The source's state, from the `NegativeStockSource` this subject was given.
    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        self.source.version_hash(tx, object_id)
    }

    /// APPROVED writes nothing: the caller retries with `allow_negative`.
    fn on_decided<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _object_id: &'a str,
        _outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move { Ok(()) })
    }
}

/// `may_decide` ⇔ BRANCH_MANAGER of the request's branch; never the requester.
pub struct NegativeStockPolicy;

impl ApprovalPolicy for NegativeStockPolicy {
    fn may_decide(&self, who: &Actor, request: &ApprovalRequest) -> Result<(), Refusal> {
        let req_branch = request.branch_id.as_deref();
        let is_bm_of_branch = who.roles.iter().any(|r| {
            r.role == Role::BranchManager
                && (match (r.branch.as_ref(), req_branch) {
                    (Some(b), Some(rb)) => b.as_str() == rb,
                    _ => false,
                })
        });
        if is_bm_of_branch {
            Ok(())
        } else if !who.roles.iter().any(|r| r.role == Role::BranchManager) {
            Err(Refusal::new(
                "outOfScope",
                "only branch manager may decide negative stock",
            ))
        } else {
            Err(Refusal::new(
                "outOfScope",
                "branch manager can only decide requests for their own branch",
            ))
        }
    }

    fn allows_self_decision(&self, _who: &Actor) -> bool {
        false
    }

    fn policy_version(&self) -> String {
        "1".to_string()
    }
}
