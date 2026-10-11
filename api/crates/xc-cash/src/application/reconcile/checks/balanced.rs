//! Reconcile check 1 (LG-7.1 #1055; design `docs/design/blueprint/ledger-inventory.md` §6, §6.1):
//! every posting balances per currency.

use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use super::{Offender, ReconcileCheck, ReconcileScope};

/// The single statement this check runs; it returns the offender rows (empty = passed), at most 101.
pub const QUERY: &str = "SELECT e.posting_id, e.currency, sum(e.quantity_signed) \
FROM ledger_entries e \
JOIN ledger_postings p ON p.id = e.posting_id \
WHERE p.organization_id = $1::uuid \
  AND ($2::text IS NULL OR p.branch_id = $2) \
GROUP BY e.posting_id, e.currency \
HAVING sum(e.quantity_signed) <> 0 \
ORDER BY e.posting_id, e.currency \
LIMIT 101";

pub fn check() -> std::sync::Arc<dyn super::ReconcileCheck> {
    std::sync::Arc::new(BalancedCheck)
}

/// Check 1: `Σ quantity_signed` per (posting, currency) is 0.
pub struct BalancedCheck;

impl ReconcileCheck for BalancedCheck {
    fn id(&self) -> u8 {
        1
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
                .map_err(pos_error::AppError::Database)?;

            let is_truncated = rows.len() > 100;
            let mut offenders = Vec::with_capacity(rows.len());

            for row in rows.into_iter().take(100) {
                let posting_id: Uuid = row.try_get(0).map_err(pos_error::AppError::Database)?;
                let currency: String = row.try_get(1).map_err(pos_error::AppError::Database)?;
                let sum: Decimal = row.try_get(2).map_err(pos_error::AppError::Database)?;

                offenders.push(Offender {
                    key: format!("{posting_id}:{}", currency.trim()),
                    detail: sum.to_string(),
                });
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
