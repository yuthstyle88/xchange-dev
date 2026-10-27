//! `bookingQrReissue` command (`POST /api/bookings/{id}/qr`, design §4.4, §4.5, BK-6 #1388 / #1913).

use std::sync::Arc;

use actix_web::http::{Method, StatusCode};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use xc_auth::domain::access::{Actor, Permission, RoleGrants};
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_booking::domain::booking::booking_error::BookingError;
use xc_booking::domain::booking::booking_status::BookingStatus;
use xc_booking::domain::booking::effective_status::{OverdueFacts, effective_status};
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

#[derive(sqlx::FromRow)]
struct BookingQrRow {
    customer_id: String,
    branch_id: String,
    status: String,
    confirm_expires_at: Option<DateTime<Utc>>,
    grace_until: DateTime<Utc>,
    service_date: NaiveDate,
    branch_today: NaiveDate,
    has_active_ticket: bool,
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
        let now = Utc::now();

        let row: Option<BookingQrRow> = sqlx::query_as(
            "SELECT b.customer_id, b.branch_id, b.status, \
                    b.confirm_expires_at, \
                    (s.start_at + (COALESCE(p.no_show_grace_minutes, 15) * interval '1 minute')) AS grace_until, \
                    s.service_date, \
                    ($2 AT TIME ZONE COALESCE(NULLIF(br.timezone, ''), 'Asia/Bangkok'))::date AS branch_today, \
                    EXISTS ( \
                        SELECT 1 FROM queue_tickets qt_active \
                        WHERE qt_active.booking_id = b.id \
                          AND qt_active.status IN ('WAITING', 'CALLED', 'SERVING') \
                    ) AS has_active_ticket \
             FROM bookings b \
             JOIN booking_slots s ON s.id = b.slot_id \
             JOIN branches br ON br.id = b.branch_id \
             LEFT JOIN booking_policies p ON p.organization_id = br.organization_id \
             WHERE b.id = $1",
        )
        .bind(&input.id)
        .bind(now)
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

        let Some(row) = row else {
            return Err(V3Error::new(
                "notFound",
                None,
                "NOT_FOUND",
                StatusCode::NOT_FOUND,
            ));
        };

        if row.branch_id != branch.as_str() {
            return Err(out_of_scope());
        }

        let is_staff = who
            .roles
            .iter()
            .any(|r| RoleGrants::holds(r.group_code(), Permission::BookingsDesk));

        if !is_staff && who.customer_id.as_deref() != Some(row.customer_id.as_str()) {
            return Err(out_of_scope());
        }

        let facts = OverdueFacts {
            confirm_expires_at: row.confirm_expires_at,
            grace_until: row.grace_until,
            service_date: row.service_date,
            branch_today: row.branch_today,
            has_active_ticket: row.has_active_ticket,
        };
        let effective =
            BookingStatus::parse(&row.status).map(|st| effective_status(st, &facts, now));
        if !matches!(
            effective,
            Some(
                BookingStatus::PendingCustomerConfirmation
                    | BookingStatus::Confirmed
                    | BookingStatus::CheckedIn
            )
        ) {
            return Err(V3Error::new(
                "staleVersion",
                None,
                "STALE_VERSION",
                StatusCode::CONFLICT,
            ));
        }

        let reissued = services
            .booking_service
            .reissue_qr_in(tx, &input.id, &who.user_id, now)
            .await
            .map_err(|err| match err {
                BookingError::StaleVersion => {
                    V3Error::new("staleVersion", None, "STALE_VERSION", StatusCode::CONFLICT)
                }
                BookingError::NotFound => {
                    V3Error::new("notFound", None, "NOT_FOUND", StatusCode::NOT_FOUND)
                }
                other => V3Error::from(other),
            })?;

        Ok(BookingQrReissueOutput {
            id: reissued.booking.id,
            qr_token: Some(reissued.qr_token),
            qr_version: reissued.booking.qr_version,
        })
    }
}
