//! Ticket #1008 (AL-3; the design's RA-3): publishing a rate enqueues one `rate_alerts.publication` outbox event in
//! `PublishService::publish`'s transaction (design `docs/design/blueprint/rate-alerts.md` §2 R12, §5, §9.1 row RA-3).
//!
//! Written before the implementation by the up-front test-writer claude/07d3765f, on the BE lane controller
//! claude/ad05bf8d's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! **What is pinned** (ticket §3, §7–§9), read from `outbox_events` (OB-1's table) with SQL:
//! - a committed publish writes exactly one row: `event_type = 'rate_alerts.publication'`,
//!   `destination = 'rate-alerts'`, `aggregate_type = 'branch'`, `aggregate_id = 'branch:<id>'`,
//!   `idempotency_key` = the publication id, `status = 'PENDING'`, `organization_id` = the branch's organisation,
//!   and a payload `{branchId, publicationId, version}`;
//! - a refused publish (`staleVersion`) and a publish whose caller rolls back write no row;
//! - two publishes of one branch write two rows, in version order (`seq`);
//! - the same publication cannot be enqueued twice (OB-1's unique key on `(organization_id, event_type,
//!   idempotency_key)`); read on the row this ticket writes;
//! - an enqueue that fails fails the publish: no publication, the pointer unmoved (§8, one transaction, R12).
//!
//! **Choices of this file**, each marked where used:
//! - The refusal and roll-back rows first make one committed publish and assert its row, so on `main` (where nothing
//!   is enqueued) they fail by result, not by passing vacuously.
//! - The "retried caller" row does not call `OutboxWriter`: `xc-rate` gains its `xc-outbox` dependency only with
//!   the implementation (§5 `Cargo.toml`), and a test file that names it would not compile on `main`. It checks the
//!   unique key on the row the publish wrote instead.
//! - `version` in the payload is compared as text (`payload_redacted->>'version'`), so a number and a string both
//!   pass; the ticket does not fix its JSON type.
//! - The failing enqueue is a test-only trigger on `outbox_events` in this test's own schema.
//! - "Never a constant" (ticket §3) is not pinned here: a branch of a second organisation cannot get a draft today,
//!   because `RateDraftService::create` writes `rate_drafts` without `organization_id` (raised on #1008).
//!
//! Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips. Each test works in a schema of its own.

mod rate_engine_common;

use rate_engine_common::*;
use serde_json::json;
use sqlx::{AssertSqlSafe, Postgres, Transaction};
use xc_kernel::ids::BranchId;
use xc_rate::domain::draft::rate_draft::{DraftLine, DraftStatus, LineSide, RateDraft};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::publication::published_rates::PublicationRef;

const EVENT: &str = "rate_alerts.publication";

// ── helpers ─────────────────────────────────────────────────────────────────

/// Commit on success, roll back on a refusal; answer the refusal's code.
async fn settle<T>(
    tx: Transaction<'static, Postgres>,
    result: Result<T, RateEngineError>,
) -> Result<T, &'static str> {
    match result {
        Ok(value) => {
            tx.commit().await.expect("commit");
            Ok(value)
        }
        Err(e) => {
            tx.rollback().await.expect("roll back");
            Err(e.code())
        }
    }
}

/// USD AUTO quoted manually far outside `seed_usd`'s guard: OWNER's own submit approves it.
fn out_of_bound_usd() -> Vec<DraftLine> {
    vec![DraftLine::new(
        code("USD"),
        LineSide::Auto,
        manual("40"),
        Some("a spike".to_owned()),
    )]
}

/// The reference batch and `branch`'s policy (as `rate_draft_flow_test.rs` seeds MAIN, in the branch's organisation).
async fn seed_usd(db: &TestDb, branch: &str) {
    db.seed_batch(
        chrono::Utc::now() - chrono::Duration::hours(1),
        &[("USD", "1", Some("32.1"), Some("32.45"))],
    )
    .await;
    let guard = |lo: &str, hi: &str| json!({ "min_pct": lo, "max_pct": hi });
    // `seed_policy`'s row, with the branch's own organisation (the shared helper takes the default one).
    sqlx::query(
        "INSERT INTO rate_policy_versions \
           (id, organization_id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
            max_reference_age_hours, on_stale, stale_limit_thb, valid_from, valid_to, \
            approved_by, reason, source_version) \
         SELECT gen_random_uuid(), b.organization_id, b.id, 1, 0, 0, $2, NULL, NULL, NULL, \
                now() - interval '1 day', NULL, $3, 'seeded by the test', NULL \
         FROM branches b WHERE b.id = $1",
    )
    .bind(branch)
    .bind(
        json!({ "USD": { "buy": guard("-1.0000", "1.0000"), "sell": guard("-1.0000", "1.0000") } }),
    )
    .bind(OWNER_ID)
    .execute(&db.pool)
    .await
    .expect("insert into rate_policy_versions");
}

