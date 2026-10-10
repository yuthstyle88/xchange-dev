//! PostgreSQL implementation of [`RateAlertRepository`]: `rate_alerts`, `rate_alert_settings` and the
//! `source_snapshot` of a published line (migration `20261107100000_rate_alerts.sql`).

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use rust_decimal::Decimal;
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Side, UnitSize};
use xc_rate::domain::publication::published_rates::PublishedRateLineId;

use crate::domain::condition::AlertCondition;
use crate::domain::rate_alert::{RateAlert, RateAlertId};
use crate::domain::relation::Relation;
use crate::domain::repository::{KeyedAlert, RateAlertRepository};
use crate::domain::settings::AlertSettings;
use crate::domain::source_policy::{ObservePolicy, SideMode};

/// The columns [`alert_from_row`] reads.
const ALERT_COLUMNS: &str = "id, organization_id, customer_id, branch_id, currency_code, shop_side, unit_size, \
     condition, target, enabled, cooldown_seconds, expires_at, last_triggered_at, relation, \
     last_publication_version, episode_no, disabled_reason, deleted_at, version, created_at";

/// The one mapping of a `rate_alerts` row.
fn alert_from_row(r: &PgRow) -> AppResult<RateAlert> {
    let currency: String = r.try_get("currency_code").map_err(AppError::Database)?;
    let shop_side: String = r.try_get("shop_side").map_err(AppError::Database)?;
    let unit_size: Decimal = r.try_get("unit_size").map_err(AppError::Database)?;
    let condition: String = r.try_get("condition").map_err(AppError::Database)?;
    let relation: String = r.try_get("relation").map_err(AppError::Database)?;
    let branch: String = r.try_get("branch_id").map_err(AppError::Database)?;
    Ok(RateAlert {
        id: RateAlertId::new(r.try_get("id").map_err(AppError::Database)?),
        organization_id: r.try_get("organization_id").map_err(AppError::Database)?,
        customer_id: r.try_get("customer_id").map_err(AppError::Database)?,
        branch: BranchId::new(branch),
        currency: CurrencyCode::parse(&currency)
            .map_err(|e| AppError::Internal(format!("invalid currency {currency}: {e}")))?,
        shop_side: match shop_side.as_str() {
            "BUY" => Side::Buy,
            "SELL" => Side::Sell,
            other => return Err(AppError::Internal(format!("invalid shop side: {other}"))),
        },
        unit_size: UnitSize::from_decimal(unit_size)
            .map_err(|e| AppError::Internal(format!("invalid unit size {unit_size}: {e}")))?,
        condition: AlertCondition::parse(&condition)
            .ok_or_else(|| AppError::Internal(format!("invalid condition: {condition}")))?,
        target: r.try_get("target").map_err(AppError::Database)?,
        enabled: r.try_get("enabled").map_err(AppError::Database)?,
        cooldown_seconds: r.try_get("cooldown_seconds").map_err(AppError::Database)?,
        expires_at: r.try_get("expires_at").map_err(AppError::Database)?,
        last_triggered_at: r.try_get("last_triggered_at").map_err(AppError::Database)?,
        relation: Relation::parse(&relation)
            .ok_or_else(|| AppError::Internal(format!("invalid relation: {relation}")))?,
        last_publication_version: r
            .try_get("last_publication_version")
            .map_err(AppError::Database)?,
        episode_no: r.try_get("episode_no").map_err(AppError::Database)?,
        disabled_reason: r.try_get("disabled_reason").map_err(AppError::Database)?,
        deleted_at: r.try_get("deleted_at").map_err(AppError::Database)?,
        version: r.try_get("version").map_err(AppError::Database)?,
    })
}

fn side_word(side: Side) -> &'static str {
    match side {
        Side::Buy => "BUY",
        Side::Sell => "SELL",
    }
}

pub struct PgRateAlertRepository {
    _pool: PgPool,
}

impl PgRateAlertRepository {
    /// Every method works in the caller's transaction; the pool is kept for the constructor's contract.
    pub fn new(pool: PgPool) -> Self {
        Self { _pool: pool }
    }
}

