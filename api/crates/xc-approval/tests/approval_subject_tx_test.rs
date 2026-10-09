//! Ticket #731 (AP-port) — `ApprovalSubject::version_hash` reads the object's hash through the
//! caller's transaction, so a hash written earlier in that transaction is the one a request binds
//! and a decision checks.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §5.2 (the port; `on_decided` already takes the
//! caller's transaction, `version_hash` gains the same), §4.6 (a submit writes the draft's
//! `evaluation_hash` and opens its request in ONE transaction), §5.6. Before #731 the core asks the
//! subject through the pool, which cannot see an uncommitted row, so RE-1 staged the hash in a
//! process-wide map (`rate_publication.rs`, a stopgap naming #731).
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.
//!
//! Signature this test assumes (#731 §7, [required]):
//! `fn version_hash<'a>(&'a self, tx: &'a mut Transaction<'_, Postgres>, object_id: &'a str)
//!     -> BoxFuture<'a, AppResult<String>>;`
//! `ApprovalService::{request, decide}` keep their public signatures.

#[allow(unused_imports)]
mod common;

use std::sync::Arc;

use common::*;
use futures::future::BoxFuture;
use pos_error::{AppError, AppResult};
use serde_json::json;
use sqlx::{AssertSqlSafe, Postgres, Transaction};
use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::domain::request::Outcome;
use xc_approval::domain::subject::ApprovalSubject;

const TYPE_T: &str = "TEST_TX_HASH";

/// A subject whose hash is a row of its own table, read only through the transaction it is given.
struct TableSubject;

impl ApprovalSubject for TableSubject {
    fn type_name(&self) -> &'static str {
        TYPE_T
    }

    fn version_hash<'a>(
        &'a self,
        tx: &'a mut Transaction<'_, Postgres>,
        object_id: &'a str,
    ) -> BoxFuture<'a, AppResult<String>> {
        Box::pin(async move {
            let hash: Option<String> =
                sqlx::query_scalar("SELECT hash FROM subject_hashes WHERE object_id = $1")
                    .bind(object_id)
                    .fetch_optional(&mut **tx)
                    .await?;
            hash.ok_or_else(|| AppError::NotFound {
                entity: "SubjectHash",
                id: object_id.to_owned(),
            })
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

async fn with_table(db: &TestDb) {
    sqlx::query(AssertSqlSafe(
        "CREATE TABLE subject_hashes (object_id text PRIMARY KEY, hash text NOT NULL)",
    ))
    .execute(&db.pool)
    .await
    .expect("create the subject's table");
}

fn service(db: &TestDb) -> Arc<xc_approval::application::approval_service::ApprovalService> {
    let subject: Arc<dyn ApprovalSubject> = Arc::new(TableSubject);
    let policy: Arc<dyn ApprovalPolicy> = Arc::new(AnyonePolicy {
        self_decision: false,
    });
    db.service(vec![(subject, policy)])
}

async fn bound_hash(db: &TestDb, request_id: &str) -> String {
    sqlx::query_scalar("SELECT object_version_hash FROM approval_requests WHERE id::text = $1")
        .bind(request_id)
        .fetch_one(&db.pool)
        .await
        .expect("read the request's bound hash")
}

#[tokio::test]
async fn a_hash_written_earlier_in_the_same_transaction_is_the_one_the_request_binds() {
    let db = TestDb::start().await;
    with_table(&db).await;
    let svc = service(&db);

    let mut tx = db.begin().await;
    sqlx::query("INSERT INTO subject_hashes (object_id, hash) VALUES ('o1', 'h-written-in-tx')")
        .execute(&mut *tx)
        .await
        .expect("write the hash in the caller's transaction");
    let req = within(
        MUST_FINISH,
        "request",
        svc.request(
            &mut tx,
            TYPE_T,
            "o1",
            &json!({ "currency": "USD" }),
            &rates("rates-1"),
            &key(),
        ),
    )
    .await
    .expect("the request opens: the subject's hash is visible in the caller's transaction");
    tx.commit().await.expect("commit");

    assert_eq!(req.object_version_hash, "h-written-in-tx");
    assert_eq!(bound_hash(&db, req.id.as_ref()).await, "h-written-in-tx");
    db.finish().await;
}

#[tokio::test]
async fn a_hash_moved_earlier_in_the_deciding_transaction_makes_the_decision_stale() {
    let db = TestDb::start().await;
    with_table(&db).await;
    let svc = service(&db);

    sqlx::query("INSERT INTO subject_hashes (object_id, hash) VALUES ('o2', 'h1')")
        .execute(&db.pool)
        .await
        .expect("seed the committed hash");
    let mut tx = db.begin().await;
    let req = svc
        .request(&mut tx, TYPE_T, "o2", &json!({}), &rates("rates-1"), &key())
        .await
        .expect("the request opens");
    tx.commit().await.expect("commit the request");
    assert_eq!(req.object_version_hash, "h1");

    // The object changes in the same transaction that then decides: the core must see it.
    let mut tx = db.begin().await;
    sqlx::query("UPDATE subject_hashes SET hash = 'h2' WHERE object_id = 'o2'")
        .execute(&mut *tx)
        .await
        .expect("move the hash inside the deciding transaction");
    let got = svc
        .decide(
            &mut tx,
            &req.id,
            &owner("owner-1"),
            Outcome::Approved,
            "ok",
            &key(),
        )
        .await;
    tx.rollback().await.expect("roll back");

    assert_eq!(code(got), "staleVersion");
    assert_eq!(db.status(req.id.as_ref()).await, "PENDING");
    assert!(db.decisions(req.id.as_ref()).await.is_empty());
    db.finish().await;
}

#[tokio::test]
async fn an_unchanged_hash_read_through_the_transaction_lets_the_decision_through() {
    let db = TestDb::start().await;
    with_table(&db).await;
    let svc = service(&db);

    sqlx::query("INSERT INTO subject_hashes (object_id, hash) VALUES ('o3', 'h1')")
        .execute(&db.pool)
        .await
        .expect("seed the committed hash");
    let mut tx = db.begin().await;
    let req = svc
        .request(&mut tx, TYPE_T, "o3", &json!({}), &rates("rates-1"), &key())
        .await
        .expect("the request opens");
    tx.commit().await.expect("commit the request");

    let mut tx = db.begin().await;
    svc.decide(
        &mut tx,
        &req.id,
        &owner("owner-1"),
        Outcome::Approved,
        "ok",
        &key(),
    )
    .await
    .expect("an unchanged hash is decided");
    tx.commit().await.expect("commit the decision");

    assert_eq!(db.status(req.id.as_ref()).await, "APPROVED");
    db.finish().await;
}
