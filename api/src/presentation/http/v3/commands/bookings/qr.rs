//! `bookingQrReissue` command (`POST /api/bookings/{id}/qr`, design §4.4, BK-6 #1388).

use actix_web::http::{Method, StatusCode};
use actix_web::{HttpRequest, web};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

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

pub struct BookingQrReissue;

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
        "/api/bookings/{id}/qr",
        web::post().to(move |req: HttpRequest, payload: web::Payload| {
            let services = services.clone();
            async move {
                handle_booking_command(
                    req,
                    payload,
                    services,
                    BookingQrReissue::NAME,
                    BookingQrReissue::INHERITS,
                    false,
                    |_, value, user, services, tx| {
                        Box::pin(async move {
                            let input: BookingQrReissueInput = serde_json::from_value(value)
                                .map_err(|_| {
                                    V3Error::new(
                                        "invalidRequest",
                                        None,
                                        "คำขอไม่ถูกต้อง",
                                        StatusCode::BAD_REQUEST,
                                    )
                                })?;

                            let user = user.ok_or_else(|| V3Error::from(PolicyError::SignIn))?;

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

                            let is_staff = user.roles.iter().any(|r| {
                                RolePermissions::of(r.role).contains(&Permission::BookingsDesk)
                            });

                            if !is_staff
                                && user.customer_id.as_deref() != Some(booking_cust_id.as_str())
                            {
                                return Err(V3Error::new(
                                    "outOfScope",
                                    None,
                                    "OUT_OF_SCOPE",
                                    StatusCode::FORBIDDEN,
                                ));
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

                            let branch = BranchId::new(branch_id);
                            BranchTx::lock_also(tx, &branch).await.map_err(|e| {
                                V3Error::new(
                                    "serverError",
                                    None,
                                    e.to_string(),
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                )
                            })?;

                            let now = Utc::now();
                            let reissued = services
                                .booking_service
                                .reissue_qr_in(tx, &input.id, &user.user_id, now)
                                .await
                                .map_err(|err| match err {
                                    BookingError::StaleVersion => V3Error::new(
                                        "staleVersion",
                                        None,
                                        "STALE_VERSION",
                                        StatusCode::BAD_REQUEST,
                                    ),
                                    BookingError::NotFound => V3Error::new(
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

                            let out = BookingQrReissueOutput {
                                id: reissued.booking.id,
                                qr_token: Some(reissued.qr_token),
                                qr_version: reissued.booking.qr_version,
                            };
                            let resp = serde_json::to_value(&out).map_err(|_| server_error())?;

                            let mut replay = resp.clone();
                            replay["qrToken"] = Value::Null;

                            Ok(BookingCommandResult {
                                response_val: resp,
                                replay_val: replay,
                            })
                        })
                    },
                )
                .await
            }
        }),
    );
}
