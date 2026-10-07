//! The real `KioskPorts` and the kiosk's audit log (ticket #172, T12c).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use pos_infra::transactions::Transactor;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::PgPool;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Rate, Side};
use xc_kernel::payment::PaymentRules;
use xc_kiosk::domain::kiosk::commands::save_counter_rate::SaveCounterRate;
use xc_kiosk::domain::kiosk::repositories::kiosk_ports::KioskPorts;
use xc_kiosk::domain::kiosk::value_objects::audit_line::{AuditLine, CUSTOMER_KIOSK, KioskAudit};
use xc_kiosk::domain::kiosk::value_objects::counter_view::{CounterRate, CounterView};
use xc_rate::application::rate::service::RateService;
use xc_rate::domain::rate::commands::save_rate::SaveRate;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::branch::branch_repository::PgBranchRepository;

use crate::application::settings::payment_settings::PaymentSettingsService;
use crate::infrastructure::devices::device_repository::{DeviceRepository, NewEvent};
use crate::infrastructure::devices::pg_device_repository::PgDeviceRepository;
use crate::infrastructure::settings::branch_setting_store::BranchSettingStore;

/// Type alias for box future without external crate dependency.
type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Where the kiosk's audit lines go: one line, written **inside the transaction it is handed**
/// (the transaction of the step it records). Looked up as `web::Data<dyn KioskAuditLog>`, so a
/// test can register its own after the route table.
pub trait KioskAuditLog: Send + Sync {
    fn record_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        line: &'a AuditLine,
        at: DateTime<Utc>,
    ) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>>;
}

#[derive(Clone)]
pub struct PgKioskAuditLog {
    events: PgDeviceRepository,
}

impl PgKioskAuditLog {
    pub fn new(events: PgDeviceRepository) -> Self {
        Self { events }
    }

    pub fn with_pool(pool: PgPool) -> Self {
        Self {
            events: PgDeviceRepository::new(pool),
        }
    }
}

fn detail_of(what: &KioskAudit) -> Value {
    match what {
        KioskAudit::Request { quote } => json!({
            "type": match quote.side { Side::Buy => "BUY", Side::Sell => "SELL" },
            "currency": quote.currency.as_str(),
            "rate": quote.rate.to_string(),
            "foreign": quote.foreign.to_string(),
            "thb": quote.thb.to_string(),
            "inputAmount": quote.input_amount.to_string(),
            "paymentMethod": quote.payment_method.as_str(),
        }),
        KioskAudit::Accepted | KioskAudit::IdChecked => json!({}),
        KioskAudit::Completed { document_ref } => json!({ "documentRef": document_ref }),
        KioskAudit::CurrencyConfigured { enabled, buy, sell } => {
            json!({ "enabled": enabled, "buy": buy, "sell": sell })
        }
    }
}

impl KioskAuditLog for PgKioskAuditLog {
    fn record_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        line: &'a AuditLine,
        at: DateTime<Utc>,
    ) -> Pin<Box<dyn Future<Output = AppResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let actor: String = if line.actor == CUSTOMER_KIOSK {
                CUSTOMER_KIOSK.to_string()
            } else {
                let maybe_username: Option<String> = sqlx::query_scalar(
                    "SELECT username FROM employees WHERE id = $1 \
                     UNION ALL \
                     SELECT username FROM owners WHERE id = $1 OR $1 = 'owner' \
                     LIMIT 1",
                )
                .bind(&line.actor)
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)?;
                maybe_username.unwrap_or_else(|| line.actor.clone())
            };

            let detail = detail_of(&line.what);
            let event = NewEvent {
                dedupe: None,
                actor: &actor,
                kind: line.what.kind(),
                reference: &line.subject,
                detail: &detail,
                amounts: &[],
                at,
            };
            self.events.insert_event(tx, branch, &event).await?;
            Ok(())
        })
    }
}

#[derive(Clone)]
pub struct PgKioskPorts {
    pool: PgPool,
    audit: Arc<dyn KioskAuditLog>,
    source: Arc<dyn RateSource>,
    /// The branch's payment rules (PM-API), built from the same pool.
    payment: PaymentSettingsService,
}

impl PgKioskPorts {
    pub fn new(pool: PgPool, audit: Arc<dyn KioskAuditLog>, source: Arc<dyn RateSource>) -> Self {
        let payment = PaymentSettingsService::new(
            BranchTx::new(Transactor::new(pool.clone())),
            BranchSettingStore::new(pool.clone()),
            PgBranchRepository::new(pool.clone()),
        );
        Self {
            pool,
            audit,
            source,
            payment,
        }
    }
}

fn parse_rate(dec: Decimal) -> AppResult<Rate> {
    Rate::parse(&dec.normalize().to_string())
        .map_err(|e| AppError::Internal(format!("invalid rate decimal {dec}: {e}")))
}

#[derive(sqlx::FromRow)]
struct RateRow {
    code: String,
    buy: Decimal,
    sell: Decimal,
}

impl KioskPorts for PgKioskPorts {
    fn branch_name<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<Option<String>>> {
        Box::pin(async move {
            let row: Option<(String,)> = sqlx::query_as("SELECT name FROM branches WHERE id = $1")
                .bind(branch.as_str())
                .fetch_optional(&self.pool)
                .await
                .map_err(AppError::Database)?;
            Ok(row.map(|r| r.0))
        })
    }

