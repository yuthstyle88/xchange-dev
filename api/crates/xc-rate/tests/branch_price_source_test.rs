//! Rate feed F9 — the per-branch price source (`docs/design/rate-feed-connector.md` §3.5;
//! owner's rulings R-7 and "F9 ตามที่เสนอ" on #986 / #1052).
//! Written before the implementation by the ticket author (claude/c954feda). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (see `rate_engine_common/mod.rs`).
//!
//! What this pins:
//! - `xc_rate::domain::branch::price_source::{PriceSource, BranchPriceSource}`:
//!   `PriceSource::{BotDraft, RateBoard}`; the trait's `source_of(tx, branch)`,
//!   `set(tx, who, branch, to)` and `switch_back(tx, who, branch, draft, expected_version)`,
//!   each answering `Result<_, RateEngineError>`.
//! - `xc_rate::infrastructure::branch::price_source::PgBranchPriceSource::new(pool, audit)`
//!   with `audit: Arc<dyn AuditLog>`.
//! - `set`: OWNER only (`outOfScope`); unknown branch `notFound`; the same value writes nothing;
//!   `BOT_DRAFT → RATE_BOARD` writes the column and one audit event; `RATE_BOARD → BOT_DRAFT` is
//!   refused (`validationFailed`) — that way is `switch_back`.
//! - `switch_back`: in the caller's transaction, sets `BOT_DRAFT` and publishes the given approved
//!   BoT draft through `PublishService`; if the publish fails, the caller rolls back and the
//!   branch stays on the board.
//! - `PublishService::publish` refuses (`validationFailed`) on a `RATE_BOARD` branch.
//!
//! OR-7.1 #1020 (design `org-roles-branches.md` §3.3, §4.3), by #1020's ticket author: `RateDraftService::submit`
//! takes `access::Actor`, so its callers here pass the B1-mapped user (`rate_engine_common::owner_user`: EXECUTIVE
//! + ADMIN organisation-wide). Every assertion is unchanged.

mod rate_engine_common;

use std::sync::Arc;

use rate_engine_common::*;
use sqlx::{Postgres, Transaction};
use xc_approval::domain::audit_log::AuditLog;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access;
use xc_kernel::ids::BranchId;
use xc_rate::domain::branch::price_source::{BranchPriceSource, PriceSource};
use xc_rate::domain::draft::rate_draft::{DraftLine, DraftStatus, RateDraft};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::infrastructure::branch::price_source::PgBranchPriceSource;

fn sources(db: &TestDb) -> PgBranchPriceSource {
    let audit: Arc<dyn AuditLog> = Arc::new(PgAuditLog::new(db.pool.clone()));
    PgBranchPriceSource::new(db.pool.clone(), audit)
}

async fn settle<T>(
    tx: Transaction<'static, Postgres>,
    result: Result<T, RateEngineError>,
) -> Result<T, &'static str> {
    match result {
        Ok(v) => {
            tx.commit().await.expect("commit");
            Ok(v)
        }
        Err(e) => {
            tx.rollback().await.expect("roll back");
            Err(e.code())
        }
    }
}

async fn source_of(db: &TestDb, branch: &BranchId) -> Result<PriceSource, &'static str> {
    let mut tx = db.begin().await;
    let got = sources(db).source_of(&mut tx, branch).await;
    settle(tx, got).await
}

async fn set(
    db: &TestDb,
    who: &access::Actor,
    branch: &BranchId,
    to: PriceSource,
) -> Result<(), &'static str> {
    let mut tx = db.begin().await;
    let got = sources(db).set(&mut tx, who, branch, to).await;
    settle(tx, got).await
}

async fn column(db: &TestDb, branch: &str) -> String {
    sqlx::query_scalar("SELECT price_source FROM branches WHERE id = $1")
        .bind(branch)
        .fetch_one(&db.pool)
        .await
        .expect("read branches.price_source")
}

