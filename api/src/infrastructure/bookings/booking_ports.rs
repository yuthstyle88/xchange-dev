use std::future::Future;
use std::pin::Pin;

use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use sqlx::PgPool;
use xc_exchange::domain::booking::entities::booking_steps::LiveCustomer;
use xc_exchange::domain::booking::repositories::booking_ports::BookingPorts;
use xc_exchange::domain::booking::value_objects::booking_rules::BookingRules;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate, Side};

use crate::application::bookings::booking_rules_service::BookingRulesService;
use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone)]
pub struct PgBookingPorts {
    pool: PgPool,
    service: BookingRulesService,
}

impl PgBookingPorts {
    pub fn new(pool: PgPool) -> Self {
        let branch_tx = BranchTx::new(Transactor::new(pool.clone()));
        let store = BranchSettingStore::new(pool.clone());
        let service = BookingRulesService::new(branch_tx, store);
        Self { pool, service }
    }
}

fn parse_rate(dec: Decimal) -> AppResult<Rate> {
    Rate::parse(&dec.normalize().to_string())
        .map_err(|e| AppError::Internal(format!("invalid rate decimal {dec}: {e}")))
}

impl BookingPorts for PgBookingPorts {
    fn branch_exists<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM branches WHERE id = $1)")
                    .bind(branch.as_str())
                    .fetch_one(&self.pool)
                    .await
                    .map_err(AppError::Database)?;
            Ok(exists)
        })
    }

    fn live_customer<'a>(
        &'a self,
        branch: &'a BranchId,
        id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<LiveCustomer>>> {
        Box::pin(async move {
            let row: Option<(String, String)> = sqlx::query_as(
                "SELECT id, name FROM customers WHERE id = $1 AND branch_id = $2 AND mode = 'LIVE'",
            )
            .bind(id)
            .bind(branch.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(AppError::Database)?;
            Ok(row.map(|(id, name)| LiveCustomer { id, name }))
        })
    }

    fn rules<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<Option<BookingRules>>> {
        Box::pin(self.service.rules(branch))
    }

    fn rules_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<BookingRules>>> {
        Box::pin(self.service.rules_in(tx, branch))
    }

    fn has_open_shift_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM shifts WHERE branch_id = $1 AND status = 'OPEN')",
            )
            .bind(branch.as_str())
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;
            Ok(exists)
        })
    }

    fn counter_rate_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        code: &'a CurrencyCode,
        side: Side,
    ) -> BoxFuture<'a, AppResult<Option<Rate>>> {
        Box::pin(async move {
            let row: Option<(Decimal, Decimal)> = sqlx::query_as(
                "SELECT buy, sell FROM rates WHERE branch_id = $1 AND code = $2 ORDER BY id DESC LIMIT 1",
            )
            .bind(branch.as_str())
            .bind(code.as_str())
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            match row {
                Some((buy, sell)) => {
                    let dec = match side {
                        Side::Buy => buy,
                        Side::Sell => sell,
                    };
                    let rate = parse_rate(dec)?;
                    Ok(Some(rate))
                }
                None => Ok(None),
            }
        })
    }
}
