//! `bookingReschedule` command (`POST /api/bookings/{id}/reschedule`, design §4.4, BK-6 #1388).

use actix_web::http::{Method, StatusCode};
use actix_web::{HttpRequest, web};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use xc_auth::domain::access::{Actor, Permission, RolePermissions};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_booking::domain::booking::booking_error::BookingError;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use super::{BookingCommandResult, BookingServices, handle_booking_command};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{server_error, CommandAction};
use crate::presentation::http::v3::error::V3Error;

pub struct BookingReschedule;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingRescheduleInput {
    pub id: String,
    #[ts(type = "string")]
    pub slot_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingRescheduleOutput {
    pub id: String,
    pub status: String,
    #[ts(type = "string")]
    pub slot_id: Uuid,
    pub booking_no: String,
}

impl CommandAction for BookingReschedule {
    const NAME: &'static str = "bookingReschedule";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "bookingReschedule");
    type Input = BookingRescheduleInput;
    type Output = BookingRescheduleOutput;
    const ROUTE: &'static str = "/api/bookings/{id}/reschedule";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        _tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        _who: &Actor,
        _input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        Err(server_error())
    }
}

pub fn register(cfg: &mut web::ServiceConfig, services: web::Data<BookingServices>) {
    cfg.route(
        "/api/bookings/{id}/reschedule",
        web::post().to(move |req: HttpRequest, payload: web::Payload| {
            let services = services.clone();
            async move {
                handle_booking_command(
                    req,
                    payload,
                    services,
                    BookingReschedule::NAME,
                    BookingReschedule::INHERITS,
                    false,
                    |_, value, user, services, tx| Box::pin(async move {
                        let input: BookingRescheduleInput =
                            serde_json::from_value(value).map_err(|_| {
                                V3Error::new(
                                    "invalidRequest",
                                    None,
                                    "คำขอไม่ถูกต้อง",
                                    StatusCode::BAD_REQUEST,
                                )
                            })?;

                        let user = user.ok_or_else(|| V3Error::from(PolicyError::SignIn))?;

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
                        .map_err(|e| V3Error::new("serverError", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR))?;

                        let (booking_cust_id, branch_id, start_at, cancel_cutoff) = match row {
                            Some(r) => r,
                            None => return Err(V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND)),
                        };

                        let is_staff = user
                            .roles
                            .iter()
                            .any(|r| RolePermissions::of(r.role).contains(&Permission::BookingsDesk));

                        let now = Utc::now();
                        if !is_staff {
                            if user.customer_id.as_deref() != Some(booking_cust_id.as_str()) {
                                return Err(V3Error::new("outOfScope", None, "OUT_OF_SCOPE", StatusCode::FORBIDDEN));
                            }
                            let cutoff_minutes = cancel_cutoff.unwrap_or(60);
                            if now > start_at - Duration::minutes(cutoff_minutes as i64) {
                                return Err(V3Error::new("cancelCutoffPassed", None, "CANCEL_CUTOFF_PASSED", StatusCode::BAD_REQUEST));
                            }
                        }

                        let branch = BranchId::new(branch_id);
                        BranchTx::lock_also(tx, &branch)
                            .await
                            .map_err(|e| V3Error::new("serverError", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR))?;

                        let updated = services
                            .booking_service
                            .reschedule_in(tx, &input.id, input.slot_id, &user.user_id, is_staff, now)
                            .await
                            .map_err(|err| match err {
                                BookingError::CancelCutoffPassed => V3Error::new("cancelCutoffPassed", None, "CANCEL_CUTOFF_PASSED", StatusCode::BAD_REQUEST),
                                BookingError::CapacityFull => V3Error::new("capacityFull", None, "CAPACITY_FULL", StatusCode::BAD_REQUEST),
                                BookingError::SlotUnavailable => V3Error::new("capacityFull", None, "CAPACITY_FULL", StatusCode::BAD_REQUEST),
                                BookingError::NotFound => V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND),
                                other => V3Error::new("badRequest", None, other.to_string(), StatusCode::BAD_REQUEST),
                            })?;

                        let out = BookingRescheduleOutput {
                            id: updated.id,
                            status: updated.status.as_str().to_string(),
                            slot_id: updated.slot_id,
                            booking_no: updated.booking_no,
                        };
                        let resp = serde_json::to_value(&out).map_err(|_| server_error())?;

                        Ok(BookingCommandResult::identical(resp))
                    }),
                )
                .await
            }
        }),
    );
}