/// An OWNER draft at `branch`, approved at its submit.
async fn approved(db: &TestDb, e: &Engine, branch: &BranchId) -> RateDraft {
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .create(&mut tx, branch, &owner_user(), out_of_bound_usd())
        .await;
    let d = settle(tx, got).await.expect("the draft is created");
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .submit(&mut tx, branch, &owner_user(), &d.id, &key())
        .await;
    let outcome = settle(tx, got).await.expect("the submit is accepted");
    assert_eq!(
        outcome.status,
        DraftStatus::Approved,
        "an owner draft is approved at submit"
    );
    d
}

/// Publishes `draft` at `branch` and commits on success.
async fn publish(
    db: &TestDb,
    e: &Engine,
    branch: &BranchId,
    draft: &RateDraft,
    expected_version: Option<i32>,
) -> Result<PublicationRef, &'static str> {
    let mut tx = db.begin().await;
    let got = e
        .publisher
        .publish(&mut tx, branch, &owner_user(), &draft.id, expected_version)
        .await;
    settle(tx, got).await
}

/// `(organization_id, destination, aggregate_type, aggregate_id, idempotency_key, status, branchId, publicationId,
/// version)` of every `rate_alerts.publication` row, in `seq` order.
type Row = (
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

async fn rows(db: &TestDb) -> Vec<Row> {
    sqlx::query_as(
        "SELECT organization_id::text, destination, aggregate_type, aggregate_id, idempotency_key, status, \
                payload_redacted->>'branchId', payload_redacted->>'publicationId', payload_redacted->>'version' \
         FROM outbox_events WHERE event_type = $1 ORDER BY seq",
    )
    .bind(EVENT)
    .fetch_all(&db.pool)
    .await
    .expect("read outbox_events")
}

async fn org_of(db: &TestDb, branch: &str) -> String {
    sqlx::query_scalar("SELECT organization_id::text FROM branches WHERE id = $1")
        .bind(branch)
        .fetch_one(&db.pool)
        .await
        .expect("the branch's organisation")
}

/// The one row a publish of `p` at `branch` must have written.
fn want(org: &str, branch: &str, p: &PublicationRef) -> Row {
    (
        org.to_owned(),
        "rate-alerts".to_owned(),
        "branch".to_owned(),
        format!("branch:{branch}"),
        p.id.to_string(),
        "PENDING".to_owned(),
        Some(branch.to_owned()),
        Some(p.id.to_string()),
        Some(p.version.to_string()),
    )
}

// ── §9 rows ─────────────────────────────────────────────────────────────────

/// §9: a successful publish writes exactly one `rate_alerts.publication` row: the key is the publication id, the
/// aggregate `branch:<id>`, the branch's organisation, `PENDING`, and the payload `{branchId, publicationId, version}`.
#[tokio::test]
async fn a_publish_enqueues_exactly_one_rate_alerts_publication_row() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd(&db, MAIN).await;
    let d = approved(&db, &e, &main_branch()).await;
    let p = publish(&db, &e, &main_branch(), &d, None)
        .await
        .expect("an approved draft publishes");
    assert_eq!(
        rows(&db).await,
        vec![want(&org_of(&db, MAIN).await, MAIN, &p)]
    );
    db.finish().await;
}

/// §9: a refused publish (`staleVersion`) writes no row. The first publish's row is there before and after.
#[tokio::test]
async fn a_refused_publish_writes_no_row() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd(&db, MAIN).await;
    let first = approved(&db, &e, &main_branch()).await;
    let p1 = publish(&db, &e, &main_branch(), &first, None)
        .await
        .expect("the first publishes");
    let org = org_of(&db, MAIN).await;
    assert_eq!(rows(&db).await, vec![want(&org, MAIN, &p1)]);

    let second = approved(&db, &e, &main_branch()).await;
    let refused = publish(&db, &e, &main_branch(), &second, None).await;
    assert_eq!(
        refused.map(|p| p.version),
        Err("staleVersion"),
        "expected version None while version 1 is current"
    );
    assert_eq!(
        rows(&db).await,
        vec![want(&org, MAIN, &p1)],
        "the refusal wrote no row"
    );
    db.finish().await;
}

