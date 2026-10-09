//! Ticket #596 (AP-1) — the approval core's rules: the proposer rule, one pending request
//! per object, concurrent decisions, the hash bound, `decide_by_policy`,
//! `decide_self`, idempotency, redaction on write, and the append-only audit log.
//! Written before the implementation by the ticket author (claude/e5ab6c).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §5.1 (tables), §5.2 (the core and the
//! per-type trait), §5.4 (the proposer, A5), §5.5 (redaction), §5.6 (concurrency
//! and idempotency), §4.6 (who moves the states), §7 (class diagram), §9 (AP-1).
//!
//! The subjects and policies are fakes (`common/mod.rs`), so these tests pin the
//! core and nothing of any one type. Needs `TEST_DATABASE_URL` (see
//! `common/mod.rs`); nothing here skips.
//!
//! Signatures this test assumes (where §5.2 leaves the exact Rust open):
//! - Every `ApprovalService` method takes the caller's transaction first, so a
//!   type's own write (RE-1's submit) and the approval write are one transaction
//!   (§4.6 "same transaction as submit"). The caller commits:
//!   - `request(&self, tx: &mut Transaction<'_, Postgres>, type_name: &str,
//!     object_id: &str, payload: &Value, by: &Actor, key: &str)
//!     -> Result<ApprovalRequest, ApprovalError>`;
//!   - `decide(&self, tx, request_id: &<id>, who: &Actor, outcome: Outcome,
//!     reason: &str, key: &str) -> Result<T, ApprovalError>`;
//!   - `decide_by_policy(&self, tx, request_id: &<id>, policy_version: &str)
//!     -> Result<T, ApprovalError>`;
//!   - `decide_self(&self, tx, request_id: &<id>, who: &Actor, reason: &str)
//!     -> Result<T, ApprovalError>`;
//!   - every `T` is `Debug`, and every future is `Send`.
//! - `ApprovalRequest` is `Debug` and has a public `id` whose type is
//!   `Clone + PartialEq + Debug + Display + Send + 'static`; `Display` prints the
//!   uuid of `approval_requests.id`.
//! - `ApprovalError` is `Debug` and has `code(&self) -> &'static str`, the v3
//!   code of §4.7 ("validationFailed", "outOfScope", "staleVersion",
//!   "approvalRequired", "idempotencyConflict", …). A `Refusal` from
//!   `ApprovalPolicy::may_decide` reaches the caller with its own code.
//! - `decide_by_policy` writes `approver_id = "policy:<version>"` and reason
//!   `in_bound` (§4.6: "the decision is written by the policy itself"), and its
//!   audit row's `actor_id` is the same `"policy:<version>"`.
//! - A person's decision writes `approver_id = who.id()`, `approver_role =
//!   who.role().as_str()`, and an audit row whose `actor_id` is `who.id()`.
//! - Codes the design does not name for these exact cases, chosen here:
//!   a decision after the object's hash moved is `staleVersion` (§5.6 uses it for
//!   every "changed since you read it"); `decide_self` when the policy does not
//!   allow it is `outOfScope` (§5.4: the proposer deciding).
//!   A second PENDING request for the same (type, object) is asserted refused,
//!   with no code: the design names none.
//! - `AuditLog` has exactly two required methods (§5.1 "append and paged reads
//!   only"):
//!   - `append<'a>(&'a self, tx: &'a mut Transaction<'_, Postgres>,
//!     event: &'a AuditEvent) -> BoxFuture<'a, AppResult<i64>>` (the new id);
//!   - `page<'a>(&'a self, after_id: Option<i64>, limit: i64)
//!     -> BoxFuture<'a, AppResult<Vec<AuditEntry>>>`, ascending by id, keyset.
//! - `AuditEvent { actor_id, actor_role, scope, action, object_ref: String,
//!   branch_id: Option<String>, before, after: Option<Value> }`, all public, and
//!   `AuditEntry { id: i64, at: DateTime<Utc>, event: AuditEvent }`.
//!   `append` redacts `before`/`after` itself: §5.5 redacts every
//!   `after_redacted` "before it is written", and §7.1 lets other areas append.
//! - The idempotency of `request` is kept on `approval_requests`
//!   (`UNIQUE (type, idempotency_key)`, §5.1): the same key with the same object
//!   and payload replays the stored request; anything else under that key is
//!   `idempotencyConflict` (§5.6). `decide`'s key is not tested here: AP-1 has no
//!   table for it (`idempotency_records` is CM-1's, migration 3).

