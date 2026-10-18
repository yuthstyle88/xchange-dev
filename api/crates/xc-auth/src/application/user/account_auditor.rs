//! `AccountAuditor` (design §3.4): one `user_audit` row per account change, in the change's transaction.

use pos_error::{AppError, AppResult};
use sqlx::{Postgres, Transaction};

use crate::domain::user::UserRepository;

pub struct AccountAuditor;

impl AccountAuditor {
    /// Action names: `userGrant`, `userRevoke`, `userDisable`, `userSuspend`, `userResume`, `userEnable`.
    #[allow(clippy::too_many_arguments)]
    pub async fn record<R: UserRepository>(
        repo: &R,
        tx: &mut Transaction<'_, Postgres>,
        actor_id: &str,
        subject_id: &str,
        action: &str,
        organization_id: &str,
        before: Option<serde_json::Value>,
        after: Option<serde_json::Value>,
        ip: Option<&str>,
    ) -> AppResult<()> {
        repo.append_audit(tx, actor_id, subject_id, action, organization_id)
            .await?;

        if before.is_some() || after.is_some() || ip.is_some() {
            let before_json = before.as_ref().map(|v| v.to_string());
            let after_json = after.as_ref().map(|v| v.to_string());

            sqlx::query(
                r#"
                UPDATE user_audit
                SET before = $1::jsonb,
                    after = $2::jsonb,
                    ip = $3::inet
                WHERE id = (
                    SELECT id FROM user_audit
                    WHERE actor_id = $4
                      AND subject_id = $5
                      AND action = $6
                      AND organization_id = $7::uuid
                    ORDER BY at DESC, ctid DESC
                    LIMIT 1
                )
                "#,
            )
            .bind(before_json)
            .bind(after_json)
            .bind(ip)
            .bind(actor_id)
            .bind(subject_id)
            .bind(action)
            .bind(organization_id)
            .execute(&mut **tx)
            .await
            .map_err(AppError::Database)?;
        }

        Ok(())
    }
}
