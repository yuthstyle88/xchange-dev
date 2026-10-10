//! `PgQueueRepository` (design §5): `QueueRepository` on PostgreSQL. Ids are `uuid` columns read
//! and written as text (`$1::uuid`, `id::text`).

use chrono::NaiveDate;
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use sqlx::{PgPool, Postgres, Row, Transaction};
use xc_kernel::ids::BranchId;

use crate::domain::repository::{BranchFacts, QueueRepository};
use crate::domain::ticket::queue_event::QueueEvent;
use crate::domain::ticket::queue_ticket::{QueueTicket, TicketStatus};
use crate::domain::ticket::service_type::ServiceType;

#[derive(Clone)]
pub struct PgQueueRepository {
    pool: PgPool,
}

impl PgQueueRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The pool, for reads outside a transaction (QU-A2 views).
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

fn map_ticket_row(row: &sqlx::postgres::PgRow) -> AppResult<QueueTicket> {
    let service_type_str: String = row.get("service_type");
    let service_type = ServiceType::parse(&service_type_str)
        .ok_or_else(|| AppError::Internal(format!("invalid service_type: {service_type_str}")))?;

    let status_str: String = row.get("status");
    let status = TicketStatus::parse(&status_str)
        .ok_or_else(|| AppError::Internal(format!("invalid status: {status_str}")))?;

    let branch_str: String = row.get("branch_id");

    Ok(QueueTicket {
        id: row.get("id"),
        organization_id: row.get("organization_id"),
        branch_id: BranchId::new(branch_str),
        service_date: row.get("service_date"),
        service_type,
        sequence: row.get("sequence"),
        display_number: row.get("display_number"),
        status,
        customer_id: row.get("customer_id"),
        booking_id: row.get("booking_id"),
        kiosk_request_id: row.get("kiosk_request_id"),
        counter_teller_id: row.get("counter_teller_id"),
        issued_at: row.get("issued_at"),
        called_at: row.get("called_at"),
        served_at: row.get("served_at"),
        completed_at: row.get("completed_at"),
        ended_reason: row.get("ended_reason"),
        idempotency_key: row.get("idempotency_key"),
    })
}

const SELECT_BY_KEY_SQL: &str = "\
    SELECT \
        id::text AS id, \
        organization_id::text AS organization_id, \
        branch_id, \
        service_date, \
        service_type, \
        sequence, \
        display_number, \
        status, \
        customer_id, \
        booking_id, \
        kiosk_request_id, \
        counter_teller_id::text AS counter_teller_id, \
        issued_at, \
        called_at, \
        served_at, \
        completed_at, \
        ended_reason, \
        idempotency_key \
    FROM queue_tickets \
    WHERE branch_id = $1 AND idempotency_key = $2";

const SELECT_BY_KIOSK_REQUEST_SQL: &str = "\
    SELECT \
        id::text AS id, \
        organization_id::text AS organization_id, \
        branch_id, \
        service_date, \
        service_type, \
        sequence, \
        display_number, \
        status, \
        customer_id, \
        booking_id, \
        kiosk_request_id, \
        counter_teller_id::text AS counter_teller_id, \
        issued_at, \
        called_at, \
        served_at, \
        completed_at, \
        ended_reason, \
        idempotency_key \
    FROM queue_tickets \
    WHERE branch_id = $1 AND kiosk_request_id = $2";

const SELECT_ACTIVE_BY_BOOKING_SQL: &str = "\
    SELECT \
        id::text AS id, \
        organization_id::text AS organization_id, \
        branch_id, \
        service_date, \
        service_type, \
        sequence, \
        display_number, \
        status, \
        customer_id, \
        booking_id, \
        kiosk_request_id, \
        counter_teller_id::text AS counter_teller_id, \
        issued_at, \
        called_at, \
        served_at, \
        completed_at, \
        ended_reason, \
        idempotency_key \
    FROM queue_tickets \
    WHERE branch_id = $1 AND booking_id = $2 AND status IN ('WAITING', 'CALLED', 'SERVING')";

impl QueueRepository for PgQueueRepository {
    fn branch_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<BranchFacts>>> {
        Box::pin(async move {
            let row: Option<(String, String)> = sqlx::query_as(
                "SELECT organization_id::text, timezone FROM branches WHERE id = $1",
            )
            .bind(branch.as_str())
            .fetch_optional(&mut **tx)
            .await?;

            Ok(row.map(|(organization_id, timezone)| BranchFacts {
                organization_id,
                timezone,
            }))
        })
    }

