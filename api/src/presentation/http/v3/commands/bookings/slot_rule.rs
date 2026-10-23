//! `slotRuleSet` command (`PUT /api/branches/{id}/booking-slot-rule`, design §4.4, §4.5, BK-6 #1388 / #1913).

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use chrono::Utc;
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_booking::domain::slot::slot_rule::SlotRule;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::IntBound;

use super::{BookingOkOutput, BookingServices};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, Locks};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::input;

/// `booking_slot_rules` column `CHECK`s (`migrations/20261205131000_booking_slots.sql:36-38`).
const SLOT_MINUTES: IntBound = IntBound::integer(5, 240);
const CAPACITY: IntBound = IntBound::integer(1, i32::MAX as i64);
const HORIZON_DAYS: IntBound = IntBound::integer(1, 90);

pub struct SlotRuleSet {
    services: Arc<BookingServices>,
}

impl SlotRuleSet {
    pub fn new(services: Arc<BookingServices>) -> Self {
        Self { services }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct SlotRuleSetInput {
    pub id: String,
    pub slot_minutes: i32,
    #[ts(type = "number")]
    pub capacity: i64,
    pub horizon_days: i32,
}

impl CommandAction for SlotRuleSet {
    const NAME: &'static str = "slotRuleSet";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "slotRuleSet");
    type Input = SlotRuleSetInput;
    type Output = BookingOkOutput;
    const ROUTE: &'static str = "/api/branches/{id}/booking-slot-rule";
    const METHOD: Method = Method::PUT;

    fn locks(&self, _branch: &BranchId, input: &Self::Input) -> Locks {
        Locks::One(BranchId::new(&input.id))
    }

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let services = self.services.services()?;
        input::integer("slotMinutes", input.slot_minutes as i64, SLOT_MINUTES)?;
        input::integer("capacity", input.capacity, CAPACITY)?;
        input::integer("horizonDays", input.horizon_days as i64, HORIZON_DAYS)?;
        let user_uuid = Uuid::parse_str(&who.user_id).unwrap_or_else(|_| Uuid::new_v4());

        let branch = BranchId::new(&input.id);
        BranchTx::lock_also(tx, &branch).await.map_err(|e| {
            V3Error::new(
                "serverError",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

        let rule = SlotRule {
            slot_minutes: input.slot_minutes,
            capacity: input.capacity as i32,
            horizon_days: input.horizon_days,
        };

        services
            .slot_service
            .set_rule_in(tx, &branch, &rule, user_uuid)
            .await
            .map_err(V3Error::from)?;

        let tz_str: Option<String> =
            sqlx::query_scalar("SELECT timezone FROM branches WHERE id = $1")
                .bind(branch.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|e| {
                    V3Error::new(
                        "serverError",
                        None,
                        e.to_string(),
                        StatusCode::INTERNAL_SERVER_ERROR,
                    )
                })?;

        let tz: Tz = tz_str
            .as_deref()
            .unwrap_or("Asia/Bangkok")
            .parse()
            .unwrap_or(chrono_tz::Asia::Bangkok);
        let today = Utc::now().with_timezone(&tz).date_naive();

        services
            .slot_service
            .generate_in(tx, &branch, today, input.horizon_days as u32)
            .await
            .map_err(V3Error::from)?;

        Ok(BookingOkOutput { ok: true })
    }
}
