//! Ticket #598 (RE-1+RE-3) — the draft → approve → publish flow against a real database (RE-1):
//! every submit opens one approval request, who closes it, edits after submit, REJECTED, publish
//! with `expected_version`, concurrent publishes, and SUPERSEDED as a derived state.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.1 (tables), §4.5 (the evaluation is stored with its
//! hash and the approval binds to it), §4.6 (states, who decides: in-bound by the policy itself
//! `policy:<policy_version_id>` reason `in_bound`; out-of-bound by OWNER through
//! `ApprovalService::decide_self` reason `proposer_is_owner`; out-of-bound by RATES waits for
//! OWNER; edits after submit refused `staleVersion`; publish in one transaction with
//! `expected_version`, else `staleVersion` and nothing written; publishing a request that is not
//! APPROVED is `approvalRequired`; SUPERSEDED derived from the branch pointer), §5.2–§5.4
//! (`RatePublicationPolicy`: OWNER decides, OWNER may self-decide), §5.6 (two publishes race; one
//! wins), §9 RE-1.
//!
//! Needs `TEST_DATABASE_URL` (see `rate_engine_common/mod.rs`); nothing here skips.
//!
//! Signatures this test assumes (besides `rate_engine_common/mod.rs` and `rate_proposal_test.rs`):
//! - `xc_rate::domain::draft::rate_draft::{RateDraft, DraftId, DraftStatus}`:
//!   - `RateDraft` has public `id: DraftId`, `status: DraftStatus`, `version: i32` and
//!     `lines: Vec<DraftLine>`;
//!   - `DraftId: Clone + Debug + PartialEq + Display + Send + Sync + 'static`, `Display` printing
//!     the uuid of `rate_drafts.id`, which is also the `object_id` of its approval requests;
//!   - `enum DraftStatus { Draft, PendingApproval, Approved, Rejected, Published }`,
//!     `Debug + Clone + Copy + PartialEq`.
//! - `RateDraftService` (`application/draft/rate_draft_service.rs`); every write takes the
//!   caller's transaction first (R-TX, §6: `CommandAction::run` hands it one) and the caller
//!   commits; every future is `Send`:
//!   - `create(&self, tx: &mut Transaction<'_, Postgres>, branch: &BranchId, who: &Actor,
//!     lines: Vec<DraftLine>) -> Result<RateDraft, RateEngineError>` (status DRAFT, version 1);
//!   - `edit(&self, tx, branch, who, id: &DraftId, expected_version: i32, lines: Vec<DraftLine>)
//!     -> Result<RateDraft, RateEngineError>` (version + 1);
//!   - `submit(&self, tx, branch, who, id: &DraftId, key: &str)
//!     -> Result<SubmitOutcome, RateEngineError>`, `SubmitOutcome { request_id: RequestId,
//!     status: DraftStatus }` (public fields; `RequestId` is AP-1's);
//!   - `copy(&self, tx, branch, who, from: &DraftId) -> Result<RateDraft, RateEngineError>`
//!     (§4.6 "a new draft is started (copy)");
//!   - `get(&self, branch: &BranchId, id: &DraftId) -> Result<Option<RateDraft>, RateEngineError>`.
//!   - Lines are bound at create/edit to the newest reference line of their currency and to the
//!     branch's policy version valid now; submit evaluates under them.
//! - `PublishService::publish(&self, tx, branch: &BranchId, who: &Actor, id: &DraftId,
//!   expected_version: Option<i32>) -> Result<PublicationRef, RateEngineError>` — `None` = "the
//!   branch has no publication yet"; the future is `Send`.
//! - `xc_rate::domain::publication::published_rates::{PublishedRates, PublicationRef}` (§4.8):
//!   `PublicationRef { id: PublicationId, version: i32 }`, `PublicationId: Display` printing the
//!   uuid of `rate_publications.id`; `PublishedRates::current` and `line_in` exactly as §4.8.
//!   `PublishedSide` has public `rate: Rate`, `unit_size: UnitSize`, `publication: PublicationRef`
//!   (and `line_id`, `stale`, `source_date`, which `published_rates_test.rs` reads).
//! - `xc_rate::domain::publication::publication::{Publication, PublicationState}`: `Publication`
//!   has public `id: PublicationId` and `version: i32`, and `Publication::state(&self,
//!   current_version: i32) -> PublicationState`, `enum PublicationState { Published, Superseded }`
//!   (`Debug + PartialEq`).
//! - Codes the design does not name for these exact cases, chosen here: a decision by a person
//!   `RatePublicationPolicy::may_decide` refuses is `outOfScope` (§5.4); a second submit of a
//!   draft that is no longer DRAFT is refused (no code pinned).
//!
//! OR-7.1 #1020 (design `org-roles-branches.md` §3.3, §4.3), by #1020's ticket author: `submit`,
//! `ApprovalService::decide` and `decide_self` take `access::Actor`, so those calls pass the B1-mapped users
//! (`owner_user`: EXECUTIVE + ADMIN organisation-wide; `rates_user`/`rates_2_user`: RATE_MANAGER on MAIN).
//! `create`, `edit`, `copy` and `publish` keep the old actor. Every assertion is unchanged.

