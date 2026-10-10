//! PostgreSQL implementation of `BranchPriceSource` (§3.5, R-7).

use std::sync::Arc;

use futures::future::BoxFuture;
use sqlx::{PgPool, Postgres, Transaction};
use xc_approval::domain::audit_log::{AuditEvent, AuditLog};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;
use xc_kernel::ids::BranchId;

use crate::application::publication::publish_service::PublishService;
use crate::domain::branch::price_source::{BranchPriceSource, PriceSource};
use crate::domain::draft::rate_draft::DraftId;
use crate::domain::draft::rate_engine_error::RateEngineError;
use crate::domain::publication::published_rates::PublicationRef;

/// PostgreSQL repository and coordinator for branch price source.
pub struct PgBranchPriceSource {
    pool: PgPool,
    audit: Arc<dyn AuditLog>,
}

impl PgBranchPriceSource {
    pub fn new(pool: PgPool, audit: Arc<dyn AuditLog>) -> Self {
        Self { pool, audit }
    }
}

impl BranchPriceSource for PgBranchPriceSource {
    fn source_of<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        branch: &'a BranchId,
    ) -> BoxFuture<'a, Result<PriceSource, RateEngineError>> {
        Box::pin(async move {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT price_source FROM branches WHERE id = $1")
                    .bind(branch.as_str())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let (source_str,) = row.ok_or_else(|| {
                RateEngineError::NotFound(format!("branch not found: {}", branch.as_str()))
            })?;

            source_str.parse()
        })
    }

    fn set<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        who: &'a Actor,
        branch: &'a BranchId,
        to: PriceSource,
    ) -> BoxFuture<'a, Result<(), RateEngineError>> {
        Box::pin(async move {
            if who.role() != Role::Owner {
                return Err(RateEngineError::OutOfScope(
                    "only owner may set branch price source".to_string(),
                ));
            }

            let row: Option<(String,)> =
                sqlx::query_as("SELECT price_source FROM branches WHERE id = $1 FOR UPDATE")
                    .bind(branch.as_str())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let (current_str,) = row.ok_or_else(|| {
                RateEngineError::NotFound(format!("branch not found: {}", branch.as_str()))
            })?;

            let current: PriceSource = current_str.parse()?;

            if current == to {
                return Ok(());
            }

            if to == PriceSource::BotDraft {
                return Err(RateEngineError::ValidationFailed(
                    "a RATE_BOARD -> BOT_DRAFT call to set is refused: that way is switch_back"
                        .to_string(),
                ));
            }

            sqlx::query("UPDATE branches SET price_source = $1 WHERE id = $2")
                .bind(to.as_str())
                .bind(branch.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let event = AuditEvent {
                actor_id: who.id().to_string(),
                actor_role: who.role().as_str().to_string(),
                scope: "branch".to_string(),
                action: "branchPriceSourceSet".to_string(),
                object_ref: branch.as_str().to_string(),
                branch_id: Some(branch.as_str().to_string()),
                before: Some(serde_json::json!({ "priceSource": current.as_str() })),
                after: Some(serde_json::json!({ "priceSource": to.as_str() })),
            };

            self.audit
                .append(tx, &event)
                .await
                .map_err(|e| RateEngineError::Internal(e.to_string()))?;

            Ok(())
        })
    }

    fn switch_back<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        who: &'a Actor,
        branch: &'a BranchId,
        draft: &'a DraftId,
        expected_version: Option<i32>,
    ) -> BoxFuture<'a, Result<PublicationRef, RateEngineError>> {
        Box::pin(async move {
            if who.role() != Role::Owner {
                return Err(RateEngineError::OutOfScope(
                    "only owner may switch branch price source back".to_string(),
                ));
            }

            let row: Option<(String,)> =
                sqlx::query_as("SELECT price_source FROM branches WHERE id = $1 FOR UPDATE")
                    .bind(branch.as_str())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let (current_str,) = row.ok_or_else(|| {
                RateEngineError::NotFound(format!("branch not found: {}", branch.as_str()))
            })?;

            let current: PriceSource = current_str.parse()?;

            if current != PriceSource::RateBoard {
                return Err(RateEngineError::ValidationFailed(format!(
                    "branch {} is not on RATE_BOARD",
                    branch.as_str()
                )));
            }

            sqlx::query("UPDATE branches SET price_source = $1 WHERE id = $2")
                .bind(PriceSource::BotDraft.as_str())
                .bind(branch.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|e| RateEngineError::Database(e.to_string()))?;

            let event = AuditEvent {
                actor_id: who.id().to_string(),
                actor_role: who.role().as_str().to_string(),
                scope: "branch".to_string(),
                action: "branchPriceSourceSet".to_string(),
                object_ref: branch.as_str().to_string(),
                branch_id: Some(branch.as_str().to_string()),
                before: Some(serde_json::json!({ "priceSource": PriceSource::RateBoard.as_str() })),
                after: Some(serde_json::json!({ "priceSource": PriceSource::BotDraft.as_str() })),
            };

            self.audit
                .append(tx, &event)
                .await
                .map_err(|e| RateEngineError::Internal(e.to_string()))?;

            let publish_service = PublishService::new(self.pool.clone());
            let pub_ref = publish_service
                .publish(tx, branch, who, draft, expected_version)
                .await?;

            Ok(pub_ref)
        })
    }
}
