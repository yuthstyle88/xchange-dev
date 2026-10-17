//! `bookingCheckIn` command (`POST /api/bookings/{id}/check-in`, design §4.4, §4.5, BK-6 #1388 / #1913).

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_booking::application::check_in::{CheckInBy, CheckInError};
use xc_kernel::ids::BranchId;

use super::BookingServices;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, out_of_scope};
use crate::presentation::http::v3::error::V3Error;

pub struct BookingCheckIn {
    services: Arc<BookingServices>,
}

impl BookingCheckIn {
    pub fn new(services: Arc<BookingServices>) -> Self {
        Self { services }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingCheckInInput {
    pub id: String,
    pub qr_token: Option<String>,
    pub identity_checked: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingCheckInOutput {
    pub id: String,
    pub status: String,
    pub ticket_number: String,
    pub booking_no: String,
}

type CheckInLookupRow = (
    String,
    String,
    Vec<u8>,
    NaiveDate,
    DateTime<Utc>,
    Option<i32>,
    String,
);

impl CommandAction for BookingCheckIn {
    const NAME: &'static str = "bookingCheckIn";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "bookingCheckIn");
    type Input = BookingCheckInInput;
    type Output = BookingCheckInOutput;
    const ROUTE: &'static str = "/api/bookings/{id}/check-in";
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

        let has_qr = input.qr_token.as_ref().is_some_and(|s| !s.trim().is_empty());
        let has_id = input.identity_checked == Some(true);

        if (has_qr && has_id) || (!has_qr && !has_id) {
            return Err(V3Error::new(
                "validationFailed",
                None,
                "Exactly one of qrToken or identityChecked must be provided",
                StatusCode::BAD_REQUEST,
            ));
        }

        let row: Option<CheckInLookupRow> = sqlx::query_as(
            "SELECT b.branch_id, b.status, b.qr_token_hash, s.service_date, s.start_at, p.no_show_grace_minutes, br.timezone \
             FROM bookings b \
             JOIN booking_slots s ON s.id = b.slot_id \
             JOIN branches br ON br.id = b.branch_id \
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

        let (branch_id, _status, qr_token_hash, service_date, start_at, no_show_grace, timezone_str) =
            match row {
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

        let now = Utc::now();
        let tz: Tz = timezone_str.parse().unwrap_or(chrono_tz::Asia::Bangkok);
        let today = now.with_timezone(&tz).date_naive();
        let grace = no_show_grace.unwrap_or(15);

        if service_date != today || now > start_at + Duration::minutes(grace as i64) {
            return Err(V3Error::new(
                "checkInWindow",
                None,
                "CHECK_IN_WINDOW",
                StatusCode::BAD_REQUEST,
            ));
        }

        let check_in_by = if let Some(token) = input.qr_token {
            let hash = Sha256::digest(token.as_bytes());
            if &hash[..] != qr_token_hash.as_slice() {
                return Err(V3Error::new(
                    "qrMismatch",
                    None,
                    "QR_MISMATCH",
                    StatusCode::BAD_REQUEST,
                ));
            }
            CheckInBy::QrToken(token)
        } else {
            CheckInBy::IdentityChecked
        };

        let (updated_booking, ticket) = services
            .check_in_service
            .check_in_in(tx, branch, &input.id, check_in_by, &who.user_id, now)
            .await
            .map_err(|err| match err {
                CheckInError::QrMismatch => V3Error::new(
                    "qrMismatch",
                    None,
                    "QR_MISMATCH",
                    StatusCode::BAD_REQUEST,
                ),
                CheckInError::CheckInWindow => V3Error::new(
                    "checkInWindow",
                    None,
                    "CHECK_IN_WINDOW",
                    StatusCode::BAD_REQUEST,
                ),
                CheckInError::QueueClosed => V3Error::new(
                    "queueClosed",
                    None,
                    "QUEUE_CLOSED",
                    StatusCode::BAD_REQUEST,
                ),
                CheckInError::NotFound => {
                    V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND)
                }
                other => V3Error::from(other),
            })?;

        Ok(BookingCheckInOutput {
            id: updated_booking.id,
            status: updated_booking.status.as_str().to_string(),
            ticket_number: ticket.display_number,
            booking_no: updated_booking.booking_no,
        })
    }
}

