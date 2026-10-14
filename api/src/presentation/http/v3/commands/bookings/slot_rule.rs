//! `slotRuleSet` command (`PUT /api/branches/{id}/booking-slot-rule`, design §4.4, BK-6 #1388).

use actix_web::http::{Method, StatusCode};
use actix_web::{HttpRequest, web};
use chrono::Utc;
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use xc_auth::domain::access::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_booking::domain::slot::slot_rule::SlotRule;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use super::{BookingCommandResult, BookingOkOutput, BookingServices, handle_booking_command};
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::{server_error, CommandAction};
use crate::presentation::http::v3::error::V3Error;

pub struct SlotRuleSet;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct SlotRuleSetInput {
    pub id: String,
    pub slot_minutes: i32,
    pub capacity: i32,
    pub horizon_days: i32,
}

impl CommandAction for SlotRuleSet {
    const NAME: &'static str = "slotRuleSet";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "slotRuleSet");
    type Input = SlotRuleSetInput;
    type Output = BookingOkOutput;
    const ROUTE: &'static str = "/api/branches/{id}/booking-slot-rule";
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
        "/api/branches/{id}/booking-slot-rule",
        web::put().to(move |req: HttpRequest, payload: web::Payload| {
            let services = services.clone();
            async move {
                handle_booking_command(
                    req,
                    payload,
                    services,
                    SlotRuleSet::NAME,
                    SlotRuleSet::INHERITS,
                    false,
                    |_, value, user, services, tx| {
                        Box::pin(async move {
                            let input: SlotRuleSetInput =
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

                            let rule = SlotRule {
                                slot_minutes: input.slot_minutes,
                                capacity: input.capacity,
                                horizon_days: input.horizon_days,
                            };

                            services
                                .slot_service
                                .set_rule_in(tx, &branch, &rule, user_uuid)
                                .await
                                .map_err(|e| {
                                    V3Error::new(
                                        "badRequest",
                                        None,
                                        e.to_string(),
                                        StatusCode::BAD_REQUEST,
                                    )
                                })?;

                            let tz_str: Option<String> =
                                sqlx::query_scalar("SELECT timezone FROM branches WHERE id = $1")
                                    .bind(branch.as_str())
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

                            let tz: Tz = tz_str
                                .as_deref()
                                .unwrap_or("Asia/Bangkok")
                                .parse()
                                .unwrap_or(chrono_tz::Asia::Bangkok);
                            let today = Utc::now().with_timezone(&tz).date_naive();

                            let _ = services
                                .slot_service
                                .generate_in(tx, &branch, today, input.horizon_days as u32)
                                .await;

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
