//! `TicketIssuer` (design §5, §4.3 rows 1–3, Q3, Q4, Q11, Q12): the only writer of new tickets.
//! It implements the kernel port `QueueTickets`, so the kiosk and the booking check-in issue a
//! ticket inside their own transaction without depending on this crate.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use xc_auth::domain::access::Actor;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::queue_tickets::{IssuedTicket, QueueTicketSource, QueueTickets};

use crate::application::desk::queue_desk::{QueueDesk, check_ticket_owner};
use crate::application::status::status_service::StatusService;
use crate::domain::repository::QueueRepository;
use crate::domain::ticket::display_number::DisplayNumber;
use crate::domain::ticket::issue_source::IssueSource;
use crate::domain::ticket::queue_error::QueueError;
use crate::domain::ticket::queue_event::{EventKind, QueueEvent};
use crate::domain::ticket::queue_ticket::{QueueTicket, TicketStatus};

#[derive(Clone)]
pub struct TicketIssuer {
    branch_tx: BranchTx,
    repo: Arc<dyn QueueRepository>,
    status: StatusService,
    /// The teller the kiosk's `accept` starts tickets for (`acting_as`); `None` refuses it.
    actor: Option<Actor>,
    booking_events: Option<Arc<dyn xc_kernel::booking_queue_events::BookingQueueEvents>>,
}

fn source_matches_ticket(source: &IssueSource, ticket: &QueueTicket) -> bool {
    match source {
        IssueSource::Kiosk { request_id } => {
            ticket.kiosk_request_id.as_deref() == Some(request_id) && ticket.booking_id.is_none()
        }
        IssueSource::Booking { booking_id } => {
            ticket.booking_id.as_deref() == Some(booking_id) && ticket.kiosk_request_id.is_none()
        }
        IssueSource::Desk => {
            ticket.customer_id.is_none()
                && ticket.kiosk_request_id.is_none()
                && ticket.booking_id.is_none()
        }
        IssueSource::App { customer_id } => {
            ticket.customer_id.as_deref() == Some(customer_id)
                && ticket.kiosk_request_id.is_none()
                && ticket.booking_id.is_none()
        }
    }
}

