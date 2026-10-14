//! `bookingPolicySet` command (`PUT /api/booking-policy`, design §4.4, BK-6 #1388).

use actix_web::http::{Method, StatusCode};
use actix_web::{HttpRequest, web};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_kernel::ids::BranchId;

use super::{BookingCommandResult, BookingOkOutput, BookingServices, handle_booking_command};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{server_error, CommandAction};
use crate::presentation::http::v3::error::V3Error;

pub struct BookingPolicySet;

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
        "/api/booking-policy",
        web::put().to(move |req: HttpRequest, payload: web::Payload| {
            let services = services.clone();
            async move {
                handle_booking_command(
                    req,
                    payload,
                    services,
                    BookingPolicySet::NAME,
                    BookingPolicySet::INHERITS,
                    false,
                    |_, value, user, services, tx| {
                        Box::pin(async move {
                            let input: BookingPolicySetInput = serde_json::from_value(value)
                                .map_err(|_| {
                                    V3Error::new(
                                        "invalidRequest",
                                        None,
                                        "คำขอไม่ถูกต้อง",
                                        StatusCode::BAD_REQUEST,
                                    )
                                })?;

                            let user = user.ok_or_else(|| V3Error::from(PolicyError::SignIn))?;
                            let org_id = Uuid::parse_str(&user.organization_id).map_err(|_| {
                                V3Error::new(
                                    "badRequest",
                                    None,
                                    "Invalid organization ID",
                                    StatusCode::BAD_REQUEST,
                                )
                            })?;
                            let user_uuid = Uuid::parse_str(&user.user_id).ok();

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
                                .map_err(|e| {
                                    V3Error::new(
                                        "badRequest",
                                        None,
                                        e.to_string(),
                                        StatusCode::BAD_REQUEST,
                                    )
                                })?;

                            let resp = serde_json::to_value(BookingOkOutput { ok: true })
                                .map_err(|_| server_error())?;
                            Ok(BookingCommandResult::identical(resp))
                        })
                    },
                )
                .await
            }
        }),
    );
}
