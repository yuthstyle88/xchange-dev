//! B05 `PgStockReservations` (design `docs/design/blueprint/stock-reservation.md` §3, §4, D-1 … D-8, D-15, D-16).
//!
//! Skeleton by the B05 sub-controller (claude/6530d490) on `feat/b05-stock`; SR-1 fills the bodies.
//! `new()` takes no pool: every statement runs on the caller's transaction or connection, as
//! `LedgerCashPostings` does.

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::{PgConnection, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;

use crate::domain::reservation::availability::StockAvailability;
use crate::domain::reservation::port::{ReserveLine, StockReservations};
use crate::domain::reservation::stock_reservation::{
    ReleaseReason, ReservationId, ReservationStatus,
};

#[derive(Clone, Default)]
pub struct PgStockReservations;

impl PgStockReservations {
    pub fn new() -> Self {
        Self
    }
}

fn db_err(e: sqlx::Error) -> AppError {
    AppError::Database(e)
}

fn validation(detail: &str) -> AppError {
    AppError::Domain(format!("validationFailed: {detail}"))
}

fn stock_insufficient(detail: &str) -> AppError {
    AppError::Domain(format!("stockInsufficient: {detail}"))
}

async fn lock_on_hand(
    conn: &mut PgConnection,
    organization_id: Uuid,
    branch: &str,
    currency: &str,
) -> AppResult<Decimal> {
    let quantity: Option<Decimal> = sqlx::query_scalar(
        "SELECT quantity FROM inventory_positions \
         WHERE organization_id = $1 AND branch_id = $2 AND currency = $3 \
         FOR UPDATE",
    )
    .bind(organization_id)
    .bind(branch)
    .bind(currency)
    .fetch_optional(conn)
    .await
    .map_err(db_err)?;
    Ok(quantity.unwrap_or(Decimal::ZERO))
}

/// Σ `quantity` of the active reservations (D-3: `RESERVED` and `expires_at > now`) of one (organisation, branch,
/// currency). The one reading of "reserved": `availability_in`, the `branch_stock` view (SR-4), the low-stock alert
/// (SR-5) and the completion's check in `LedgerCashPostings` (SR-6) all call this.
pub async fn active_reserved_in(
    conn: &mut PgConnection,
    organization_id: Uuid,
    branch: &BranchId,
    currency: CurrencyCode,
    now: DateTime<Utc>,
) -> AppResult<Decimal> {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(quantity), 0) FROM stock_reservations \
         WHERE organization_id = $1 AND branch_id = $2 AND currency = $3 \
           AND status = $4 AND expires_at > $5",
    )
    .bind(organization_id)
    .bind(branch.as_str())
    .bind(currency.as_str())
    .bind(ReservationStatus::Reserved.as_str())
    .bind(now)
    .fetch_one(conn)
    .await
    .map_err(db_err)
}