mod rate_engine_common;

use std::sync::Arc;

use rate_engine_common::*;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use tokio::sync::Barrier;
use xc_approval::domain::request::Outcome;
use xc_auth::domain::access;
use xc_kernel::money::{Side, UnitSize};
use xc_rate::application::draft::rate_draft_service::SubmitOutcome;
use xc_rate::domain::draft::rate_draft::{DraftLine, DraftStatus, LineSide, RateDraft};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::publication::publication::PublicationState;
use xc_rate::domain::publication::published_rates::{PublicationRef, PublishedRates};

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

async fn create(db: &TestDb, e: &Engine, who: &access::Actor, lines: Vec<DraftLine>) -> RateDraft {
    let mut tx = db.begin().await;
    let got = e.drafts.create(&mut tx, &main_branch(), who, lines).await;
    settle(tx, got).await.expect("the draft is created")
}

async fn try_submit(
    db: &TestDb,
    e: &Engine,
    who: &access::Actor,
    draft: &RateDraft,
) -> Result<SubmitOutcome, &'static str> {
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .submit(&mut tx, &main_branch(), who, &draft.id, &key())
        .await;
    settle(tx, got).await
}

async fn submit(db: &TestDb, e: &Engine, who: &access::Actor, draft: &RateDraft) -> SubmitOutcome {
    try_submit(db, e, who, draft)
        .await
        .unwrap_or_else(|code| panic!("the submit is accepted, got {code}"))
}

async fn try_edit(
    db: &TestDb,
    e: &Engine,
    who: &access::Actor,
    draft: &RateDraft,
    expected_version: i32,
    lines: Vec<DraftLine>,
) -> Result<RateDraft, &'static str> {
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .edit(
            &mut tx,
            &main_branch(),
            who,
            &draft.id,
            expected_version,
            lines,
        )
        .await;
    settle(tx, got).await
}

async fn try_publish(
    db: &TestDb,
    e: &Engine,
    draft: &RateDraft,
    expected_version: Option<i32>,
) -> Result<PublicationRef, &'static str> {
    let mut tx = db.begin().await;
    let got = e
        .publisher
        .publish(
            &mut tx,
            &main_branch(),
            &owner_user(),
            &draft.id,
            expected_version,
        )
        .await;
    settle(tx, got).await
}

/// An OWNER draft under no policy: out-of-bound, so OWNER's own decision approves it at submit.
async fn approved(db: &TestDb, e: &Engine, lines: Vec<DraftLine>) -> RateDraft {
    let d = create(db, e, &owner_user(), lines).await;
    let outcome = submit(db, e, &owner_user(), &d).await;
    assert_eq!(
        outcome.status,
        DraftStatus::Approved,
        "an owner draft is approved at submit"
    );
    d
}

