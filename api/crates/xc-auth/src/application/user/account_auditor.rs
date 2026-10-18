//! `AccountAuditor` (design §3.4): one `user_audit` row per account change, in the change's transaction.

use pos_error::AppResult;
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
        let _ = (
            repo,
            tx,
            actor_id,
            subject_id,
            action,
            organization_id,
            before,
            after,
            ip,
        );
        todo!("RBAC function ticket: AccountAuditor::record (§3.4)")
    }
}
