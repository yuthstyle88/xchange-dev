//! #4138: `ApprovalService::decide_self` and `decide_by_policy` must ask the subject through
//! `version_hash_or_unavailable` / `on_decided_or_unavailable`, as `decide` does
//! (`approval_service.rs:287-296`, `:322-331`), so a subject that reports itself unavailable is answered
//! `DependencyUnavailable` with the subject's own code, not an `Internal` 500.
//!
//! Written before the implementation by the BE-R6 sub-controller (claude/studio-claude-8). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Row 1 (the rate publication paths are unchanged) is pinned by the existing tests of `decide_self` and
//! `decide_by_policy`; rows 2 and 3 are here, each for the hash read and for the decision hook.
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.

#[allow(unused_imports)]
mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use common::*;
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use serde_json::json;
use sqlx::{Postgres, Transaction};
use xc_approval::application::approval_service::{ApprovalError, ApprovalService};
use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::domain::request::{ApprovalRequest, Outcome, RequestId};
use xc_approval::domain::subject::{ApprovalSubject, SubjectError};

const TYPE_U: &str = "TEST_UNAVAILABLE";
const UNAVAILABLE: &str = "testDependencyUnavailable";

const AVAILABLE: u8 = 0;
const HASH_UNAVAILABLE: u8 = 1;
const HOOK_UNAVAILABLE: u8 = 2;

/// A subject whose `_or_unavailable` answers can be switched to "unavailable"; its raw methods fail as a plain
/// error, the way `LoyaltyRuleVersionSubject` does (`rule_approval.rs:130-141`).
struct SwitchableSubject {
    mode: AtomicU8,
}

impl SwitchableSubject {
    fn unavailable() -> SubjectError {
        SubjectError::DependencyUnavailable {
            code: UNAVAILABLE,
            message: "the subject's dependency is unavailable".to_owned(),
        }
    }
}

impl ApprovalSubject for SwitchableSubject {
    fn type_name(&self) -> &'static str {
        TYPE_U
    }

    fn version_hash<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        let mode = self.mode.load(Ordering::SeqCst);
        Box::pin(async move {
            if mode == HASH_UNAVAILABLE {
                Err(AppError::Internal(
                    "raw version_hash is not the path".to_owned(),
                ))
            } else {
                Ok("h1".to_owned())
            }
        })
    }

    fn on_decided<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _object_id: &'a str,
        _outcome: Outcome,
    ) -> BoxFuture<'a, AppResult<()>> {
        let mode = self.mode.load(Ordering::SeqCst);
        Box::pin(async move {
            if mode == HOOK_UNAVAILABLE {
                Err(AppError::Internal(
                    "raw on_decided is not the path".to_owned(),
                ))
            } else {
                Ok(())
            }
        })
    }

    fn version_hash_or_unavailable<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _object_id: &'a str,
    ) -> BoxFuture<'a, Result<String, SubjectError>> {
        let mode = self.mode.load(Ordering::SeqCst);
        Box::pin(async move {
            if mode == HASH_UNAVAILABLE {
                Err(Self::unavailable())
            } else {
                Ok("h1".to_owned())
            }
        })
    }

    fn on_decided_or_unavailable<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _request: &'a ApprovalRequest,
        _outcome: Outcome,
    ) -> BoxFuture<'a, Result<(), SubjectError>> {
        let mode = self.mode.load(Ordering::SeqCst);
        Box::pin(async move {
            if mode == HOOK_UNAVAILABLE {
                Err(Self::unavailable())
            } else {
                Ok(())
            }
        })
    }
}

