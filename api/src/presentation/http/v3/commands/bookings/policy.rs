//! `bookingPolicySet` command (`PUT /api/booking-policy`, design §4.4, §4.5, BK-6 #1388 / #1913).

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;

use super::{BookingOkOutput, BookingServices};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::CommandAction;
use crate::presentation::http::v3::error::V3Error;

pub struct BookingPolicySet {
    services: Arc<BookingServices>,
}

impl BookingPolicySet {
    pub fn new(services: Arc<BookingServices>) -> Self {
        Self { services }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingPolicySetInput {
    pub cancel_cutoff_minutes: i32,
    pub no_show_grace_minutes: i32,
    pub confirm_expiry_minutes: i32,
}

impl CommandAction for BookingPolicySet {
    const NAME: &'static str = "bookingPolicySet";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "bookingPolicySet");
    type Input = BookingPolicySetInput;
    type Output = BookingOkOutput;
    const ROUTE: &'static str = "/api/booking-policy";
    const METHOD: Method = Method::PUT;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let services = self.services.services()?;

        let org_id = Uuid::parse_str(&who.organization_id).map_err(|_| {
            V3Error::new(
                "badRequest",
                None,
                "Invalid organization ID",
                StatusCode::BAD_REQUEST,
            )
        })?;
        let user_uuid = Uuid::parse_str(&who.user_id).ok();

        services
            .slot_service
            .set_policy_in(
                tx,
                org_id,
                input.cancel_cutoff_minutes,
                input.no_show_grace_minutes,
                input.confirm_expiry_minutes,
                user_uuid,
            )
            .await
            .map_err(|e| {
                V3Error::new(
                    "badRequest",
                    None,
                    e.to_string(),
                    StatusCode::BAD_REQUEST,
                )
            })?;

        Ok(BookingOkOutput { ok: true })
    }
}