mod common;

use std::fmt::Debug;
use std::sync::Arc;

use common::*;
use futures::future::BoxFuture;
use pos_error::AppResult;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use tokio::sync::Barrier;
use xc_auth::domain::policy::value_objects::actor::Actor;

// ── helpers ─────────────────────────────────────────────────────────────────

/// One registry entry: a fake subject with its policy.
fn pair(
    subject: &Arc<FakeSubject>,
    policy: impl ApprovalPolicy + 'static,
) -> (Arc<dyn ApprovalSubject>, Arc<dyn ApprovalPolicy>) {
    let subject: Arc<dyn ApprovalSubject> = subject.clone();
    let policy: Arc<dyn ApprovalPolicy> = Arc::new(policy);
    (subject, policy)
}

/// Audit rows written by `actor_id` so far.
async fn audit_count(db: &TestDb, actor_id: &str) -> usize {
    db.audit_by(actor_id).await.len()
}

/// Open a request in its own transaction and commit it.
async fn open(
    db: &TestDb,
    svc: &ApprovalService,
    type_name: &str,
    object_id: &str,
    by: &Actor,
) -> ApprovalRequest {
    let mut tx = db.begin().await;
    let request = svc
        .request(
            &mut tx,
            type_name,
            object_id,
            &json!({ "currency": "USD", "buyRate": "32.10000000" }),
            by,
            &key(),
        )
        .await
        .expect("the request opens");
    tx.commit().await.expect("commit the request");
    request
}

/// Commit on success, roll back on a refusal; answer the refusal's code.
async fn settle<T: Debug>(
    tx: Transaction<'static, Postgres>,
    result: Result<T, ApprovalError>,
) -> Result<(), &'static str> {
    match result {
        Ok(_) => {
            tx.commit().await.expect("commit");
            Ok(())
        }
        Err(e) => {
            tx.rollback().await.expect("roll back");
            Err(e.code())
        }
    }
}

async fn pending_for(db: &TestDb, type_name: &str, object_id: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM approval_requests \
         WHERE type = $1 AND object_id = $2 AND status = 'PENDING'",
    )
    .bind(type_name)
    .bind(object_id)
    .fetch_one(&db.pool)
    .await
    .expect("count pending requests")
}

// ── the proposer (§5.4) ─────────────────────────────────────────────────────

#[tokio::test]
async fn the_proposer_cannot_decide_their_own_request_even_when_the_policy_allows_anyone() {
    let db = TestDb::start().await;
    let a = FakeSubject::new(TYPE_A);
    let svc = db.service(vec![pair(
        &a,
        AnyonePolicy {
            self_decision: true,
        },
    )]);

    for proposer in [rates("rates-1"), owner("owner-1")] {
        let object = format!("obj-{}", proposer.id());
        let req = open(&db, &svc, TYPE_A, &object, &proposer).await;
        let id = req.id.to_string();

        for outcome in [Outcome::Approved, Outcome::Rejected] {
            let mut tx = db.begin().await;
            let got = svc
                .decide(&mut tx, &req.id, &proposer, outcome, "mine", &key())
                .await;
            assert_eq!(
                settle(tx, got).await,
                Err("outOfScope"),
                "{} decided their own request ({outcome:?})",
                proposer.id()
            );
        }
        assert_eq!(db.status(&id).await, "PENDING");
        assert!(db.decisions(&id).await.is_empty(), "a decision was written");
    }
    assert!(a.decided().is_empty(), "on_decided ran for a refusal");
    db.finish().await;
}

