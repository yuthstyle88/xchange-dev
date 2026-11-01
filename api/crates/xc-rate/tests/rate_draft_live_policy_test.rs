//! Ticket #5930 (rates §4.5.1) — `simulate` and `submit` evaluate a rate draft under the branch's
//! policy version valid at that moment, not under the version its lines were stamped with at save;
//! `submit` stamps the version it used on every line.
//! Written before the implementation by the ticket author (claude/b091f5e0). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.5.1 (amendment 2026-11-02), §4.5 (out-of-bound rules,
//! the stored evaluation), §4.6 (who decides), §4.7 (`simulate` writes nothing), D-5, D-6.
//! Symptom: #564 issuecomment-6564959973 — EXECUTIVE saved the EUR/JPY/CNY guards as MAIN v14,
//! re-simulated draft `6eac13f3` and still got out-of-bound under v13.
//!
//! Fixture: a BoT USD reference 32.1 / 32.45 (as `rate_draft_flow_test.rs`). Version 1 of a branch
//! has no USD guard, so a USD line that changes is out-of-bound under it (§4.5). Version 2 has a USD
//! guard of `[-1, 1]` % on both sides, so an AUTO USD line (`adjust_pct` 0) is in-bound under it.
//! `new_version` closes the live version and inserts the next one, as `ratePolicySave` does.
//!
//! Needs `TEST_DATABASE_URL` (see `rate_engine_common/mod.rs`); nothing here skips.
//!
//! Signatures this test assumes (main's at `4985bfb`; none change):
//! - `RateDraftService::{create, submit, evaluate}` as in `rate_draft_flow_test.rs` and
//!   `rate_draft_evaluate_batch_test.rs`; `evaluate(&self, tx, branch, id) ->
//!   Result<DraftEvaluationOutcome, RateEngineError>`, with public `evaluation` (`is_in_bound()`)
//!   and `policy: Option<RatePolicy>` (`RatePolicy::id() -> &str`, the version's uuid);
//! - `PublishService::publish(&self, tx, branch, who, id, expected_version)`;
//! - `ApprovalService::decide(&self, tx, request_id, who, Outcome, reason, key)`;
//! - `Evaluation::out_of_bound() -> Vec<(CurrencyCode, Side)>`.
//!
//! Added by the same author on the design reviewer's follow-up (#5926 issuecomment-6565246345):
//! row 6 of §4.5.1 must hold for AUTO sides too. Under no policy an AUTO side has no proposed rate
//! (`rate_proposal.rs:97-104`) and `DraftEvaluation` skipped it, so an all-AUTO draft evaluated
//! in-bound and submit closed it "by policy" with the fallback id `"1"`. D-6: under no policy every
//! line is out-of-bound, whatever its mode, and no decision names a policy that does not exist.

mod rate_engine_common;

use rate_engine_common::*;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use xc_approval::domain::request::Outcome;
use xc_auth::domain::access;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_rate::application::draft::rate_draft_service::{DraftEvaluationOutcome, SubmitOutcome};
use xc_rate::domain::draft::rate_draft::{DraftLine, DraftStatus, RateDraft};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;

// ── helpers ─────────────────────────────────────────────────────────────────

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

fn guard(min: &str, max: &str) -> Value {
    json!({ "min_pct": min, "max_pct": max })
}

fn usd_guard() -> Value {
    json!({ "USD": { "buy": guard("-1.0000", "1.0000"), "sell": guard("-1.0000", "1.0000") } })
}

fn usd_auto() -> Vec<DraftLine> {
    vec![auto_line("USD")]
}

async fn seed_usd_reference(db: &TestDb) {
    db.seed_batch(
        chrono::Utc::now() - chrono::Duration::hours(1),
        &[("USD", "1", Some("32.1"), Some("32.45"))],
    )
    .await;
}

