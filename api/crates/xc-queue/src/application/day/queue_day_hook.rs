//! `QueueDayHook` (design §5, Q8): when the branch day closes, every WAITING or CALLED ticket of
//! the branch becomes CANCELLED with `ended_reason = 'dayClosed'`. Tested directly; the call from
//! `BranchDay` is a follow-up (lane controller's ruling 3 on #880).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use xc_kernel::ids::BranchId;

use crate::application::status::status_service::StatusService;
use crate::domain::repository::QueueRepository;
use crate::domain::ticket::queue_event::{EventKind, QueueEvent};
use crate::domain::ticket::queue_ticket::{QueueTicket, TicketStatus};
use crate::domain::ticket::service_type::ServiceType;

#[derive(Clone)]
pub struct QueueDayHook {
    repo: Arc<dyn QueueRepository>,
    status: StatusService,
    booking_events: Option<Arc<dyn xc_kernel::booking_queue_events::BookingQueueEvents>>,
}

impl QueueDayHook {
    pub fn new(repo: Arc<dyn QueueRepository>, status: StatusService) -> Self {
        Self {
            repo,
            status,
            booking_events: None,
        }
    }

    pub fn with_booking_events(
        mut self,
        booking_events: Arc<dyn xc_kernel::booking_queue_events::BookingQueueEvents>,
    ) -> Self {
        self.booking_events = Some(booking_events);
        self
    }

    /// Cancels the branch's WAITING and CALLED tickets (`dayClosed`), one CANCEL event each, and
    /// refreshes the status, inside the caller's transaction. Answers the tickets it cancelled.
    pub async fn on_day_closed(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<QueueTicket>> {
        let rows = sqlx::query(
            r#"
            SELECT
                id::text AS id,
                organization_id::text AS organization_id,
                branch_id,
                service_date,
                service_type,
                sequence,
                display_number,
                status,
                customer_id,
                booking_id,
                kiosk_request_id,
                counter_teller_id::text AS counter_teller_id,
                issued_at,
                called_at,
                served_at,
                completed_at,
                ended_reason,
                idempotency_key
            FROM queue_tickets
            WHERE branch_id = $1 AND status IN ('WAITING', 'CALLED')
            FOR UPDATE
            "#,
        )
        .bind(branch.as_str())
        .fetch_all(&mut **tx)
        .await?;

        let mut cancelled_tickets = Vec::new();

        for row in rows {
            use sqlx::Row;
            let id: String = row.get("id");
            let organization_id: String = row.get("organization_id");
            let branch_str: String = row.get("branch_id");
            let service_type_str: String = row.get("service_type");
            let service_type = ServiceType::parse(&service_type_str).ok_or_else(|| {
                AppError::Internal(format!("invalid service_type: {service_type_str}"))
            })?;

            sqlx::query(
                "UPDATE queue_tickets SET status = 'CANCELLED', ended_reason = 'dayClosed' WHERE id = $1::uuid",
            )
            .bind(&id)
            .execute(&mut **tx)
            .await?;

            let event = QueueEvent {
                id: uuid::Uuid::new_v4().to_string(),
                organization_id: organization_id.clone(),
                ticket_id: id.clone(),
                status: TicketStatus::Cancelled,
                kind: EventKind::Cancel,
                actor_id: None,
                at: now,
                reason: Some("dayClosed".to_owned()),
            };

            self.repo.insert_event_in(tx, &event).await?;

            let booking_id: Option<String> = row.get("booking_id");
            if let Some(bid) = &booking_id {
                if let Some(events) = &self.booking_events {
                    events
                        .on_ticket_ended_in(tx, branch, bid, "dayClosed", now)
                        .await?;
                }
            }

            cancelled_tickets.push(QueueTicket {
                id,
                organization_id,
                branch_id: BranchId::new(branch_str),
                service_date: row.get("service_date"),
                service_type,
                sequence: row.get("sequence"),
                display_number: row.get("display_number"),
                status: TicketStatus::Cancelled,
                customer_id: row.get("customer_id"),
                booking_id,
                kiosk_request_id: row.get("kiosk_request_id"),
                counter_teller_id: row.get("counter_teller_id"),
                issued_at: row.get("issued_at"),
                called_at: row.get("called_at"),
                served_at: row.get("served_at"),
                completed_at: row.get("completed_at"),
                ended_reason: Some("dayClosed".to_owned()),
                idempotency_key: row.get("idempotency_key"),
            });
        }

        self.status.refresh_in(tx, branch, now).await?;

        Ok(cancelled_tickets)
    }
}