async fn decide(
    db: &TestDb,
    e: &Engine,
    outcome: &SubmitOutcome,
    who: &access::Actor,
    decision: Outcome,
    reason: &str,
) -> Result<(), &'static str> {
    let mut tx = db.begin().await;
    let got = e
        .approvals
        .decide(&mut tx, &outcome.request_id, who, decision, reason, &key())
        .await;
    match got {
        Ok(_) => {
            tx.commit().await.expect("commit");
            Ok(())
        }
        Err(err) => {
            tx.rollback().await.expect("roll back");
            Err(err.code())
        }
    }
}

fn guard(min: &str, max: &str) -> Value {
    json!({ "min_pct": min, "max_pct": max })
}

/// A USD reference 32.1 / 32.45 fetched an hour ago, and a MAIN policy with no adjustment and a
/// USD guard of `[-1, 1]` on both sides. Answers the policy version id.
async fn seed_usd(db: &TestDb) -> String {
    db.seed_batch(
        chrono::Utc::now() - chrono::Duration::hours(1),
        &[("USD", "1", Some("32.1"), Some("32.45"))],
    )
    .await;
    db.seed_policy(
        MAIN,
        "0",
        "0",
        json!({ "USD": { "buy": guard("-1.0000", "1.0000"), "sell": guard("-1.0000", "1.0000") } }),
        None,
        None,
        None,
    )
    .await
}

/// USD AUTO on both sides: in-bound under `seed_usd`'s policy.
fn in_bound_usd() -> Vec<DraftLine> {
    vec![auto_line("USD")]
}

/// USD MANUAL sell 40 (+23% of 32.45): out-of-bound under `seed_usd`'s policy.
fn out_of_bound_usd() -> Vec<DraftLine> {
    vec![DraftLine::new(
        code("USD"),
        LineSide::Auto,
        manual("40"),
        Some("a spike".to_owned()),
    )]
}

async fn exactly_one_request(db: &TestDb, draft: &RateDraft) -> (String, String, String) {
    let id = draft.id.to_string();
    let requests = db.requests_of(&id).await;
    assert_eq!(
        requests.len(),
        1,
        "draft {id}: exactly one request, got {requests:?}"
    );
    let request = requests.into_iter().next().expect("one");
    let hash = db.evaluation_hash(&id).await;
    assert!(
        hash.as_deref().is_some_and(|h| !h.is_empty()),
        "draft {id}: the evaluation hash is stored at submit"
    );
    assert_eq!(
        Some(&request.2),
        hash.as_ref(),
        "draft {id}: the request binds the draft's evaluation_hash (§4.5)"
    );
    request
}

// ── every submit opens exactly one request (§4.6) ──────────────────────────────

#[tokio::test]
async fn an_in_bound_submit_by_rates_is_approved_by_the_policy_in_the_same_transaction() {
    let db = TestDb::start().await;
    let e = db.engine();
    let policy_id = seed_usd(&db).await;
    let d = create(&db, &e, &rates_user(), in_bound_usd()).await;
    assert_eq!(d.status, DraftStatus::Draft);
    assert_eq!(d.version, 1);

    // the submit and its decision are one transaction: roll it back and neither exists
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .submit(&mut tx, &main_branch(), &rates_user(), &d.id, &key())
        .await
        .expect("the submit is accepted");
    assert_eq!(got.status, DraftStatus::Approved);
    tx.rollback().await.expect("roll back");
    assert!(
        db.requests_of(&d.id.to_string()).await.is_empty(),
        "rolled back: no request"
    );
    assert_eq!(db.draft_status(&d.id.to_string()).await, "DRAFT");
    assert_eq!(db.count("approval_decisions").await, 0);

    // committed: one request, APPROVED by the policy itself
    let outcome = submit(&db, &e, &rates_user(), &d).await;
    assert_eq!(outcome.status, DraftStatus::Approved);
    let (request_id, status, _) = exactly_one_request(&db, &d).await;
    assert_eq!(request_id, outcome.request_id.to_string());
    assert_eq!(status, "APPROVED");
    assert_eq!(db.draft_status(&d.id.to_string()).await, "APPROVED");
    assert_eq!(
        db.decisions_of(&request_id).await,
        vec![(
            format!("policy:{policy_id}"),
            "APPROVED".to_owned(),
            "in_bound".to_owned()
        )],
        "no person approves an in-bound set"
    );
    db.finish().await;
}