/// Closes `branch`'s live version (if any) and inserts the next one, valid from now, with no
/// adjustment and `guards`. Answers the new version's id.
async fn new_version(db: &TestDb, branch: &str, guards: Value) -> String {
    sqlx::query(
        "UPDATE rate_policy_versions SET valid_to = now() \
         WHERE branch_id = $1 AND valid_to IS NULL",
    )
    .bind(branch)
    .execute(&db.pool)
    .await
    .expect("close the live version");
    sqlx::query_scalar(
        "INSERT INTO rate_policy_versions \
           (id, branch_id, version, buy_adjust_pct, sell_adjust_pct, guards, \
            max_reference_age_hours, on_stale, stale_limit_thb, valid_from, valid_to, \
            approved_by, reason, source_version) \
         SELECT gen_random_uuid(), $1, COALESCE(max(version), 0) + 1, 0, 0, $2, NULL, NULL, NULL, \
                now(), NULL, $3, 'a newer version', NULL \
         FROM rate_policy_versions WHERE branch_id = $1 \
         RETURNING id::text",
    )
    .bind(branch)
    .bind(guards)
    .bind(OWNER_ID)
    .fetch_one(&db.pool)
    .await
    .expect("insert the next rate_policy_versions row")
}

/// Closes `branch`'s live version and inserts nothing after it.
async fn close_live(db: &TestDb, branch: &str) {
    sqlx::query(
        "UPDATE rate_policy_versions SET valid_to = now() \
         WHERE branch_id = $1 AND valid_to IS NULL",
    )
    .bind(branch)
    .execute(&db.pool)
    .await
    .expect("close the live version");
}

async fn create_on(db: &TestDb, e: &Engine, branch: &BranchId, who: &access::Actor) -> RateDraft {
    let mut tx = db.begin().await;
    let got = e.drafts.create(&mut tx, branch, who, usd_auto()).await;
    settle(tx, got).await.expect("the draft is created")
}

/// A MAIN draft of one MANUAL USD line at the reference itself (32.1 / 32.45): in-bound under a
/// USD guard; out-of-bound under no policy, where every side with a proposed rate is out-of-bound
/// (D-6, `draft_evaluation.rs` `None => true`). MANUAL, because an AUTO side has no proposed rate
/// under no policy and is not measured at all.
async fn create_manual_at_reference(db: &TestDb, e: &Engine) -> RateDraft {
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .create(
            &mut tx,
            &main_branch(),
            &owner_user(),
            vec![manual_line("USD", "32.1", "32.45")],
        )
        .await;
    settle(tx, got).await.expect("the draft is created")
}

async fn simulate_on(
    db: &TestDb,
    e: &Engine,
    branch: &BranchId,
    d: &RateDraft,
) -> DraftEvaluationOutcome {
    let mut tx = db.begin().await;
    let got = e.drafts.evaluate(&mut tx, branch, &d.id).await;
    settle(tx, got).await.expect("the draft is evaluated")
}

async fn submit(db: &TestDb, e: &Engine, who: &access::Actor, d: &RateDraft) -> SubmitOutcome {
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .submit(&mut tx, &main_branch(), who, &d.id, &key())
        .await;
    settle(tx, got)
        .await
        .unwrap_or_else(|code| panic!("the submit is accepted, got {code}"))
}

/// Every line's `policy_version_id` of one draft, as text (`None` for NULL), in currency order.
async fn line_stamps(db: &TestDb, d: &RateDraft) -> Vec<Option<String>> {
    sqlx::query_scalar(
        "SELECT policy_version_id::text FROM rate_draft_lines \
         WHERE draft_id::text = $1 ORDER BY currency_code",
    )
    .bind(d.id.to_string())
    .fetch_all(&db.pool)
    .await
    .expect("read the draft's line stamps")
}

/// `(rate_drafts.version, rate_drafts.evaluation::text)` of one draft.
async fn draft_row(db: &TestDb, d: &RateDraft) -> (i32, Option<String>) {
    sqlx::query_as("SELECT version, evaluation::text FROM rate_drafts WHERE id::text = $1")
        .bind(d.id.to_string())
        .fetch_one(&db.pool)
        .await
        .expect("read the draft row")
}

