use std::future::Future;
use std::pin::Pin;

use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;
use xc_exchange::domain::booking::entities::booking_steps::LiveCustomer;
use xc_exchange::domain::booking::repositories::booking_ports::{BookingPorts, CounterLine};
use xc_exchange::domain::booking::value_objects::booking_rules::BookingRules;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ExchangeArithmetic, ForeignAmount, Money, Side};
use xc_rate::domain::policy::stale_policy::{OnStale, StalePolicy};
use xc_rate::domain::publication::published_rates::PublishedRates;
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
        amount: Option<Money>,
    ) -> BoxFuture<'a, AppResult<Option<CounterLine>>> {
        Box::pin(async move {
            let published_rates = PgPublishedRates::new(self.pool.clone());
            let published_side = published_rates.line_in(tx, branch, *code, side).await?;

            let published_side = match published_side {
                Some(s) => s,
                None => return Ok(None),
            };

            if let Some(amount) = amount {
                let foreign =
                    ForeignAmount::from_decimal(Decimal::new(amount.minor_i128() as i64, 2));
                let thb_amount =
                    ExchangeArithmetic::thb(foreign, published_side.rate, published_side.unit_size);

                if published_side.stale {
                    let snapshot_row: Option<serde_json::Value> = sqlx::query_scalar(
                        "SELECT source_snapshot FROM published_rate_lines WHERE id = $1",
                    )
                    .bind(published_side.line_id.as_uuid())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;

                    let policy_id = snapshot_row
                        .as_ref()
                        .and_then(|s| s.get("policy_version_id"))
                        .and_then(|v| v.as_str())
                        .and_then(|s| Uuid::parse_str(s).ok());

                    if let Some(policy_id) = policy_id {
                        let policy_row: Option<(Option<String>, Option<Decimal>)> = sqlx::query_as(
                            "SELECT on_stale, stale_limit_thb FROM rate_policy_versions WHERE id = $1",
                        )
                        .bind(policy_id)
                        .fetch_optional(&mut **tx)
                        .await
                        .map_err(AppError::Database)?;

                        if let Some((on_stale_str, stale_limit_dec)) = policy_row {
                            let on_stale = on_stale_str.as_deref().and_then(OnStale::parse_db);
                            let stale_limit_thb =
                                stale_limit_dec.and_then(|d| Money::parse(&d.to_string()).ok());
                            let stale_policy = StalePolicy::new(on_stale, stale_limit_thb);
                            if stale_policy.admits(&published_side, thb_amount).is_err() {
                                return Ok(None);
                            }
                        }
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
}