#[tokio::test]
async fn an_out_of_bound_submit_by_rates_waits_for_the_owner_and_no_other_rates_may_decide() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd(&db).await;
    let d = create(&db, &e, &rates_user(), out_of_bound_usd()).await;
    let outcome = submit(&db, &e, &rates_user(), &d).await;
    assert_eq!(outcome.status, DraftStatus::PendingApproval);
    let (request_id, status, _) = exactly_one_request(&db, &d).await;
    assert_eq!(status, "PENDING");
    assert_eq!(db.draft_status(&d.id.to_string()).await, "PENDING_APPROVAL");
    assert!(db.decisions_of(&request_id).await.is_empty());

    // the proposer may not decide (the core, §5.4), nor may another RATES (the policy, A2)
    assert_eq!(
        decide(&db, &e, &outcome, &rates_user(), Outcome::Approved, "mine").await,
        Err("outOfScope")
    );
    assert_eq!(
        decide(
            &db,
            &e,
            &outcome,
            &rates_2_user(),
            Outcome::Approved,
            "looks fine"
        )
        .await,
        Err("outOfScope")
    );
    assert_eq!(db.draft_status(&d.id.to_string()).await, "PENDING_APPROVAL");

    // OWNER decides: the draft is APPROVED through the subject's on_decided
    decide(
        &db,
        &e,
        &outcome,
        &owner_user(),
        Outcome::Approved,
        "checked the market",
    )
    .await
    .expect("OWNER decides");
    assert_eq!(db.draft_status(&d.id.to_string()).await, "APPROVED");
    assert_eq!(
        db.decisions_of(&request_id).await,
        vec![(
            OWNER_ID.to_owned(),
            "APPROVED".to_owned(),
            "checked the market".to_owned()
        )]
    );
    // and it publishes
    let p = try_publish(&db, &e, &d, None)
        .await
        .expect("an approved draft publishes");
    assert_eq!(p.version, 1);
    db.finish().await;
}

