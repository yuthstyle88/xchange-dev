//! `QueueDesk` (design §5, §3, §4.3, Q6, Q7, Q11): the teller's commands on tickets.
//!
//! Every command checks `QueueOperate` through `Authorizer::check` (a refusal answers
//! `AppError::Forbidden`), needs the caller's OPEN shift in the branch (`noOpenShift`) and their
//! counter, compares-and-sets on the expected status (`staleVersion`), writes one `queue_events`
//! row with the caller as actor, and runs in the branch's `BranchTx` with
//! `StatusService::refresh_in`. Start, finish and recall act only on the caller's own ticket,
//! unless the caller is BRANCH_MANAGER (`notYourTicket`).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};
use xc_auth::domain::access::{Actor, Authorizer, Permission, Role, Target};
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;

use crate::application::status::status_service::StatusService;
use crate::domain::repository::{BranchFacts, QueueRepository};
use crate::domain::ticket::queue_error::QueueError;
use crate::domain::ticket::queue_event::{EventKind, QueueEvent};
use crate::domain::ticket::queue_ticket::{QueueTicket, TicketStatus};
use crate::domain::ticket::service_type::ServiceType;

#[derive(Clone)]
pub struct QueueDesk {
    branch_tx: BranchTx,
    repo: Arc<dyn QueueRepository>,
    status: StatusService,
}

async fn check_counter_active(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    teller_id: &str,
) -> AppResult<()> {
    let counter_status: Option<(String,)> = sqlx::query_as(
        "SELECT status FROM queue_counters WHERE branch_id = $1 AND teller_id = $2::uuid",
    )
    .bind(branch.as_str())
    .bind(teller_id)
    .fetch_optional(&mut **tx)
    .await?;

    match counter_status {
        None => Err(QueueError::NoOpenShift.into()),
        Some((s,)) if s == "PAUSED" => Err(QueueError::CounterPaused.into()),
        _ => Ok(()),
    }
}

async fn check_not_serving(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    teller_id: &str,
) -> AppResult<()> {
    let count: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM queue_tickets \
         WHERE branch_id = $1 AND counter_teller_id = $2::uuid AND status IN ('CALLED', 'SERVING')",
    )
    .bind(branch.as_str())
    .bind(teller_id)
    .fetch_one(&mut **tx)
    .await?;

    if count.0 > 0 {
        return Err(QueueError::AlreadyServing.into());
    }
    Ok(())
}

fn check_ticket_owner(actor: &Actor, ticket: &QueueTicket) -> AppResult<()> {
    if ticket.counter_teller_id.as_deref() == Some(&actor.user_id)
        || actor.roles.iter().any(|r| {
            r.role == Role::BranchManager
                && r.branch.as_ref().is_none_or(|b| b == &ticket.branch_id)
        })
    {
        Ok(())
    } else {
        Err(QueueError::NotYourTicket.into())
    }
}

