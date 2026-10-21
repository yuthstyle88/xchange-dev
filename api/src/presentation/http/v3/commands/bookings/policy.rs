//! `bookingPolicySet` command (`PUT /api/booking-policy`, design §4.4, §4.5, BK-6 #1388 / #1913).

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use xc_auth::domain::access::Actor;
use xc_auth::domain::access::branch_scope::BranchScope;
use xc_auth::domain::access::permission::Permission;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_kernel::ids::BranchId;
use xc_kernel::money::IntBound;

use super::{BookingOkOutput, BookingServices};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, out_of_scope};
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::input;

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
        let now = chrono::Utc::now();
        if !matches!(
            BranchScope::of(who, Permission::BranchesManage, now),
            BranchScope::All
        ) {
            return Err(out_of_scope());
        }

        input::integer(
            "cancelCutoffMinutes",
            i64::from(input.cancel_cutoff_minutes),
            IntBound::integer(0, i64::MAX),
        )?;
        input::integer(
            "noShowGraceMinutes",
            i64::from(input.no_show_grace_minutes),
            IntBound::integer(0, i64::MAX),
        )?;
        input::integer(
            "confirmExpiryMinutes",
            i64::from(input.confirm_expiry_minutes),
            IntBound::integer(1, i64::MAX),
        )?;

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
            .map_err(V3Error::from)?;

        Ok(BookingOkOutput { ok: true })
    }
}