/// `rate_drafts.evaluation ->> 'policy_version_id'` of one draft.
async fn evaluated_under(db: &TestDb, d: &RateDraft) -> Option<String> {
    sqlx::query_scalar(
        "SELECT evaluation ->> 'policy_version_id' FROM rate_drafts WHERE id::text = $1",
    )
    .bind(d.id.to_string())
    .fetch_one(&db.pool)
    .await
    .expect("read the stored evaluation")
}

fn policy_of(outcome: &DraftEvaluationOutcome) -> Option<String> {
    outcome.policy.as_ref().map(|p| p.id().to_owned())
}

// ── simulate ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn simulate_evaluates_under_the_live_version_not_the_one_the_lines_were_saved_under() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd_reference(&db).await;
    let v1 = new_version(&db, MAIN, json!({})).await;
    let d = create_on(&db, &e, &main_branch(), &owner_user()).await;
    assert_eq!(
        line_stamps(&db, &d).await,
        vec![Some(v1.clone())],
        "create stamps v1"
    );
    let before = simulate_on(&db, &e, &main_branch(), &d).await;
    assert!(
        !before.evaluation.is_in_bound(),
        "under v1 (no USD guard) the line is out-of-bound"
    );

    let v2 = new_version(&db, MAIN, usd_guard()).await;
    let after = simulate_on(&db, &e, &main_branch(), &d).await;
    assert_eq!(
        policy_of(&after),
        Some(v2),
        "simulate names the live version v2"
    );
    assert!(
        after.evaluation.is_in_bound(),
        "under v2 the AUTO USD line is in-bound, with no re-save"
    );
    db.finish().await;
}

#[tokio::test]
async fn simulate_under_a_newer_version_writes_nothing() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd_reference(&db).await;
    let v1 = new_version(&db, MAIN, json!({})).await;
    let d = create_on(&db, &e, &main_branch(), &owner_user()).await;
    new_version(&db, MAIN, usd_guard()).await;
    let row_before = draft_row(&db, &d).await;

    simulate_on(&db, &e, &main_branch(), &d).await;

    assert_eq!(
        line_stamps(&db, &d).await,
        vec![Some(v1)],
        "simulate leaves the lines' stamp as it was (§4.7: no mutation)"
    );
    assert_eq!(
        draft_row(&db, &d).await,
        row_before,
        "simulate changes neither the draft's version nor its evaluation"
    );
    db.finish().await;
}

#[tokio::test]
async fn with_no_live_version_simulate_evaluates_under_no_policy() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd_reference(&db).await;
    new_version(&db, MAIN, usd_guard()).await;
    let d = create_manual_at_reference(&db, &e).await;
    assert!(
        simulate_on(&db, &e, &main_branch(), &d)
            .await
            .evaluation
            .is_in_bound(),
        "in-bound while the guarded version is live"
    );

    close_live(&db, MAIN).await;
    let got = simulate_on(&db, &e, &main_branch(), &d).await;
    assert_eq!(policy_of(&got), None, "no version is live, so no policy");
    assert!(
        !got.evaluation.is_in_bound(),
        "under no policy every changed side is out-of-bound (D-6)"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_newer_version_of_another_branch_does_not_reach_this_draft() {
    let db = TestDb::start().await;
    let e = db.engine();
    db.seed_branch("BPIN").await;
    seed_usd_reference(&db).await;
    let main_v1 = new_version(&db, MAIN, json!({})).await;
    new_version(&db, "BPIN", json!({})).await;
    let d = create_on(&db, &e, &main_branch(), &owner_user()).await;

    new_version(&db, "BPIN", usd_guard()).await;
    let got = simulate_on(&db, &e, &main_branch(), &d).await;
    assert_eq!(
        policy_of(&got),
        Some(main_v1),
        "MAIN's draft uses MAIN's live version"
    );
    assert!(
        !got.evaluation.is_in_bound(),
        "BPIN's guard does not reach a MAIN draft"
    );
    db.finish().await;
}

// ── submit ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn submit_evaluates_under_the_live_version_and_stamps_it_on_every_line() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd_reference(&db).await;
    new_version(&db, MAIN, json!({})).await;
    let d = create_on(&db, &e, &main_branch(), &rates_user()).await;
    let v2 = new_version(&db, MAIN, usd_guard()).await;

    let outcome = submit(&db, &e, &rates_user(), &d).await;

    assert_eq!(
        outcome.status,
        DraftStatus::Approved,
        "in-bound under v2, so the policy closes it at submit (D-5)"
    );
    assert_eq!(evaluated_under(&db, &d).await, Some(v2.clone()));
    assert_eq!(line_stamps(&db, &d).await, vec![Some(v2.clone())]);
    let requests = db.requests_of(&d.id.to_string()).await;
    assert_eq!(requests.len(), 1, "one request, got {requests:?}");
    let decisions = db.decisions_of(&requests[0].0).await;
    assert_eq!(
        decisions,
        vec![(
            format!("policy:{v2}"),
            "APPROVED".to_owned(),
            "in_bound".to_owned()
        )],
        "the request is closed by v2, the version it was evaluated under"
    );
    db.finish().await;
}