/// §9: the caller rolls back after `publish` succeeded: no row, and the pointer stays where it was.
#[tokio::test]
async fn a_publish_whose_caller_rolls_back_writes_no_row() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd(&db, MAIN).await;
    let first = approved(&db, &e, &main_branch()).await;
    let p1 = publish(&db, &e, &main_branch(), &first, None)
        .await
        .expect("the first publishes");
    let org = org_of(&db, MAIN).await;
    assert_eq!(rows(&db).await, vec![want(&org, MAIN, &p1)]);

    let second = approved(&db, &e, &main_branch()).await;
    let mut tx = db.begin().await;
    e.publisher
        .publish(&mut tx, &main_branch(), &owner_user(), &second.id, Some(1))
        .await
        .expect("the second publish succeeds inside the caller's transaction");
    tx.rollback().await.expect("the caller rolls back");
    assert_eq!(
        rows(&db).await,
        vec![want(&org, MAIN, &p1)],
        "the rolled-back publish left no row"
    );
    assert_eq!(
        db.current_publication(MAIN).await,
        Some(p1.id.to_string()),
        "the pointer is the first publication's"
    );
    db.finish().await;
}

/// §9: two publishes of one branch write two rows, in version order (`seq`, OB-1's per-aggregate FIFO).
#[tokio::test]
async fn two_publishes_write_two_rows_in_version_order() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd(&db, MAIN).await;
    let d1 = approved(&db, &e, &main_branch()).await;
    let p1 = publish(&db, &e, &main_branch(), &d1, None)
        .await
        .expect("version 1");
    let d2 = approved(&db, &e, &main_branch()).await;
    let p2 = publish(&db, &e, &main_branch(), &d2, Some(1))
        .await
        .expect("version 2");
    assert_eq!((p1.version, p2.version), (1, 2));
    let org = org_of(&db, MAIN).await;
    assert_eq!(
        rows(&db).await,
        vec![want(&org, MAIN, &p1), want(&org, MAIN, &p2)]
    );
    db.finish().await;
}

/// §9: the same publication cannot be enqueued twice. On the row the publish wrote, OB-1's unique key
/// `(organization_id, event_type, idempotency_key)` keeps a second copy out: `ON CONFLICT DO NOTHING` adds nothing,
/// and a plain insert of the same key is refused.
#[tokio::test]
async fn the_same_publication_is_enqueued_once() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd(&db, MAIN).await;
    let d = approved(&db, &e, &main_branch()).await;
    let p = publish(&db, &e, &main_branch(), &d, None)
        .await
        .expect("an approved draft publishes");
    let org = org_of(&db, MAIN).await;
    assert_eq!(rows(&db).await, vec![want(&org, MAIN, &p)]);
    let copy = |conflict: &str| {
        format!(
            "INSERT INTO outbox_events (id, organization_id, event_type, destination, aggregate_type, aggregate_id, \
               idempotency_key, payload_hash, payload_redacted, status, attempts, next_attempt_at, created_at) \
             SELECT gen_random_uuid(), organization_id, event_type, destination, aggregate_type, aggregate_id, \
               idempotency_key, payload_hash, payload_redacted, 'PENDING', 0, now(), now() \
             FROM outbox_events WHERE event_type = '{EVENT}' {conflict}"
        )
    };
    sqlx::query(AssertSqlSafe(copy("ON CONFLICT DO NOTHING")))
        .execute(&db.pool)
        .await
        .expect("a retried enqueue is a no-op");
    assert_eq!(rows(&db).await, vec![want(&org, MAIN, &p)], "still one row");
    assert!(
        sqlx::query(AssertSqlSafe(copy("")))
            .execute(&db.pool)
            .await
            .is_err(),
        "a second row with the same publication id is refused by the unique key"
    );
    db.finish().await;
}

/// §8: if the enqueue fails, the publish fails (`databaseError`) and no publication is made: one transaction (R12).
#[tokio::test]
async fn a_failed_enqueue_fails_the_publish_and_makes_no_publication() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd(&db, MAIN).await;
    sqlx::query(
        "CREATE FUNCTION al3_refuse_outbox() RETURNS trigger AS $$ \
         BEGIN RAISE EXCEPTION 'test: the outbox is down'; END; $$ LANGUAGE plpgsql",
    )
    .execute(&db.pool)
    .await
    .expect("a refusing function");
    sqlx::query(
        "CREATE TRIGGER al3_refuse_outbox BEFORE INSERT ON outbox_events \
         FOR EACH ROW EXECUTE FUNCTION al3_refuse_outbox()",
    )
    .execute(&db.pool)
    .await
    .expect("a refusing trigger");
    let d = approved(&db, &e, &main_branch()).await;
    let got = publish(&db, &e, &main_branch(), &d, None).await;
    assert_eq!(
        got.map(|p| p.version),
        Err("databaseError"),
        "the publish fails with RateEngineError::Database"
    );
    assert_eq!(db.current_publication(MAIN).await, None, "no pointer moved");
    assert_eq!(
        db.publications(MAIN).await,
        Vec::<(i32, String)>::new(),
        "no publication was made"
    );
    db.finish().await;
}