/// OR-7.1 #1020 ruling 3 (`rate_draft_service.rs:559` is the same EXECUTIVE check): an EXECUTIVE with **no** ADMIN
/// closes their own out-of-bound draft at submit, as the owner does (rule-A review must-fix, issuecomment-6116472666).
#[tokio::test]
async fn an_out_of_bound_submit_by_an_executive_without_admin_is_approved_by_decide_self() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd(&db).await;

    let executive = access::Actor::new(
        "exec-1",
        ORG,
        vec![access::ScopedRole::new(
            access::Role::Executive,
            None,
            chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    );
    let d = create(&db, &e, &executive, out_of_bound_usd()).await;
    let outcome = submit(&db, &e, &executive, &d).await;
    assert_eq!(outcome.status, DraftStatus::Approved);
    let (request_id, status, _) = exactly_one_request(&db, &d).await;
    assert_eq!(status, "APPROVED");
    assert_eq!(
        db.decisions_of(&request_id).await,
        vec![(
            "exec-1".to_owned(),
            "APPROVED".to_owned(),
            "proposer_is_owner".to_owned()
        )]
    );
}

#[tokio::test]
async fn an_out_of_bound_submit_by_the_owner_is_approved_by_decide_self_and_rates_cannot_self_decide()
 {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd(&db).await;

    // OWNER: approved at submit, by OWNER's own decision, reason proposer_is_owner
    let d = create(&db, &e, &owner_user(), out_of_bound_usd()).await;
    let outcome = submit(&db, &e, &owner_user(), &d).await;
    assert_eq!(outcome.status, DraftStatus::Approved);
    let (request_id, status, _) = exactly_one_request(&db, &d).await;
    assert_eq!(status, "APPROVED");
    assert_eq!(db.draft_status(&d.id.to_string()).await, "APPROVED");
    assert_eq!(
        db.decisions_of(&request_id).await,
        vec![(
            OWNER_ID.to_owned(),
            "APPROVED".to_owned(),
            "proposer_is_owner".to_owned()
        )]
    );
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_events WHERE actor_id = $1 AND object_ref = $2",
    )
    .bind(OWNER_ID)
    .bind(d.id.to_string())
    .fetch_one(&db.pool)
    .await
    .expect("count audit rows");
    assert_eq!(
        audited, 1,
        "the audit shows the owner's self-decision (§5.4)"
    );

    // RATES: the same lines wait; RATES may not close its own request through decide_self
    let r = create(&db, &e, &rates_user(), out_of_bound_usd()).await;
    let pending = submit(&db, &e, &rates_user(), &r).await;
    assert_eq!(pending.status, DraftStatus::PendingApproval);
    let mut tx = db.begin().await;
    let got = e
        .approvals
        .decide_self(
            &mut tx,
            &pending.request_id,
            &rates_user(),
            "proposer_is_owner",
        )
        .await;
    let code = match got {
        Err(err) => err.code(),
        Ok(v) => panic!("RATES self-decided: {v:?}"),
    };
    tx.rollback().await.expect("roll back");
    assert_eq!(code, "outOfScope");
    assert_eq!(db.draft_status(&r.id.to_string()).await, "PENDING_APPROVAL");
    let (rid, status, _) = exactly_one_request(&db, &r).await;
    assert_eq!(status, "PENDING");
    assert!(db.decisions_of(&rid).await.is_empty());
    db.finish().await;
}

#[tokio::test]
async fn an_owner_draft_under_no_policy_is_out_of_bound_and_still_opens_one_request() {
    // D-6: no policy ⇒ every line out-of-bound; OWNER closes it at submit (A5)
    let db = TestDb::start().await;
    let e = db.engine();
    let d = approved(&db, &e, vec![manual_line("USD", "32.1", "32.45")]).await;
    let (request_id, status, _) = exactly_one_request(&db, &d).await;
    assert_eq!(status, "APPROVED");
    assert_eq!(db.decisions_of(&request_id).await[0].2, "proposer_is_owner");
    db.finish().await;
}

// ── edits ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn an_edit_moves_the_version_and_an_edit_after_submit_is_refused() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd(&db).await;
    let d = create(&db, &e, &rates_user(), out_of_bound_usd()).await;

    // before submit: the expected version must be the current one, and an edit moves it
    let edited = try_edit(&db, &e, &rates_user(), &d, 1, in_bound_usd())
        .await
        .expect("an edit at the current version");
    assert_eq!(edited.version, 2);
    assert_eq!(edited.lines, in_bound_usd());
    assert_eq!(
        try_edit(&db, &e, &rates_user(), &d, 1, out_of_bound_usd())
            .await
            .map(|_| ()),
        Err("staleVersion"),
        "an edit at an old version"
    );

    // after submit: refused at the current version too, and nothing changes
    submit(&db, &e, &rates_user(), &d).await;
    assert_eq!(
        try_edit(&db, &e, &rates_user(), &d, 2, out_of_bound_usd())
            .await
            .map(|_| ()),
        Err("staleVersion"),
        "an edit after submit (§4.6 \"a change needs a new draft\")"
    );
    let now = e
        .drafts
        .get(&main_branch(), &d.id)
        .await
        .expect("read the draft")
        .expect("the draft exists");
    assert_eq!(now.version, 2);
    assert_eq!(now.lines, in_bound_usd());
    assert_eq!(now.status, DraftStatus::Approved, "in-bound after the edit");
    db.finish().await;
}

