//! `PgBranchPolicyStore`: `pricing_branch_policy` (pricing-tiers.md §15.2, §15.4).
//!
//! SKELETON (XC-V9-PRICING): every body is PV-a's. Signatures stay.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;

use crate::domain::pricing::branch_policy::{
    BranchPolicyPatch, BranchPolicyStore, BranchPricingPolicy,
};
use crate::domain::pricing::tier_set::StoreFuture;

pub struct PgBranchPolicyStore;

impl BranchPolicyStore for PgBranchPolicyStore {
    fn for_branch_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
    ) -> StoreFuture<'a, BranchPricingPolicy> {
        let _ = (conn, branch);
        Box::pin(async move { todo!("PV-a: PgBranchPolicyStore::for_branch_in") })
    }

    fn save_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
        scope: &'a str,
        patch: BranchPolicyPatch,
        by: &'a str,
        at: DateTime<Utc>,
    ) -> StoreFuture<'a, ()> {
        let _ = (conn, branch, scope, patch, by, at);
        Box::pin(async move { todo!("PV-a: PgBranchPolicyStore::save_in") })
    }
}