/// The service over one switchable subject whose policy lets the proposer close their own request; and a PENDING
/// request of `proposer`, opened while the subject is available.
async fn pending(
    db: &TestDb,
    proposer: &str,
) -> (Arc<ApprovalService>, Arc<SwitchableSubject>, RequestId) {
    let subject = Arc::new(SwitchableSubject {
        mode: AtomicU8::new(AVAILABLE),
    });
    let as_subject: Arc<dyn ApprovalSubject> = subject.clone();
    let policy: Arc<dyn ApprovalPolicy> = Arc::new(AnyonePolicy {
        self_decision: true,
    });
    let svc = db.service(vec![(as_subject, policy)]);
    let mut tx = db.begin().await;
    let req = svc
        .request(
            &mut tx,
            TYPE_U,
            &unique("obj"),
            &json!({}),
            &owner(proposer),
            &key(),
        )
        .await
        .expect("the request opens while the subject is available");
    tx.commit().await.expect("commit the request");
    (svc, subject, req.id)
}

fn assert_unavailable(got: Result<(), ApprovalError>, what: &str) {
    match got {
        Err(ApprovalError::DependencyUnavailable { code, .. }) => {
            assert_eq!(code, UNAVAILABLE, "{what}: the subject's own code")
        }
        other => panic!("{what}: expected DependencyUnavailable, got {other:?}"),
    }
}

/// Row 2: `decide_self` with the subject's hash unavailable.
#[tokio::test]
async fn decide_self_answers_dependency_unavailable_when_the_hash_is_unavailable() {
    let db = TestDb::start().await;
    let (svc, subject, id) = pending(&db, "owner-1").await;
    subject.mode.store(HASH_UNAVAILABLE, Ordering::SeqCst);
    let mut tx = db.begin().await;
    let got = svc
        .decide_self(&mut tx, &id, &owner("owner-1"), "self")
        .await;
    tx.rollback().await.expect("roll back");
    assert_unavailable(got, "decide_self, hash");
    assert_eq!(db.status(id.as_ref()).await, "PENDING");
    db.finish().await;
}

/// Row 2: `decide_self` with the subject's decision hook unavailable.
#[tokio::test]
async fn decide_self_answers_dependency_unavailable_when_the_hook_is_unavailable() {
    let db = TestDb::start().await;
    let (svc, subject, id) = pending(&db, "owner-1").await;
    subject.mode.store(HOOK_UNAVAILABLE, Ordering::SeqCst);
    let mut tx = db.begin().await;
    let got = svc
        .decide_self(&mut tx, &id, &owner("owner-1"), "self")
        .await;
    tx.rollback().await.expect("roll back");
    assert_unavailable(got, "decide_self, hook");
    assert_eq!(db.status(id.as_ref()).await, "PENDING");
    db.finish().await;
}

/// Row 3: `decide_by_policy` with the subject's hash unavailable.
#[tokio::test]
async fn decide_by_policy_answers_dependency_unavailable_when_the_hash_is_unavailable() {
    let db = TestDb::start().await;
    let (svc, subject, id) = pending(&db, "owner-1").await;
    subject.mode.store(HASH_UNAVAILABLE, Ordering::SeqCst);
    let mut tx = db.begin().await;
    let got = svc.decide_by_policy(&mut tx, &id, POLICY_VERSION).await;
    tx.rollback().await.expect("roll back");
    assert_unavailable(got, "decide_by_policy, hash");
    assert_eq!(db.status(id.as_ref()).await, "PENDING");
    db.finish().await;
}

/// Row 3: `decide_by_policy` with the subject's decision hook unavailable.
#[tokio::test]
async fn decide_by_policy_answers_dependency_unavailable_when_the_hook_is_unavailable() {
    let db = TestDb::start().await;
    let (svc, subject, id) = pending(&db, "owner-1").await;
    subject.mode.store(HOOK_UNAVAILABLE, Ordering::SeqCst);
    let mut tx = db.begin().await;
    let got = svc.decide_by_policy(&mut tx, &id, POLICY_VERSION).await;
    tx.rollback().await.expect("roll back");
    assert_unavailable(got, "decide_by_policy, hook");
    assert_eq!(db.status(id.as_ref()).await, "PENDING");
    db.finish().await;
}
