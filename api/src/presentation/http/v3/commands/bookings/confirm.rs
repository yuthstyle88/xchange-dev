//! `bookingConfirm` command (`POST /api/bookings/{id}/confirm`, design §4.4, §4.5, BK-6 #1388 / #1913).

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_booking::domain::booking::booking_error::BookingError;
use xc_kernel::ids::BranchId;

use super::BookingServices;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, out_of_scope};
use crate::presentation::http::v3::error::V3Error;

pub struct BookingConfirm {
    services: Arc<BookingServices>,
}

impl BookingConfirm {
    pub fn new(services: Arc<BookingServices>) -> Self {
        Self { services }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingConfirmInput {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingConfirmOutput {
    pub id: String,
    pub status: String,
    pub booking_no: String,
}

impl CommandAction for BookingConfirm {
    const NAME: &'static str = "bookingConfirm";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "bookingConfirm");
    type Input = BookingConfirmInput;
    type Output = BookingConfirmOutput;
    const ROUTE: &'static str = "/api/bookings/{id}/confirm";
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

        let row: Option<(String, String, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT customer_id, branch_id, confirm_expires_at FROM bookings WHERE id = $1",
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

        let (booking_cust_id, branch_id, confirm_expires_at) = match row {
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

        if branch_id != branch.as_str()
            || who.customer_id.as_deref() != Some(booking_cust_id.as_str())
        {
            return Err(out_of_scope());
        }

        let now = Utc::now();
        if confirm_expires_at.is_some_and(|exp| now > exp) {
            return Err(V3Error::new(
                "bookingExpired",
                None,
                "BOOKING_EXPIRED",
                StatusCode::BAD_REQUEST,
            ));
        }

        let updated = services
            .booking_service
            .confirm_in(tx, &input.id, &who.user_id, now)
            .await
            .map_err(|err| match err {
                BookingError::BookingExpired => V3Error::new(
                    "bookingExpired",
                    None,
                    "BOOKING_EXPIRED",
                    StatusCode::BAD_REQUEST,
                ),
                BookingError::NotFound => {
                    V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND)
                }
                other => V3Error::from(other),
            })?;

        Ok(BookingConfirmOutput {
            id: updated.id,
            status: updated.status.as_str().to_string(),
            booking_no: updated.booking_no,
        })
    }
}