#[tokio::test]
async fn another_person_decides_through_the_policy_and_the_decision_is_audited() {
    let db = TestDb::start().await;
    let a = FakeSubject::new(TYPE_A);
    let svc = db.service(vec![pair(&a, OwnerPolicy)]);
    let req = open(&db, &svc, TYPE_A, "obj-1", &rates("rates-1")).await;
    let id = req.id.to_string();

    // Not the proposer, but the policy refuses: its refusal reaches the caller.
    let mut tx = db.begin().await;
    let got = svc
        .decide(
            &mut tx,
            &req.id,
            &rates("rates-2"),
            Outcome::Approved,
            "ok",
            &key(),
        )
        .await;
    assert_eq!(settle(tx, got).await, Err("outOfScope"));
    assert!(db.decisions(&id).await.is_empty());
    assert!(a.decided().is_empty());

    let audit_before = audit_count(&db, "owner-1").await;
    let mut tx = db.begin().await;
    let got = svc
        .decide(
            &mut tx,
            &req.id,
            &owner("owner-1"),
            Outcome::Approved,
            "checked",
            &key(),
        )
        .await;
    assert_eq!(settle(tx, got).await, Ok(()));

    assert_eq!(db.status(&id).await, "APPROVED");
    assert_eq!(
        db.decisions(&id).await,
        vec![(
            "owner-1".to_owned(),
            "OWNER".to_owned(),
            "APPROVED".to_owned(),
            "checked".to_owned()
        )]
    );
    let audit = db.audit_by("owner-1").await;
    assert_eq!(
        audit.len(),
        audit_before + 1,
        "one audit row for the decision: {audit:?}"
    );
    assert!(audit.iter().all(|(role, _)| role == "OWNER"), "{audit:?}");
    assert_eq!(a.decided(), vec![("obj-1".to_owned(), Outcome::Approved)]);
    db.finish().await;
}

#[tokio::test]
async fn a_rejection_closes_the_request_and_tells_the_subject() {
    let db = TestDb::start().await;
    let a = FakeSubject::new(TYPE_A);
    let svc = db.service(vec![pair(&a, OwnerPolicy)]);
    let req = open(&db, &svc, TYPE_A, "obj-1", &rates("rates-1")).await;
    let id = req.id.to_string();

    let mut tx = db.begin().await;
    let got = svc
        .decide(
            &mut tx,
            &req.id,
            &owner("owner-1"),
            Outcome::Rejected,
            "spread too wide",
            &key(),
        )
        .await;
    assert_eq!(settle(tx, got).await, Ok(()));

    assert_eq!(db.status(&id).await, "REJECTED");
    let decisions = db.decisions(&id).await;
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].2, "REJECTED");
    assert_eq!(decisions[0].3, "spread too wide");
    assert_eq!(a.decided(), vec![("obj-1".to_owned(), Outcome::Rejected)]);
    db.finish().await;
}

// ── one pending request per (type, object) (§5.1 `one_pending_per_object`) ──

#[tokio::test]
async fn only_one_request_per_type_and_object_may_be_pending() {
    let db = TestDb::start().await;
    let a = FakeSubject::new(TYPE_A);
    let b = FakeSubject::new(TYPE_B);
    let svc = db.service(vec![pair(&a, OwnerPolicy), pair(&b, OwnerPolicy)]);
    let first = open(&db, &svc, TYPE_A, "obj-1", &rates("rates-1")).await;

    // A second request for the same object, under a new key, is refused.
    let mut tx = db.begin().await;
    let second = svc
        .request(
            &mut tx,
            TYPE_A,
            "obj-1",
            &json!({ "currency": "EUR" }),
            &rates("rates-2"),
            &key(),
        )
        .await;
    assert!(
        second.is_err(),
        "a second PENDING request was opened: {second:?}"
    );
    tx.rollback().await.expect("roll back");
    assert_eq!(pending_for(&db, TYPE_A, "obj-1").await, 1);

    // The same object id under another type is another object.
    open(&db, &svc, TYPE_B, "obj-1", &rates("rates-1")).await;
    assert_eq!(pending_for(&db, TYPE_B, "obj-1").await, 1);

    // Once the first is decided, the object may be asked about again.
    let mut tx = db.begin().await;
    let got = svc
        .decide(
            &mut tx,
            &first.id,
            &owner("owner-1"),
            Outcome::Rejected,
            "no",
            &key(),
        )
        .await;
    assert_eq!(settle(tx, got).await, Ok(()));
    open(&db, &svc, TYPE_A, "obj-1", &rates("rates-1")).await;
    assert_eq!(pending_for(&db, TYPE_A, "obj-1").await, 1);
    assert_eq!(db.count("approval_requests").await, 3);
    db.finish().await;
}

// ── concurrent decisions (§5.6: `FOR UPDATE`, the other gets staleVersion) ──