impl StockReservations for PgStockReservations {
    fn reserve_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        line: &'a ReserveLine,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<ReservationId>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT ql.organization_id, q.branch_id, ql.currency, ql.shop_side, ql.quantity, q.expires_at \
                 FROM quote_lines ql \
                 JOIN quotes q ON q.id = ql.quote_id AND q.organization_id = ql.organization_id \
                 WHERE ql.id = $1",
            )
            .bind(line.quote_line_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db_err)?
            .ok_or_else(|| validation("quote line not found"))?;

            let organization_id: Uuid = row.try_get("organization_id").map_err(db_err)?;
            let branch_id: String = row.try_get("branch_id").map_err(db_err)?;
            let currency_str: String = row.try_get("currency").map_err(db_err)?;
            let shop_side: String = row.try_get("shop_side").map_err(db_err)?;
            let quantity: Decimal = row.try_get("quantity").map_err(db_err)?;
            let expires_at: DateTime<Utc> = row.try_get("expires_at").map_err(db_err)?;

            let currency =
                CurrencyCode::parse(currency_str.trim()).map_err(|e| validation(&e.to_string()))?;
            if shop_side != "SELL" || currency.as_str() == "THB" {
                return Err(validation(
                    "only foreign currency SELL lines can be reserved",
                ));
            }

            let branch = BranchId::new(branch_id);
            let on_hand =
                lock_on_hand(tx, organization_id, branch.as_str(), currency.as_str()).await?;

            if let Some(existing) = sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM stock_reservations \
                 WHERE quote_line_id = $1 AND status = $2 AND expires_at > $3",
            )
            .bind(line.quote_line_id)
            .bind(ReservationStatus::Reserved.as_str())
            .bind(now)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db_err)?
            {
                return Ok(ReservationId(existing));
            }

            sqlx::query(
                "UPDATE stock_reservations \
                 SET status = $1, released_reason = $2, updated_at = $3 \
                 WHERE organization_id = $4 AND branch_id = $5 AND currency = $6 \
                   AND status = $7 AND expires_at <= $3",
            )
            .bind(ReservationStatus::Released(ReleaseReason::Expired).as_str())
            .bind(ReleaseReason::Expired.as_str())
            .bind(now)
            .bind(organization_id)
            .bind(branch.as_str())
            .bind(currency.as_str())
            .bind(ReservationStatus::Reserved.as_str())
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;

            let reserved = active_reserved_in(tx, organization_id, &branch, currency, now).await?;
            let available = StockAvailability { on_hand, reserved }.available();
            if available < quantity {
                return Err(stock_insufficient(
                    "available stock is less than quote line quantity",
                ));
            }

            let id = Uuid::new_v4();
            sqlx::query(
                "INSERT INTO stock_reservations ( \
                    id, organization_id, branch_id, quote_line_id, currency, denomination_id, \
                    quantity, expires_at, status, transaction_id, released_reason, created_at, updated_at \
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NULL, NULL, $10, $10)",
            )
            .bind(id)
            .bind(organization_id)
            .bind(branch.as_str())
            .bind(line.quote_line_id)
            .bind(currency.as_str())
            .bind(line.denomination_id)
            .bind(quantity)
            .bind(expires_at)
            .bind(ReservationStatus::Reserved.as_str())
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;

            Ok(ReservationId(id))
        })
    }

    fn consume_for_quote_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        quote_id: Uuid,
        transaction_id: Uuid,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            // No row lock here: an expired row locks its position first (L-5 order, as `reserve_in`), and the
            // UPDATE below re-checks `RESERVED`, so a row a concurrent reserve released lazily is left as it is.
            let rows = sqlx::query(
                "SELECT sr.id, sr.organization_id, sr.branch_id, sr.currency, sr.quantity, sr.expires_at \
                 FROM stock_reservations sr \
                 JOIN quote_lines ql ON ql.id = sr.quote_line_id AND ql.organization_id = sr.organization_id \
                 WHERE ql.quote_id = $1 AND sr.status = $2 \
                 ORDER BY sr.branch_id, sr.currency, sr.id",
            )
            .bind(quote_id)
            .bind(ReservationStatus::Reserved.as_str())
            .fetch_all(&mut **tx)
            .await
            .map_err(db_err)?;

            for row in rows {
                let id: Uuid = row.try_get("id").map_err(db_err)?;
                let organization_id: Uuid = row.try_get("organization_id").map_err(db_err)?;
                let branch_id: String = row.try_get("branch_id").map_err(db_err)?;
                let currency_str: String = row.try_get("currency").map_err(db_err)?;
                let quantity: Decimal = row.try_get("quantity").map_err(db_err)?;
                let expires_at: DateTime<Utc> = row.try_get("expires_at").map_err(db_err)?;

                if expires_at <= now {
                    let currency = CurrencyCode::parse(currency_str.trim())
                        .map_err(|e| validation(&e.to_string()))?;
                    let branch = BranchId::new(branch_id);
                    let on_hand =
                        lock_on_hand(tx, organization_id, branch.as_str(), currency.as_str())
                            .await?;
                    let reserved =
                        active_reserved_in(tx, organization_id, &branch, currency, now).await?;
                    let available = StockAvailability { on_hand, reserved }.available();
                    if available < quantity {
                        return Err(stock_insufficient(
                            "available stock is less than expired reservation quantity",
                        ));
                    }
                }

                sqlx::query(
                    "UPDATE stock_reservations \
                     SET status = $1, transaction_id = $2, updated_at = $3 \
                     WHERE id = $4 AND status = $5",
                )
                .bind(ReservationStatus::Consumed.as_str())
                .bind(transaction_id)
                .bind(now)
                .bind(id)
                .bind(ReservationStatus::Reserved.as_str())
                .execute(&mut **tx)
                .await
                .map_err(db_err)?;
            }

            Ok(())
        })
    }

    fn release_for_quote_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        quote_id: Uuid,
        reason: ReleaseReason,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE stock_reservations sr \
                 SET status = $1, released_reason = $2, updated_at = now() \
                 FROM quote_lines ql \
                 WHERE ql.id = sr.quote_line_id \
                   AND ql.organization_id = sr.organization_id \
                   AND ql.quote_id = $3 \
                   AND sr.status = $4",
            )
            .bind(ReservationStatus::Released(reason).as_str())
            .bind(reason.as_str())
            .bind(quote_id)
            .bind(ReservationStatus::Reserved.as_str())
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
            Ok(())
        })
    }

    fn availability_in<'a>(
        &'a self,
        conn: &'a mut PgConnection,
        organization_id: Uuid,
        branch: &'a BranchId,
        currency: CurrencyCode,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<StockAvailability>> {
        Box::pin(async move {
            let on_hand: Decimal = sqlx::query_scalar(
                "SELECT COALESCE((SELECT quantity FROM inventory_positions \
                  WHERE organization_id = $1 AND branch_id = $2 AND currency = $3), 0)",
            )
            .bind(organization_id)
            .bind(branch.as_str())
            .bind(currency.as_str())
            .fetch_one(&mut *conn)
            .await
            .map_err(db_err)?;
            let reserved = active_reserved_in(conn, organization_id, branch, currency, now).await?;
            Ok(StockAvailability { on_hand, reserved })
        })
    }
}