impl RateAlertRepository for PgRateAlertRepository {
    fn lock_due_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        version: i32,
    ) -> BoxFuture<'a, AppResult<Vec<RateAlert>>> {
        Box::pin(async move {
            let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT {ALERT_COLUMNS} FROM rate_alerts \
                 WHERE branch_id = $1 \
                   AND enabled = true \
                   AND deleted_at IS NULL \
                   AND (expires_at IS NULL OR expires_at > now()) \
                   AND (last_publication_version IS NULL OR last_publication_version < $2) \
                 FOR UPDATE SKIP LOCKED"
            )))
            .bind(branch.as_str())
            .bind(version)
            .fetch_all(&mut **tx)
            .await
            .map_err(AppError::Database)?;
            rows.iter().map(alert_from_row).collect()
        })
    }

    fn lock_customer_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        customer_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<Uuid>>> {
        Box::pin(async move {
            sqlx::query_scalar("SELECT organization_id FROM customers WHERE id = $1 FOR UPDATE")
                .bind(customer_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)
        })
    }

    fn settings_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: Uuid,
    ) -> BoxFuture<'a, AppResult<Option<AlertSettings>>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT default_cooldown_seconds, max_active_per_customer, default_lifetime_days, \
                     observe_policy, updated_by, updated_at \
                 FROM rate_alert_settings WHERE organization_id = $1",
            )
            .bind(organization_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;
            let Some(r) = row else {
                return Ok(None);
            };
            let policy: String = r.try_get("observe_policy").map_err(AppError::Database)?;
            Ok(Some(AlertSettings {
                organization_id,
                default_cooldown_seconds: r
                    .try_get("default_cooldown_seconds")
                    .map_err(AppError::Database)?,
                max_active_per_customer: r
                    .try_get("max_active_per_customer")
                    .map_err(AppError::Database)?,
                default_lifetime_days: r
                    .try_get("default_lifetime_days")
                    .map_err(AppError::Database)?,
                observe_policy: ObservePolicy::parse(&policy).ok_or_else(|| {
                    AppError::Internal(format!("invalid observe_policy: {policy}"))
                })?,
                updated_by: r.try_get("updated_by").map_err(AppError::Database)?,
                updated_at: r.try_get("updated_at").map_err(AppError::Database)?,
            }))
        })
    }

    fn save_settings_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        settings: &'a AlertSettings,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO rate_alert_settings (organization_id, default_cooldown_seconds, \
                     max_active_per_customer, default_lifetime_days, observe_policy, updated_by, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7) \
                 ON CONFLICT (organization_id) DO UPDATE SET \
                     default_cooldown_seconds = EXCLUDED.default_cooldown_seconds, \
                     max_active_per_customer = EXCLUDED.max_active_per_customer, \
                     default_lifetime_days = EXCLUDED.default_lifetime_days, \
                     observe_policy = EXCLUDED.observe_policy, \
                     updated_by = EXCLUDED.updated_by, \
                     updated_at = EXCLUDED.updated_at",
            )
            .bind(settings.organization_id)
            .bind(settings.default_cooldown_seconds)
            .bind(settings.max_active_per_customer)
            .bind(settings.default_lifetime_days)
            .bind(settings.observe_policy.as_str())
            .bind(settings.updated_by)
            .bind(settings.updated_at)
            .execute(&mut **tx)
            .await
            .map_err(AppError::Database)?;
            Ok(())
        })
    }

    fn find_by_key_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        customer_id: &'a str,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<KeyedAlert>>> {
        Box::pin(async move {
            let row = sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT {ALERT_COLUMNS} FROM rate_alerts WHERE customer_id = $1 AND idempotency_key = $2"
            )))
            .bind(customer_id)
            .bind(idempotency_key)
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;
            match row {
                Some(r) => {
                    let created_at = r.try_get("created_at").map_err(AppError::Database)?;
                    Ok(Some(KeyedAlert {
                        alert: alert_from_row(&r)?,
                        created_at,
                    }))
                }
                None => Ok(None),
            }
        })
    }

    fn count_active_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        customer_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<i64>> {
        Box::pin(async move {
            sqlx::query_scalar(
                "SELECT count(*) FROM rate_alerts \
                 WHERE customer_id = $1 AND deleted_at IS NULL AND (expires_at IS NULL OR expires_at > $2)",
            )
            .bind(customer_id)
            .bind(now)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)
        })
    }

    fn insert_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        alert: &'a RateAlert,
        idempotency_key: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO rate_alerts (id, organization_id, customer_id, branch_id, currency_code, \
                     shop_side, unit_size, condition, target, enabled, cooldown_seconds, expires_at, \
                     last_triggered_at, relation, last_publication_version, episode_no, disabled_reason, \
                     deleted_at, version, idempotency_key, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, \
                     $19, $20, $21, $21)",
            )
            .bind(alert.id.as_uuid())
            .bind(alert.organization_id)
            .bind(&alert.customer_id)
            .bind(alert.branch.as_str())
            .bind(alert.currency.as_str())
            .bind(side_word(alert.shop_side))
            .bind(alert.unit_size.value())
            .bind(alert.condition.as_str())
            .bind(alert.target)
            .bind(alert.enabled)
            .bind(alert.cooldown_seconds)
            .bind(alert.expires_at)
            .bind(alert.last_triggered_at)
            .bind(alert.relation.as_str())
            .bind(alert.last_publication_version)
            .bind(alert.episode_no)
            .bind(&alert.disabled_reason)
            .bind(alert.deleted_at)
            .bind(alert.version)
            .bind(idempotency_key)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(AppError::Database)?;
            Ok(())
        })
    }

    fn lock_alert_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: RateAlertId,
    ) -> BoxFuture<'a, AppResult<Option<RateAlert>>> {
        Box::pin(async move {
            let row = sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT {ALERT_COLUMNS} FROM rate_alerts WHERE id = $1 FOR UPDATE"
            )))
            .bind(id.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?;
            row.as_ref().map(alert_from_row).transpose()
        })
    }

    fn save_lifecycle_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        alert: &'a RateAlert,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE rate_alerts SET enabled = $2, relation = $3, last_publication_version = $4, \
                     deleted_at = $5, version = $6, updated_at = $7 \
                 WHERE id = $1",
            )
            .bind(alert.id.as_uuid())
            .bind(alert.enabled)
            .bind(alert.relation.as_str())
            .bind(alert.last_publication_version)
            .bind(alert.deleted_at)
            .bind(alert.version)
            .bind(now)
            .execute(&mut **tx)
            .await
            .map_err(AppError::Database)?;
            Ok(())
        })
    }

    fn side_mode_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        line: PublishedRateLineId,
        side: Side,
    ) -> BoxFuture<'a, AppResult<SideMode>> {
        Box::pin(async move {
            let key = match side {
                Side::Buy => "buy_mode",
                Side::Sell => "sell_mode",
            };
            let mode: Option<String> = sqlx::query_scalar(
                "SELECT source_snapshot ->> $2 FROM published_rate_lines WHERE id = $1",
            )
            .bind(line.as_uuid())
            .bind(key)
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?
            .flatten();
            Ok(match mode.as_deref() {
                Some("MANUAL") => SideMode::Manual,
                _ => SideMode::Auto,
            })
        })
    }
}
