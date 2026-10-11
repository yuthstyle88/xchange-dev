//! Reconcile check 4 (LG-7.4 #1058; design `docs/design/blueprint/ledger-inventory.md` §6, §6.1):
//! margin equals sales minus cost.

use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Row, Transaction};

use super::{Offender, ReconcileCheck, ReconcileScope};

/// The single statement this check runs; it returns the offender rows (empty = passed), at most 101.
pub const QUERY: &str = "\
SELECT key, detail FROM ( \
    SELECT \
        t.branch_id || ':' || t.value_date::text AS key, \
        'margin ' || coalesce(sum(l.realized_margin_thb), 0)::text || ' vs sales ' || sum(l.thb_amount)::text || ' - cost ' || sum(l.cost_basis_thb)::text AS detail \
    FROM transaction_lines l \
    JOIN transactions t ON t.id = l.transaction_id \
    WHERE t.organization_id = $1::uuid \
      AND ($2::text IS NULL OR t.branch_id = $2) \
      AND ($3::date IS NULL OR t.value_date BETWEEN $3::date AND $4::date) \
      AND l.shop_side = 'SELL' \
      AND l.cost_basis_thb IS NOT NULL \
      AND t.status = 'COMPLETED' \
    GROUP BY t.branch_id, t.value_date \
    HAVING coalesce(sum(l.realized_margin_thb), 0) <> sum(l.thb_amount) - sum(l.cost_basis_thb) \
    UNION ALL \
    SELECT \
        l.id::text AS key, \
        'uncosted' AS detail \
    FROM transaction_lines l \
    JOIN transactions t ON t.id = l.transaction_id \
    WHERE t.organization_id = $1::uuid \
      AND ($2::text IS NULL OR t.branch_id = $2) \
      AND ($3::date IS NULL OR t.value_date BETWEEN $3::date AND $4::date) \
      AND l.shop_side = 'SELL' \
      AND l.cost_basis_thb IS NULL \
      AND t.status = 'COMPLETED' \
) offenders \
ORDER BY key \
LIMIT 101";

/// Check 4: Σ `realized_margin_thb` = Σ `thb_amount` (SELL) − Σ `cost_basis_thb`.
pub struct MarginCheck;

impl ReconcileCheck for MarginCheck {
    fn id(&self) -> u8 {
        4
    }

    fn run_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        scope: &'a ReconcileScope,
    ) -> BoxFuture<'a, AppResult<Vec<Offender>>> {
        Box::pin(async move {
            let branch = scope.branch.as_ref().map(|b| b.as_str());
            let (from_date, to_date) = match scope.period {
                Some((from, to)) => (Some(from), Some(to)),
                None => (None, None),
            };

            let rows = sqlx::query(QUERY)
                .bind(&scope.organization_id)
                .bind(branch)
                .bind(from_date)
                .bind(to_date)
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

pub fn check() -> std::sync::Arc<dyn super::ReconcileCheck> {
    std::sync::Arc::new(MarginCheck)
}