#[tokio::test]
async fn a_decision_waits_for_the_one_in_flight_and_is_then_refused() {
    let db = TestDb::start().await;
    let a = FakeSubject::new(TYPE_A);
    let svc = db.service(vec![pair(&a, OwnerPolicy)]);
    let req = open(&db, &svc, TYPE_A, "obj-1", &rates("rates-1")).await;
    let id = req.id.to_string();

    // The first decision is written but not committed: it holds the request.
    let mut first = db.begin().await;
    svc.decide(
        &mut first,
        &req.id,
        &owner("owner-1"),
        Outcome::Approved,
        "first",
        &key(),
    )
    .await
    .expect("the first decision applies");

    let second = {
        let svc = svc.clone();
        let pool = db.pool.clone();
        let request_id = req.id.clone();
        tokio::spawn(async move {
            let mut tx = pool.begin().await.expect("begin");
            let got = svc
                .decide(
                    &mut tx,
                    &request_id,
                    &owner("owner-2"),
                    Outcome::Rejected,
                    "second",
                    &key(),
                )
                .await
                .map(|_| ())
                .map_err(|e| e.code());
            match got {
                Ok(()) => tx.commit().await.expect("commit"),
                Err(_) => tx.rollback().await.expect("roll back"),
            }
            got
        })
    };

    tokio::time::sleep(STAYS_OUT).await;
    assert!(
        !second.is_finished(),
        "the second decision did not wait for the request the first one holds"
    );

    first.commit().await.expect("commit the first decision");
    let got = within(MUST_FINISH, "the second decision", second)
        .await
        .expect("the task does not panic");
    assert_eq!(got, Err("staleVersion"));

    assert_eq!(db.status(&id).await, "APPROVED");
    let decisions = db.decisions(&id).await;
    assert_eq!(decisions.len(), 1, "{decisions:?}");
    assert_eq!(decisions[0].0, "owner-1");
    assert_eq!(a.decided(), vec![("obj-1".to_owned(), Outcome::Approved)]);
    db.finish().await;
}

#[tokio::test]
async fn two_decisions_started_together_leave_exactly_one_decision() {
    let db = TestDb::start().await;
    let a = FakeSubject::new(TYPE_A);
    let svc = db.service(vec![pair(&a, OwnerPolicy)]);
    let req = open(&db, &svc, TYPE_A, "obj-1", &rates("rates-1")).await;
    let id = req.id.to_string();
    let gate = Arc::new(Barrier::new(2));

    let tasks: Vec<_> = [
        ("owner-1", Outcome::Approved),
        ("owner-2", Outcome::Rejected),
    ]
    .into_iter()
    .map(|(who, outcome)| {
        let svc = svc.clone();
        let pool = db.pool.clone();
        let gate = gate.clone();
        let request_id = req.id.clone();
        tokio::spawn(async move {
            let mut tx = pool.begin().await.expect("begin");
            gate.wait().await;
            let got = svc
                .decide(&mut tx, &request_id, &owner(who), outcome, "race", &key())
                .await
                .map(|_| ())
                .map_err(|e| e.code());
            match got {
                Ok(()) => tx.commit().await.expect("commit"),
                Err(_) => tx.rollback().await.expect("roll back"),
            }
            got
        })
    })
    .collect();

    let mut answers = Vec::new();
    for task in tasks {
        answers.push(
            within(MUST_FINISH, "a racing decision", task)
                .await
                .expect("the task does not panic"),
        );
    }
    answers.sort();
    assert_eq!(answers, vec![Ok(()), Err("staleVersion")]);
    assert_eq!(db.decisions(&id).await.len(), 1);
    assert_eq!(a.decided().len(), 1);
    assert_ne!(db.status(&id).await, "PENDING");
    db.finish().await;
}

// ── the hash bound (§5.2 `version_hash`) ────────────────────────────────────

#[tokio::test]
async fn a_request_is_bound_to_the_hash_and_policy_version_it_was_opened_with() {
    let db = TestDb::start().await;
    let a = FakeSubject::new(TYPE_A);
    a.set_hash("obj-1", "h-1");
    let svc = db.service(vec![pair(&a, OwnerPolicy)]);
    let req = open(&db, &svc, TYPE_A, "obj-1", &rates("rates-1")).await;

    let (hash, policy, by): (String, String, String) = sqlx::query_as(
        "SELECT object_version_hash, policy_version, requested_by \
         FROM approval_requests WHERE id::text = $1",
    )
    .bind(req.id.to_string())
    .fetch_one(&db.pool)
    .await
    .expect("read the request row");
    assert_eq!(hash, "h-1");
    assert_eq!(policy, POLICY_VERSION);
    assert_eq!(by, "rates-1");
    db.finish().await;
}

