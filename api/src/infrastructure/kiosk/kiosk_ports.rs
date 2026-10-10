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
use xc_auth::domain::policy::value_objects::role::Role;
use xc_exchange::domain::quote::quote::QuoteChannel;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, MinorUnit, Money, Rate, Side, UnitSize};
use xc_kernel::payment::PaymentRules;
use xc_kiosk::domain::kiosk::commands::save_counter_rate::SaveCounterRate;
use xc_kiosk::domain::kiosk::repositories::kiosk_ports::KioskPorts;
use xc_kiosk::domain::kiosk::value_objects::audit_line::{AuditLine, CUSTOMER_KIOSK, KioskAudit};
use xc_kiosk::domain::kiosk::value_objects::counter_facts::LineId;
use xc_kiosk::domain::kiosk::value_objects::counter_view::{CounterRate, CounterView};
use xc_rate::application::rate::service::RateService;
use xc_rate::domain::publication::published_rates::PublishedRates;
use xc_rate::domain::rate::commands::save_rate::SaveRate;
use xc_rate::domain::rate::entities::rate_source::RateSource;
use xc_rate::infrastructure::branch::branch_repository::PgBranchRepository;
use xc_rate::infrastructure::publication::published_rates::PgPublishedRates;

use crate::application::settings::payment_settings::PaymentSettingsService;
use crate::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use crate::application::workflows::save_step::SaveActor;
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
struct PublishedLineRow {
    id: String,
    currency_code: String,
    buy_rate: Option<Decimal>,
    sell_rate: Option<Decimal>,
    unit_size: Decimal,
    minor_unit: i16,
}

fn parse_counter_rates(rows: Vec<PublishedLineRow>) -> AppResult<Vec<CounterRate>> {
    let mut rate_map = HashMap::with_capacity(rows.len());
    for r in rows {
        rate_map.insert(r.currency_code.trim().to_string(), r);
    }

    let mut rates = Vec::new();
    for c in &CurrencyCode::all()[1..] {
        let Some(r) = rate_map.get(c.as_str()) else {
            continue;
        };
        let (Some(buy_dec), Some(sell_dec)) = (r.buy_rate, r.sell_rate) else {
            continue;
        };
        let buy = parse_rate(buy_dec)?;
        let sell = parse_rate(sell_dec)?;
        let unit_size = UnitSize::from_decimal(r.unit_size)
            .map_err(|e| AppError::Internal(format!("invalid unit size {}: {e}", r.unit_size)))?;
        let minor_unit = MinorUnit::new(r.minor_unit as u8)
            .map_err(|e| AppError::Internal(format!("invalid minor unit {}: {e}", r.minor_unit)))?;
        let line_id = LineId::parse_uuid(&r.id)
            .ok_or_else(|| AppError::Internal(format!("invalid line id uuid {}", r.id)))?;
        rates.push(CounterRate {
            code: *c,
            buy,
            sell,
            unit_size,
            minor_unit,
            line_id,
        });
    }
    Ok(rates)
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
            let branch_row: Option<(String, Option<uuid::Uuid>)> =
                sqlx::query_as("SELECT name, current_publication_id FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(AppError::Database)?;
            let Some((branch_name, current_pub_id)) = branch_row else {
                return Ok(None);
            };

            let open: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM shifts WHERE branch_id = $1 AND status = 'OPEN')",
            )
            .bind(branch.as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(AppError::Database)?;

            let rates = match current_pub_id {
                Some(pub_id) => {
                    let rows: Vec<PublishedLineRow> = sqlx::query_as(
                        "SELECT l.id::text, l.currency_code, l.buy_rate, l.sell_rate, l.unit_size, c.minor_unit \
                         FROM published_rate_lines l \
                         JOIN currencies c ON c.code = l.currency_code \
                         WHERE l.publication_id = $1 AND l.denomination_group = 'ALL'",
                    )
                    .bind(pub_id)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(AppError::Database)?;
                    parse_counter_rates(rows)?
                }
                None => Vec::new(),
            };

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
            let branch_row: Option<(String, Option<uuid::Uuid>)> =
                sqlx::query_as("SELECT name, current_publication_id FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;
            let Some((branch_name, current_pub_id)) = branch_row else {
                return Ok(None);
            };

            let open: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM shifts WHERE branch_id = $1 AND status = 'OPEN')",
            )
            .bind(branch.as_str())
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;

            let rates = match current_pub_id {
                Some(pub_id) => {
                    let rows: Vec<PublishedLineRow> = sqlx::query_as(
                        "SELECT l.id::text, l.currency_code, l.buy_rate, l.sell_rate, l.unit_size, c.minor_unit \
                         FROM published_rate_lines l \
                         JOIN currencies c ON c.code = l.currency_code \
                         WHERE l.publication_id = $1 AND l.denomination_group = 'ALL'",
                    )
                    .bind(pub_id)
                    .fetch_all(&mut **tx)
                    .await
                    .map_err(AppError::Database)?;
                    parse_counter_rates(rows)?
                }
                None => Vec::new(),
            };

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

    #[allow(clippy::too_many_arguments)]
    fn check_stale_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        line_id: LineId,
        currency: CurrencyCode,
        side: Side,
        thb: Money,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        let _ = now;
        Box::pin(async move {
            let published_rates = PgPublishedRates::new(self.pool.clone());
            let published_side = match published_rates.line_in(tx, branch, currency, side).await? {
                Some(s) => s,
                None => return Ok(false),
            };

            let quoted_uuid = uuid::Uuid::from_u128(line_id.0);
            if published_side.line_id.as_uuid() != quoted_uuid {
                return Ok(false);
            }

            if published_side.stale {
                let stale_policy = published_rates.stale_policy_in(tx, &published_side).await?;
                if stale_policy.admits(&published_side, thb).is_err() {
                    return Ok(false);
                }
            }

            Ok(true)
        })
    }

    fn quote_in<'a>(
        &'a self,
        tx: &'a mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &'a BranchId,
        currency: CurrencyCode,
        side: Side,
        quantity: ForeignAmount,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<Option<String>>> {
        Box::pin(async move {
            let quote_service = QuoteService::new(self.pool.clone());
            let req = QuoteRequest {
                branch_id: branch.clone(),
                customer_id: None,
                channel: QuoteChannel::Kiosk,
                lines: vec![QuoteRequestLine {
                    currency,
                    shop_side: side,
                    quantity,
                    fee_thb: None,
                }],
            };
            let who = SaveActor {
                id: CUSTOMER_KIOSK.to_string(),
                username: CUSTOMER_KIOSK.to_string(),
                employee_id: None,
                role: Role::System,
            };
            let quote = quote_service.quote_in(tx, req, &who, now).await?;
            Ok(Some(quote.id.to_string()))
        })
    }
}
