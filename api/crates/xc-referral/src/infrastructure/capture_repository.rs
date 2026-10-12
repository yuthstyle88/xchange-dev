//! `PgCaptureRepository`: `CaptureRepository` on PostgreSQL. No pool: every call runs in the
//! caller's transaction.

use futures::future::BoxFuture;
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::application::capture_service::CaptureError;
use crate::domain::capture::{Capture, CaptureId, CaptureSource, CaptureStatus, CaptureTarget};
use crate::domain::capture_repository::{CaptureRepository, TargetInfo};

#[derive(Debug, Clone, Copy, Default)]
pub struct PgCaptureRepository;

impl PgCaptureRepository {
    pub fn new() -> Self {
        Self
    }
}

impl CaptureRepository for PgCaptureRepository {
    fn resolve_target<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        target: &'a CaptureTarget,
    ) -> BoxFuture<'a, Result<TargetInfo, CaptureError>> {
        Box::pin(async move {
            match target {
                CaptureTarget::Transaction(tx_id) => {
                    let row = sqlx::query(
                        "SELECT t.customer_id, t.branch_id, b.organization_id \
                         FROM transactions t \
                         JOIN branches b ON b.id = t.branch_id \
                         WHERE t.id = $1",
                    )
                    .bind(tx_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(CaptureError::from)?;

                    match row {
                        Some(r) => Ok(TargetInfo {
                            organization_id: r.get("organization_id"),
                            branch_id: r.get("branch_id"),
                            customer_id: r.get("customer_id"),
                        }),
                        None => Err(CaptureError::NoSuchTarget),
                    }
                }
                CaptureTarget::Booking(booking_id) => {
                    // A Blueprint booking (BK-1 M1); a v2 booking id is not in `bookings`, so it is `NoSuchTarget`.
                    let row = sqlx::query(
                        "SELECT branch_id, organization_id, customer_id FROM bookings WHERE id = $1",
                    )
                    .bind(booking_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(CaptureError::from)?;

                    match row {
                        Some(r) => Ok(TargetInfo {
                            organization_id: r.get("organization_id"),
                            branch_id: r.get("branch_id"),
                            customer_id: r.get("customer_id"),
                        }),
                        None => Err(CaptureError::NoSuchTarget),
                    }
                }
            }
        })
    }

    fn find_active_for_target<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        target: &'a CaptureTarget,
    ) -> BoxFuture<'a, Result<Option<(CaptureId, String)>, CaptureError>> {
        Box::pin(async move {
            let row = match target {
                CaptureTarget::Transaction(tx_id) => sqlx::query(
                    "SELECT id, code FROM referral_attributions \
                         WHERE transaction_id = $1 AND status = 'ACTIVE'",
                )
                .bind(tx_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(CaptureError::from)?,
                CaptureTarget::Booking(booking_id) => sqlx::query(
                    "SELECT id, code FROM referral_attributions \
                         WHERE booking_id = $1 AND status = 'ACTIVE'",
                )
                .bind(booking_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(CaptureError::from)?,
            };

            Ok(row.map(|r| {
                let id: Uuid = r.get("id");
                let code: String = r.get("code");
                (CaptureId::new(id), code)
            }))
        })
    }

    fn insert<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        capture: &'a Capture,
    ) -> BoxFuture<'a, Result<(), CaptureError>> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO referral_attributions (\
                     id, organization_id, branch_id, code, customer_id, \
                     customer_sub, booking_id, transaction_id, source, \
                     captured_at, captured_by, status, supersedes_id, \
                     reason, pushed_at, broker_outcome\
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
            )
            .bind(capture.id.as_uuid())
            .bind(capture.organization_id)
            .bind(&capture.branch_id)
            .bind(&capture.code)
            .bind(&capture.customer_id)
            .bind(&capture.customer_sub)
            .bind(&capture.booking_id)
            .bind(capture.transaction_id)
            .bind(capture.source.as_str())
            .bind(capture.captured_at)
            .bind(&capture.captured_by)
            .bind(capture.status.as_str())
            .bind(capture.supersedes_id.map(|id| id.as_uuid()))
            .bind(&capture.reason)
            .bind(capture.pushed_at)
            .bind(&capture.broker_outcome)
            .execute(&mut **tx)
            .await
            .map_err(CaptureError::from)?;

            Ok(())
        })
    }

    fn find_by_id<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a CaptureId,
    ) -> BoxFuture<'a, Result<Option<Capture>, CaptureError>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT id, organization_id, branch_id, code, customer_id, \
                        customer_sub, booking_id, transaction_id, source, \
                        captured_at, captured_by, status, supersedes_id, \
                        reason, pushed_at, broker_outcome \
                 FROM referral_attributions WHERE id = $1",
            )
            .bind(id.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(CaptureError::from)?;

            let Some(r) = row else { return Ok(None) };

            let source_str: String = r.get("source");
            let status_str: String = r.get("status");
            let supersedes_uuid: Option<Uuid> = r.get("supersedes_id");

            Ok(Some(Capture {
                id: CaptureId::new(r.get("id")),
                organization_id: r.get("organization_id"),
                branch_id: r.get("branch_id"),
                code: r.get("code"),
                customer_id: r.get("customer_id"),
                customer_sub: r.get("customer_sub"),
                booking_id: r.get("booking_id"),
                transaction_id: r.get("transaction_id"),
                source: CaptureSource::parse(&source_str).unwrap_or(CaptureSource::Desk),
                captured_at: r.get("captured_at"),
                captured_by: r.get("captured_by"),
                status: CaptureStatus::parse(&status_str).unwrap_or(CaptureStatus::Active),
                supersedes_id: supersedes_uuid.map(CaptureId::new),
                reason: r.get("reason"),
                pushed_at: r.get("pushed_at"),
                broker_outcome: r.get("broker_outcome"),
            }))
        })
    }

    fn mark_superseded<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        id: &'a CaptureId,
    ) -> BoxFuture<'a, Result<bool, CaptureError>> {
        Box::pin(async move {
            let res = sqlx::query(
                "UPDATE referral_attributions SET status = 'SUPERSEDED' \
                 WHERE id = $1 AND status = 'ACTIVE'",
            )
            .bind(id.as_uuid())
            .execute(&mut **tx)
            .await
            .map_err(CaptureError::from)?;

            Ok(res.rows_affected() > 0)
        })
    }

    fn mark_pushed_for_customer<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        customer: &'a str,
        sub: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(CaptureId, Uuid)>, CaptureError>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT id, organization_id \
                 FROM referral_attributions \
                 WHERE customer_id = $1 AND status = 'ACTIVE' AND pushed_at IS NULL \
                 ORDER BY captured_at ASC, id ASC \
                 FOR UPDATE",
            )
            .bind(customer)
            .fetch_all(&mut **tx)
            .await
            .map_err(CaptureError::from)?;

            if rows.is_empty() {
                return Ok(Vec::new());
            }

            let ids: Vec<(CaptureId, Uuid)> = rows
                .iter()
                .map(|r| {
                    let id: Uuid = r.get("id");
                    let org_id: Uuid = r.get("organization_id");
                    (CaptureId::new(id), org_id)
                })
                .collect();

            sqlx::query(
                "UPDATE referral_attributions \
                 SET customer_sub = $2, pushed_at = now() \
                 WHERE customer_id = $1 AND status = 'ACTIVE' AND pushed_at IS NULL",
            )
            .bind(customer)
            .bind(sub)
            .execute(&mut **tx)
            .await
            .map_err(CaptureError::from)?;

            Ok(ids)
        })
    }

    fn link_transaction<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        booking_id: &'a str,
        transaction_id: Uuid,
    ) -> BoxFuture<'a, Result<(), CaptureError>> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE referral_attributions \
                 SET transaction_id = $2 \
                 WHERE booking_id = $1 AND status = 'ACTIVE'",
            )
            .bind(booking_id)
            .bind(transaction_id)
            .execute(&mut **tx)
            .await
            .map_err(CaptureError::from)?;

            Ok(())
        })
    }
}
