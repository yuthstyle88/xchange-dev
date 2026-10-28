//! Shift-close notification delivery (#5096).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use pos_error::{AppError, AppResult};
use sqlx::{AssertSqlSafe, Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::notification_params::{DisplayValue, NotificationParams};
use xc_kernel::notifications::{NotificationObject, NotificationRequest, Notifications};

use crate::domain::count::CashCount;
use crate::domain::repository::CountTotal;

static SAVEPOINT_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct CloseNotices {
    notifications: Arc<dyn Notifications>,
}

impl CloseNotices {
    pub fn new(notifications: Arc<dyn Notifications>) -> Self {
        Self { notifications }
    }

    pub async fn notify_waiting_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        count: &CashCount,
        totals: &[CountTotal],
        actor_id: &str,
    ) -> AppResult<()> {
        let event_id = format!("close-count-waiting:{}", count.id);
        self.send_in(
            tx,
            count,
            totals,
            actor_id,
            "close_count_waiting",
            &event_id,
        )
        .await
    }

    pub async fn notify_certified_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        count: &CashCount,
        totals: &[CountTotal],
        actor_id: &str,
    ) -> AppResult<()> {
        let event_id = format!("shift-close-certified:{}", count.id);
        self.send_in(
            tx,
            count,
            totals,
            actor_id,
            "shift_close_certified",
            &event_id,
        )
        .await
    }

    pub async fn notify_rejected_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        count: &CashCount,
        reason: &str,
    ) -> AppResult<()> {
        let bkk = count.counted_at.with_timezone(&xc_kernel::time::bangkok());
        let params = NotificationParams::new()
            .text("kind", count.kind.as_str())
            .display(
                "branch",
                DisplayValue::server_id(count.branch_id.as_str())
                    .map_err(|e| AppError::Internal(e.to_string()))?,
            )
            .display("counted_on", DisplayValue::date(bkk.date_naive()))
            .text("counted_time", bkk.format("%H:%M").to_string())
            .text("reason", xc_kernel::pii::redact_text(reason));
        let request = NotificationRequest::with_params(
            count.counted_by.clone(),
            format!("count-rejected:{}", count.id),
            "count_rejected",
            Some(NotificationObject {
                object_type: "cash_count".to_string(),
                object_id: count.id.clone(),
            }),
            params,
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
        self.send_notice_in(tx, &request).await
    }

    async fn send_notice_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        request: &NotificationRequest,
    ) -> AppResult<()> {
        let sp_id = SAVEPOINT_COUNTER.fetch_add(1, Ordering::Relaxed);
        let sp_name = format!("sp_count_notice_{sp_id}");
        sqlx::query(AssertSqlSafe(format!("SAVEPOINT {sp_name}")))
            .execute(&mut **tx)
            .await?;

        match self.notifications.notify_in(tx, request).await {
            Ok(_) => {
                sqlx::query(AssertSqlSafe(format!("RELEASE SAVEPOINT {sp_name}")))
                    .execute(&mut **tx)
                    .await?;
            }
            Err(_) => {
                sqlx::query(AssertSqlSafe(format!("ROLLBACK TO SAVEPOINT {sp_name}")))
                    .execute(&mut **tx)
                    .await?;
            }
        }
        Ok(())
    }

    async fn send_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        count: &CashCount,
        totals: &[CountTotal],
        actor_id: &str,
        template_key: &str,
        event_id: &str,
    ) -> AppResult<()> {
        let org_id = Uuid::parse_str(&count.organization_id).map_err(|e| {
            AppError::Validation(format!(
                "invalid organization_id {}: {e}",
                count.organization_id
            ))
        })?;
        let branch_str = count.branch_id.as_str();

        let recipients: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT u.id \
             FROM users u \
             JOIN user_roles r ON r.user_id = u.id AND r.organization_id = u.organization_id \
             WHERE u.organization_id = $1 \
               AND u.status = 'ACTIVE' \
               AND ( \
                   (r.role_code = 'EXECUTIVE' AND r.branch_id IS NULL) \
                   OR (r.role_code = 'BRANCH_MANAGER' AND r.branch_id = $2) \
               ) \
               AND (r.valid_from IS NULL OR r.valid_from <= now()) \
               AND (r.valid_to IS NULL OR r.valid_to > now()) \
               AND u.id::text <> $3 \
             ORDER BY u.id",
        )
        .bind(org_id)
        .bind(branch_str)
        .bind(actor_id)
        .fetch_all(&mut **tx)
        .await?;

        if recipients.is_empty() {
            return Ok(());
        }

        let by_name: Option<String> =
            sqlx::query_scalar("SELECT name FROM users WHERE id::text = $1")
                .bind(actor_id)
                .fetch_optional(&mut **tx)
                .await?;
        let raw_by = by_name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| actor_id.to_string());
        let safe_by = xc_kernel::pii::redact_text(&raw_by);

        let mut params = NotificationParams::new()
            .display(
                "branch",
                DisplayValue::server_id(branch_str)
                    .map_err(|e| AppError::Internal(e.to_string()))?,
            )
            .text("by", &safe_by);
        if let Some(ref s_id) = count.shift_id {
            params = params.display(
                "shift",
                DisplayValue::server_id(s_id.as_str())
                    .map_err(|e| AppError::Internal(e.to_string()))?,
            );
        }
        if let Some(thb) = totals
            .iter()
            .find(|t| t.currency.trim().eq_ignore_ascii_case("THB"))
        {
            params = params.display("counted_thb", DisplayValue::amount(thb.counted));
            if let Some(exp) = thb.expected {
                params = params.display("expected_thb", DisplayValue::amount(exp));
            }
        }

        for recipient_id in &recipients {
            let request = match NotificationRequest::with_params(
                recipient_id.to_string(),
                event_id.to_string(),
                template_key,
                Some(NotificationObject {
                    object_type: "cash_count".to_string(),
                    object_id: count.id.clone(),
                }),
                params.clone(),
            ) {
                Ok(req) => req,
                Err(e) => {
                    tracing::warn!(
                        template = %template_key,
                        recipient = %recipient_id,
                        error = %e,
                        "failed to build shift close notification request: {e}"
                    );
                    continue;
                }
            };

            self.send_notice_in(tx, &request).await?;
        }

        Ok(())
    }
}
