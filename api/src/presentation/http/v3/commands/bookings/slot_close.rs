//! `slotClose` command (`POST /api/booking-slots/{id}/close`, design §4.4, §4.5, BK-6 #1388 / #1913).

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_booking::domain::slot::slot_error::SlotError;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use super::BookingServices;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{CommandAction, out_of_scope};
use crate::presentation::http::v3::error::V3Error;

pub struct SlotClose {
    services: Arc<BookingServices>,
}

impl SlotClose {
    pub fn new(services: Arc<BookingServices>) -> Self {
        Self { services }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct SlotCloseInput {
    #[ts(type = "string")]
    pub id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct SlotCloseOutput {
    #[ts(type = "string")]
    pub id: Uuid,
    pub status: String,
}

impl CommandAction for SlotClose {
    const NAME: &'static str = "slotClose";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "slotClose");
    type Input = SlotCloseInput;
    type Output = SlotCloseOutput;
    const ROUTE: &'static str = "/api/booking-slots/{id}/close";
    const METHOD: Method = Method::POST;

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        _who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let services = self.services.services()?;

        let row: Option<(String, i32)> =
            sqlx::query_as("SELECT branch_id, reserved FROM booking_slots WHERE id = $1")
                .bind(input.id)
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

        let (branch_id, reserved) = match row {
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

        let slot_branch = BranchId::new(branch_id);
        if slot_branch != *branch {
            return Err(out_of_scope());
        }

        if reserved > 0 {
            return Err(V3Error::new(
                "slotHasBookings",
                None,
                "Slot has existing bookings",
                StatusCode::BAD_REQUEST,
            ));
        }

        BranchTx::lock_also(tx, &slot_branch).await.map_err(|e| {
            V3Error::new(
                "serverError",
                None,
                e.to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            )
        })?;

        let updated =
            services
                .slot_service
                .close_in(tx, input.id)
                .await
                .map_err(|err| match err {
                    SlotError::SlotHasBookings => V3Error::new(
                        "slotHasBookings",
                        None,
                        "SLOT_HAS_BOOKINGS",
                        StatusCode::BAD_REQUEST,
                    ),
                    SlotError::NotFound => {
                        V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND)
                    }
                    other => V3Error::from(other),
                })?;

        Ok(SlotCloseOutput {
            id: updated.id,
            status: updated.status.as_str().to_string(),
        })
    }
}