    fn queue_open_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            let row: Option<(bool,)> =
                sqlx::query_as("SELECT queue_open FROM branch_service_status WHERE branch_id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(&mut **tx)
                    .await?;

            Ok(row.map(|(open,)| open).unwrap_or(false))
        })
    }

    fn next_sequence_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        organization_id: &'a str,
        branch: &'a BranchId,
        service_date: NaiveDate,
        service_type: ServiceType,
    ) -> BoxFuture<'a, AppResult<i32>> {
        let org_id = organization_id.to_owned();
        Box::pin(async move {
            let (seq,): (i32,) = sqlx::query_as(
                r#"
                INSERT INTO queue_sequences (organization_id, branch_id, service_date, service_type, last_sequence)
                VALUES ($1::uuid, $2, $3, $4, 1)
                ON CONFLICT (branch_id, service_date, service_type)
                DO UPDATE SET last_sequence = queue_sequences.last_sequence + 1, updated_at = now()
                RETURNING last_sequence
                "#,
            )
            .bind(&org_id)
            .bind(branch.as_str())
            .bind(service_date)
            .bind(service_type.as_str())
            .fetch_one(&mut **tx)
            .await?;

            Ok(seq)
        })
    }

    fn find_by_key_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        idempotency_key: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<QueueTicket>>> {
        Box::pin(async move {
            let row = sqlx::query(SELECT_BY_KEY_SQL)
                .bind(branch.as_str())
                .bind(idempotency_key)
                .fetch_optional(&mut **tx)
                .await?;

            row.as_ref().map(map_ticket_row).transpose()
        })
    }

    fn find_by_kiosk_request_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        request_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<QueueTicket>>> {
        Box::pin(async move {
            let row = sqlx::query(SELECT_BY_KIOSK_REQUEST_SQL)
                .bind(branch.as_str())
                .bind(request_id)
                .fetch_optional(&mut **tx)
                .await?;

            row.as_ref().map(map_ticket_row).transpose()
        })
    }

    fn find_active_by_booking_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
        booking_id: &'a str,
    ) -> BoxFuture<'a, AppResult<Option<QueueTicket>>> {
        Box::pin(async move {
            let row = sqlx::query(SELECT_ACTIVE_BY_BOOKING_SQL)
                .bind(branch.as_str())
                .bind(booking_id)
                .fetch_optional(&mut **tx)
                .await?;

            row.as_ref().map(map_ticket_row).transpose()
        })
    }

    fn insert_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        ticket: &'a QueueTicket,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO queue_tickets (
                    id, organization_id, branch_id, service_date, service_type,
                    sequence, display_number, status, customer_id, booking_id,
                    kiosk_request_id, counter_teller_id, issued_at, called_at,
                    served_at, completed_at, ended_reason, idempotency_key
                ) VALUES (
                    $1::uuid, $2::uuid, $3, $4, $5,
                    $6, $7, $8, $9, $10,
                    $11, $12::uuid, $13, $14,
                    $15, $16, $17, $18
                )
                "#,
            )
            .bind(&ticket.id)
            .bind(&ticket.organization_id)
            .bind(ticket.branch_id.as_str())
            .bind(ticket.service_date)
            .bind(ticket.service_type.as_str())
            .bind(ticket.sequence)
            .bind(&ticket.display_number)
            .bind(ticket.status.as_str())
            .bind(ticket.customer_id.as_deref())
            .bind(ticket.booking_id.as_deref())
            .bind(ticket.kiosk_request_id.as_deref())
            .bind(ticket.counter_teller_id.as_deref())
            .bind(ticket.issued_at)
            .bind(ticket.called_at)
            .bind(ticket.served_at)
            .bind(ticket.completed_at)
            .bind(ticket.ended_reason.as_deref())
            .bind(&ticket.idempotency_key)
            .execute(&mut **tx)
            .await?;

            Ok(())
        })
    }

    fn insert_event_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        event: &'a QueueEvent,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO queue_events (
                    id, organization_id, ticket_id, status, kind, actor_id, at, reason
                ) VALUES (
                    $1::uuid, $2::uuid, $3::uuid, $4, $5, $6::uuid, $7, $8
                )
                "#,
            )
            .bind(&event.id)
            .bind(&event.organization_id)
            .bind(&event.ticket_id)
            .bind(event.status.as_str())
            .bind(event.kind.as_str())
            .bind(event.actor_id.as_deref())
            .bind(event.at)
            .bind(event.reason.as_deref())
            .execute(&mut **tx)
            .await?;

            Ok(())
        })
    }

    fn lock_next_waiting_in<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, AppResult<Option<QueueTicket>>> {
        Box::pin(async move {
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
                WHERE branch_id = $1 AND status = 'WAITING'
                ORDER BY
                    CASE service_type
                        WHEN 'BOOKING' THEN 0
                        WHEN 'WALK_IN' THEN 1
                        ELSE 2
                    END,
                    sequence ASC
                LIMIT 1
                FOR UPDATE SKIP LOCKED
                "#,
            )
            .bind(branch.as_str())
            .fetch_optional(&mut **tx)
            .await?;

            row.as_ref().map(map_ticket_row).transpose()
        })
    }
}