#[tokio::test]
async fn a_manual_side_without_a_reason_is_refused_at_save_and_writes_nothing() {
    let db = TestDb::start().await;
    let e = db.engine();
    let no_reason = vec![DraftLine::new(
        code("USD"),
        manual("32.1"),
        manual("32.45"),
        None,
    )];
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .create(&mut tx, &main_branch(), &rates_user(), no_reason)
        .await;
    assert_eq!(settle(tx, got).await.map(|_| ()), Err("validationFailed"));
    assert_eq!(db.count("rate_drafts").await, 0);
    assert_eq!(db.count("rate_draft_lines").await, 0);
    db.finish().await;
}

// ── REJECTED ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn rejected_is_final_and_a_new_draft_is_a_copy_with_a_new_id() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd(&db).await;
    let d = create(&db, &e, &rates_user(), out_of_bound_usd()).await;
    let pending = submit(&db, &e, &rates_user(), &d).await;
    decide(
        &db,
        &e,
        &pending,
        &owner_user(),
        Outcome::Rejected,
        "too far from the market",
    )
    .await
    .expect("OWNER rejects");
    let id = d.id.to_string();
    assert_eq!(db.draft_status(&id).await, "REJECTED");

    // nothing moves it any more
    assert_eq!(
        try_edit(&db, &e, &rates_user(), &d, 1, in_bound_usd())
            .await
            .map(|_| ()),
        Err("staleVersion")
    );
    assert!(
        try_submit(&db, &e, &rates_user(), &d).await.is_err(),
        "a rejected draft is not resubmitted"
    );
    assert_eq!(
        try_publish(&db, &e, &d, None).await.map(|_| ()),
        Err("approvalRequired")
    );
    assert_eq!(db.draft_status(&id).await, "REJECTED");
    assert_eq!(
        db.requests_of(&id).await.len(),
        1,
        "no second request on the same draft"
    );

    // a copy: a new id, DRAFT, the same lines; the rejected one stays as it was
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .copy(&mut tx, &main_branch(), &rates_user(), &d.id)
        .await;
    let copy = settle(tx, got).await.expect("a copy of a rejected draft");
    assert_ne!(copy.id, d.id, "never the same draft");
    assert_eq!(copy.status, DraftStatus::Draft);
    assert_eq!(copy.lines, out_of_bound_usd());
    assert!(
        db.requests_of(&copy.id.to_string()).await.is_empty(),
        "a copy opens no request"
    );
    assert_eq!(db.draft_status(&id).await, "REJECTED");

    // the copy goes its own way: a new request
    let again = submit(&db, &e, &rates_user(), &copy).await;
    assert_eq!(again.status, DraftStatus::PendingApproval);
    assert_ne!(again.request_id.to_string(), pending.request_id.to_string());
    db.finish().await;
}

// ── publish ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn publishing_a_pending_draft_is_approval_required_and_writes_nothing() {
    let db = TestDb::start().await;
    let e = db.engine();
    seed_usd(&db).await;
    let d = create(&db, &e, &rates_user(), out_of_bound_usd()).await;
    // a draft never submitted, and a draft waiting for OWNER
    assert_eq!(
        try_publish(&db, &e, &d, None).await.map(|_| ()),
        Err("approvalRequired"),
        "a DRAFT"
    );
    submit(&db, &e, &rates_user(), &d).await;
    assert_eq!(
        try_publish(&db, &e, &d, None).await.map(|_| ()),
        Err("approvalRequired"),
        "a PENDING_APPROVAL"
    );
    assert_eq!(db.count("rate_publications").await, 0);
    assert_eq!(db.count("published_rate_lines").await, 0);
    assert_eq!(db.current_publication(MAIN).await, None);
    assert_eq!(db.draft_status(&d.id.to_string()).await, "PENDING_APPROVAL");
    db.finish().await;
}

