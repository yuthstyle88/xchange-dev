//! `closureAdd` command (`POST /api/branches/{id}/closures`, design §4.4, BK-6 #1388).

use actix_web::http::{Method, StatusCode};
use actix_web::{HttpRequest, web};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_booking::domain::slot::branch_closure::BranchClosure;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use super::{BookingCommandResult, BookingOkOutput, BookingServices, handle_booking_command};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{server_error, CommandAction};
use crate::presentation::http::v3::error::V3Error;

pub struct ClosureAdd;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct ClosureAddInput {
    pub id: String,
    #[ts(type = "string")]
    pub closed_on: NaiveDate,
    pub reason: String,
}

impl CommandAction for ClosureAdd {
    const NAME: &'static str = "closureAdd";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "closureAdd");
    type Input = ClosureAddInput;
    type Output = BookingOkOutput;
    const ROUTE: &'static str = "/api/branches/{id}/closures";
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
        "/api/branches/{id}/closures",
        web::post().to(move |req: HttpRequest, payload: web::Payload| {
            let services = services.clone();
            async move {
                handle_booking_command(
                    req,
                    payload,
                    services,
                    ClosureAdd::NAME,
                    ClosureAdd::INHERITS,
                    false,
                    |_, value, user, services, tx| {
                        Box::pin(async move {
                            let input: ClosureAddInput =
                                serde_json::from_value(value).map_err(|_| {
                                    V3Error::new(
                                        "invalidRequest",
                                        None,
                                        "คำขอไม่ถูกต้อง",
                                        StatusCode::BAD_REQUEST,
                                    )
                                })?;

                            let user = user.ok_or_else(|| V3Error::from(PolicyError::SignIn))?;
                            let user_uuid =
                                Uuid::parse_str(&user.user_id).unwrap_or_else(|_| Uuid::new_v4());

                            let branch = BranchId::new(&input.id);
                            BranchTx::lock_also(tx, &branch).await.map_err(|e| {
                                V3Error::new(
                                    "serverError",
                                    None,
                                    e.to_string(),
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                )
                            })?;

                            let closure = BranchClosure {
                                closed_on: input.closed_on,
                                reason: input.reason,
                            };

                            services
                                .slot_service
                                .add_closure_in(tx, &branch, &closure, user_uuid)
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
