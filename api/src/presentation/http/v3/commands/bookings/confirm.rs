//! `bookingConfirm` command (`POST /api/bookings/{id}/confirm`, design §4.4, BK-6 #1388).

use actix_web::http::{Method, StatusCode};
use actix_web::{HttpRequest, web};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_booking::domain::booking::booking_error::BookingError;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use super::{BookingCommandResult, BookingServices, handle_booking_command};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{server_error, CommandAction};
use crate::presentation::http::v3::error::V3Error;

pub struct BookingConfirm;

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
        "/api/bookings/{id}/confirm",
        web::post().to(move |req: HttpRequest, payload: web::Payload| {
            let services = services.clone();
            async move {
                handle_booking_command(
                    req,
                    payload,
                    services,
                    BookingConfirm::NAME,
                    BookingConfirm::INHERITS,
                    false,
                    |_, value, user, services, tx| Box::pin(async move {
                        let input: BookingConfirmInput =
                            serde_json::from_value(value).map_err(|_| {
                                V3Error::new(
                                    "invalidRequest",
                                    None,
                                    "คำขอไม่ถูกต้อง",
                                    StatusCode::BAD_REQUEST,
                                )
                            })?;

                        let user = user.ok_or_else(|| V3Error::from(PolicyError::SignIn))?;

                        let row: Option<(String, String, Option<DateTime<Utc>>)> = sqlx::query_as(
                            "SELECT customer_id, branch_id, confirm_expires_at FROM bookings WHERE id = $1",
                        )
                        .bind(&input.id)
                        .fetch_optional(&mut **tx)
                        .await
                        .map_err(|e| V3Error::new("serverError", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR))?;

                        let (booking_cust_id, branch_id, confirm_expires_at) = match row {
                            Some(r) => r,
                            None => return Err(V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND)),
                        };

                        if user.customer_id.as_deref() != Some(booking_cust_id.as_str()) {
                            return Err(V3Error::new("outOfScope", None, "OUT_OF_SCOPE", StatusCode::FORBIDDEN));
                        }

                        let now = Utc::now();
                        if confirm_expires_at.is_some_and(|exp| now > exp) {
                            return Err(V3Error::new("bookingExpired", None, "BOOKING_EXPIRED", StatusCode::BAD_REQUEST));
                        }

                        let branch = BranchId::new(branch_id);
                        BranchTx::lock_also(tx, &branch)
                            .await
                            .map_err(|e| V3Error::new("serverError", None, e.to_string(), StatusCode::INTERNAL_SERVER_ERROR))?;

                        let updated = services
                            .booking_service
                            .confirm_in(tx, &input.id, &user.user_id, now)
                            .await
                            .map_err(|err| match err {
                                BookingError::BookingExpired => V3Error::new("bookingExpired", None, "BOOKING_EXPIRED", StatusCode::BAD_REQUEST),
                                BookingError::NotFound => V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND),
                                other => V3Error::new("badRequest", None, other.to_string(), StatusCode::BAD_REQUEST),
                            })?;

                        let out = BookingConfirmOutput {
                            id: updated.id,
                            status: updated.status.as_str().to_string(),
                            booking_no: updated.booking_no,
                        };
                        let resp = serde_json::to_value(&out).map_err(|_| server_error())?;

                        Ok(BookingCommandResult::identical(resp))
                    }),
                )
                .await
            }
        }),
    );
}