#[tokio::test]
async fn submit_by_rates_under_a_live_version_with_no_guard_waits_for_the_executive() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd_reference(&db).await;
    new_version(&db, MAIN, usd_guard()).await;
    let d = create_on(&db, &e, &main_branch(), &rates_user()).await;
    let v2 = new_version(&db, MAIN, json!({})).await;

    let outcome = submit(&db, &e, &rates_user(), &d).await;

    assert_eq!(
        outcome.status,
        DraftStatus::PendingApproval,
        "out-of-bound under v2 (no USD guard): RATES's submit waits"
    );
    assert_eq!(evaluated_under(&db, &d).await, Some(v2.clone()));
    assert_eq!(line_stamps(&db, &d).await, vec![Some(v2)]);
    db.finish().await;
}

#[tokio::test]
async fn a_version_saved_after_submit_does_not_change_what_is_published() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd_reference(&db).await;
    new_version(&db, MAIN, usd_guard()).await;
    let d = create_on(&db, &e, &main_branch(), &rates_user()).await;
    let v2 = new_version(&db, MAIN, json!({})).await;
    let outcome = submit(&db, &e, &rates_user(), &d).await;
    assert_eq!(outcome.status, DraftStatus::PendingApproval);

    // v3 is saved while the request waits; the EXECUTIVE then approves it
    new_version(&db, MAIN, usd_guard()).await;
    let mut tx = db.begin().await;
    let decided = e
        .approvals
        .decide(
            &mut tx,
            &outcome.request_id,
            &owner_user(),
            Outcome::Approved,
            "ok",
            &key(),
        )
        .await;
    match decided {
        Ok(_) => tx.commit().await.expect("commit"),
        Err(err) => panic!("the executive's approval is accepted, got {err:?}"),
    }

    let mut tx = db.begin().await;
    let got = e
        .publisher
        .publish(&mut tx, &main_branch(), &owner_user(), &d.id, None)
        .await;
    let publication = settle(tx, got).await.expect("the approved draft publishes");

    let stamped: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT source_snapshot ->> 'policy_version_id' FROM published_rate_lines \
         WHERE publication_id::text = $1",
    )
    .bind(publication.id.to_string())
    .fetch_all(&db.pool)
    .await
    .expect("read the published lines");
    assert!(!stamped.is_empty(), "the publication has lines");
    assert!(
        stamped.iter().all(|s| s.as_deref() == Some(v2.as_str())),
        "every published line names v2, the version submit evaluated under; got {stamped:?}"
    );
    db.finish().await;
}

// ── no policy, AUTO sides (D-6; #5926 follow-up) ────────────────────────────