#[tokio::test]
async fn a_decision_after_the_object_changed_is_refused_and_writes_nothing() {
    let db = TestDb::start().await;
    let a = FakeSubject::new(TYPE_A);
    a.set_hash("obj-1", "h-1");
    let svc = db.service(vec![pair(&a, OwnerPolicy)]);
    let req = open(&db, &svc, TYPE_A, "obj-1", &rates("rates-1")).await;
    let id = req.id.to_string();
    let audit_before = db.count("audit_events").await;

    a.set_hash("obj-1", "h-2");
    let mut tx = db.begin().await;
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
    assert_eq!(settle(tx, got).await, Err("staleVersion"));

    // The policy path is bound the same way.
    let mut tx = db.begin().await;
    let got = svc.decide_by_policy(&mut tx, &req.id, POLICY_VERSION).await;
    assert_eq!(settle(tx, got).await, Err("staleVersion"));

    assert_eq!(db.status(&id).await, "PENDING");
    assert!(db.decisions(&id).await.is_empty(), "a decision was written");
    assert_eq!(db.count("audit_events").await, audit_before);
    assert!(a.decided().is_empty());
    db.finish().await;
}

// ── decisions written at submit (§5.2, §5.4, A2, A5) ────────────────────────

#[tokio::test]
async fn decide_by_policy_writes_the_policy_as_approver_and_an_audit_row() {
    let db = TestDb::start().await;
    let a = FakeSubject::new(TYPE_A);
    let svc = db.service(vec![pair(&a, OwnerPolicy)]);
    // A RATES proposer: the policy deciding is not a person, so §5.4 does not apply.
    let req = open(&db, &svc, TYPE_A, "obj-1", &rates("rates-1")).await;
    let id = req.id.to_string();

    let mut tx = db.begin().await;
    let got = svc.decide_by_policy(&mut tx, &req.id, POLICY_VERSION).await;
    assert_eq!(settle(tx, got).await, Ok(()));

    assert_eq!(db.status(&id).await, "APPROVED");
    let decisions = db.decisions(&id).await;
    assert_eq!(decisions.len(), 1, "{decisions:?}");
    assert_eq!(decisions[0].0, format!("policy:{POLICY_VERSION}"));
    assert_eq!(decisions[0].2, "APPROVED");
    assert_eq!(decisions[0].3, "in_bound");
    let audit = db.audit_by(&format!("policy:{POLICY_VERSION}")).await;
    assert_eq!(audit.len(), 1, "one audit row for the decision: {audit:?}");
    assert_eq!(a.decided(), vec![("obj-1".to_owned(), Outcome::Approved)]);
    db.finish().await;
}

#[tokio::test]
async fn decide_self_approves_when_the_policy_allows_the_proposer() {
    let db = TestDb::start().await;
    let a = FakeSubject::new(TYPE_A);
    let svc = db.service(vec![pair(&a, OwnerPolicy)]);
    let req = open(&db, &svc, TYPE_A, "obj-1", &owner("owner-1")).await;
    let id = req.id.to_string();

    let audit_before = audit_count(&db, "owner-1").await;
    let mut tx = db.begin().await;
    let got = svc
        .decide_self(&mut tx, &req.id, &owner("owner-1"), "proposer_is_owner")
        .await;
    assert_eq!(settle(tx, got).await, Ok(()));

    assert_eq!(db.status(&id).await, "APPROVED");
    assert_eq!(
        db.decisions(&id).await,
        vec![(
            "owner-1".to_owned(),
            "OWNER".to_owned(),
            "APPROVED".to_owned(),
            "proposer_is_owner".to_owned()
        )]
    );
    let audit = db.audit_by("owner-1").await;
    assert_eq!(
        audit.len(),
        audit_before + 1,
        "one audit row for the owner's own decision: {audit:?}"
    );
    assert_eq!(a.decided(), vec![("obj-1".to_owned(), Outcome::Approved)]);
    db.finish().await;
}

