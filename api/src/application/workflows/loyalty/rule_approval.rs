//! AP-1 approval subject and policy for loyalty rule versions (LY-6 #1411, design §6, §6.4).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use pos_error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::policy::{ApprovalPolicy, Refusal};
use xc_approval::domain::request::{ApprovalRequest, Outcome};
use xc_approval::domain::subject::{ApprovalSubject, SubjectError};
use xc_auth::domain::access::{Actor, ApprovalDecider, Permission};
use xc_loyalty::application::rule_publisher::{
    LoyaltyRuleError, LoyaltyRuleWriter, RuleKind, RuleVersion,
};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub const TYPE: &str = "LOYALTY_RULE_VERSION";
const UNAVAILABLE_CODE: &str = "loyaltyUnavailable";

pub struct LoyaltyRuleVersionSubject {
    writer: Option<Arc<dyn LoyaltyRuleWriter>>,
}

impl LoyaltyRuleVersionSubject {
    pub fn new(writer: Option<Arc<dyn LoyaltyRuleWriter>>) -> Self {
        Self { writer }
    }

    fn require_writer(&self) -> Result<&Arc<dyn LoyaltyRuleWriter>, SubjectError> {
        self.writer
            .as_ref()
            .ok_or_else(|| SubjectError::DependencyUnavailable {
                code: UNAVAILABLE_CODE,
                message: "loyalty service token is not configured".to_owned(),
            })
    }

    async fn resolve_version(
        &self,
        writer: &Arc<dyn LoyaltyRuleWriter>,
        object_id: &str,
    ) -> Result<(RuleKind, Uuid, RuleVersion), SubjectError> {
        if let Some((kind_str, id_str)) = object_id.split_once('/') {
            let kind = RuleKind::parse(kind_str).ok_or_else(|| {
                SubjectError::Failed(AppError::Validation(format!(
                    "invalid loyalty rule kind in object_id: {object_id}"
                )))
            })?;
            let id = Uuid::parse_str(id_str).map_err(|_| {
                SubjectError::Failed(AppError::Validation(format!(
                    "invalid loyalty rule id in object_id: {object_id}"
                )))
            })?;
            let version = writer
                .get(kind, id)
                .await
                .map_err(map_rule_error)?
                .ok_or_else(|| {
                    SubjectError::Failed(AppError::NotFound {
                        entity: "LoyaltyRuleVersion",
                        id: object_id.to_owned(),
                    })
                })?;
            return Ok((kind, id, version));
        }

        let id = Uuid::parse_str(object_id).map_err(|_| {
            SubjectError::Failed(AppError::Validation(format!(
                "invalid loyalty rule object_id: {object_id}"
            )))
        })?;
        for kind in [RuleKind::Point, RuleKind::Tier] {
            if let Some(version) = writer.get(kind, id).await.map_err(map_rule_error)? {
                return Ok((kind, id, version));
            }
        }
        Err(SubjectError::Failed(AppError::NotFound {
            entity: "LoyaltyRuleVersion",
            id: object_id.to_owned(),
        }))
    }
}

fn map_rule_error(err: LoyaltyRuleError) -> SubjectError {
    tracing::warn!(error = %err, "loyalty rule approval error");
    let message = err.client_message();
    match err {
        LoyaltyRuleError::Unavailable(_) => SubjectError::DependencyUnavailable {
            code: UNAVAILABLE_CODE,
            message: message.to_owned(),
        },
        LoyaltyRuleError::NotFound => SubjectError::Failed(AppError::NotFound {
            entity: "LoyaltyRuleVersion",
            id: String::new(),
        }),
        LoyaltyRuleError::Invalid(m) => SubjectError::Failed(AppError::Validation(m)),
        LoyaltyRuleError::InvalidTransition(m) => SubjectError::Failed(AppError::Conflict(m)),
    }
}

fn map_subject_error_to_app(err: SubjectError) -> AppError {
    match err {
        SubjectError::DependencyUnavailable { message, .. } => AppError::Internal(message),
        SubjectError::Failed(e) => e,
    }
}

