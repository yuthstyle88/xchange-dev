//! Implementer tests for lane-controller rulings on #1402 (claude/ad05bf8d):
//! 1. A successful tick clears last_error (sets it to NULL in the same transaction as the cursor move).
//! 2. When the feed fails, tick returns Err (the feed error, redacted), after writing last_error in its own short transaction.

mod common;

use std::collections::VecDeque;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};
use common::{ORG, TestDb};
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use uuid::Uuid;
use xc_kernel::pii::{LookupHasher, PiiKeys};
use xc_loyalty::application::sync_worker::{MembershipSyncWorker, TickOutcome};
use xc_loyalty::domain::membership_feed::{MembershipChange, MembershipFeed, MembershipPage};
use xc_loyalty::infrastructure::pg_membership_mirror::PgMembershipMirror;

fn keys() -> PiiKeys {
    PiiKeys::parse("test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=").expect("a test key")
}

fn hasher() -> LookupHasher {
    LookupHasher::new(&keys())
}

fn org() -> Uuid {
    Uuid::from_str(ORG).expect("ORG is a uuid")
}

fn mirror(db: &TestDb) -> PgMembershipMirror {
    PgMembershipMirror::new(db.pool.clone(), hasher())
}

#[derive(Default)]
struct FakeFeed {
    answers: Mutex<VecDeque<AppResult<MembershipPage>>>,
}

impl FakeFeed {
    fn with(answers: Vec<AppResult<MembershipPage>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into()),
        })
    }
}

impl MembershipFeed for FakeFeed {
    fn changes_after(&self, _seq: i64) -> BoxFuture<'_, AppResult<MembershipPage>> {
        let answer = self
            .answers
            .lock()
            .expect("not poisoned")
            .pop_front()
            .expect("queued answer");
        Box::pin(async move { answer })
    }
}

fn worker(db: &TestDb, organization: Uuid, feed: Arc<FakeFeed>) -> MembershipSyncWorker {
    MembershipSyncWorker::new(db.pool.clone(), organization, feed, Arc::new(mirror(db)))
}

async fn cursor(db: &TestDb, organization: Uuid) -> Option<(i64, bool, Option<String>)> {
    sqlx::query_as(
        "SELECT cursor, last_ok_at IS NOT NULL, last_error FROM loyalty_sync_cursor WHERE organization_id = $1",
    )
    .bind(organization)
    .fetch_optional(&db.pool)
    .await
    .expect("read loyalty_sync_cursor")
}

#[tokio::test]
async fn feed_failure_returns_err_and_records_last_error() {
    let db = TestDb::start().await;
    let feed = FakeFeed::with(vec![Err(AppError::Internal(
        "loyalty unavailable 503".to_owned(),
    ))]);
    let w = worker(&db, org(), feed);

    let res = w.tick().await;
    assert!(res.is_err(), "tick must return Err on feed failure");

    let (at_cursor, last_ok, last_error) = cursor(&db, org()).await.expect("cursor row");
    assert_eq!(at_cursor, 0);
    assert!(!last_ok);
    assert!(
        last_error
            .as_deref()
            .unwrap_or_default()
            .contains("loyalty unavailable 503"),
        "expected error message, got: {last_error:?}"
    );
    db.finish().await;
}

#[tokio::test]
async fn successful_tick_clears_last_error() {
    let db = TestDb::start().await;
    let feed = FakeFeed::with(vec![
        Err(AppError::Internal("first attempt failed".to_owned())),
        Ok(MembershipPage {
            changed: true,
            items: vec![MembershipChange {
                phone_key: "812345678".to_owned(),
                customer_ref: Some(Uuid::new_v4()),
                tier_code: "GOLD".to_owned(),
                tier_rule_id: Uuid::new_v4(),
                tier_rule_version: 1,
                effective_at: Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap(),
                effective_until: None,
                seq: 10,
            }],
            next: 10,
        }),
    ]);
    let w = worker(&db, org(), feed);

    let first = w.tick().await;
    assert!(first.is_err());
    let (_, _, err1) = cursor(&db, org()).await.expect("cursor after err");
    assert!(
        err1.as_deref()
            .unwrap_or_default()
            .contains("first attempt failed"),
        "expected error message, got: {err1:?}"
    );

    let second = w.tick().await.expect("second attempt succeeds");
    assert_eq!(second, TickOutcome::Applied { items: 1 });
    let (at_cursor, last_ok, err2) = cursor(&db, org()).await.expect("cursor after success");
    assert_eq!(at_cursor, 10);
    assert!(last_ok);
    assert_eq!(err2, None, "successful tick clears last_error");
    db.finish().await;
}
