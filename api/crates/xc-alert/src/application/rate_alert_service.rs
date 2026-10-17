//! `RateAlertService`: create, pause, resume, delete an alert; save the organisation's settings (design §4.3, §4.4).
//!
//! Every method works in the caller's transaction; the caller commits. A refusal leaves nothing written. Reads and
//! writes go through [`RateAlertRepository`]; the lifecycle rules are [`RateAlert`]'s (rulings 2 and 3 on #1002).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::AppError;
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Side};
use xc_rate::domain::currency::currency_register::CurrencyRegister;
use xc_rate::domain::publication::freshness::Freshness;
use xc_rate::domain::publication::published_rates::PublishedRates;

use crate::domain::CustomerSide;
use crate::domain::condition::AlertCondition;
pub use crate::domain::error::RateAlertError;
use crate::domain::rate_alert::{RateAlert, RateAlertId};
use crate::domain::relation::Relation;
use crate::domain::repository::{KeyedAlert, RateAlertRepository};
use crate::domain::settings::{AlertSettings, SaveAlertSettings};
use crate::domain::source_policy::{AlertSourcePolicy, Observation, ObservePolicy, SideSnapshot};

/// What `rateAlertCreate` carries (`{branch, currency, customerSide, condition, target, expiresAt?}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRateAlert {
    pub branch: BranchId,
    pub currency: CurrencyCode,
    pub customer_side: CustomerSide,
    pub condition: AlertCondition,
    /// THB per the currency's `unit_size`; must be > 0 with ≤ 8 places.
    pub target: Decimal,
    /// `None` = `now + default_lifetime_days` (or no expiry when that is NULL).
    pub expires_at: Option<DateTime<Utc>>,
}

/// What the current publication says about one alert's side, observed without firing (R5).
struct CurrentObservation {
    relation: Relation,
    publication_version: i32,
}

fn storage(e: AppError) -> RateAlertError {
    match e {
        AppError::Database(e) => RateAlertError::Database(e.to_string()),
        other => RateAlertError::Internal(other.to_string()),
    }
}

fn refused(reason: &'static str, message: &str) -> RateAlertError {
    RateAlertError::ValidationFailed {
        reason: Some(reason),
        message: message.into(),
    }
}

fn not_found() -> RateAlertError {
    refused("alertNotFound", "alert not found")
}

fn settings_missing() -> RateAlertError {
    refused(
        "alertSettingsMissing",
        "rate alert settings not configured for organization",
    )
}

async fn branch_in_organisation_in(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    organization_id: Uuid,
) -> Result<bool, RateAlertError> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM branches WHERE id = $1 AND organization_id = $2)",
    )
    .bind(branch.as_str())
    .bind(organization_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| storage(AppError::Database(e)))
}

pub struct RateAlertService {
    repository: Arc<dyn RateAlertRepository>,
    register: Arc<dyn CurrencyRegister>,
    published: Arc<dyn PublishedRates>,
}

impl RateAlertService {
    pub fn new(
        repository: Arc<dyn RateAlertRepository>,
        register: Arc<dyn CurrencyRegister>,
        published: Arc<dyn PublishedRates>,
    ) -> Self {
        Self {
            repository,
            register,
            published,
        }
    }

