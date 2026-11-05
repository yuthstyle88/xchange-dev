//! `WalkInDesk` (XC-WALKIN-QUEUE, `docs/design/walkin-queue.md` §4, W3/W4/W6/W8): the queue entry a
//! desk sale is served under. It issues through `TicketIssuer::issue_in` (the only writer of new
//! tickets) and moves tickets through `QueueDesk`, so no second writer exists.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::postgres::PgRow;
use sqlx::{Postgres, Row, Transaction};
use xc_auth::domain::access::Actor;
use xc_kernel::ids::BranchId;

use crate::application::desk::queue_desk::{QueueDesk, ticket_has_live_sale};
use crate::application::issue::ticket_issuer::TicketIssuer;
use crate::application::status::status_service::StatusService;
use crate::domain::repository::QueueRepository;
use crate::domain::ticket::issue_source::IssueSource;
use crate::domain::ticket::queue_error::QueueError;
use crate::domain::ticket::queue_event::{EventKind, QueueEvent};
use crate::domain::ticket::queue_ticket::{QueueTicket, TicketStatus};
use crate::domain::ticket::service_type::ServiceType;

/// The prefix of the idempotency key a desk sale's ticket is stored under (W5): `desk-sale:<key>`.
pub const DESK_SALE_KEY_PREFIX: &str = "desk-sale:";

macro_rules! select_tickets {
    ($where_clause:literal) => {
        concat!(
            "SELECT id::text AS id, organization_id::text AS organization_id, branch_id, \
             service_date, service_type, sequence, display_number, status, customer_id, \
             booking_id, kiosk_request_id, counter_teller_id::text AS counter_teller_id, \
             issued_at, called_at, served_at, completed_at, ended_reason, idempotency_key \
             FROM queue_tickets ",
            $where_clause
        )
    };
}

fn map_row(r: PgRow) -> AppResult<QueueTicket> {
    let service_type_str: String = r.get("service_type");
    let service_type = ServiceType::parse(&service_type_str)
        .ok_or_else(|| AppError::Internal(format!("invalid service_type: {service_type_str}")))?;
    let status_str: String = r.get("status");
    let status = TicketStatus::parse(&status_str)
        .ok_or_else(|| AppError::Internal(format!("invalid status: {status_str}")))?;
    let branch_str: String = r.get("branch_id");
    Ok(QueueTicket {
        id: r.get("id"),
        organization_id: r.get("organization_id"),
        branch_id: BranchId::new(branch_str),
        service_date: r.get("service_date"),
        service_type,
        sequence: r.get("sequence"),
        display_number: r.get("display_number"),
        status,
        customer_id: r.get("customer_id"),
        booking_id: r.get("booking_id"),
        kiosk_request_id: r.get("kiosk_request_id"),
        counter_teller_id: r.get("counter_teller_id"),
        issued_at: r.get("issued_at"),
        called_at: r.get("called_at"),
        served_at: r.get("served_at"),
        completed_at: r.get("completed_at"),
        ended_reason: r.get("ended_reason"),
        idempotency_key: r.get("idempotency_key"),
    })
}

#[derive(Clone)]
pub struct WalkInDesk {
    issuer: TicketIssuer,
    desk: QueueDesk,
    /// The finish step writes the `FINISH` event itself (W8 skips `QueueDesk`'s checks).
    repo: Arc<dyn QueueRepository>,
    /// The estimate refresh after the finish, as `QueueDesk::finish_in` does it.
    status: StatusService,
}

impl WalkInDesk {
    pub fn new(
        issuer: TicketIssuer,
        desk: QueueDesk,
        repo: Arc<dyn QueueRepository>,
        status: StatusService,
    ) -> Self {
        Self {
            issuer,
            desk,
            repo,
            status,
        }
    }

    /// The ticket the customer at the counter is served under, SERVING and held by `actor` when it
    /// answers (W3). In this order (W4):
    /// 1. the ticket stored under `desk-sale:<sale_key>` in the branch (a replay), unchanged;
    /// 2. the actor's held CALLED/SERVING walk-in ticket with no booking, no kiosk request and
    ///    either no other desk sale's key or no non-CANCELLED transaction naming it: started if
    ///    CALLED, then returned;
    /// 3. `customer_id`'s active `App` ticket of today in the branch, called and started;
    /// 4. a new `IssueSource::Desk` ticket under `desk-sale:<sale_key>`, called and started.
    ///
    /// A held booking or kiosk ticket, or a held ticket of another desk sale with a live
    /// transaction, refuses `QueueError::AlreadyServing`. When `may_issue` is false (W6), (1) and
    /// (2) still answer and (3) and (4) refuse `QueueError::OutsideServiceHours`. A closed queue
    /// refuses `QueueError::QueueClosed` (W7) and uses no number.
    #[allow(clippy::too_many_arguments)]
    pub async fn serve_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        customer_id: Option<&str>,
        sale_key: &str,
        may_issue: bool,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        let key = format!("{DESK_SALE_KEY_PREFIX}{sale_key}");
        if let Some(stored) = self.repo.find_by_key_in(tx, branch, &key).await? {
            return Ok(stored);
        }

