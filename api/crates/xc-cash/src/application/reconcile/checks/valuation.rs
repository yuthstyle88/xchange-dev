//! Reconcile check 5 (LG-7.5 #1059; design `docs/design/blueprint/ledger-inventory.md` §6, §6.1):
//! every posting's `Σ valuation_thb` is 0 (§3.1); the balance trigger checks quantities only.

use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Row, Transaction};

use super::{Offender, ReconcileCheck, ReconcileScope};

/// The single statement this check runs; it returns the offender rows (empty = passed), at most 101.
pub const QUERY: &str = "\
SELECT p.id::text AS posting_id, sum(e.valuation_thb)::text AS detail \
FROM ledger_entries e \
JOIN ledger_postings p ON p.id = e.posting_id \
WHERE p.organization_id = $1::uuid \
  AND ($2::text IS NULL OR p.branch_id = $2) \
GROUP BY p.id \
HAVING sum(e.valuation_thb) <> 0 \
ORDER BY p.id \
LIMIT 101";

pub fn check() -> std::sync::Arc<dyn super::ReconcileCheck> {
    std::sync::Arc::new(ValuationCheck)
}

/// Check 5: `Σ valuation_thb` per posting is 0.
pub struct ValuationCheck;

impl ReconcileCheck for ValuationCheck {
    fn id(&self) -> u8 {
        5
    }

    fn run_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        scope: &'a ReconcileScope,
    ) -> BoxFuture<'a, AppResult<Vec<Offender>>> {
        Box::pin(async move {
            let branch = scope.branch.as_ref().map(|b| b.as_str());
            let rows = sqlx::query(QUERY)
                .bind(&scope.organization_id)
                .bind(branch)
                .fetch_all(&mut **tx)
                .await
                .map_err(AppError::Database)?;

            let mut offenders = Vec::with_capacity(rows.len());
            for row in rows {
                let key: String = row.try_get("posting_id").map_err(AppError::Database)?;
                let detail: String = row.try_get("detail").map_err(AppError::Database)?;
                offenders.push(Offender { key, detail });
            }

            if offenders.len() > 100 {
                offenders.truncate(100);
                offenders.push(Offender {
                    key: "truncated".to_string(),
                    detail: "more than 100 offenders; the first 100 are listed".to_string(),
                });
            }

            Ok(offenders)
        })
    }
}