async fn fetch_ticket_for_update(
    tx: &mut Transaction<'_, Postgres>,
    branch: &BranchId,
    ticket_id: &str,
) -> AppResult<QueueTicket> {
    let row = sqlx::query(
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
        WHERE branch_id = $1 AND id = $2::uuid
        FOR UPDATE
        "#,
    )
    .bind(branch.as_str())
    .bind(ticket_id)
    .fetch_optional(&mut **tx)
    .await?;

    match row {
        Some(r) => {
            use sqlx::Row;
            let service_type_str: String = r.get("service_type");
            let service_type = ServiceType::parse(&service_type_str).ok_or_else(|| {
                AppError::Internal(format!("invalid service_type: {service_type_str}"))
            })?;

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
        None => Err(QueueError::NotFound.into()),
    }
}

impl QueueDesk {
    pub fn new(branch_tx: BranchTx, repo: Arc<dyn QueueRepository>, status: StatusService) -> Self {
        Self {
            branch_tx,
            repo,
            status,
        }
    }

    async fn authorize_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<BranchFacts> {
        let facts = self
            .repo
            .branch_in(tx, branch)
            .await?
            .ok_or_else(|| AppError::NotFound {
                entity: "branch",
                id: branch.as_str().to_owned(),
            })?;

        Authorizer::check(
            Some(actor),
            Permission::QueueOperate,
            &Target::new(&facts.organization_id, Some(branch.clone()), None),
            now,
        )
        .map_err(|_| AppError::Forbidden)?;

        Ok(facts)
    }

    /// Calls the first WAITING ticket in `CallOrder` (BOOKING first, then the lowest sequence),
    /// locked `FOR UPDATE SKIP LOCKED`. Refused `counterPaused`, `alreadyServing` (the caller
    /// holds a CALLED or SERVING ticket) or `nothingWaiting`.
    pub async fn call_next(
        &self,
        actor: &Actor,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        let desk = self.clone();
        let b = branch.clone();
        let act = actor.clone();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { desk.call_next_in(tx, &act, &b, now).await })
            })
            .await
    }

    /// `call_next` inside the caller's transaction, which must already hold the branch's lock (the v3
    /// command seam's `BranchTx`, QU-7 #1266).
    pub async fn call_next_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        self.authorize_in(tx, actor, branch, now).await?;
        check_counter_active(tx, branch, &actor.user_id).await?;
        check_not_serving(tx, branch, &actor.user_id).await?;

        let next = self.repo.lock_next_waiting_in(tx, branch).await?;
        let ticket = match next {
            Some(t) => t,
            None => return Err(QueueError::NothingWaiting.into()),
        };

        let rows = sqlx::query(
            "UPDATE queue_tickets \
             SET status = 'CALLED', counter_teller_id = $2::uuid, called_at = $3 \
             WHERE id = $1::uuid AND status = 'WAITING'",
        )
        .bind(&ticket.id)
        .bind(&actor.user_id)
        .bind(now)
        .execute(&mut **tx)
        .await?;

        if rows.rows_affected() == 0 {
            return Err(QueueError::StaleVersion.into());
        }

        let event = QueueEvent {
            id: uuid::Uuid::new_v4().to_string(),
            organization_id: ticket.organization_id.clone(),
            ticket_id: ticket.id.clone(),
            status: TicketStatus::Called,
            kind: EventKind::Call,
            actor_id: Some(actor.user_id.clone()),
            at: now,
            reason: None,
        };
        self.repo.insert_event_in(tx, &event).await?;
        self.status.refresh_in(tx, branch, now).await?;

        Ok(QueueTicket {
            status: TicketStatus::Called,
            counter_teller_id: Some(actor.user_id.clone()),
            called_at: Some(now),
            ..ticket
        })
    }

    /// Calls a chosen WAITING ticket. Same refusals as `call_next`, plus `notFound`.
    pub async fn call(
        &self,
        actor: &Actor,
        branch: &BranchId,
        ticket_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        let desk = self.clone();
        let b = branch.clone();
        let act = actor.clone();
        let tid = ticket_id.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { desk.call_in(tx, &act, &b, &tid, now).await })
            })
            .await
    }

    /// `call` inside the caller's transaction, which must already hold the branch's lock (the v3
    /// command seam's `BranchTx`, QU-7 #1266).
    pub async fn call_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        ticket_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        self.authorize_in(tx, actor, branch, now).await?;
        check_counter_active(tx, branch, &actor.user_id).await?;
        check_not_serving(tx, branch, &actor.user_id).await?;

        let ticket = fetch_ticket_for_update(tx, branch, ticket_id).await?;
        if ticket.status != TicketStatus::Waiting {
            return Err(QueueError::StaleVersion.into());
        }

        let rows = sqlx::query(
            "UPDATE queue_tickets \
             SET status = 'CALLED', counter_teller_id = $2::uuid, called_at = $3 \
             WHERE id = $1::uuid AND status = 'WAITING'",
        )
        .bind(&ticket.id)
        .bind(&actor.user_id)
        .bind(now)
        .execute(&mut **tx)
        .await?;

        if rows.rows_affected() == 0 {
            return Err(QueueError::StaleVersion.into());
        }

        let event = QueueEvent {
            id: uuid::Uuid::new_v4().to_string(),
            organization_id: ticket.organization_id.clone(),
            ticket_id: ticket.id.clone(),
            status: TicketStatus::Called,
            kind: EventKind::Call,
            actor_id: Some(actor.user_id.clone()),
            at: now,
            reason: None,
        };
        self.repo.insert_event_in(tx, &event).await?;
        self.status.refresh_in(tx, branch, now).await?;

        Ok(QueueTicket {
            status: TicketStatus::Called,
            counter_teller_id: Some(actor.user_id.clone()),
            called_at: Some(now),
            ..ticket
        })
    }

    /// CALLED → CALLED, an event only.
    pub async fn recall(
        &self,
        actor: &Actor,
        branch: &BranchId,
        ticket_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        let desk = self.clone();
        let b = branch.clone();
        let act = actor.clone();
        let tid = ticket_id.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { desk.recall_in(tx, &act, &b, &tid, now).await })
            })
            .await
    }

    /// `recall` inside the caller's transaction, which must already hold the branch's lock (the v3
    /// command seam's `BranchTx`, QU-7 #1266).
    pub async fn recall_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        ticket_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        self.authorize_in(tx, actor, branch, now).await?;
        let ticket = fetch_ticket_for_update(tx, branch, ticket_id).await?;
        if ticket.status != TicketStatus::Called {
            return Err(QueueError::StaleVersion.into());
        }
        check_ticket_owner(actor, &ticket)?;

        let event = QueueEvent {
            id: uuid::Uuid::new_v4().to_string(),
            organization_id: ticket.organization_id.clone(),
            ticket_id: ticket.id.clone(),
            status: TicketStatus::Called,
            kind: EventKind::Recall,
            actor_id: Some(actor.user_id.clone()),
            at: now,
            reason: None,
        };
        self.repo.insert_event_in(tx, &event).await?;
        self.status.refresh_in(tx, branch, now).await?;

        Ok(ticket)
    }

    /// CALLED → NO_SHOW, final (owner Q-A3).
    pub async fn skip(
        &self,
        actor: &Actor,
        branch: &BranchId,
        ticket_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        let desk = self.clone();
        let b = branch.clone();
        let act = actor.clone();
        let tid = ticket_id.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { desk.skip_in(tx, &act, &b, &tid, now).await })
            })
            .await
    }

    /// `skip` inside the caller's transaction, which must already hold the branch's lock (the v3
    /// command seam's `BranchTx`, QU-7 #1266).
    pub async fn skip_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        ticket_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        self.authorize_in(tx, actor, branch, now).await?;
        let ticket = fetch_ticket_for_update(tx, branch, ticket_id).await?;
        if ticket.status != TicketStatus::Called {
            return Err(QueueError::StaleVersion.into());
        }
        check_ticket_owner(actor, &ticket)?;

        let rows = sqlx::query(
            "UPDATE queue_tickets \
             SET status = 'NO_SHOW' \
             WHERE id = $1::uuid AND status = 'CALLED'",
        )
        .bind(&ticket.id)
        .execute(&mut **tx)
        .await?;

        if rows.rows_affected() == 0 {
            return Err(QueueError::StaleVersion.into());
        }

        let event = QueueEvent {
            id: uuid::Uuid::new_v4().to_string(),
            organization_id: ticket.organization_id.clone(),
            ticket_id: ticket.id.clone(),
            status: TicketStatus::NoShow,
            kind: EventKind::Skip,
            actor_id: Some(actor.user_id.clone()),
            at: now,
            reason: None,
        };
        self.repo.insert_event_in(tx, &event).await?;
        self.status.refresh_in(tx, branch, now).await?;

        Ok(QueueTicket {
            status: TicketStatus::NoShow,
            ..ticket
        })
    }

    /// CALLED → SERVING.
    pub async fn start(
        &self,
        actor: &Actor,
        branch: &BranchId,
        ticket_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        let desk = self.clone();
        let b = branch.clone();
        let act = actor.clone();
        let tid = ticket_id.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { desk.start_in(tx, &act, &b, &tid, now).await })
            })
            .await
    }

    /// `start` inside the caller's transaction, which must already hold the branch's lock (the v3
    /// command seam's `BranchTx`, QU-7 #1266).
    pub async fn start_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        ticket_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        self.authorize_in(tx, actor, branch, now).await?;
        let ticket = fetch_ticket_for_update(tx, branch, ticket_id).await?;
        if ticket.status != TicketStatus::Called {
            return Err(QueueError::StaleVersion.into());
        }
        check_ticket_owner(actor, &ticket)?;

        let rows = sqlx::query(
            "UPDATE queue_tickets \
             SET status = 'SERVING', served_at = $2 \
             WHERE id = $1::uuid AND status = 'CALLED'",
        )
        .bind(&ticket.id)
        .bind(now)
        .execute(&mut **tx)
        .await?;

        if rows.rows_affected() == 0 {
            return Err(QueueError::StaleVersion.into());
        }

        let event = QueueEvent {
            id: uuid::Uuid::new_v4().to_string(),
            organization_id: ticket.organization_id.clone(),
            ticket_id: ticket.id.clone(),
            status: TicketStatus::Serving,
            kind: EventKind::Start,
            actor_id: Some(actor.user_id.clone()),
            at: now,
            reason: None,
        };
        self.repo.insert_event_in(tx, &event).await?;
        self.status.refresh_in(tx, branch, now).await?;

        Ok(QueueTicket {
            status: TicketStatus::Serving,
            served_at: Some(now),
            ..ticket
        })
    }

    /// SERVING → COMPLETED.
    pub async fn finish(
        &self,
        actor: &Actor,
        branch: &BranchId,
        ticket_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        let desk = self.clone();
        let b = branch.clone();
        let act = actor.clone();
        let tid = ticket_id.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { desk.finish_in(tx, &act, &b, &tid, now).await })
            })
            .await
    }

    /// `finish` inside the caller's transaction, which must already hold the branch's lock (the v3
    /// command seam's `BranchTx`, QU-7 #1266).
    pub async fn finish_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        ticket_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        self.authorize_in(tx, actor, branch, now).await?;
        let ticket = fetch_ticket_for_update(tx, branch, ticket_id).await?;
        if ticket.status != TicketStatus::Serving {
            return Err(QueueError::StaleVersion.into());
        }
        check_ticket_owner(actor, &ticket)?;

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
            return Err(QueueError::StaleVersion.into());
        }

        let event = QueueEvent {
            id: uuid::Uuid::new_v4().to_string(),
            organization_id: ticket.organization_id.clone(),
            ticket_id: ticket.id.clone(),
            status: TicketStatus::Completed,
            kind: EventKind::Finish,
            actor_id: Some(actor.user_id.clone()),
            at: now,
            reason: None,
        };
        self.repo.insert_event_in(tx, &event).await?;
        self.status.refresh_in(tx, branch, now).await?;

        Ok(QueueTicket {
            status: TicketStatus::Completed,
            completed_at: Some(now),
            ..ticket
        })
    }

    /// WAITING | CALLED → CANCELLED with `reason`.
    pub async fn cancel(
        &self,
        actor: &Actor,
        branch: &BranchId,
        ticket_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        let desk = self.clone();
        let b = branch.clone();
        let act = actor.clone();
        let tid = ticket_id.to_owned();
        let r = reason.to_owned();
        self.branch_tx
            .run(branch, move |tx| {
                Box::pin(async move { desk.cancel_in(tx, &act, &b, &tid, &r, now).await })
            })
            .await
    }

    /// `cancel` inside the caller's transaction, which must already hold the branch's lock (the v3
    /// command seam's `BranchTx`, QU-7 #1266).
    pub async fn cancel_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        branch: &BranchId,
        ticket_id: &str,
        reason: &str,
        now: DateTime<Utc>,
    ) -> AppResult<QueueTicket> {
        self.authorize_in(tx, actor, branch, now).await?;
        let ticket = fetch_ticket_for_update(tx, branch, ticket_id).await?;
        if ticket.status != TicketStatus::Waiting && ticket.status != TicketStatus::Called {
            return Err(QueueError::StaleVersion.into());
        }

        let rows = sqlx::query(
            "UPDATE queue_tickets \
             SET status = 'CANCELLED', ended_reason = $2 \
             WHERE id = $1::uuid AND status IN ('WAITING', 'CALLED')",
        )
        .bind(&ticket.id)
        .bind(reason)
        .execute(&mut **tx)
        .await?;

        if rows.rows_affected() == 0 {
            return Err(QueueError::StaleVersion.into());
        }

        let event = QueueEvent {
            id: uuid::Uuid::new_v4().to_string(),
            organization_id: ticket.organization_id.clone(),
            ticket_id: ticket.id.clone(),
            status: TicketStatus::Cancelled,
            kind: EventKind::Cancel,
            actor_id: Some(actor.user_id.clone()),
            at: now,
            reason: Some(reason.to_owned()),
        };
        self.repo.insert_event_in(tx, &event).await?;
        self.status.refresh_in(tx, branch, now).await?;

        Ok(QueueTicket {
            status: TicketStatus::Cancelled,
            ended_reason: Some(reason.to_owned()),
            ..ticket
        })
    }
}
