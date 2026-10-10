//! `RateAlertEvent`: one fired episode of one alert (R6, R9). Row of `rate_alert_events`; append-only.
//! Written by AL-4's evaluator; declared here (design §5).

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;
use xc_rate::domain::publication::published_rates::PublishedRateLineId;

use super::rate_alert::RateAlertId;

/// One row of `rate_alert_events` (design §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateAlertEvent {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub alert_id: RateAlertId,
    pub published_rate_line_id: PublishedRateLineId,
    pub observed_rate: Decimal,
    /// The publication's `published_at`.
    pub trigger_at: DateTime<Utc>,
    pub episode_no: i32,
}
