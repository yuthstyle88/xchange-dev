//! `MembershipSyncWorker` (design D-8, §4.5, LY-4 #1402): one tick reads the cursor, fetches one page,
//! applies it and advances the cursor in one transaction.

use std::sync::Arc;

use pos_error::{AppError, AppResult};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::membership_feed::MembershipFeed;
use crate::domain::membership_mirror::MembershipMirror;

/// What one tick did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickOutcome {
    /// `{changed:false}`: nothing written.
    Unchanged,
    /// A page of `items` items was applied and the cursor moved to its `next`.
    Applied { items: usize },
}

pub struct MembershipSyncWorker {
    pool: PgPool,
    org: Uuid,
    feed: Arc<dyn MembershipFeed>,
    mirror: Arc<dyn MembershipMirror>,
}

impl MembershipSyncWorker {
    pub fn new(
        pool: PgPool,
        org: Uuid,
        feed: Arc<dyn MembershipFeed>,
        mirror: Arc<dyn MembershipMirror>,
    ) -> Self {
        Self {
            pool,
            org,
            feed,
            mirror,
        }
    }

    pub async fn tick(&self) -> AppResult<TickOutcome> {
        let current_cursor: i64 =
            sqlx::query_scalar("SELECT cursor FROM loyalty_sync_cursor WHERE organization_id = $1")
                .bind(self.org)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?
                .unwrap_or(0);

        let page = match self.feed.changes_after(current_cursor).await {
            Ok(p) => p,
            Err(err) => {
                let err_str = err.to_string();
                let redacted: String = err_str.chars().take(500).collect();

                sqlx::query(
                    "INSERT INTO loyalty_sync_cursor (organization_id, cursor, last_ok_at, last_error) \
                     VALUES ($1, 0, NULL, $2) \
                     ON CONFLICT (organization_id) \
                     DO UPDATE SET last_error = EXCLUDED.last_error",
                )
                .bind(self.org)
                .bind(&redacted)
                .execute(&self.pool)
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?;

                return Err(AppError::Internal(redacted));
            }
        };

        if !page.changed {
            return Ok(TickOutcome::Unchanged);
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;

        self.mirror.apply_in(&mut tx, self.org, &page).await?;

        sqlx::query(
            "INSERT INTO loyalty_sync_cursor (organization_id, cursor, last_ok_at, last_error) \
             VALUES ($1, $2, now(), NULL) \
             ON CONFLICT (organization_id) \
             DO UPDATE SET \
                 cursor = EXCLUDED.cursor, \
                 last_ok_at = EXCLUDED.last_ok_at, \
                 last_error = NULL",
        )
        .bind(self.org)
        .bind(page.next)
        .execute(&mut *tx)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;

        Ok(TickOutcome::Applied {
            items: page.items.len(),
        })
    }
}