    fn counter<'a>(
        &'a self,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<CounterView>>> {
        Box::pin(async move {
            let branch_row: Option<(String,)> =
                sqlx::query_as("SELECT name FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(AppError::Database)?;
            let Some((branch_name,)) = branch_row else {
                return Ok(None);
            };

            let open: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM shifts WHERE branch_id = $1 AND status = 'OPEN')",
            )
            .bind(branch.as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)?;

            let rows: Vec<RateRow> = sqlx::query_as(
                "SELECT DISTINCT ON (code) code, buy, sell \
                 FROM rates WHERE branch_id = $1 ORDER BY code, id DESC",
            )
            .bind(branch.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(AppError::Database)?;

            let mut rate_map = HashMap::with_capacity(rows.len());
            for r in rows {
                rate_map.insert(r.code.trim().to_string(), (r.buy, r.sell));
            }

            let mut rates = Vec::new();
            for c in &CurrencyCode::all()[1..] {
                if let Some(&(buy_dec, sell_dec)) = rate_map.get(c.as_str()) {
                    rates.push(CounterRate {
                        code: *c,
                        buy: parse_rate(buy_dec)?,
                        sell: parse_rate(sell_dec)?,
                    });
                }
            }

            Ok(Some(CounterView {
                branch_name,
                open,
                rates,
            }))
        })
    }

    fn counter_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<CounterView>>> {
        Box::pin(async move {
            let branch_row: Option<(String,)> =
                sqlx::query_as("SELECT name FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;
            let Some((branch_name,)) = branch_row else {
                return Ok(None);
            };

            let open: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM shifts WHERE branch_id = $1 AND status = 'OPEN')",
            )
            .bind(branch.as_str())
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            let rows: Vec<RateRow> = sqlx::query_as(
                "SELECT DISTINCT ON (code) code, buy, sell \
                 FROM rates WHERE branch_id = $1 ORDER BY code, id DESC",
            )
            .bind(branch.as_str())
            .fetch_all(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            let mut rate_map = HashMap::with_capacity(rows.len());
            for r in rows {
                rate_map.insert(r.code.trim().to_string(), (r.buy, r.sell));
            }

            let mut rates = Vec::new();
            for c in &CurrencyCode::all()[1..] {
                if let Some(&(buy_dec, sell_dec)) = rate_map.get(c.as_str()) {
                    rates.push(CounterRate {
                        code: *c,
                        buy: parse_rate(buy_dec)?,
                        sell: parse_rate(sell_dec)?,
                    });
                }
            }

            Ok(Some(CounterView {
                branch_name,
                open,
                rates,
            }))
        })
    }

    fn save_rate<'a>(
        &'a self,
        branch: &'a BranchId,
        rate: &'a SaveCounterRate,
        actor: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let actual_actor: String = {
                let maybe_username: Option<String> = sqlx::query_scalar(
                    "SELECT username FROM employees WHERE id = $1 \
                     UNION ALL \
                     SELECT username FROM owners WHERE id = $1 OR $1 = 'owner' \
                     LIMIT 1",
                )
                .bind(actor)
                .fetch_optional(&self.pool)
                .await
                .map_err(AppError::Database)?;
                maybe_username.unwrap_or_else(|| actor.to_string())
            };

            let branch_tx = BranchTx::new(Transactor::new(self.pool.clone()));
            let rate_service = RateService::new(branch_tx, self.source.clone());
            let cmd = SaveRate {
                request_id: rate.request_id.clone(),
                code: rate.code.as_str().to_string(),
                buy: rate.buy.clone(),
                sell: rate.sell.clone(),
                partner_name: rate.partner_name.clone(),
                partner_cost: rate.partner_cost.clone(),
                previous_updated: rate.previous_updated.clone(),
            };
            rate_service
                .save_rate(branch, cmd, &actual_actor, now)
                .await
        })
    }

    fn record_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        line: &'a AuditLine,
        at: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let actor: String = if line.actor == CUSTOMER_KIOSK {
                CUSTOMER_KIOSK.to_string()
            } else {
                let maybe_username: Option<String> = sqlx::query_scalar(
                    "SELECT username FROM employees WHERE id = $1 \
                     UNION ALL \
                     SELECT username FROM owners WHERE id = $1 OR $1 = 'owner' \
                     LIMIT 1",
                )
                .bind(&line.actor)
                .fetch_optional(&mut **tx)
                .await
                .map_err(AppError::Database)?;
                maybe_username.unwrap_or_else(|| line.actor.clone())
            };

            let resolved = AuditLine {
                actor,
                subject: line.subject.clone(),
                what: line.what.clone(),
            };
            self.audit.record_in(tx, branch, &resolved, at).await
        })
    }

    fn payment_rules<'a>(&'a self, branch: &'a BranchId) -> BoxFuture<'a, AppResult<PaymentRules>> {
        Box::pin(self.payment.rules(branch))
    }

    fn payment_rules_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<PaymentRules>> {
        Box::pin(self.payment.rules_in(tx, branch))
    }
}
