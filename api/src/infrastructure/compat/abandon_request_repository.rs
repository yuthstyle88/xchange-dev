//! `abandoned_requests`: the one write `adminAbandonRequest` makes.

use chrono::{DateTime, Utc};
use pos_error::{AppError, AppResult};
use xc_kernel::ids::BranchId;

use crate::application::compat::abandon_request::AbandonedRequests;

#[derive(Debug, Clone, Copy, Default)]
pub struct PgAbandonedRequests;

impl AbandonedRequests for PgAbandonedRequests {
    async fn insert_if_absent(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        branch: &BranchId,
        request_id: &str,
        at: DateTime<Utc>,
        actor: &str,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO abandoned_requests (branch_id, request_id, at, actor) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (branch_id, request_id) DO NOTHING",
        )
        .bind(branch.as_str())
        .bind(request_id)
        .bind(at)
        .bind(actor)
        .execute(&mut **tx)
        .await
        .map_err(AppError::Database)?;
        Ok(())
    }
}