fn both_usd_sides(outcome: &DraftEvaluationOutcome) -> bool {
    let out = outcome.evaluation.out_of_bound();
    out.iter()
        .any(|(c, s)| c.as_str() == "USD" && *s == Side::Buy)
        && out
            .iter()
            .any(|(c, s)| c.as_str() == "USD" && *s == Side::Sell)
}

#[tokio::test]
async fn with_no_live_version_an_all_auto_draft_is_out_of_bound_on_both_sides() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd_reference(&db).await;
    new_version(&db, MAIN, usd_guard()).await;
    let d = create_on(&db, &e, &main_branch(), &owner_user()).await;
    close_live(&db, MAIN).await;

    let got = simulate_on(&db, &e, &main_branch(), &d).await;
    assert_eq!(policy_of(&got), None, "no version is live, so no policy");
    assert!(
        !got.evaluation.is_in_bound(),
        "an AUTO line under no policy is out-of-bound (D-6)"
    );
    assert!(
        both_usd_sides(&got),
        "both AUTO sides are listed, got {:?}",
        got.evaluation.out_of_bound()
    );
    db.finish().await;
}

#[tokio::test]
async fn a_draft_created_when_the_branch_never_had_a_policy_is_out_of_bound() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd_reference(&db).await;
    let d = create_on(&db, &e, &main_branch(), &owner_user()).await;
    assert_eq!(
        line_stamps(&db, &d).await,
        vec![None],
        "no version to stamp"
    );

    let got = simulate_on(&db, &e, &main_branch(), &d).await;
    assert!(
        !got.evaluation.is_in_bound(),
        "an AUTO line under no policy is out-of-bound (D-6)"
    );
    assert!(both_usd_sides(&got));
    db.finish().await;
}

#[tokio::test]
async fn with_no_live_version_a_rates_submit_of_an_all_auto_draft_waits_for_the_executive() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd_reference(&db).await;
    new_version(&db, MAIN, usd_guard()).await;
    let d = create_on(&db, &e, &main_branch(), &rates_user()).await;
    close_live(&db, MAIN).await;

    let outcome = submit(&db, &e, &rates_user(), &d).await;

    assert_eq!(
        outcome.status,
        DraftStatus::PendingApproval,
        "out-of-bound under no policy: RATES's submit waits, it is not closed by a policy"
    );
    assert_eq!(evaluated_under(&db, &d).await, None);
    assert_eq!(line_stamps(&db, &d).await, vec![None]);
    let requests = db.requests_of(&d.id.to_string()).await;
    assert_eq!(requests.len(), 1, "one request, got {requests:?}");
    assert!(
        db.decisions_of(&requests[0].0).await.is_empty(),
        "nobody has decided it yet"
    );
    db.finish().await;
}

#[tokio::test]
async fn with_no_live_version_an_executive_submit_is_closed_by_the_proposer_never_by_policy_1() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd_reference(&db).await;
    new_version(&db, MAIN, usd_guard()).await;
    let d = create_on(&db, &e, &main_branch(), &owner_user()).await;
    close_live(&db, MAIN).await;

    let outcome = submit(&db, &e, &owner_user(), &d).await;

    assert_eq!(outcome.status, DraftStatus::Approved);
    let requests = db.requests_of(&d.id.to_string()).await;
    assert_eq!(requests.len(), 1, "one request, got {requests:?}");
    let decisions = db.decisions_of(&requests[0].0).await;
    assert_eq!(decisions.len(), 1, "one decision, got {decisions:?}");
    let (approver, outcome_text, reason) = &decisions[0];
    assert_eq!(outcome_text, "APPROVED");
    assert_eq!(
        reason, "proposer_is_owner",
        "out-of-bound by the EXECUTIVE: the proposer's own decision (§4.6)"
    );
    assert!(
        !approver.starts_with("policy:"),
        "no policy closes a draft evaluated under no policy, got approver {approver}"
    );
    db.finish().await;
}
