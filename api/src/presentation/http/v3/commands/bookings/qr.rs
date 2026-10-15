//! `bookingQrReissue` command (`POST /api/bookings/{id}/qr`, design §4.4, §4.5, BK-6 #1388 / #1913).

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use xc_auth::domain::access::{Actor, Permission, RolePermissions};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_booking::domain::booking::booking_error::BookingError;
use xc_kernel::ids::BranchId;

use super::BookingServices;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, out_of_scope};
use crate::presentation::http::v3::error::V3Error;

pub struct BookingQrReissue {
    services: Arc<BookingServices>,
}

impl BookingQrReissue {
    pub fn new(services: Arc<BookingServices>) -> Self {
        Self { services }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingQrReissueInput {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingQrReissueOutput {
    pub id: String,
    pub qr_token: Option<String>,
    pub qr_version: u32,
}

impl CommandAction for BookingQrReissue {
    const NAME: &'static str = "bookingQrReissue";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "bookingQrReissue");
    type Input = BookingQrReissueInput;
    type Output = BookingQrReissueOutput;
    const ROUTE: &'static str = "/api/bookings/{id}/qr";
    const METHOD: Method = Method::POST;

    fn stored_output(&self, output: &Self::Output) -> Value {
        let mut val = serde_json::to_value(output).unwrap_or_default();
        val["qrToken"] = Value::Null;
        val
    }

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let services = self.services.services()?;

        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT customer_id, branch_id, status FROM bookings WHERE id = $1",
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

        let (booking_cust_id, branch_id, status) = match row {
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

        if !is_staff && who.customer_id.as_deref() != Some(booking_cust_id.as_str()) {
            return Err(out_of_scope());
        }

        match status.as_str() {
            "PENDING_CUSTOMER_CONFIRMATION" | "CONFIRMED" | "CHECKED_IN" => {}
            _ => {
                return Err(V3Error::new(
                    "staleVersion",
                    None,
                    "STALE_VERSION",
                    StatusCode::BAD_REQUEST,
                ));
            }
        }

        let now = Utc::now();
        let reissued = services
            .booking_service
            .reissue_qr_in(tx, &input.id, &who.user_id, now)
            .await
            .map_err(|err| match err {
                BookingError::StaleVersion => V3Error::new(
                    "staleVersion",
                    None,
                    "STALE_VERSION",
                    StatusCode::BAD_REQUEST,
                ),
                BookingError::NotFound => {
                    V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND)
                }
                other => V3Error::new(
                    "badRequest",
                    None,
                    other.to_string(),
                    StatusCode::BAD_REQUEST,
                ),
            })?;

        Ok(BookingQrReissueOutput {
            id: reissued.booking.id,
            qr_token: Some(reissued.qr_token),
            qr_version: reissued.booking.qr_version,
        })
    }
}

