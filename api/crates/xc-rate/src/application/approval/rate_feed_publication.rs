//! Approval subject and policy for rate feed publication.
//!
//! Follows `rate_publication.rs` (§2, §3.3; R-5).

use std::sync::Arc;

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Transaction};
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;

pub struct RateFeedPublicationSubject {
    _pool: PgPool,
}

impl RateFeedPublicationSubject {
    pub fn new(pool: PgPool) -> Self {
        Self { _pool: pool }
    }
}

impl ApprovalSubject for RateFeedPublicationSubject {
    fn type_name(&self) -> &'static str {
        "RATE_FEED_PUBLICATION"
    }

    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        Box::pin(async move {
            let batch_id = match object_id.rsplit_once(':') {
                Some((_, b)) => b,
                None => object_id,
            };
            let checksum: Option<String> = sqlx::query_scalar(
                "SELECT raw_checksum FROM rate_reference_batches WHERE id::text = $1",
            )
            .bind(batch_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(pos_error::AppError::Database)?;

            match checksum {
                Some(c) => Ok(c),
                None => Err(pos_error::AppError::NotFound {
                    entity: "RateReferenceBatch",
                    id: object_id.to_string(),
                }),
            }
        })
    }

    fn on_decided<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _object_id: &'a str,
        _outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move { Ok(()) })
    }
}

#[derive(Debug, Default, Clone)]
pub struct RateFeedPublicationPolicy;

impl RateFeedPublicationPolicy {
    pub fn new() -> Self {
        Self
    }
}

impl ApprovalPolicy for RateFeedPublicationPolicy {
    fn may_decide(&self, _who: &Actor, _request: &ApprovalRequest) -> Result<(), Refusal> {
        Err(Refusal::new(
            "outOfScope",
            "rate feed publication cannot be decided manually",
        ))
    }

    fn allows_self_decision(&self, who: &Actor) -> bool {
        who.role() == Role::System
    }

    fn policy_version(&self) -> String {
        "1".to_string()
    }
}

pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    let subject = Arc::new(RateFeedPublicationSubject::new(pool));
    let policy = Arc::new(RateFeedPublicationPolicy::new());
    registry.register(subject, policy);
}
