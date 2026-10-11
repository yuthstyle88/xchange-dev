//! Reconcile check 2 (LG-7.2 #1056; design `docs/design/blueprint/ledger-inventory.md` §6, §6.1):
//! every `inventory_positions` row equals the sum of its movements, in quantity and in cost.

use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use sqlx::{FromRow, Postgres, Transaction};

use super::{Offender, ReconcileCheck, ReconcileScope};

/// The single statement this check runs; it returns the offender rows (empty = passed), at most 101.
pub const QUERY: &str = r#"
SELECT
    p.branch_id,
    p.currency,
    p.quantity,
    COALESCE(SUM(m.quantity_signed), 0) AS sum_qty,
    p.cost_thb,
    COALESCE(SUM(m.cost_thb_signed), 0) AS sum_cost
FROM inventory_positions p
LEFT JOIN inventory_movements m
    ON m.organization_id = p.organization_id
   AND m.branch_id = p.branch_id
   AND m.currency = p.currency
WHERE p.organization_id = $1::uuid
  AND ($2::text IS NULL OR p.branch_id = $2::text)
GROUP BY p.organization_id, p.branch_id, p.currency, p.quantity, p.cost_thb
HAVING p.quantity <> COALESCE(SUM(m.quantity_signed), 0)
    OR p.cost_thb <> COALESCE(SUM(m.cost_thb_signed), 0)
ORDER BY p.branch_id, p.currency
LIMIT 101
"#;

#[derive(FromRow)]
struct OffenderRow {
    branch_id: String,
    currency: String,
    quantity: Decimal,
    sum_qty: Decimal,
    cost_thb: Decimal,
    sum_cost: Decimal,
}

pub fn check() -> std::sync::Arc<dyn super::ReconcileCheck> {
    std::sync::Arc::new(PositionsCheck)
}

/// Check 2: a position equals Σ of its movements (quantity and cost).
pub struct PositionsCheck;

impl ReconcileCheck for PositionsCheck {
    fn id(&self) -> u8 {
        2
    }

    fn run_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        scope: &'a ReconcileScope,
    ) -> BoxFuture<'a, AppResult<Vec<Offender>>> {
        Box::pin(async move {
            let branch = scope.branch.as_ref().map(|b| b.as_str());
            let rows = sqlx::query_as::<_, OffenderRow>(QUERY)
                .bind(&scope.organization_id)
                .bind(branch)
                .fetch_all(&mut **tx)
                .await
                .map_err(pos_error::AppError::Database)?;

            let is_truncated = rows.len() > 100;
            let mut offenders: Vec<Offender> = rows
                .into_iter()
                .take(100)
                .map(|r| Offender {
                    key: format!("{}:{}", r.branch_id.trim(), r.currency.trim()),
                    detail: format!(
                        "quantity {} vs Σ {}; cost {} vs Σ {}",
                        r.quantity, r.sum_qty, r.cost_thb, r.sum_cost
                    ),
                })
                .collect();

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