#[tokio::test]
async fn a_publish_carries_every_line_with_its_unit_size_and_moves_the_pointer() {
    let db = TestDb::start().await;
    let e = db.engine();
    let d = approved(
        &db,
        &e,
        vec![
            manual_line("JPY", "23.4", "23.5"),
            manual_line("USD", "32.1", "32.45"),
        ],
    )
    .await;
    let p = try_publish(&db, &e, &d, None).await.expect("publish");
    assert_eq!(p.version, 1);
    assert_eq!(db.current_publication(MAIN).await, Some(p.id.to_string()));
    assert_eq!(db.publications(MAIN).await, vec![(1, p.id.to_string())]);
    assert_eq!(db.count("published_rate_lines").await, 2);
    assert_eq!(db.draft_status(&d.id.to_string()).await, "PUBLISHED");

    let mut tx = db.begin().await;
    let jpy = e
        .published
        .line_in(&mut tx, &main_branch(), code("JPY"), Side::Sell)
        .await
        .expect("read")
        .expect("JPY sell is published");
    assert_eq!(jpy.rate, rate("23.5"));
    assert_eq!(jpy.unit_size, UnitSize::parse("100").expect("100"));
    assert_eq!(jpy.publication.version, 1);
    assert_eq!(jpy.publication.id, p.id);
    let usd = e
        .published
        .line_in(&mut tx, &main_branch(), code("USD"), Side::Buy)
        .await
        .expect("read")
        .expect("USD buy is published");
    assert_eq!(usd.rate, rate("32.1"));
    assert_eq!(usd.unit_size, UnitSize::ONE);
    assert!(
        e.published
            .line_in(&mut tx, &main_branch(), code("EUR"), Side::Buy)
            .await
            .expect("read")
            .is_none(),
        "a currency the publication has no line for"
    );
    tx.rollback().await.expect("roll back");

    let current = e
        .published
        .current(&main_branch())
        .await
        .expect("read")
        .expect("MAIN has a publication");
    assert_eq!(current.version, 1);
    assert_eq!(current.id, p.id);
    db.finish().await;
}