#[tokio::test]
async fn decide_self_is_refused_when_the_policy_does_not_allow_the_proposer() {
    let db = TestDb::start().await;
    let a = FakeSubject::new(TYPE_A);
    let b = FakeSubject::new(TYPE_B);
    let svc = db.service(vec![
        pair(&a, OwnerPolicy),
        // The trait's default: nobody closes their own request.
        pair(&b, DefaultPolicy),
    ]);

    let mut refused_audit = 0;
    let cases = [
        (TYPE_A, rates("rates-1")),
        (TYPE_B, owner("owner-1")),
        (TYPE_B, rates("rates-1")),
    ];
    for (type_name, proposer) in cases {
        let object = format!("obj-{}", proposer.id());
        let req = open(&db, &svc, type_name, &object, &proposer).await;
        let id = req.id.to_string();
        let audit_before = db.count("audit_events").await;
        let mut tx = db.begin().await;
        let got = svc
            .decide_self(&mut tx, &req.id, &proposer, "proposer_is_owner")
            .await;
        assert_eq!(
            settle(tx, got).await,
            Err("outOfScope"),
            "{type_name}: {} closed their own request",
            proposer.id()
        );
        assert_eq!(db.status(&id).await, "PENDING");
        assert!(db.decisions(&id).await.is_empty());
        refused_audit += db.count("audit_events").await - audit_before;
    }
    assert_eq!(refused_audit, 0, "a refusal was audited");
    assert!(a.decided().is_empty() && b.decided().is_empty());
    db.finish().await;
}

// ── idempotency of `request` (§5.6) ─────────────────────────────────────────

#[tokio::test]
async fn the_same_key_and_payload_replays_the_request_without_writing_again() {
    let db = TestDb::start().await;
    let a = FakeSubject::new(TYPE_A);
    let svc = db.service(vec![pair(&a, OwnerPolicy)]);
    let k = key();
    let payload = json!({ "currency": "USD", "buyRate": "32.10000000" });

    let mut tx = db.begin().await;
    let first = svc
        .request(&mut tx, TYPE_A, "obj-1", &payload, &rates("rates-1"), &k)
        .await
        .expect("the request opens");
    tx.commit().await.expect("commit");
    let requests = db.count("approval_requests").await;
    let audit = db.count("audit_events").await;

    let mut tx = db.begin().await;
    let again = svc
        .request(&mut tx, TYPE_A, "obj-1", &payload, &rates("rates-1"), &k)
        .await
        .expect("a retry with the same key and payload replays");
    tx.commit().await.expect("commit");

    assert_eq!(again.id, first.id, "the replay answered another request");
    assert_eq!(db.count("approval_requests").await, requests);
    assert_eq!(db.count("audit_events").await, audit);
    assert_eq!(pending_for(&db, TYPE_A, "obj-1").await, 1);
    db.finish().await;
}

#[tokio::test]
async fn the_same_key_with_anything_else_is_an_idempotency_conflict() {
    let db = TestDb::start().await;
    let a = FakeSubject::new(TYPE_A);
    let svc = db.service(vec![pair(&a, OwnerPolicy)]);
    let k = key();
    let payload = json!({ "currency": "USD", "buyRate": "32.10000000" });

    let mut tx = db.begin().await;
    svc.request(&mut tx, TYPE_A, "obj-1", &payload, &rates("rates-1"), &k)
        .await
        .expect("the request opens");
    tx.commit().await.expect("commit");

    let other_payload = json!({ "currency": "USD", "buyRate": "32.20000000" });
    let mut tx = db.begin().await;
    let got = svc
        .request(
            &mut tx,
            TYPE_A,
            "obj-1",
            &other_payload,
            &rates("rates-1"),
            &k,
        )
        .await;
    assert_eq!(settle(tx, got).await, Err("idempotencyConflict"));

    let mut tx = db.begin().await;
    let got = svc
        .request(&mut tx, TYPE_A, "obj-2", &payload, &rates("rates-1"), &k)
        .await;
    assert_eq!(settle(tx, got).await, Err("idempotencyConflict"));

    assert_eq!(db.count("approval_requests").await, 1);
    db.finish().await;
}

// ── redaction on write (§5.5) ───────────────────────────────────────────────

