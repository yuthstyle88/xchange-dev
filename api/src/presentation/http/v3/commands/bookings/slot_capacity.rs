//! `slotCapacitySet` command (`PUT /api/booking-slots/{id}/capacity`, design §4.4, BK-6 #1388).

use actix_web::http::{Method, StatusCode};
use actix_web::{HttpRequest, web};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_booking::domain::slot::slot_error::SlotError;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use super::{BookingCommandResult, BookingServices, handle_booking_command};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{server_error, CommandAction};
use crate::presentation::http::v3::error::V3Error;

pub struct SlotCapacitySet;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct SlotCapacitySetInput {
    #[ts(type = "string")]
    pub id: Uuid,
    pub capacity: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct SlotCapacitySetOutput {
    #[ts(type = "string")]
    pub id: Uuid,
    pub capacity: i32,
    pub reserved: i32,
    pub status: String,
}

impl CommandAction for SlotCapacitySet {
    const NAME: &'static str = "slotCapacitySet";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "slotCapacitySet");
    type Input = SlotCapacitySetInput;
    type Output = SlotCapacitySetOutput;
    const ROUTE: &'static str = "/api/booking-slots/{id}/capacity";
    const METHOD: Method = Method::PUT;

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
        "/api/booking-slots/{id}/capacity",
        web::put().to(move |req: HttpRequest, payload: web::Payload| {
            let services = services.clone();
            async move {
                handle_booking_command(
                    req,
                    payload,
                    services,
                    SlotCapacitySet::NAME,
                    SlotCapacitySet::INHERITS,
                    false,
                    |_, value, user, services, tx| {
                        Box::pin(async move {
                            let input: SlotCapacitySetInput = serde_json::from_value(value)
                                .map_err(|_| {
                                    V3Error::new(
                                        "invalidRequest",
                                        None,
                                        "คำขอไม่ถูกต้อง",
                                        StatusCode::BAD_REQUEST,
                                    )
                                })?;

                            let _ = user.ok_or_else(|| V3Error::from(PolicyError::SignIn))?;

                            let row: Option<(String, i32)> = sqlx::query_as(
                                "SELECT branch_id, reserved FROM booking_slots WHERE id = $1",
                            )
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

                            if input.capacity < reserved {
                                return Err(V3Error::new(
                                    "capacityBelowReserved",
                                    None,
                                    "Capacity cannot be set below reserved count",
                                    StatusCode::BAD_REQUEST,
                                ));
                            }

                            let branch = BranchId::new(branch_id);
                            BranchTx::lock_also(tx, &branch).await.map_err(|e| {
                                V3Error::new(
                                    "serverError",
                                    None,
                                    e.to_string(),
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                )
                            })?;

                            let updated = services
                                .slot_service
                                .set_capacity_in(tx, input.id, input.capacity)
                                .await
                                .map_err(|err| match err {
                                    SlotError::CapacityBelowReserved => V3Error::new(
                                        "capacityBelowReserved",
                                        None,
                                        "CAPACITY_BELOW_RESERVED",
                                        StatusCode::BAD_REQUEST,
                                    ),
                                    SlotError::NotFound => V3Error::new(
                                        "notFound",
                                        None,
                                        "NOT_FOUND",
                                        StatusCode::NOT_FOUND,
                                    ),
                                    other => V3Error::new(
                                        "badRequest",
                                        None,
                                        other.to_string(),
                                        StatusCode::BAD_REQUEST,
                                    ),
                                })?;

                            let out = SlotCapacitySetOutput {
                                id: updated.id,
                                capacity: updated.capacity,
                                reserved: updated.reserved,
                                status: updated.status.as_str().to_string(),
                            };
                            let resp = serde_json::to_value(&out).map_err(|_| server_error())?;

                            Ok(BookingCommandResult::identical(resp))
                        })
                    },
                )
                .await
            }
        }),
    );
}