    /// R5: the relation the branch's current line for `side` gives, without firing, and that publication's version.
    /// `None` when the branch publishes no such line. A side the policy skips (e.g. stale under `ALL_FRESH`) stays
    /// `UNKNOWN`.
    #[allow(clippy::too_many_arguments)]
    async fn observe_current_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        currency: CurrencyCode,
        side: Side,
        condition: AlertCondition,
        target: Decimal,
        policy: ObservePolicy,
    ) -> Result<Option<CurrentObservation>, RateAlertError> {
        let Some(line) = self
            .published
            .line_in(tx, branch, currency, side)
            .await
            .map_err(storage)?
        else {
            return Ok(None);
        };
        let mode = self
            .repository
            .side_mode_in(tx, line.line_id, side)
            .await
            .map_err(storage)?;
        let observation = AlertSourcePolicy::new(policy).observes(
            &SideSnapshot {
                rate: line.rate.value(),
                mode,
            },
            &Freshness {
                stale: line.stale,
                source_date: line.source_date,
            },
        );
        let relation = match observation {
            Observation::Compare(rate) if condition.satisfied(rate, target) => Relation::Satisfied,
            Observation::Compare(_) => Relation::NotSatisfied,
            Observation::Skip => Relation::Unknown,
        };
        Ok(Some(CurrentObservation {
            relation,
            publication_version: line.publication.version,
        }))
    }

    /// The caller's alert `id`, locked; an unknown id or another customer's → `alertNotFound` (ruling 2: never reveal
    /// that someone else's alert exists); a wrong `expected_version` → `staleVersion`.
    async fn own_alert_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        customer_id: &str,
        id: RateAlertId,
        expected_version: i32,
    ) -> Result<RateAlert, RateAlertError> {
        let alert = self
            .repository
            .lock_alert_in(tx, id)
            .await
            .map_err(storage)?
            .filter(|a| a.customer_id == customer_id)
            .ok_or_else(not_found)?;
        if alert.version != expected_version {
            return Err(RateAlertError::StaleVersion(format!(
                "expected version {}, found {}",
                expected_version, alert.version
            )));
        }
        Ok(alert)
    }

    /// `rateAlertCreate` for the signed-in customer `customer_id` (§4.3 row 2, R1, R2, R5, R11, R14).
    pub async fn create(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        customer_id: &str,
        input: &CreateRateAlert,
        idempotency_key: &str,
        now: DateTime<Utc>,
    ) -> Result<RateAlert, RateAlertError> {
        if input.target <= Decimal::ZERO || input.target.scale() > 8 {
            return Err(RateAlertError::ValidationFailed {
                reason: None,
                message: "target must be positive and at most 8 decimal places".into(),
            });
        }
        if input.expires_at.is_some_and(|at| at <= now) {
            return Err(refused(
                "expiresAtNotInFuture",
                "expires_at must be in the future",
            ));
        }

        // The customer row is locked first, so the limit below is counted with no concurrent create of this customer.
        let organization_id = self
            .repository
            .lock_customer_in(tx, customer_id)
            .await
            .map_err(storage)?
            .ok_or_else(|| refused("alertNotFound", "customer not found"))?;
        let settings = self
            .repository
            .settings_in(tx, organization_id)
            .await
            .map_err(storage)?
            .ok_or_else(settings_missing)?;

        let shop_side = input.customer_side.to_shop();
        let lifetime_from = |start: DateTime<Utc>| {
            settings
                .default_lifetime_days
                .map(|days| start + chrono::Duration::days(i64::from(days)))
        };
        let expires_at = input.expires_at.or_else(|| lifetime_from(now));

        if let Some(KeyedAlert {
            alert: existing,
            created_at,
        }) = self
            .repository
            .find_by_key_in(tx, customer_id, idempotency_key)
            .await
            .map_err(storage)?
        {
            let same_expiry = match input.expires_at {
                Some(at) => existing.expires_at == Some(at),
                None => existing.expires_at == lifetime_from(created_at),
            };
            let same_body = existing.branch == input.branch
                && existing.currency == input.currency
                && existing.shop_side == shop_side
                && existing.condition == input.condition
                && existing.target == input.target
                && same_expiry;
            return if same_body {
                Ok(existing)
            } else {
                Err(RateAlertError::IdempotencyConflict(
                    "idempotency key already used with different parameters".into(),
                ))
            };
        }

        let active = self
            .repository
            .count_active_in(tx, customer_id, now)
            .await
            .map_err(storage)?;
        if active >= i64::from(settings.max_active_per_customer) {
            return Err(refused(
                "alertLimitReached",
                "maximum active rate alerts reached",
            ));
        }

        let units = self
            .register
            .units(input.currency)
            .await
            .map_err(storage)?
            .ok_or_else(|| RateAlertError::ValidationFailed {
                reason: None,
                message: format!("currency {} not registered", input.currency),
            })?;
        let not_publishing = || RateAlertError::ValidationFailed {
            reason: None,
            message: format!(
                "branch {} does not publish {} {:?}",
                input.branch, input.currency, shop_side
            ),
        };
        if !branch_in_organisation_in(tx, &input.branch, organization_id).await? {
            return Err(not_publishing());
        }
        let current = self
            .observe_current_in(
                tx,
                &input.branch,
                input.currency,
                shop_side,
                input.condition,
                input.target,
                settings.observe_policy,
            )
            .await?
            .ok_or_else(not_publishing)?;

        let mut alert = RateAlert {
            id: RateAlertId::new(Uuid::new_v4()),
            organization_id,
            customer_id: customer_id.to_owned(),
            branch: input.branch.clone(),
            currency: input.currency,
            shop_side,
            unit_size: units.unit_size(),
            condition: input.condition,
            target: input.target,
            enabled: true,
            cooldown_seconds: settings.default_cooldown_seconds,
            expires_at,
            last_triggered_at: None,
            relation: Relation::Unknown,
            last_publication_version: None,
            episode_no: 0,
            disabled_reason: None,
            deleted_at: None,
            version: 1,
        };
        alert.observed(current.relation, Some(current.publication_version));
        self.repository
            .insert_in(tx, &alert, idempotency_key, now)
            .await
            .map_err(storage)?;
        Ok(alert)
    }

    /// `rateAlertPause` (R10, R15; ruling 2).
    pub async fn pause(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        customer_id: &str,
        id: RateAlertId,
        expected_version: i32,
        now: DateTime<Utc>,
    ) -> Result<RateAlert, RateAlertError> {
        let mut alert = self
            .own_alert_in(tx, customer_id, id, expected_version)
            .await?;
        if alert.pause(now)? {
            self.repository
                .save_lifecycle_in(tx, &alert, now)
                .await
                .map_err(storage)?;
        }
        Ok(alert)
    }

    /// `rateAlertResume`: relation back to `UNKNOWN`, re-observed against the current publication without firing, and
    /// `last_publication_version` set to that publication's (R5, R10, R15; rulings 2 and 3). With no settings row the
    /// resume is refused `alertSettingsMissing`, as create is.
    pub async fn resume(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        customer_id: &str,
        id: RateAlertId,
        expected_version: i32,
        now: DateTime<Utc>,
    ) -> Result<RateAlert, RateAlertError> {
        let mut alert = self
            .own_alert_in(tx, customer_id, id, expected_version)
            .await?;
        if !alert.resume(now)? {
            return Ok(alert);
        }
        let settings = self
            .repository
            .settings_in(tx, alert.organization_id)
            .await
            .map_err(storage)?
            .ok_or_else(settings_missing)?;
        let current = self
            .observe_current_in(
                tx,
                &alert.branch,
                alert.currency,
                alert.shop_side,
                alert.condition,
                alert.target,
                settings.observe_policy,
            )
            .await?;
        match current {
            Some(c) => alert.observed(c.relation, Some(c.publication_version)),
            None => alert.observed(Relation::Unknown, None),
        }
        self.repository
            .save_lifecycle_in(tx, &alert, now)
            .await
            .map_err(storage)?;
        Ok(alert)
    }

    /// `rateAlertDelete`: `deleted_at` set, events kept (R10, R15; ruling 2).
    pub async fn delete(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        customer_id: &str,
        id: RateAlertId,
        expected_version: i32,
        now: DateTime<Utc>,
    ) -> Result<RateAlert, RateAlertError> {
        let mut alert = self
            .own_alert_in(tx, customer_id, id, expected_version)
            .await?;
        if alert.delete(now)? {
            self.repository
                .save_lifecycle_in(tx, &alert, now)
                .await
                .map_err(storage)?;
        }
        Ok(alert)
    }

    /// `rateAlertSettingsSave`: insert or replace the organisation's one row (R14). Who may call it (EXECUTIVE) is
    /// the area row's decision, not this method's.
    pub async fn save_settings(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        organization_id: Uuid,
        input: &SaveAlertSettings,
        updated_by: Uuid,
        now: DateTime<Utc>,
    ) -> Result<AlertSettings, RateAlertError> {
        let invalid = |message: &str| RateAlertError::ValidationFailed {
            reason: None,
            message: message.into(),
        };
        if input.default_cooldown_seconds < 0 {
            return Err(invalid("default_cooldown_seconds must be >= 0"));
        }
        if input.max_active_per_customer <= 0 {
            return Err(invalid("max_active_per_customer must be > 0"));
        }
        if input.default_lifetime_days.is_some_and(|days| days <= 0) {
            return Err(invalid("default_lifetime_days must be > 0"));
        }
        let settings = AlertSettings {
            organization_id,
            default_cooldown_seconds: input.default_cooldown_seconds,
            max_active_per_customer: input.max_active_per_customer,
            default_lifetime_days: input.default_lifetime_days,
            observe_policy: input.observe_policy,
            updated_by,
            updated_at: now,
        };
        self.repository
            .save_settings_in(tx, &settings)
            .await
            .map_err(storage)?;
        Ok(settings)
    }
}
