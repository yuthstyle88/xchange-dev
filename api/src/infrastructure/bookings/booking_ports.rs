use std::future::Future;
use std::pin::Pin;

use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;
use xc_exchange::domain::booking::entities::booking_steps::LiveCustomer;
use xc_exchange::domain::booking::repositories::booking_ports::{BookingPorts, CounterLine};
use xc_exchange::domain::booking::value_objects::booking_error::BookingError;
use xc_exchange::domain::booking::value_objects::booking_rules::BookingRules;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ExchangeArithmetic, ForeignAmount, Money, Side};
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::domain::publication::published_rates::{
    LineSelector, PublishedRateLineId, PublishedRates,
};
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;

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
                "SELECT c.id, c.display_name \
                 FROM customers c \
                 JOIN branches b ON b.organization_id = c.organization_id \
                 WHERE c.id = $1 AND b.id = $2 AND c.mode = 'LIVE'",
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
        amount: Option<Money>,
    ) -> BoxFuture<'a, AppResult<Option<CounterLine>>> {
        Box::pin(async move {
            let published_rates = PgPublishedRates::new(self.pool.clone());
            let quantity =
                amount.map(|a| ForeignAmount::from_decimal(Decimal::new(a.minor_i128() as i64, 2)));
            let published_side = published_rates
                .line_for(
                    tx,
                    branch,
                    LineSelector {
                        code: *code,
                        side,
                        group: GroupCode::all(),
                        quantity,
                    },
                )
                .await?;

            let published_side = match published_side {
                Some(s) => s,
                None => return Ok(None),
            };

            if let Some(amount) = amount {
                let foreign =
                    ForeignAmount::from_decimal(Decimal::new(amount.minor_i128() as i64, 2));
                let thb_amount = ExchangeArithmetic::try_thb(
                    foreign,
                    published_side.rate,
                    published_side.unit_size,
                )
                .map_err(|_| AppError::Domain(BookingError::AmountOutOfRange.to_string()))?;

                if published_side.stale {
                    let stale_policy = published_rates.stale_policy_in(tx, &published_side).await?;
                    if stale_policy.admits(&published_side, thb_amount).is_err() {
                        return Ok(None);
                    }
                }
            }

            Ok(Some(CounterLine {
                line_id: published_side.line_id.as_uuid(),
                rate: published_side.rate,
                unit_size: published_side.unit_size,
            }))
        })
    }

    fn stored_line_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        _code: &'a CurrencyCode,
        side: Side,
        line_id: Uuid,
    ) -> BoxFuture<'a, AppResult<Option<CounterLine>>> {
        Box::pin(async move {
            let published_rates = PgPublishedRates::new(self.pool.clone());
            let published_side = published_rates
                .line_by_id(tx, branch, PublishedRateLineId::new(line_id), side)
                .await?;

            let published_side = match published_side {
                Some(s) => s,
                None => return Ok(None),
            };

            Ok(Some(CounterLine {
                line_id: published_side.line_id.as_uuid(),
                rate: published_side.rate,
                unit_size: published_side.unit_size,
            }))
        })
    }
}