impl TicketIssuer {
    pub fn new(branch_tx: BranchTx, repo: Arc<dyn QueueRepository>) -> Self {
        let status = StatusService::new(branch_tx.clone(), repo.clone());
        Self {
            branch_tx,
            repo,
            status,
            actor: None,
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

    /// This issuer acting for `actor`, the teller whose kiosk `accept` starts a ticket
    /// (`QueueTickets::start_for_kiosk_in`). `QueueDesk` checks the actor's `QueueOperate`.
    pub fn acting_as(self, actor: Actor) -> Self {
        Self {
            actor: Some(actor),
            ..self
        }
    }

    /// Calls (if WAITING) and starts the kiosk request's ticket through `QueueDesk::call_in` and
    /// `start_in`, so the desk's rules apply unchanged. A SERVING ticket is left as it is; a
    /// skipped, cancelled or finished one is refused `staleVersion`.
    /// `Ok(false)` when the request has no ticket.
    pub async fn start_for_kiosk_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        request_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let Some(ticket) = self
            .repo
            .find_by_kiosk_request_in(tx, branch, request_id)
            .await?
        else {
            return Ok(false);
        };
        let actor = self.actor.as_ref().ok_or(AppError::Forbidden)?;
        let mut desk = QueueDesk::new(
            self.branch_tx.clone(),
            self.repo.clone(),
            self.status.clone(),
        );
        if let Some(events) = &self.booking_events {
            desk = desk.with_booking_events(events.clone());
        }
        match ticket.status {
            TicketStatus::Waiting => {
                desk.call_in(tx, actor, branch, &ticket.id, now).await?;
                desk.start_in(tx, actor, branch, &ticket.id, now).await?;
            }
            TicketStatus::Called => {
                desk.start_in(tx, actor, branch, &ticket.id, now).await?;
            }
            TicketStatus::Serving => {
                check_ticket_owner(actor, &ticket)?;
            }
            // A skipped, cancelled or finished ticket is not started again (§4.3).
            TicketStatus::Completed | TicketStatus::Cancelled | TicketStatus::NoShow => {
                return Err(QueueError::StaleVersion.into());
            }
        }
        Ok(true)
    }

    /// Issues a ticket in a branch transaction of its own (the desk's walk-in, QU-B's route).
    pub async fn issue(
        &self,
        branch: &BranchId,
        source: IssueSource,
        idempotency_key: &str,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        let issuer = self.clone();
        let target_branch = branch.clone();
        let key = idempotency_key.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move {
                    issuer
                        .issue_in(tx, &target_branch, &source, &key, now)
                        .await
                })
            })
            .await
    }

    /// Issues a ticket inside the caller's transaction. In this order:
    /// 1. a ticket already stored under `idempotency_key` in the branch is returned unchanged;
    /// 2. a kiosk request that already has a ticket, or a booking with an active ticket, returns
    ///    that ticket;
    /// 3. a closed queue (or no status row) refuses `QueueError::QueueClosed`, consuming no number;
    /// 4. otherwise the next number of (branch, the branch's local date, the source's type) is
    ///    taken, the WAITING ticket is inserted with its `ISSUE` event.
    pub async fn issue_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        source: &IssueSource,
        idempotency_key: &str,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        // 1. A ticket already stored under idempotency_key in the branch is returned unchanged,
        // unless the key was reused for a different source (Ruling 4 on #879).
        if let Some(stored) = self
            .repo
            .find_by_key_in(tx, branch, idempotency_key)
            .await?
        {
            if !source_matches_ticket(source, &stored) {
                return Err(AppError::Conflict("idempotencyConflict".to_owned()));
            }
            return Ok(stored);
        }

        // 2. A kiosk request that already has a ticket, or a booking with an active ticket, returns that ticket.
        match source {
            IssueSource::Kiosk { request_id } => {
                if let Some(existing) = self
                    .repo
                    .find_by_kiosk_request_in(tx, branch, request_id)
                    .await?
                {
                    return Ok(existing);
                }
            }
            IssueSource::Booking { booking_id } => {
                if let Some(existing) = self
                    .repo
                    .find_active_by_booking_in(tx, branch, booking_id)
                    .await?
                {
                    return Ok(existing);
                }
            }
            IssueSource::Desk | IssueSource::App { .. } => {}
        }

        // 3. A closed queue (or no status row) refuses `QueueError::QueueClosed`, consuming no number.
        if !self.repo.queue_open_in(tx, branch).await? {
            return Err(QueueError::QueueClosed.into());
        }

        // 4. Otherwise the next number of (branch, the branch's local date, the source's type) is
        // taken, the WAITING ticket is inserted with its `ISSUE` event.
        let facts = self
            .repo
            .branch_in(tx, branch)
            .await?
            .ok_or_else(|| AppError::Validation("queueClosed".to_owned()))?;

        let tz: chrono_tz::Tz = facts
            .timezone
            .parse()
            .map_err(|e| AppError::Internal(format!("invalid timezone {}: {e}", facts.timezone)))?;
        let service_date = now.with_timezone(&tz).date_naive();

        if let IssueSource::App { customer_id } = source {
            let existing: Option<(String,)> = sqlx::query_as(
                "SELECT id::text FROM queue_tickets \
                 WHERE branch_id = $1 AND service_date = $2 AND customer_id = $3 \
                   AND service_type = 'WALK_IN' AND status IN ('WAITING', 'CALLED', 'SERVING') \
                 LIMIT 1",
            )
            .bind(branch.as_str())
            .bind(service_date)
            .bind(customer_id)
            .fetch_optional(&mut **tx)
            .await?;
            if let Some((existing_id,)) = existing {
                return Err(AppError::Conflict(format!("alreadyQueued:{existing_id}")));
            }
        }

        let service_type = source.service_type();
        let sequence = self
            .repo
            .next_sequence_in(
                tx,
                &facts.organization_id,
                branch,
                service_date,
                service_type,
            )
            .await?;

        let display_number = DisplayNumber::format(service_type, sequence);
        let ticket_id = uuid::Uuid::new_v4().to_string();

        let (customer_id, kiosk_request_id, booking_id) = match source {
            IssueSource::Kiosk { request_id } => (None, Some(request_id.clone()), None),
            IssueSource::Booking { booking_id } => (None, None, Some(booking_id.clone())),
            IssueSource::Desk => (None, None, None),
            IssueSource::App { customer_id } => (Some(customer_id.clone()), None, None),
        };

        let ticket = QueueTicket {
            id: ticket_id.clone(),
            organization_id: facts.organization_id.clone(),
            branch_id: branch.clone(),
            service_date,
            service_type,
            sequence,
            display_number,
            status: TicketStatus::Waiting,
            customer_id,
            booking_id,
            kiosk_request_id,
            counter_teller_id: None,
            issued_at: now,
            called_at: None,
            served_at: None,
            completed_at: None,
            ended_reason: None,
            idempotency_key: idempotency_key.to_owned(),
        };

        if let Err(err) = self.repo.insert_in(tx, &ticket).await {
            if err
                .to_string()
                .contains("queue_tickets_one_active_app_ticket")
            {
                return Err(AppError::Conflict("alreadyQueued".to_owned()));
            }
            return Err(err);
        }

        let event = QueueEvent {
            id: uuid::Uuid::new_v4().to_string(),
            organization_id: facts.organization_id,
            ticket_id,
            status: TicketStatus::Waiting,
            kind: EventKind::Issue,
            actor_id: None,
            at: now,
            reason: None,
        };

        self.repo.insert_event_in(tx, &event).await?;
        self.status.refresh_in(tx, branch, now).await?;

        Ok(ticket)
    }

    /// Cancels any active (WAITING or CALLED) ticket for `booking_id` inside `tx`.
    pub async fn cancel_active_for_booking_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        booking_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let Some(ticket) = self
            .repo
            .find_active_by_booking_in(tx, branch, booking_id)
            .await?
        else {
            return Ok(false);
        };

        if ticket.status != TicketStatus::Waiting && ticket.status != TicketStatus::Called {
            return Ok(false);
        }

        let rows = sqlx::query(
            "UPDATE queue_tickets SET status = 'CANCELLED', ended_reason = 'bookingCancelled' WHERE id = $1::uuid AND status IN ('WAITING', 'CALLED')",
        )
        .bind(&ticket.id)
        .execute(&mut **tx)
        .await?;

        if rows.rows_affected() == 0 {
            return Ok(false);
        }

        let event = QueueEvent {
            id: uuid::Uuid::new_v4().to_string(),
            organization_id: ticket.organization_id.clone(),
            ticket_id: ticket.id.clone(),
            status: TicketStatus::Cancelled,
            kind: EventKind::Cancel,
            actor_id: self.actor.as_ref().map(|a| a.user_id.clone()),
            at: now,
            reason: Some("bookingCancelled".to_owned()),
        };

        self.repo.insert_event_in(tx, &event).await?;
        self.status.refresh_in(tx, branch, now).await?;

        Ok(true)
    }
}

impl QueueTickets for TicketIssuer {
    fn issue_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        source: &'a QueueTicketSource,
        idempotency_key: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<IssuedTicket>> {
        Box::pin(async move {
            let domain_source = IssueSource::from(source);
            let ticket = self
                .issue_in(tx, branch, &domain_source, idempotency_key, now)
                .await?;
            Ok(IssuedTicket {
                id: ticket.id,
                service_date: ticket.service_date,
                service_type: ticket.service_type.as_str().to_owned(),
                sequence: ticket.sequence,
                display_number: ticket.display_number,
            })
        })
    }

    fn start_for_kiosk_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move { self.start_for_kiosk_in(tx, branch, request_id, now).await })
    }

    fn cancel_active_for_booking_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        booking_id: &'a str,
        now: DateTime<Utc>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            self.cancel_active_for_booking_in(tx, branch, booking_id, now)
                .await
        })
    }
}
