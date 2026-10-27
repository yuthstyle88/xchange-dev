//! `PgBranchPolicyStore`: `pricing_branch_policy` (pricing-tiers.md §15.2, §15.4).
//!
//! SKELETON (XC-V9-PRICING): every body is PV-a4b's. Signatures stay.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Row};

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::pricing::branch_policy::{
    BranchPolicyPatch, BranchPolicyRow, BranchPolicyStore, BranchPricingPolicy,
};
use crate::domain::pricing::tier_set::StoreFuture;

pub struct PgBranchPolicyStore;

impl BranchPolicyStore for PgBranchPolicyStore {
    fn for_branch_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        branch: &'a str,
    ) -> StoreFuture<'a, BranchPricingPolicy> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT currency_code, buy_bp, sell_bp, own, follow \
                 FROM pricing_branch_policy \
                 WHERE branch_id = $1",
            )
            .bind(branch)
            .fetch_all(&mut *conn)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let rows = rows
                .into_iter()
                .map(|row| BranchPolicyRow {
                    currency: row.get("currency_code"),
                    buy_bp: row.get("buy_bp"),
                    sell_bp: row.get("sell_bp"),
                    own: row.get("own"),
                    follow: row.get("follow"),
                })
                .collect();

            Ok(BranchPricingPolicy { rows })
        })
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
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO pricing_branch_policy \
                 (branch_id, currency_code, buy_bp, sell_bp, own, follow, updated_by, updated_at) \
                 VALUES ($1, $2, $3, $4, COALESCE($5, false), COALESCE($6, false), $7, $8) \
                 ON CONFLICT (branch_id, currency_code) DO UPDATE \
                 SET buy_bp = COALESCE($3, pricing_branch_policy.buy_bp), \
                     sell_bp = COALESCE($4, pricing_branch_policy.sell_bp), \
                     own = COALESCE($5, pricing_branch_policy.own), \
                     follow = COALESCE($6, pricing_branch_policy.follow), \
                     updated_by = EXCLUDED.updated_by, \
                     updated_at = EXCLUDED.updated_at",
            )
            .bind(branch)
            .bind(scope)
            .bind(patch.buy_bp)
            .bind(patch.sell_bp)
            .bind(patch.own)
            .bind(patch.follow)
            .bind(by)
            .bind(at)
            .execute(&mut *conn)
            .await
            .map_err(|e| RateEngineError::Database(e.to_string()))?;

            Ok(())
        })
    }
}