#[tokio::test]
async fn the_stored_payload_is_redacted() {
    let db = TestDb::start().await;
    let a = FakeSubject::new(TYPE_A);
    let svc = db.service(vec![pair(&a, OwnerPolicy)]);

    let mut tx = db.begin().await;
    let req = svc
        .request(
            &mut tx,
            TYPE_A,
            "obj-1",
            &json!({
                "currency": "USD",
                "customer": { "passportNo": "XK9988776655", "password": "hunter2-ZXCV" },
            }),
            &rates("rates-1"),
            &key(),
        )
        .await
        .expect("the request opens");
    tx.commit().await.expect("commit");

    let stored: String = sqlx::query_scalar(
        "SELECT proposed_payload::text FROM approval_requests WHERE id::text = $1",
    )
    .bind(req.id.to_string())
    .fetch_one(&db.pool)
    .await
    .expect("read the stored payload");
    assert!(!stored.contains("XK99887766"), "a full passport: {stored}");
    assert!(
        stored.contains("6655"),
        "the passport's last 4 are gone: {stored}"
    );
    assert!(!stored.contains("hunter2"), "a password: {stored}");
    assert!(
        !stored.contains("ZXCV"),
        "a password was masked, not dropped: {stored}"
    );
    assert!(stored.contains("USD"), "{stored}");
    db.finish().await;
}

// ── the audit log is append-only (§5.1) ─────────────────────────────────────

/// Compiles only while `append` and `page` are the trait's only required
/// methods: an `update` or `delete` added to the port breaks this file.
struct AppendAndReadOnly;

impl AuditLog for AppendAndReadOnly {
    fn append<'a>(
        &'a self,
        _tx: &'a mut Transaction<'_, Postgres>,
        _event: &'a AuditEvent,
    ) -> BoxFuture<'a, AppResult<i64>> {
        Box::pin(async { unreachable!("never called") })
    }

    fn page<'a>(
        &'a self,
        _after_id: Option<i64>,
        _limit: i64,
    ) -> BoxFuture<'a, AppResult<Vec<AuditEntry>>> {
        Box::pin(async { unreachable!("never called") })
    }
}

fn event(action: &str, after: Option<Value>) -> AuditEvent {
    AuditEvent {
        actor_id: "owner-1".to_owned(),
        actor_role: "OWNER".to_owned(),
        scope: "test".to_owned(),
        branch_id: Some("MAIN".to_owned()),
        action: action.to_owned(),
        object_ref: "obj-1".to_owned(),
        before: None,
        after,
    }
}

#[tokio::test]
async fn the_audit_log_appends_and_reads_pages_in_order() {
    let _shape: Arc<dyn AuditLog> = Arc::new(AppendAndReadOnly);
    let db = TestDb::start().await;
    let log = PgAuditLog::new(db.pool.clone());

    let mut tx = db.begin().await;
    let mut ids = Vec::new();
    for action in ["one", "two", "three"] {
        ids.push(
            log.append(&mut tx, &event(action, None))
                .await
                .expect("append"),
        );
    }
    tx.commit().await.expect("commit");
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "ids rise: {ids:?}");

    let first = log.page(None, 2).await.expect("first page");
    let actions: Vec<&str> = first.iter().map(|e| e.event.action.as_str()).collect();
    assert_eq!(actions, ["one", "two"]);
    assert_eq!(first[0].id, ids[0]);
    let rest = log.page(Some(first[1].id), 2).await.expect("next page");
    let actions: Vec<&str> = rest.iter().map(|e| e.event.action.as_str()).collect();
    assert_eq!(actions, ["three"]);
    assert!(
        log.page(Some(ids[2]), 2)
            .await
            .expect("past the end")
            .is_empty()
    );
    db.finish().await;
}

#[tokio::test]
async fn an_appended_audit_row_is_redacted() {
    let db = TestDb::start().await;
    let log = PgAuditLog::new(db.pool.clone());

    let mut tx = db.begin().await;
    log.append(
        &mut tx,
        &event(
            "customer.saved",
            Some(json!({ "passportNo": "XK9988776655", "otp": "564738", "currency": "USD" })),
        ),
    )
    .await
    .expect("append");
    tx.commit().await.expect("commit");

    let stored: String = sqlx::query_scalar("SELECT after_redacted::text FROM audit_events")
        .fetch_one(&db.pool)
        .await
        .expect("read the stored row");
    assert!(!stored.contains("XK99887766"), "a full passport: {stored}");
    assert!(stored.contains("6655"), "{stored}");
    assert!(!stored.contains("564738"), "an otp: {stored}");
    assert!(
        !stored.contains("4738"),
        "an otp was masked, not dropped: {stored}"
    );
    assert!(stored.contains("USD"), "{stored}");
    db.finish().await;
}
