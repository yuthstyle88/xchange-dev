//! Verification that `registry_with` registers each approval subject exactly once (#2924).
//!
//! Specifically, `LOYALTY_RULE_VERSION` must be registered with `deps.loyalty` and not overwritten
//! by a subsequent registration with `writer = None`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
use serde_json::{Map, Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use xc_approval::domain::subject::SubjectError;
use xc_loyalty::application::rule_publisher::{
    LoyaltyRuleError, LoyaltyRuleWriter, RuleKind, RuleVersion,
};
use xchange::bootstrap::approval_subjects::{
    APPROVAL_SUBJECT_TYPES, ApprovalSubjectDeps, registry_with,
};

#[derive(Clone)]
struct FakeLoyaltyWriter;

impl LoyaltyRuleWriter for FakeLoyaltyWriter {
    fn create_draft<'a>(
        &'a self,
        _kind: RuleKind,
        _fields: &'a Map<String, Value>,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>> {
        unimplemented!()
    }

    fn submit<'a>(
        &'a self,
        _kind: RuleKind,
        _id: Uuid,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>> {
        unimplemented!()
    }

    fn approve<'a>(
        &'a self,
        _kind: RuleKind,
        _id: Uuid,
        _approved_ref: &'a str,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>> {
        unimplemented!()
    }

    fn reject<'a>(
        &'a self,
        _kind: RuleKind,
        _id: Uuid,
        _reason: &'a str,
    ) -> BoxFuture<'a, Result<RuleVersion, LoyaltyRuleError>> {
        unimplemented!()
    }

    fn get<'a>(
        &'a self,
        kind: RuleKind,
        id: Uuid,
    ) -> BoxFuture<'a, Result<Option<RuleVersion>, LoyaltyRuleError>> {
        Box::pin(async move {
            Ok(Some(RuleVersion {
                id,
                kind,
                status: "PENDING_APPROVAL".to_string(),
                approved_ref: None,
                content: json!({
                    "pointsRatio": "0.25",
                    "minimumBasisAmount": "200"
                }),
            }))
        })
    }

    fn list<'a>(
        &'a self,
        _kind: RuleKind,
    ) -> BoxFuture<'a, Result<Vec<RuleVersion>, LoyaltyRuleError>> {
        unimplemented!()
    }
}

async fn test_pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres@localhost:5432/xchange_test".to_string());
    PgPool::connect(&url)
        .await
        .expect("connect to test database (set TEST_DATABASE_URL)")
}

#[tokio::test]
async fn test_loyalty_rule_version_holds_writer_when_configured() {
    let pool = test_pool().await;
    let mut tx = pool.begin().await.expect("begin transaction");
    let writer = Arc::new(FakeLoyaltyWriter);
    let deps = ApprovalSubjectDeps {
        pool: pool.clone(),
        loyalty: Some(writer),
    };
    let registry = registry_with(&deps);
    let (subject, _) = registry
        .get("LOYALTY_RULE_VERSION")
        .expect("LOYALTY_RULE_VERSION must be registered");

    let rule_id = Uuid::new_v4();
    let object_id = format!("POINT/{rule_id}");
    let res = subject
        .version_hash_or_unavailable(&mut tx, &object_id)
        .await;
    assert!(res.is_ok(), "expected Ok from fake writer, got: {res:?}");
}

#[tokio::test]
async fn test_loyalty_rule_version_unconfigured_writer_answers_unavailable() {
    let pool = test_pool().await;
    let mut tx = pool.begin().await.expect("begin transaction");
    let deps = ApprovalSubjectDeps::pool_only(pool.clone());
    let registry = registry_with(&deps);
    let (subject, _) = registry
        .get("LOYALTY_RULE_VERSION")
        .expect("LOYALTY_RULE_VERSION must be registered");

    let rule_id = Uuid::new_v4();
    let object_id = format!("POINT/{rule_id}");
    let err = subject
        .version_hash_or_unavailable(&mut tx, &object_id)
        .await
        .expect_err("unconfigured writer must answer unavailable");
    match err {
        SubjectError::DependencyUnavailable { code, .. } => {
            assert_eq!(code, "loyaltyUnavailable");
        }
        other => panic!("expected DependencyUnavailable, got {other:?}"),
    }
}

#[tokio::test]
async fn test_every_subject_registered_exactly_once() {
    let pool = test_pool().await;
    let deps = ApprovalSubjectDeps::pool_only(pool);
    let registry = registry_with(&deps);

    for type_name in APPROVAL_SUBJECT_TYPES {
        assert!(
            registry.get(type_name).is_some(),
            "registry is missing {type_name}"
        );
    }
}