        if uuid::Uuid::try_parse(&actor.user_id).is_ok() {
            if let Some(row) = sqlx::query(select_tickets!(
                "WHERE branch_id = $1 AND counter_teller_id = $2::uuid \
                 AND status IN ('CALLED', 'SERVING') LIMIT 1"
            ))
            .bind(branch.as_str())
            .bind(&actor.user_id)
            .fetch_optional(&mut **tx)
            .await?
            {
                let held = map_row(row)?;
                if held.booking_id.is_none()
                    && held.kiosk_request_id.is_none()
                    && (!held.idempotency_key.starts_with(DESK_SALE_KEY_PREFIX)
                        || !ticket_has_live_sale(tx, &held.id).await?)
                {
                    if held.status == TicketStatus::Called {
                        return self.desk.start_in(tx, actor, branch, &held.id, now).await;
                    }
                    return Ok(held);
                }
                return Err(QueueError::AlreadyServing.into());
            }
        }

        if let Some(c) = customer_id {
            if let Some(facts) = self.repo.branch_in(tx, branch).await? {
                let tz: chrono_tz::Tz = facts.timezone.parse().map_err(|e| {
                    AppError::Internal(format!("invalid timezone {}: {e}", facts.timezone))
                })?;
                let service_date = now.with_timezone(&tz).date_naive();
                if let Some(row) = sqlx::query(select_tickets!(
                    "WHERE branch_id = $1 AND service_date = $2 AND customer_id = $3 \
                     AND service_type = 'WALK_IN' AND status IN ('WAITING', 'CALLED', 'SERVING') \
                     LIMIT 1"
                ))
                .bind(branch.as_str())
                .bind(service_date)
                .bind(c)
                .fetch_optional(&mut **tx)
                .await?
                {
                    if !may_issue {
                        return Err(QueueError::OutsideServiceHours.into());
                    }
                    let app = map_row(row)?;
                    // W3: answered SERVING for the caller. The caller's own held ticket was (2); a ticket
                    // another teller holds is not handed over (#5361 review).
                    if app.status != TicketStatus::Waiting {
                        return Err(QueueError::AlreadyServing.into());
                    }
                    self.desk.call_in(tx, actor, branch, &app.id, now).await?;
                    return self.desk.start_in(tx, actor, branch, &app.id, now).await;
                }
            }
        }

        if !may_issue {
            return Err(QueueError::OutsideServiceHours.into());
        }
        let issued = self
            .issuer
            .issue_in(tx, branch, &IssueSource::Desk, &key, now)
            .await?;
        self.desk
            .call_in(tx, actor, branch, &issued.id, now)
            .await?;
        self.desk.start_in(tx, actor, branch, &issued.id, now).await
    }

    /// Finishes the sale's ticket (W8): SERVING → COMPLETED with its `FINISH` event and the
    /// estimate refresh, as `QueueDesk::finish_in` writes them, without the on-duty and owner
    /// checks. `Ok(false)` and no change when the ticket is not SERVING.
    pub async fn finish_for_sale_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        ticket_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        if uuid::Uuid::try_parse(ticket_id).is_err() {
            return Ok(false);
        }
        let Some(row) = sqlx::query(select_tickets!(
            "WHERE branch_id = $1 AND id = $2::uuid FOR UPDATE"
        ))
        .bind(branch.as_str())
        .bind(ticket_id)
        .fetch_optional(&mut **tx)
        .await?
        else {
            return Ok(false);
        };
        let ticket = map_row(row)?;
        if ticket.status != TicketStatus::Serving {
            return Ok(false);
        }

        let rows = sqlx::query(
            "UPDATE queue_tickets \
             SET status = 'COMPLETED', completed_at = $2 \
             WHERE id = $1::uuid AND status = 'SERVING'",
        )
        .bind(&ticket.id)
        .bind(now)
        .execute(&mut **tx)
        .await?;
        if rows.rows_affected() == 0 {
            return Ok(false);
        }

        let event = QueueEvent {
            id: uuid::Uuid::new_v4().to_string(),
            organization_id: ticket.organization_id,
            ticket_id: ticket.id,
            status: TicketStatus::Completed,
            kind: EventKind::Finish,
            actor_id: ticket.counter_teller_id,
            at: now,
            reason: None,
        };
        self.repo.insert_event_in(tx, &event).await?;
        self.status.refresh_in(tx, branch, now).await?;
        Ok(true)
    }
}
