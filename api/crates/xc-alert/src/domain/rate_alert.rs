//! `RateAlert`: one customer's watch on one published side (R1, R2, R10, R11). Row of `rate_alerts`.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Side, UnitSize};

use super::condition::AlertCondition;
use super::error::RateAlertError;
use super::relation::Relation;

/// `rate_alerts.id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RateAlertId(Uuid);

impl RateAlertId {
    pub fn new(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for RateAlertId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for RateAlertId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(s).map(Self)
    }
}

/// One row of `rate_alerts` (design §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateAlert {
    pub id: RateAlertId,
    pub organization_id: Uuid,
    pub customer_id: String,
    pub branch: BranchId,
    pub currency: CurrencyCode,
    /// Stored once from `CustomerSide::to_shop()` (R1).
    pub shop_side: Side,
    /// Copied from `CurrencyRegister::units` at creation (R2).
    pub unit_size: UnitSize,
    pub condition: AlertCondition,
    /// THB per `unit_size`, scale ≤ 8 (R2).
    pub target: Decimal,
    pub enabled: bool,
    pub cooldown_seconds: i32,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub relation: Relation,
    pub last_publication_version: Option<i32>,
    pub episode_no: i32,
    pub disabled_reason: Option<String>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub version: i32,
}

/// The `disabled_reason` that takes an alert out of service for good (design R2: the currency's unit changed).
pub const UNIT_CHANGED: &str = "UNIT_CHANGED";

impl RateAlert {
    /// Ruling 2 on #1002: a deleted, expired or `UNIT_CHANGED` alert is not active, and pause/resume refuse it with
    /// `alertNotActive`.
    fn ensure_active(&self, now: DateTime<Utc>) -> Result<(), RateAlertError> {
        let deleted = self.deleted_at.is_some();
        let expired = self.expires_at.is_some_and(|at| at <= now);
        let unit_changed = self.disabled_reason.as_deref() == Some(UNIT_CHANGED);
        if deleted || expired || unit_changed {
            return Err(RateAlertError::ValidationFailed {
                reason: Some("alertNotActive"),
                message: "alert is not active".into(),
            });
        }
        Ok(())
    }

    /// R10: `enabled = false`, version + 1; not evaluated while paused. Answers whether anything changed: pausing a
    /// paused alert is a no-op (ruling 2). A deleted, expired or `UNIT_CHANGED` alert → `alertNotActive`.
    pub fn pause(&mut self, now: DateTime<Utc>) -> Result<bool, RateAlertError> {
        self.ensure_active(now)?;
        if !self.enabled {
            return Ok(false);
        }
        self.enabled = false;
        self.version += 1;
        Ok(true)
    }

    /// R10: `enabled = true`, relation back to `UNKNOWN`, version + 1; the caller re-observes without firing
    /// ([`RateAlert::observed`]). Answers whether anything changed: resuming an enabled alert is a no-op that leaves
    /// it as it is (ruling 2). A deleted, expired or `UNIT_CHANGED` alert → `alertNotActive`.
    pub fn resume(&mut self, now: DateTime<Utc>) -> Result<bool, RateAlertError> {
        self.ensure_active(now)?;
        if self.enabled {
            return Ok(false);
        }
        self.enabled = true;
        self.relation = Relation::Unknown;
        self.version += 1;
        Ok(true)
    }

    /// R10: `deleted_at = now`, version + 1; its events stay. Answers whether anything changed: deleting a deleted
    /// alert is a no-op that keeps the first `deleted_at` (ruling 2).
    pub fn delete(&mut self, now: DateTime<Utc>) -> Result<bool, RateAlertError> {
        if self.deleted_at.is_some() {
            return Ok(false);
        }
        self.deleted_at = Some(now);
        self.version += 1;
        Ok(true)
    }

    /// R5 at create and resume: the relation the current publication gives, without firing, and that publication's
    /// version (ruling 3 on #1002: only a later publication can make the alert due). `None` when the branch publishes
    /// no such line.
    pub fn observed(&mut self, relation: Relation, publication_version: Option<i32>) {
        self.relation = relation;
        self.last_publication_version = publication_version;
    }
}