async fn audit_rows(
    db: &TestDb,
) -> Vec<(
    String,
    String,
    Option<serde_json::Value>,
    Option<serde_json::Value>,
)> {
    sqlx::query_as(
        "SELECT actor_role, branch_id, before_redacted, after_redacted FROM audit_events \
         WHERE action = 'branchPriceSourceSet' ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read audit_events")
}

/// An OWNER BoT draft under no policy: out-of-bound, so OWNER's own decision approves it at submit.
async fn approved_draft(db: &TestDb) -> RateDraft {
    let e = db.engine();
    let lines = vec![manual_line("USD", "32.10", "33.00")];
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .create(&mut tx, &main_branch(), &owner_user(), lines)
        .await;
    let d = settle(tx, got).await.expect("the draft is created");
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .submit(&mut tx, &main_branch(), &owner_user(), &d.id, &key())
        .await;
    let outcome = settle(tx, got).await.expect("the submit is accepted");
    assert_eq!(
        outcome.status,
        DraftStatus::Approved,
        "an owner draft is approved at submit"
    );
    d
}

async fn draft_only(db: &TestDb) -> RateDraft {
    let e = db.engine();
    let lines: Vec<DraftLine> = vec![manual_line("USD", "32.10", "33.00")];
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .create(&mut tx, &main_branch(), &owner_user(), lines)
        .await;
    settle(tx, got).await.expect("the draft is created")
}

#[tokio::test]
async fn a_branch_starts_on_bot_drafts() {
    let db = TestDb::start().await;
    assert_eq!(
        source_of(&db, &main_branch()).await,
        Ok(PriceSource::BotDraft)
    );
    db.finish().await;
}

#[tokio::test]
async fn the_owner_switches_a_branch_to_the_board_and_it_is_audited() {
    let db = TestDb::start().await;
    set(&db, &owner_user(), &main_branch(), PriceSource::RateBoard)
        .await
        .expect("OWNER may switch");
    assert_eq!(column(&db, MAIN).await, "RATE_BOARD");
    assert_eq!(
        source_of(&db, &main_branch()).await,
        Ok(PriceSource::RateBoard)
    );

    let rows = audit_rows(&db).await;
    assert_eq!(rows.len(), 1, "one audit event per switch");
    let (role, branch, before, after) = &rows[0];
    assert_eq!(role, "EXECUTIVE");
    assert_eq!(branch, MAIN);
    assert!(
        before
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default()
            .contains("BOT_DRAFT"),
        "before: {before:?}"
    );
    assert!(
        after
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default()
            .contains("RATE_BOARD"),
        "after: {after:?}"
    );
    db.finish().await;
}

#[tokio::test]
async fn only_the_owner_may_switch() {
    let db = TestDb::start().await;
    assert_eq!(
        set(&db, &rates_user(), &main_branch(), PriceSource::RateBoard).await,
        Err("outOfScope")
    );
    assert_eq!(column(&db, MAIN).await, "BOT_DRAFT", "nothing changed");
    assert!(audit_rows(&db).await.is_empty(), "nothing audited");
    db.finish().await;
}

#[tokio::test]
async fn setting_the_current_source_writes_nothing() {
    let db = TestDb::start().await;
    set(&db, &owner_user(), &main_branch(), PriceSource::BotDraft)
        .await
        .expect("the same value is accepted");
    assert_eq!(column(&db, MAIN).await, "BOT_DRAFT");
    assert!(
        audit_rows(&db).await.is_empty(),
        "no audit event for a no-op"
    );
    db.finish().await;
}

#[tokio::test]
async fn an_unknown_branch_is_not_found() {
    let db = TestDb::start().await;
    assert_eq!(
        set(
            &db,
            &owner_user(),
            &BranchId::new("NO-SUCH-BRANCH"),
            PriceSource::RateBoard
        )
        .await,
        Err("notFound")
    );
    db.finish().await;
}