impl ApprovalSubject for LoyaltyRuleVersionSubject {
    fn type_name(&self) -> &'static str {
        TYPE
    }

    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        Box::pin(async move {
            self.version_hash_or_unavailable(tx, object_id)
                .await
                .map_err(map_subject_error_to_app)
        })
    }

    fn on_decided<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _object_id: &'a str,
        _outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            Err(AppError::Internal(
                "loyalty rule versions are decided through ApprovalService::decide".to_owned(),
            ))
        })
    }

    fn version_hash_or_unavailable<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, Result<String, SubjectError>> {
        Box::pin(async move {
            let writer = self.require_writer()?;
            let (_, _, version) = self.resolve_version(writer, object_id).await?;
            let bytes = serde_json::to_vec(&version.content)
                .map_err(|e| SubjectError::Failed(AppError::Internal(e.to_string())))?;
            Ok(format!("{:x}", Sha256::digest(&bytes)))
        })
    }

    fn on_decided_or_unavailable<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        request: &'a ApprovalRequest,
        outcome: Outcome,
    ) -> BoxFuture<'a, Result<(), SubjectError>> {
        Box::pin(async move {
            let writer = self.require_writer()?;
            let (kind, version_id, _) = self.resolve_version(writer, &request.object_id).await?;
            let request_id = request.id.as_str();

            match outcome {
                Outcome::Approved => match writer.approve(kind, version_id, request_id).await {
                    Ok(_) => Ok(()),
                    Err(LoyaltyRuleError::InvalidTransition(msg)) => {
                        let current = writer.get(kind, version_id).await.map_err(map_rule_error)?;
                        if let Some(v) = current
                            && v.status == "APPROVED"
                            && v.approved_ref.as_deref() == Some(request_id)
                        {
                            return Ok(());
                        }
                        Err(SubjectError::Failed(AppError::Conflict(msg)))
                    }
                    Err(e) => Err(map_rule_error(e)),
                },
                Outcome::Rejected => {
                    let reason: Option<String> = sqlx::query_scalar(
                        "SELECT reason FROM approval_decisions \
                         WHERE request_id::text = $1 \
                         ORDER BY at DESC LIMIT 1",
                    )
                    .bind(request_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|e| SubjectError::Failed(AppError::Database(e)))?;
                    let reason_str = reason.as_deref().unwrap_or("rejected");

                    match writer.reject(kind, version_id, reason_str).await {
                        Ok(_) => Ok(()),
                        Err(LoyaltyRuleError::InvalidTransition(msg)) => {
                            let current =
                                writer.get(kind, version_id).await.map_err(map_rule_error)?;
                            if let Some(v) = current
                                && v.status == "REJECTED"
                            {
                                return Ok(());
                            }
                            Err(SubjectError::Failed(AppError::Conflict(msg)))
                        }
                        Err(e) => Err(map_rule_error(e)),
                    }
                }
            }
        })
    }
}

pub struct LoyaltyRuleVersionPolicy;

impl LoyaltyRuleVersionPolicy {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LoyaltyRuleVersionPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl ApprovalPolicy for LoyaltyRuleVersionPolicy {
    fn may_decide(&self, who: &Actor, _request: &ApprovalRequest) -> Result<(), Refusal> {
        if ApprovalDecider::holds_anywhere(who, Permission::ApproveLoyaltyRuleVersion) {
            Ok(())
        } else {
            Err(Refusal::new(
                "outOfScope",
                "only EXECUTIVE may decide loyalty rule versions",
            ))
        }
    }

    fn allows_self_decision(&self, _who: &Actor) -> bool {
        false
    }

    fn policy_version(&self) -> String {
        "1".to_owned()
    }
}

pub fn register_with_writer(
    registry: &mut ApprovalRegistry,
    writer: Option<Arc<dyn LoyaltyRuleWriter>>,
) {
    let subject = Arc::new(LoyaltyRuleVersionSubject::new(writer));
    let policy = Arc::new(LoyaltyRuleVersionPolicy::new());
    registry.register(subject, policy);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    #[tokio::test]
    async fn on_decided_without_request_returns_internal_error_instead_of_looping() {
        let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
            return;
        };
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&url)
            .await
            .expect("connect test db");
        let mut tx = pool.begin().await.expect("begin tx");
        let subject = LoyaltyRuleVersionSubject::new(None);
        let err = subject
            .on_decided(
                &mut tx,
                "POINT/00000000-0000-0000-0000-000000000001",
                Outcome::Approved,
            )
            .await
            .expect_err("on_decided without request must refuse");
        assert!(
            matches!(err, AppError::Internal(ref msg) if msg == "loyalty rule versions are decided through ApprovalService::decide"),
            "got {err}"
        );
    }
}
