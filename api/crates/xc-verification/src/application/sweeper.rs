//! Verification sweeper for case expiry and document retention purge (design §5, V4, V13; ticket #1394).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;
use xc_kernel::document_store::{DocumentStore, DocumentStoreError};

use crate::application::review_service::ReviewService;
use crate::domain::case::case_error::CaseError;
use crate::domain::case::case_machine::CaseMachine;
use crate::domain::case_repository::CaseRepository;
use crate::infrastructure::pg_case_repository::PgCaseRepository;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepReport {
    pub cases_expired: u64,
    pub objects_purged: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum SweeperError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Case error: {0}")]
    Case(#[from] CaseError),
    #[error("Document store error: {0}")]
    DocumentStore(#[from] DocumentStoreError),
}

pub struct VerificationSweeper {
    pool: PgPool,
    cases: Arc<dyn CaseRepository>,
    store: Arc<dyn DocumentStore>,
}

impl VerificationSweeper {
    pub fn new(pool: PgPool, store: Arc<dyn DocumentStore>) -> Self {
        let cases = Arc::new(PgCaseRepository::new(pool.clone()));
        Self { pool, cases, store }
    }

    pub fn from_pool(pool: PgPool, store: Arc<dyn DocumentStore>) -> Self {
        Self::new(pool, store)
    }

    pub fn with_repository(
        pool: PgPool,
        cases: Arc<dyn CaseRepository>,
        store: Arc<dyn DocumentStore>,
    ) -> Self {
        Self { pool, cases, store }
    }

    pub async fn expire(&self, now: DateTime<Utc>) -> Result<u64, SweeperError> {
        let mut tx = self.pool.begin().await?;

        let candidate_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM verification_cases \
             WHERE status = 'VERIFIED' AND expires_at <= $1",
        )
        .bind(now)
        .fetch_all(&mut *tx)
        .await?;

        let mut count = 0u64;

        for case_id in candidate_ids {
            let Some(case) = self
                .cases
                .find_case_by_id_for_update(&mut tx, case_id)
                .await?
            else {
                continue;
            };

            let updated = match CaseMachine::expire(&case, now) {
                Ok(u) => u,
                Err(_) => continue,
            };

            let rows_affected = sqlx::query(
                "UPDATE verification_cases \
                 SET status = $2, version = $3, updated_at = $4 \
                 WHERE id = $1 AND version = $5",
            )
            .bind(updated.id)
            .bind(updated.status.as_str())
            .bind(updated.version)
            .bind(now)
            .bind(case.version)
            .execute(&mut *tx)
            .await?
            .rows_affected();

            if rows_affected > 0 {
                ReviewService::set_case_retention(
                    &mut tx,
                    updated.id,
                    updated.organization_id,
                    now,
                )
                .await?;
                count += 1;
            }
        }

        tx.commit().await?;

        Ok(count)
    }

    pub async fn purge(&self, now: DateTime<Utc>) -> Result<u64, SweeperError> {
        let docs: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, encrypted_object_ref FROM verification_documents \
             WHERE retention_until IS NOT NULL \
               AND retention_until <= $1 \
               AND encrypted_object_ref IS NOT NULL \
               AND purged_at IS NULL",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;

        let mut purged_count = 0u64;

        for (doc_id, doc_ref) in docs {
            match self.store.delete(&doc_ref).await {
                Ok(()) => {
                    match sqlx::query(
                        "UPDATE verification_documents \
                         SET encrypted_object_ref = NULL, purged_at = $2 \
                         WHERE id = $1",
                    )
                    .bind(doc_id)
                    .bind(now)
                    .execute(&self.pool)
                    .await
                    {
                        Ok(_) => {
                            purged_count += 1;
                        }
                        Err(error) => {
                            tracing::error!(
                                document_id = %doc_id,
                                error = %error,
                                "failed to record document purged_at"
                            );
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(
                        document_id = %doc_id,
                        error = %error,
                        "failed to delete document from store"
                    );
                }
            }
        }

        Ok(purged_count)
    }

    pub async fn run_round(&self, now: DateTime<Utc>) -> SweepReport {
        let cases_expired = match self.expire(now).await {
            Ok(count) => {
                tracing::info!(cases_expired = count, "expired verification cases");
                count
            }
            Err(error) => {
                tracing::error!(
                    error = %error,
                    "failed to expire verification cases in sweeper round"
                );
                0
            }
        };

        let objects_purged = match self.purge(now).await {
            Ok(count) => {
                tracing::info!(objects_purged = count, "purged verification documents");
                count
            }
            Err(error) => {
                tracing::error!(
                    error = %error,
                    "failed to purge verification documents in sweeper round"
                );
                0
            }
        };

        tracing::info!(
            cases_expired = cases_expired,
            objects_purged = objects_purged,
            "verification sweeper round completed"
        );

        SweepReport {
            cases_expired,
            objects_purged,
        }
    }
}
