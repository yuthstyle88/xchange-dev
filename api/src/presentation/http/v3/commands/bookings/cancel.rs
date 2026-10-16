//! `bookingCancel` command (`POST /api/bookings/{id}/cancel`, design §4.4, §4.5, BK-6 #1388 / #1913).

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use xc_auth::domain::access::{Actor, Permission, RolePermissions};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_booking::domain::booking::booking_error::BookingError;
use xc_kernel::ids::BranchId;

use super::BookingServices;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, out_of_scope};
use crate::presentation::http::v3::error::V3Error;

pub struct BookingCancel {
    services: Arc<BookingServices>,
}

impl BookingCancel {
    pub fn new(services: Arc<BookingServices>) -> Self {
        Self { services }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingCancelInput {
    pub id: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingCancelOutput {
    pub id: String,
    pub status: String,
    pub booking_no: String,
}

impl CommandAction for BookingCancel {
    const NAME: &'static str = "bookingCancel";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "bookingCancel");
    type Input = BookingCancelInput;
    type Output = BookingCancelOutput;
    const ROUTE: &'static str = "/api/bookings/{id}/cancel";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let services = self.services.services()?;

        let row: Option<(String, String, DateTime<Utc>, Option<i32>)> = sqlx::query_as(
            "SELECT b.customer_id, b.branch_id, s.start_at, p.cancel_cutoff_minutes \
             FROM bookings b \
             JOIN booking_slots s ON s.id = b.slot_id \
             LEFT JOIN booking_policies p ON p.organization_id = b.organization_id \
             WHERE b.id = $1",
        )
        .bind(&input.id)
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

        let (booking_cust_id, branch_id, start_at, cancel_cutoff) = match row {
            Some(r) => r,
            None => {
                return Err(V3Error::new(
                    "notFound",
                    None,
                    "NOT_FOUND",
                    StatusCode::NOT_FOUND,
                ));
            }
        };

        if branch_id != branch.as_str() {
            return Err(out_of_scope());
        }

        let is_staff = who
            .roles
            .iter()
            .any(|r| RolePermissions::of(r.role).contains(&Permission::BookingsDesk));

        let now = Utc::now();
        let reason = if is_staff {
            let r = input.reason.as_deref().unwrap_or("").trim();
            if r.is_empty() {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("reason"),
                    "Staff cancellation requires a reason",
                    StatusCode::BAD_REQUEST,
                ));
            }
            r
        } else {
            if who.customer_id.as_deref() != Some(booking_cust_id.as_str()) {
                return Err(out_of_scope());
            }
            let cutoff_minutes = cancel_cutoff.unwrap_or(60);
            if now > start_at - Duration::minutes(cutoff_minutes as i64) {
                return Err(V3Error::new(
                    "cancelCutoffPassed",
                    None,
                    "CANCEL_CUTOFF_PASSED",
                    StatusCode::BAD_REQUEST,
                ));
            }
            input.reason.as_deref().unwrap_or("customerCancelled")
        };

        let updated = services
            .booking_service
            .cancel_in(tx, &input.id, &who.user_id, is_staff, Some(reason), now)
            .await
            .map_err(|err| match err {
                BookingError::CancelCutoffPassed => V3Error::new(
                    "cancelCutoffPassed",
                    None,
                    "CANCEL_CUTOFF_PASSED",
                    StatusCode::BAD_REQUEST,
                ),
                BookingError::NotFound => {
                    V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND)
                }
                other => V3Error::from(other),
            })?;

        Ok(BookingCancelOutput {
            id: updated.id,
            status: updated.status.as_str().to_string(),
            booking_no: updated.booking_no,
        })
    }
}

