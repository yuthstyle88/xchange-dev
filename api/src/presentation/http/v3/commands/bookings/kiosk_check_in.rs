//! `kioskBookingCheckIn` command (`POST /api/kiosk/bookings/check-in`, design §4.4, §4.5, BK-6 #1388 / #1913).

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;
use uuid::Uuid;

use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_booking::application::check_in::{CheckInBy, CheckInError};
use xc_booking::domain::policy::booking_policy::BookingPolicy;
use xc_booking::domain::slot::booking_slot::{BookingSlot, SlotStatus};
use xc_kernel::ids::BranchId;

use super::BookingServices;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::CommandAction;
use crate::presentation::http::v3::error::V3Error;

pub const PUBLIC_ROUTE: &str = "/api/kiosk/bookings/check-in";

pub struct KioskBookingCheckIn {
    services: Arc<BookingServices>,
}

impl KioskBookingCheckIn {
    pub fn new(services: Arc<BookingServices>) -> Self {
        Self { services }
    }

    async fn check_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: KioskBookingCheckInInput,
    ) -> Result<KioskBookingCheckInOutput, V3Error> {
        let services = self.services.services()?;
        let qr_hash = Sha256::digest(input.qr_token.as_bytes());

        let row: Option<KioskBookingRow> = sqlx::query_as(
            "SELECT b.id, b.status, s.service_date, s.start_at, p.no_show_grace_minutes, br.timezone \
             FROM bookings b \
             JOIN booking_slots s ON s.id = b.slot_id \
             JOIN branches br ON br.id = b.branch_id \
             LEFT JOIN booking_policies p ON p.organization_id = b.organization_id \
             WHERE b.branch_id = $1 AND b.qr_token_hash = $2",
        )
        .bind(branch.as_str())
        .bind(&qr_hash[..])
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

        let (booking_id, _status, service_date, start_at, no_show_grace, timezone_str) = match row {
            Some(r) => r,
            None => {
                return Err(V3Error::new(
                    "qrMismatch",
                    None,
                    "QR_MISMATCH",
                    StatusCode::BAD_REQUEST,
                ));
            }
        };

        let now = Utc::now();
        let tz: Tz = timezone_str.parse().unwrap_or(chrono_tz::Asia::Bangkok);
        let today = now.with_timezone(&tz).date_naive();
        let Some(grace) = no_show_grace else {
            return Err(V3Error::new(
                "bookingsClosed",
                None,
                "BOOKINGS_CLOSED",
                StatusCode::BAD_REQUEST,
            ));
        };
        let policy = BookingPolicy {
            cancel_cutoff_minutes: 0,
            no_show_grace_minutes: grace,
            confirm_expiry_minutes: 0,
        };
        let slot = BookingSlot {
            id: Uuid::nil(),
            organization_id: Uuid::nil(),
            branch_id: branch.clone(),
            service_date,
            start_at,
            end_at: start_at,
            capacity: 0,
            reserved: 0,
            status: SlotStatus::Open,
        };

        if service_date != today || now > policy.grace_until(&slot) {
            return Err(V3Error::new(
                "checkInWindow",
                None,
                "CHECK_IN_WINDOW",
                StatusCode::BAD_REQUEST,
            ));
        }

        let (updated_booking, ticket) = services
            .check_in_service
            .check_in_in(
                tx,
                branch,
                &booking_id,
                CheckInBy::QrToken(input.qr_token),
                &who.user_id,
                now,
            )
            .await
            .map_err(|err| match err {
                CheckInError::QrMismatch => {
                    V3Error::new("qrMismatch", None, "QR_MISMATCH", StatusCode::BAD_REQUEST)
                }
                CheckInError::CheckInWindow => V3Error::new(
                    "checkInWindow",
                    None,
                    "CHECK_IN_WINDOW",
                    StatusCode::BAD_REQUEST,
                ),
                CheckInError::QueueClosed => {
                    V3Error::new("queueClosed", None, "QUEUE_CLOSED", StatusCode::BAD_REQUEST)
                }
                CheckInError::NotFound => {
                    V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND)
                }
                other => V3Error::from(other),
            })?;

        Ok(KioskBookingCheckInOutput {
            id: updated_booking.id,
            status: updated_booking.status.as_str().to_string(),
            ticket_number: ticket.display_number,
            booking_no: updated_booking.booking_no,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct KioskBookingCheckInInput {
    #[ts(type = "string")]
    pub branch: BranchId,
    pub qr_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct KioskBookingCheckInOutput {
    pub id: String,
    pub status: String,
    pub ticket_number: String,
    pub booking_no: String,
}

type KioskBookingRow = (
    String,
    String,
    NaiveDate,
    DateTime<Utc>,
    Option<i32>,
    String,
);

impl CommandAction for KioskBookingCheckIn {
    const NAME: &'static str = "kioskBookingCheckIn";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "kioskBookingCheckIn");
    type Input = KioskBookingCheckInInput;
    type Output = KioskBookingCheckInOutput;
    const ROUTE: &'static str = PUBLIC_ROUTE;
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        self.check_in(tx, branch, who, input).await
    }
}