#[tokio::test]
async fn a_bot_draft_cannot_be_published_on_a_board_branch() {
    let db = TestDb::start().await;
    let draft = approved_draft(&db).await;
    let before = db.publications(MAIN).await;
    set(&db, &owner_user(), &main_branch(), PriceSource::RateBoard)
        .await
        .expect("switch");

    let e = db.engine();
    let mut tx = db.begin().await;
    let got = e
        .publisher
        .publish(&mut tx, &main_branch(), &owner_user(), &draft.id, None)
        .await;
    assert_eq!(settle(tx, got).await.map(|_| ()), Err("validationFailed"));
    assert_eq!(
        db.publications(MAIN).await,
        before,
        "no publication written"
    );
    db.finish().await;
}

#[tokio::test]
async fn switching_back_with_set_alone_is_refused() {
    let db = TestDb::start().await;
    set(&db, &owner_user(), &main_branch(), PriceSource::RateBoard)
        .await
        .expect("switch");
    assert_eq!(
        set(&db, &owner_user(), &main_branch(), PriceSource::BotDraft).await,
        Err("validationFailed"),
        "board prices must not keep selling after the board stops: switch back with a draft"
    );
    assert_eq!(column(&db, MAIN).await, "RATE_BOARD");
    db.finish().await;
}

#[tokio::test]
async fn switching_back_publishes_the_draft_in_the_same_transaction() {
    let db = TestDb::start().await;
    let draft = approved_draft(&db).await;
    set(&db, &owner_user(), &main_branch(), PriceSource::RateBoard)
        .await
        .expect("switch");
    let before = db.publications(MAIN).await.len();

    let mut tx = db.begin().await;
    let got = sources(&db)
        .switch_back(&mut tx, &owner_user(), &main_branch(), &draft.id, None)
        .await;
    settle(tx, got)
        .await
        .expect("switch back with an approved draft");

    assert_eq!(column(&db, MAIN).await, "BOT_DRAFT");
    assert_eq!(
        db.publications(MAIN).await.len(),
        before + 1,
        "the draft is published"
    );
    let published_from: Option<String> = sqlx::query_scalar(
        "SELECT p.draft_id::text FROM branches b JOIN rate_publications p ON p.id = b.current_publication_id \
         WHERE b.id = $1",
    )
    .bind(MAIN)
    .fetch_one(&db.pool)
    .await
    .expect("current publication");
    assert_eq!(
        published_from.as_deref(),
        Some(draft.id.to_string().as_str()),
        "it is the current one"
    );
    assert_eq!(
        audit_rows(&db).await.len(),
        2,
        "the switch and the switch back are both audited"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_failed_switch_back_leaves_the_branch_on_the_board() {
    let db = TestDb::start().await;
    let not_approved = draft_only(&db).await;
    set(&db, &owner_user(), &main_branch(), PriceSource::RateBoard)
        .await
        .expect("switch");
    let before = db.publications(MAIN).await;

    let mut tx = db.begin().await;
    let got = sources(&db)
        .switch_back(
            &mut tx,
            &owner_user(),
            &main_branch(),
            &not_approved.id,
            None,
        )
        .await;
    assert!(
        settle(tx, got).await.is_err(),
        "a draft that is not approved cannot be published"
    );
    assert_eq!(
        column(&db, MAIN).await,
        "RATE_BOARD",
        "rolled back together"
    );
    assert_eq!(
        db.publications(MAIN).await,
        before,
        "no publication written"
    );
    db.finish().await;
}

#[tokio::test]
async fn only_the_owner_may_switch_back() {
    let db = TestDb::start().await;
    let draft = approved_draft(&db).await;
    set(&db, &owner_user(), &main_branch(), PriceSource::RateBoard)
        .await
        .expect("switch");
    let mut tx = db.begin().await;
    let got = sources(&db)
        .switch_back(&mut tx, &rates_user(), &main_branch(), &draft.id, None)
        .await;
    assert_eq!(settle(tx, got).await.map(|_| ()), Err("outOfScope"));
    assert_eq!(column(&db, MAIN).await, "RATE_BOARD");
    db.finish().await;
}
