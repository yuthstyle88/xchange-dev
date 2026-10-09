//! Approval subject and policy for rate publication.

use futures::future::BoxFuture;
use pos_error::AppResult;
use sqlx::{PgPool, Postgres, Transaction};
use std::sync::Arc;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::ApprovalSubject;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::role::Role;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

// stopgap: staged evaluation hashes in a process-wide Mutex<HashMap> to allow
// ApprovalSubject::version_hash to see uncommitted draft evaluation hashes.
// The root cause is the AP-1 port's version_hash reading outside the caller's transaction (#731).
static STAGED_HASHES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn get_staged_hashes() -> &'static Mutex<HashMap<String, String>> {
    STAGED_HASHES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn stage_evaluation_hash(draft_id: &str, hash: &str) {
    let mut map = get_staged_hashes().lock().unwrap();
    map.insert(draft_id.to_string(), hash.to_string());
}

pub fn unstage_evaluation_hash(draft_id: &str) {
    let mut map = get_staged_hashes().lock().unwrap();
    map.remove(draft_id);
}

pub fn get_staged_hash(draft_id: &str) -> Option<String> {
    get_staged_hashes().lock().unwrap().get(draft_id).cloned()
}

pub struct RatePublicationSubject {
    pool: PgPool,
}

impl RatePublicationSubject {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl ApprovalSubject for RatePublicationSubject {
    fn type_name(&self) -> &'static str {
        "RATE_PUBLICATION"
    }

    fn version_hash<'a>(&'a self, object_id: &'a str) -> BoxFuture<'a, AppResult<String>> {
        Box::pin(async move {
            if let Some(hash) = get_staged_hash(object_id) {
                return Ok(hash);
            }

            let hash: Option<Option<String>> =
                sqlx::query_scalar("SELECT evaluation_hash FROM rate_drafts WHERE id::text = $1")
                    .bind(object_id)
                    .fetch_optional(&self.pool)
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
        if who.role() == Role::Owner {
            Ok(())
        } else {
            Err(Refusal::new(
                "outOfScope",
                "only owner may decide rate publication",
            ))
        }
    }

    fn allows_self_decision(&self, who: &Actor) -> bool {
        who.role() == Role::Owner
    }

    fn policy_version(&self) -> String {
        "1".to_string()
    }
}

pub fn register(registry: &mut ApprovalRegistry, pool: PgPool) {
    let subject = Arc::new(RatePublicationSubject::new(pool.clone()));
    let policy = Arc::new(RatePublicationPolicy::new(pool));
    registry.register(subject, policy);
}
