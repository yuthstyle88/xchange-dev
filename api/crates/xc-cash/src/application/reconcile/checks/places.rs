//! Reconcile check 3 (LG-7.3 #1057; design `docs/design/blueprint/ledger-inventory.md` §6, §6.1):
//! per branch and foreign currency, Σ balances of that branch's own DRAWER, SAFE and IN_TRANSIT
//! accounts equals the branch's inventory position quantity.

use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Row, Transaction};

use super::{Offender, ReconcileCheck, ReconcileScope};

/// The single statement this check runs; it returns the offender rows (empty = passed), at most 101.
pub const QUERY: &str = "\
WITH places AS ( \
    SELECT \
        a.branch_id, \
        a.currency, \
        COALESCE(SUM(e.quantity_signed), 0) AS quantity \
    FROM cash_accounts a \
    JOIN ledger_entries e ON e.account_id = a.id \
    WHERE a.organization_id = $1::uuid \
      AND a.branch_id IS NOT NULL \
      AND ($2::text IS NULL OR a.branch_id = $2) \
      AND a.kind IN ('DRAWER', 'SAFE', 'IN_TRANSIT') \
      AND a.currency <> 'THB' \
    GROUP BY a.branch_id, a.currency \
), \
positions AS ( \
    SELECT \
        pos.branch_id, \
        pos.currency, \
        pos.quantity \
    FROM inventory_positions pos \
    WHERE pos.organization_id = $1::uuid \
      AND ($2::text IS NULL OR pos.branch_id = $2) \
      AND pos.currency <> 'THB' \
) \
SELECT \
    COALESCE(pl.branch_id, pos.branch_id) || ':' || TRIM(COALESCE(pl.currency, pos.currency)) AS key, \
    'places ' || COALESCE(pl.quantity, 0)::text || ' vs position ' || COALESCE(pos.quantity, 0)::text AS detail \
FROM places pl \
FULL OUTER JOIN positions pos \
    ON pl.branch_id = pos.branch_id \
   AND pl.currency = pos.currency \
WHERE COALESCE(pl.quantity, 0) <> COALESCE(pos.quantity, 0) \
ORDER BY key \
LIMIT 101";

pub fn check() -> std::sync::Arc<dyn super::ReconcileCheck> {
    std::sync::Arc::new(PlacesCheck)
}

/// Check 3: per branch and foreign currency, Σ of DRAWER, SAFE, and IN_TRANSIT balances
/// equals inventory_positions.quantity.
pub struct PlacesCheck;

impl ReconcileCheck for PlacesCheck {
    fn id(&self) -> u8 {
        3
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

            let is_truncated = rows.len() > 100;
            let mut offenders = Vec::with_capacity(rows.len());

            for row in rows.into_iter().take(100) {
                let key: String = row.try_get("key").map_err(AppError::Database)?;
                let detail: String = row.try_get("detail").map_err(AppError::Database)?;
                offenders.push(Offender { key, detail });
            }

            if is_truncated {
                offenders.push(Offender {
                    key: "truncated".to_string(),
                    detail: "more than 100 offenders; the first 100 are listed".to_string(),
                });
            }

            Ok(offenders)
        })
    }
}
