//! `slotOpen` command (`POST /api/booking-slots/{id}/open`, design §4.4, §4.5, BK-6 #1388 / #1913).

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

pub struct SlotOpen {
    services: Arc<BookingServices>,
}

impl SlotOpen {
    pub fn new(services: Arc<BookingServices>) -> Self {
        Self { services }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct SlotOpenInput {
    #[ts(type = "string")]
    pub id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct SlotOpenOutput {
    #[ts(type = "string")]
    pub id: Uuid,
    pub status: String,
}

impl CommandAction for SlotOpen {
    const NAME: &'static str = "slotOpen";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "slotOpen");
    type Input = SlotOpenInput;
    type Output = SlotOpenOutput;
    const ROUTE: &'static str = "/api/booking-slots/{id}/open";
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

        let row: Option<(String,)> =
            sqlx::query_as("SELECT branch_id FROM booking_slots WHERE id = $1")
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

        let (branch_id,) = match row {
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
                .open_in(tx, input.id)
                .await
                .map_err(|err| match err {
                    SlotError::NotFound => {
                        V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND)
                    }
                    other => V3Error::from(other),
                })?;

        Ok(SlotOpenOutput {
            id: updated.id,
            status: updated.status.as_str().to_string(),
        })
    }
}
