use futures::future::BoxFuture;
use pos_error::AppResult;
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Row, Transaction};
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::inventory::repository::{InventoryRepository, Lot, Movement};
use crate::domain::inventory::weighted_average::WeightedAverage;

/// The PostgreSQL implementation of `InventoryRepository`.
pub struct PgInventoryRepository {
    #[allow(dead_code)]
    pool: PgPool,
}

impl PgInventoryRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl InventoryRepository for PgInventoryRepository {
    fn position_for_update_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: &'a str,
        branch: &'a BranchId,
        currency: CurrencyCode,
    ) -> BoxFuture<'a, AppResult<WeightedAverage>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT quantity, cost_thb FROM inventory_positions \
                 WHERE organization_id = $1::uuid AND branch_id = $2 AND currency = $3 \
                 FOR UPDATE",
            )
            .bind(organization_id)
            .bind(branch.as_str())
            .bind(currency.as_str())
            .fetch_optional(&mut **tx)
            .await?;

            if let Some(r) = row {
                let quantity: Decimal = r.try_get("quantity")?;
                let cost_thb: Decimal = r.try_get("cost_thb")?;
                Ok(WeightedAverage { quantity, cost_thb })
            } else {
                Ok(WeightedAverage::EMPTY)
            }
        })
    }

    fn record_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        movement: &'a Movement,
        lot: Option<&'a Lot>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if let Some(l) = lot {
                sqlx::query(
                    "INSERT INTO inventory_lots (id, organization_id, branch_id, currency, denomination_id, acquired_at, quantity, unit_cost_thb, source_posting_id) \
                     VALUES ($1, $2::uuid, $3, $4, $5, $6, $7, $8, $9)",
                )
                .bind(l.id)
                .bind(&l.organization_id)
                .bind(l.branch.as_str())
                .bind(l.currency.as_str())
                .bind(l.denomination_id)
                .bind(l.acquired_at)
                .bind(l.quantity)
                .bind(l.unit_cost_thb)
                .bind(l.source_posting_id.0)
                .execute(&mut **tx)
                .await?;
            }

            sqlx::query(
                "INSERT INTO inventory_movements (organization_id, branch_id, currency, lot_id, posting_id, transaction_line_id, movement_type, quantity_signed, cost_thb_signed, average_after, created_at) \
                 VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
            )
            .bind(&movement.organization_id)
            .bind(movement.branch.as_str())
            .bind(movement.currency.as_str())
            .bind(movement.lot_id)
            .bind(movement.posting_id.0)
            .bind(movement.transaction_line_id)
            .bind(movement.movement_type.as_str())
            .bind(movement.quantity_signed)
            .bind(movement.cost_thb_signed)
            .bind(movement.average_after)
            .bind(movement.created_at)
            .execute(&mut **tx)
            .await?;

            sqlx::query(
                "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb, version) \
                 VALUES ($1::uuid, $2, $3, $4, $5, 1) \
                 ON CONFLICT (organization_id, branch_id, currency) \
                 DO UPDATE SET \
                     quantity = inventory_positions.quantity + EXCLUDED.quantity, \
                     cost_thb = inventory_positions.cost_thb + EXCLUDED.cost_thb, \
                     version = inventory_positions.version + 1",
            )
            .bind(&movement.organization_id)
            .bind(movement.branch.as_str())
            .bind(movement.currency.as_str())
            .bind(movement.quantity_signed)
            .bind(movement.cost_thb_signed)
            .execute(&mut **tx)
            .await?;

            Ok(())
        })
    }
}
