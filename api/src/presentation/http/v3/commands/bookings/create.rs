//! `bookingCreate` command (`POST /api/bookings`, design §4.4, §4.5, BK-6 #1388 / #1913).

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use chrono::Utc;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
use uuid::Uuid;

use xc_auth::domain::access::{Actor, Permission, RoleGrants};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::policy_error::PolicyError;
use xc_booking::application::booking_service::{ContactInput, CreateBookingInput};
use xc_booking::domain::booking::booking_error::BookingError;
use xc_booking::domain::booking::source_channel::SourceChannel;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Bound, Side, Sign};

use super::BookingServices;
use crate::infrastructure::idempotency::store::IdempotencyKey;
use crate::presentation::http::v3::command::CommandAction;
use crate::presentation::http::v3::error::V3Error;
use crate::presentation::http::v3::input::{self, RowRef};

pub struct BookingCreate {
    services: Arc<BookingServices>,
}

impl BookingCreate {
    pub fn new(services: Arc<BookingServices>) -> Self {
        Self { services }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingCreateInput {
    #[ts(type = "string")]
    pub branch_id: BranchId,
    #[ts(type = "string")]
    pub slot_id: Uuid,
    pub shop_side: String,
    pub currency: String,
    #[ts(type = "number")]
    pub quantity: Value,
    #[ts(type = "string")]
    pub quote_id: Option<Uuid>,
    pub referral_code: Option<String>,
    pub customer_id: Option<String>,
    pub display_name: Option<String>,
    pub phone: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "bookings.ts")]
pub struct BookingCreateOutput {
    pub id: String,
    pub booking_no: String,
    pub status: String,
    pub source_channel: String,
    pub branch_id: String,
    #[ts(type = "string")]
    pub slot_id: Uuid,
    pub customer_id: String,
    pub shop_side: String,
    pub currency: String,
    #[ts(type = "number")]
    pub quantity: Decimal,
    pub qr_token: Option<String>,
}

impl CommandAction for BookingCreate {
    const NAME: &'static str = "bookingCreate";
    const INHERITS: (EntryPath, &'static str) = (EntryPath::Rpc, "bookingCreate");
    type Input = BookingCreateInput;
    type Output = BookingCreateOutput;
    const ROUTE: &'static str = "/api/bookings";
    const METHOD: Method = Method::POST;

    fn stored_output(&self, output: &Self::Output) -> Value {
        let mut val = serde_json::to_value(output).unwrap_or_default();
        val["qrToken"] = Value::Null;
        val
    }

    async fn run(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        _branch: &BranchId,
        who: &Actor,
        input: Self::Input,
        _key: IdempotencyKey,
    ) -> Result<Self::Output, V3Error> {
        let services = self.services.services()?;

        let is_customer = who
            .roles
            .iter()
            .any(|r| RoleGrants::holds(r.group_code(), Permission::CustomerSelf));
        let is_desk = who
            .roles
            .iter()
            .any(|r| RoleGrants::holds(r.group_code(), Permission::BookingsDesk));

        let (channel, customer_id, contact) = if is_customer {
            let cid = who.customer_id.clone().ok_or_else(|| {
                V3Error::new(
                    "outOfScope",
                    None,
                    "Customer ID required",
                    StatusCode::FORBIDDEN,
                )
            })?;
            (SourceChannel::App, Some(cid), None)
        } else if is_desk {
            if let Some(cid) = input.customer_id {
                (SourceChannel::Desk, Some(cid), None)
            } else if let Some(phone) = input.phone {
                let display_name = input.display_name.unwrap_or_else(|| "Customer".to_string());
                (
                    SourceChannel::Desk,
                    None,
                    Some(ContactInput {
                        display_name,
                        phone,
                    }),
                )
            } else {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("customerId"),
                    "customerId or phone required",
                    StatusCode::BAD_REQUEST,
                ));
            }
        } else {
            return Err(V3Error::from(PolicyError::RoleNotAllowed));
        };

        let shop_side = match input.shop_side.to_uppercase().as_str() {
            "BUY" => Side::Buy,
            "SELL" => Side::Sell,
            _ => {
                return Err(V3Error::new(
                    "validationFailed",
                    Some("shopSide"),
                    "Invalid shop side",
                    StatusCode::BAD_REQUEST,
                ));
            }
        };

        let qty_text = match &input.quantity {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => String::new(),
        };
        let qty = input::decimal("quantity", &qty_text, Bound::numeric(20, 4, Sign::Positive))?;
        input::must_exist(tx, "currency", RowRef::Currency, &input.currency).await?;

        BranchTx::lock_also(tx, &input.branch_id)
            .await
            .map_err(|e| {
                V3Error::new(
                    "serverError",
                    None,
                    e.to_string(),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
            })?;

        let now = Utc::now();
        let org_uuid = Uuid::parse_str(&who.organization_id).unwrap_or_else(|_| Uuid::new_v4());
        let create_input = CreateBookingInput {
            organization_id: org_uuid,
            branch_id: input.branch_id.clone(),
            slot_id: input.slot_id,
            shop_side,
            requested_currency: input.currency.clone(),
            requested_quantity: qty,
            channel,
            actor_id: who.user_id.clone(),
            customer_id,
            contact,
            quote_id: input.quote_id,
            referral_code: input.referral_code,
            idempotency_key: None,
        };

        let created = services
            .booking_service
            .create_in(tx, create_input, now)
            .await
            .map_err(|err| match err {
                BookingError::BookingsClosed => V3Error::new(
                    "bookingsClosed",
                    None,
                    "BOOKINGS_CLOSED",
                    StatusCode::BAD_REQUEST,
                ),
                BookingError::CapacityFull => V3Error::new(
                    "capacityFull",
                    None,
                    "CAPACITY_FULL",
                    StatusCode::BAD_REQUEST,
                ),
                BookingError::SlotUnavailable => V3Error::new(
                    "capacityFull",
                    None,
                    "CAPACITY_FULL",
                    StatusCode::BAD_REQUEST,
                ),
                BookingError::ValidationFailed { field } => V3Error::new(
                    "validationFailed",
                    Some(field),
                    "VALIDATION_FAILED",
                    StatusCode::BAD_REQUEST,
                ),
                BookingError::NotFound => {
                    V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND)
                }
                other => V3Error::from(other),
            })?;

        let b = &created.booking;
        Ok(BookingCreateOutput {
            id: b.id.clone(),
            booking_no: b.booking_no.clone(),
            status: b.status.as_str().to_string(),
            source_channel: b.source_channel.as_str().to_string(),
            branch_id: b.branch_id.as_str().to_string(),
            slot_id: b.slot_id,
            customer_id: b.customer_id.clone(),
            shop_side: match b.shop_side {
                Side::Buy => "BUY".to_string(),
                Side::Sell => "SELL".to_string(),
            },
            currency: b.requested_currency.clone(),
            quantity: b.requested_quantity,
            qr_token: Some(created.qr_token),
        })
    }
}