#[tokio::test]
async fn a_publish_with_a_stale_expected_version_writes_nothing() {
    let db = TestDb::start().await;
    let e = db.engine();
    let first = approved(&db, &e, vec![manual_line("USD", "32.1", "32.45")]).await;
    let second = approved(&db, &e, vec![manual_line("USD", "32.2", "32.55")]).await;
    let v1 = try_publish(&db, &e, &first, None)
        .await
        .expect("the first publication");
    assert_eq!(v1.version, 1);
    let before = (
        db.count("rate_publications").await,
        db.count("published_rate_lines").await,
        db.publication_snapshot(&v1.id.to_string()).await,
    );

    for (what, expected) in [
        ("as if the branch had no publication", None),
        ("a version that never existed", Some(7)),
        ("a version in the past", Some(0)),
    ] {
        assert_eq!(
            try_publish(&db, &e, &second, expected).await.map(|_| ()),
            Err("staleVersion"),
            "{what}"
        );
        assert_eq!(
            (
                db.count("rate_publications").await,
                db.count("published_rate_lines").await,
                db.publication_snapshot(&v1.id.to_string()).await,
            ),
            before,
            "{what}: nothing written"
        );
        assert_eq!(
            db.current_publication(MAIN).await,
            Some(v1.id.to_string()),
            "{what}"
        );
        assert_eq!(
            db.draft_status(&second.id.to_string()).await,
            "APPROVED",
            "{what}"
        );
    }

    // at the current version it publishes; and a draft never publishes twice
    let v2 = try_publish(&db, &e, &second, Some(1))
        .await
        .expect("the second publication");
    assert_eq!(v2.version, 2);
    assert!(
        try_publish(&db, &e, &second, Some(2)).await.is_err(),
        "a published draft is not published again"
    );
    assert_eq!(db.count("rate_publications").await, 2);
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_publishes_of_one_branch_have_exactly_one_winner() {
    let db = TestDb::start().await;
    let e = db.engine();
    for round in 1..=3i32 {
        let a = approved(&db, &e, vec![manual_line("USD", "32.1", "32.45")]).await;
        let b = approved(&db, &e, vec![manual_line("USD", "32.2", "32.55")]).await;
        let expected = if round == 1 { None } else { Some(round - 1) };
        let barrier = Arc::new(Barrier::new(2));
        let mut tasks = Vec::new();
        for draft in [a, b] {
            let pool = db.pool.clone();
            let publisher = e.publisher.clone();
            let barrier = barrier.clone();
            tasks.push(tokio::spawn(async move {
                let mut tx = pool.begin().await.expect("begin");
                barrier.wait().await;
                let got = publisher
                    .publish(&mut tx, &main_branch(), &owner_user(), &draft.id, expected)
                    .await;
                match got {
                    Ok(p) => {
                        tx.commit().await.expect("commit the winner");
                        Ok((p.version, p.id.to_string()))
                    }
                    Err(err) => {
                        tx.rollback().await.expect("roll back the loser");
                        Err(err.code())
                    }
                }
            }));
        }
        let mut wins = Vec::new();
        let mut losses = Vec::new();
        for t in tasks {
            match within(MUST_FINISH, "a concurrent publish", t)
                .await
                .expect("the task did not panic")
            {
                Ok(w) => wins.push(w),
                Err(code) => losses.push(code),
            }
        }
        assert_eq!(
            wins.len(),
            1,
            "round {round}: exactly one winner, got {wins:?} / {losses:?}"
        );
        assert_eq!(losses, vec!["staleVersion"], "round {round}: the loser");
        let (version, id) = wins.pop().expect("one");
        assert_eq!(version, round);
        assert_eq!(db.count("rate_publications").await, i64::from(round));
        assert_eq!(db.current_publication(MAIN).await, Some(id));
    }
    db.finish().await;
}

// ── SUPERSEDED is derived, never stored ─────────────────────────────────────────

#[tokio::test]
async fn after_two_publishes_the_older_reads_superseded_and_no_row_was_updated() {
    let db = TestDb::start().await;
    let e = db.engine();
    let first = approved(&db, &e, vec![manual_line("USD", "32.1", "32.45")]).await;
    let second = approved(&db, &e, vec![manual_line("USD", "32.2", "32.55")]).await;

    try_publish(&db, &e, &first, None).await.expect("v1");
    let v1 = e
        .published
        .current(&main_branch())
        .await
        .expect("read")
        .expect("v1 is current");
    assert_eq!(v1.version, 1);
    assert_eq!(v1.state(1), PublicationState::Published);
    let untouched = db.publication_snapshot(&v1.id.to_string()).await;

    try_publish(&db, &e, &second, Some(1)).await.expect("v2");
    let v2 = e
        .published
        .current(&main_branch())
        .await
        .expect("read")
        .expect("v2 is current");
    assert_eq!(v2.version, 2);
    assert_ne!(v2.id, v1.id);

    assert_eq!(v1.state(v2.version), PublicationState::Superseded);
    assert_eq!(v2.state(v2.version), PublicationState::Published);
    assert_eq!(
        db.publication_snapshot(&v1.id.to_string()).await,
        untouched,
        "the older publication and its lines were not updated (xmin and content unchanged)"
    );
    assert_eq!(
        db.publications(MAIN).await,
        vec![(1, v1.id.to_string()), (2, v2.id.to_string())]
    );
    // the draft's own status stays PUBLISHED; its effective state is its publication's
    assert_eq!(db.draft_status(&first.id.to_string()).await, "PUBLISHED");
    db.finish().await;
}
