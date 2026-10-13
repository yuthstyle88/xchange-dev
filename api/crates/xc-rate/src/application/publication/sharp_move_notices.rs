//! Sharp-move notification delivery (SM-2 #1584, rates.md §12.8, §12.10).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sqlx::{AssertSqlSafe, Postgres, Transaction};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_kernel::notifications::{NotificationObject, NotificationRequest, Notifications};

use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::publication::sharp_move::Move;

static SAVEPOINT_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct SharpMoveNotices {
    notifications: Arc<dyn Notifications>,
}

impl SharpMoveNotices {
    pub fn new(notifications: Arc<dyn Notifications>) -> Self {
        Self { notifications }
    }

    pub async fn notify_in(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        branch: &BranchId,
        moves: &[Move],
    ) -> Result<(), RateEngineError> {
        if moves.is_empty() {
            return Ok(());
        }

        let branch_str = branch.as_str();

        let branch_row: Option<(Uuid, Option<Uuid>, Option<i32>)> = sqlx::query_as(
            "SELECT b.organization_id, b.current_publication_id, p.version \
             FROM branches b \
             LEFT JOIN rate_publications p ON p.id = b.current_publication_id \
             WHERE b.id = $1",
        )
        .bind(branch_str)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        let (org_id, current_pub_id, pub_version) = branch_row.ok_or_else(|| {
            RateEngineError::ValidationFailed(format!("branch not found: {branch_str}"))
        })?;

        let pub_id = current_pub_id.ok_or_else(|| {
            RateEngineError::Internal(format!("missing publication for branch: {branch_str}"))
        })?;
        let _ = pub_version.ok_or_else(|| {
            RateEngineError::Internal(format!(
                "missing publication version for branch: {branch_str}"
            ))
        })?;

        let recipients: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT u.id \
             FROM users u \
             JOIN user_roles r ON r.user_id = u.id AND r.organization_id = u.organization_id \
             WHERE u.organization_id = $1 \
               AND u.status = 'ACTIVE' \
               AND r.role_code IN ('EXECUTIVE', 'ADMIN') \
               AND r.branch_id IS NULL \
               AND (r.valid_from IS NULL OR r.valid_from <= now()) \
               AND (r.valid_to IS NULL OR r.valid_to > now()) \
             ORDER BY u.id",
        )
        .bind(org_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| RateEngineError::Database(e.to_string()))?;

        if recipients.is_empty() {
            return Ok(());
        }

        for (rec_idx, recipient_id) in recipients.iter().enumerate() {
            for (move_idx, m) in moves.iter().enumerate() {
                let side_str = match m.side {
                    Side::Buy => "BUY",
                    Side::Sell => "SELL",
                };
                let event_id = format!(
                    "rate-sharp-move:{}:{}:{}",
                    pub_id,
                    m.currency.as_str(),
                    side_str
                );
                let mut params = BTreeMap::new();
                params.insert("branch".to_string(), branch_str.to_string());
                params.insert("currency".to_string(), m.currency.as_str().to_string());
                params.insert("side".to_string(), side_str.to_string());

                let request = match NotificationRequest::new(
                    recipient_id.to_string(),
                    event_id,
                    "rate_sharp_move",
                    Some(NotificationObject {
                        object_type: "rate_publication".to_string(),
                        object_id: pub_id.to_string(),
                    }),
                    params,
                ) {
                    Ok(req) => req,
                    Err(e) => {
                        tracing::warn!(
                            recipient_id = %recipient_id,
                            currency = %m.currency.as_str(),
                            side = %side_str,
                            "failed to build sharp move notification request: {e}"
                        );
                        continue;
                    }
                };

                let sp_id = SAVEPOINT_COUNTER.fetch_add(1, Ordering::Relaxed);
                let sp_name = format!("sp_sm_notice_{sp_id}_{rec_idx}_{move_idx}");
                sqlx::query(AssertSqlSafe(format!("SAVEPOINT {sp_name}")))
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| RateEngineError::Database(e.to_string()))?;

                let notify_result = self.notifications.notify_in(tx, &request).await;

                match notify_result {
                    Ok(_) => {
                        sqlx::query(AssertSqlSafe(format!("RELEASE SAVEPOINT {sp_name}")))
                            .execute(&mut **tx)
                            .await
                            .map_err(|e| RateEngineError::Database(e.to_string()))?;
                    }
                    Err(e) => {
                        tracing::warn!(
                            recipient_id = %recipient_id,
                            currency = %m.currency.as_str(),
                            side = %side_str,
                            "failed to deliver sharp move notification: {e}"
                        );
                        sqlx::query(AssertSqlSafe(format!("ROLLBACK TO SAVEPOINT {sp_name}")))
                            .execute(&mut **tx)
                            .await
                            .map_err(|e| RateEngineError::Database(e.to_string()))?;
                    }
                }
            }
        }

        Ok(())
    }
}
