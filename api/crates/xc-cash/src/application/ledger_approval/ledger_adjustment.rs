//! `LEDGER_ADJUSTMENT` (design `docs/design/blueprint/ledger-inventory.md` §13.3, §13.5 "Decide"): the
//! approval of a FINANCE ledger adjustment. Its object is the adjustment id (`ADJ-<uuid>`), and its
//! `proposed_payload` is the adjustment (AD-6).
//!
//! Skeleton (LA-0, sub-controller claude/f32b85db): `register`, `type_name`, `on_decided`,
//! `allows_self_decision` and `policy_version` are final; `version_hash` and `may_decide` are LA-3's.

use std::sync::Arc;

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Transaction};
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::Actor;

pub use crate::domain::adjustment::LEDGER_ADJUSTMENT;

/// Registers `LEDGER_ADJUSTMENT` with `registry`.
pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    registry.register(
        Arc::new(LedgerAdjustmentSubject::new(pool)),
        Arc::new(LedgerAdjustmentPolicy),
    );
}

pub struct LedgerAdjustmentSubject {
    #[allow(dead_code)]
    pool: PgPool,
}

impl LedgerAdjustmentSubject {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl ApprovalSubject for LedgerAdjustmentSubject {
    fn type_name(&self) -> &'static str {
        LEDGER_ADJUSTMENT
    }

    /// `"ledger-adjustment:<id>|posted:<true|false>"`, `posted` = a `ledger_postings` row with
    /// `(source_type, source_id) = ('ADJUSTMENT', id)` exists in `tx`. It reads no balance, so another
    /// posting on the branch never makes the request stale (§13.5, LA-T10); the payload cannot change
    /// (AD-6).
    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        let _ = (tx, object_id);
        todo!("LA-3")
    }

    /// Nothing: the post is its own command, under the adjustment's branch lock (AD-5).
    fn on_decided<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _object_id: &'a str,
        _outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move { Ok(()) })
    }
}

/// `may_decide` ⇔ `ApprovalDecider::holds_on(who, ApproveLedgerAdjustment, Some(<payload.branch>))`
/// (§13.3): the branch is the payload's, never `request.branch_id`, which AP-1 fills from the
/// requester's role row (`MAIN` for an organisation-wide FINANCE). A payload without a readable
/// `branch` is refused. Never the requester (AP-1, `allows_self_decision`).
pub struct LedgerAdjustmentPolicy;

impl ApprovalPolicy for LedgerAdjustmentPolicy {
    fn may_decide(&self, who: &Actor, request: &ApprovalRequest) -> Result<(), Refusal> {
        let _ = (who, request);
        todo!("LA-3")
    }

    fn allows_self_decision(&self, _who: &Actor) -> bool {
        false
    }

    fn policy_version(&self) -> String {
        "1".to_string()
    }
}
