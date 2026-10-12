//! Approval subject and policy for rate publication.

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::access::{Actor, Role};

pub struct RatePublicationSubject;

impl RatePublicationSubject {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RatePublicationSubject {
    fn default() -> Self {
        Self::new()
    }
}

impl ApprovalSubject for RatePublicationSubject {
    fn type_name(&self) -> &'static str {
        "RATE_PUBLICATION"
    }

    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        Box::pin(async move {
            let hash: Option<Option<String>> =
                sqlx::query_scalar("SELECT evaluation_hash FROM rate_drafts WHERE id::text = $1")
                    .bind(object_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(pos_error::AppError::Database)?;

            match hash {
                Some(Some(h)) => Ok(h),
                _ => Err(pos_error::AppError::NotFound {
                    entity: "RateDraft",
                    id: object_id.to_string(),
                }),
            }
        })
    }

    fn on_decided<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
        outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let new_status = match outcome {
                Outcome::Approved => "APPROVED",
                Outcome::Rejected => "REJECTED",
            };
            sqlx::query(
                "UPDATE rate_drafts SET status = $1, updated_at = now() WHERE id::text = $2",
            )
            .bind(new_status)
            .bind(object_id)
            .execute(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            Ok(())
        })
    }
}

pub struct RatePublicationPolicy {
    _pool: PgPool,
}

impl RatePublicationPolicy {
    pub fn new(pool: PgPool) -> Self {
        Self { _pool: pool }
    }
}

impl ApprovalPolicy for RatePublicationPolicy {
    fn may_decide(&self, who: &Actor, _request: &ApprovalRequest) -> Result<(), Refusal> {
        if who.roles.iter().any(|r| r.role == Role::Executive) {
            Ok(())
        } else {
            Err(Refusal::new(
                "outOfScope",
                "only owner may decide rate publication",
            ))
        }
    }

    fn allows_self_decision(&self, who: &Actor) -> bool {
        who.roles.iter().any(|r| r.role == Role::Executive)
    }

    fn policy_version(&self) -> String {
        "1".to_string()
    }
}

pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    let subject = Arc::new(RatePublicationSubject::new());
    let policy = Arc::new(RatePublicationPolicy::new(pool));
    registry.register(subject, policy);
}
