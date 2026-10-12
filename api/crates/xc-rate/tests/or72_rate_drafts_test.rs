//! Ticket #1021 (OR-7.2) — rate drafts, publishing and branch price source move to `access::Actor`.
//! Design: `docs/design/blueprint/org-roles-branches.md` §3.3, §4.3, §10 row OR-7;
//! rulings Q1–Q4 on #1021 (issuecomment-6116975975).
//! Written before the implementation by the test-writer (gemini/studio-15, ordered by claude/ad05bf8d).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (see `rate_engine_common/mod.rs`).
//!
//! Behaviour table (§9) coverage:
//! - Row 1: RATE_MANAGER can create a rate draft on MAIN.
//! - Row 2: RATE_MANAGER can edit a rate draft on MAIN.
//! - Row 3: RATE_MANAGER can copy a rate draft on MAIN.
//! - Row 4: RATE_MANAGER can publish an approved rate draft on MAIN.
//! - Row 5: EXECUTIVE with ADMIN can create, edit, copy, and publish a rate draft.
//! - (Rows 7-10 dropped per lane controller ruling: route seam handles RatesDraft check; service checks dropped).
//! - Row 11: SYSTEM rate-feed actor records `created_by` / `published_by` as SYSTEM:rate-feed:BOT.
//! - Row 12: BranchPriceSource: EXECUTIVE org-wide with ADMIN can set rate board (Ruling Q3).
//! - Row 13: BranchPriceSource: EXECUTIVE without ADMIN can set rate board (Ruling Q3).
//! - Row 14: BranchPriceSource: Non-org-wide-EXECUTIVE roles are refused OutOfScope on set.
//! - Row 15: BranchPriceSource: EXECUTIVE org-wide can switch back to BOT draft.
//! - Row 16: BranchPriceSource: Non-org-wide-EXECUTIVE roles are refused OutOfScope on switch back.
//! - Branch-scoped EXECUTIVE is refused OutOfScope on set and switch_back (kills Q3 mutant).
//! - Org-wide ADMIN without EXECUTIVE is refused OutOfScope on set and switch_back (kills 'any org-wide role accepted' mutant).

mod rate_engine_common;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use rate_engine_common::*;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;
use xc_approval::domain::audit_log::AuditLog;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{self, Actor, ScopedRole};
use xc_rate::domain::branch::price_source::{BranchPriceSource, PriceSource};
use xc_rate::domain::draft::rate_draft::{DraftId, DraftStatus};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::infrastructure::branch::price_source::PgBranchPriceSource;

fn executive_user() -> Actor {
    Actor::new(
        "exec-1",
        ORG,
        vec![ScopedRole::new(
            access::Role::Executive,
            None,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

fn branch_scoped_executive() -> Actor {
    Actor::new(
        "exec-b1",
        ORG,
        vec![ScopedRole::new(
            access::Role::Executive,
            Some(main_branch()),
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

fn org_wide_admin_user() -> Actor {
    Actor::new(
        "admin-org-1",
        ORG,
        vec![ScopedRole::new(
            access::Role::Admin,
            None,
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

fn teller_user() -> Actor {
    user_on("teller-1", access::Role::Teller, MAIN)
}

fn finance_user() -> Actor {
    user_on("fin-1", access::Role::Finance, MAIN)
}

fn branch_manager_user() -> Actor {
    user_on("mgr-1", access::Role::BranchManager, MAIN)
}

fn system_feed_user() -> Actor {
    Actor::new(
        "SYSTEM:rate-feed:BOT",
        ORG,
        vec![ScopedRole::new(
            access::Role::Admin,
            Some(main_branch()),
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    )
}

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

async fn seed_draft_in_db(db: &TestDb, branch: &str, who_id: &str) -> DraftId {
    let draft_id = DraftId::generate();
    sqlx::query(
        "INSERT INTO rate_drafts (id, branch_id, status, created_by, version) \
         VALUES ($1, $2, 'DRAFT', $3, 1)",
    )
    .bind(draft_id.as_uuid())
    .bind(branch)
    .bind(who_id)
    .execute(&db.pool)
    .await
    .expect("seed rate_drafts");

    sqlx::query(
        "INSERT INTO rate_draft_lines \
         (id, draft_id, branch_id, currency_code, denomination_group, buy_mode, sell_mode, manual_buy, manual_sell, reason) \
         VALUES (gen_random_uuid(), $1, $2, 'USD', 'ALL', 'MANUAL', 'MANUAL', 32.10, 33.00, 'seeded line')",
    )
    .bind(draft_id.as_uuid())
    .bind(branch)
    .execute(&db.pool)
    .await
    .expect("seed rate_draft_lines");

    draft_id
}

async fn seed_approved_draft_in_db(db: &TestDb, branch: &str, who_id: &str) -> DraftId {
    let draft_id = DraftId::generate();
    let eval_hash = unique("hash");

    sqlx::query(
        "INSERT INTO rate_drafts (id, branch_id, status, created_by, version, evaluation, evaluation_hash) \
         VALUES ($1, $2, 'APPROVED', $3, 1, '{}', $4)",
    )
    .bind(draft_id.as_uuid())
    .bind(branch)
    .bind(who_id)
    .bind(&eval_hash)
    .execute(&db.pool)
    .await
    .expect("seed approved rate_drafts");

    sqlx::query(
        "INSERT INTO rate_draft_lines \
         (id, draft_id, branch_id, currency_code, denomination_group, buy_mode, sell_mode, manual_buy, manual_sell, reason) \
         VALUES (gen_random_uuid(), $1, $2, 'USD', 'ALL', 'MANUAL', 'MANUAL', 32.10, 33.00, 'seeded line')",
    )
    .bind(draft_id.as_uuid())
    .bind(branch)
    .execute(&db.pool)
    .await
    .expect("seed approved rate_draft_lines");

    sqlx::query(
        "INSERT INTO approval_requests \
         (id, type, branch_id, object_id, object_version_hash, policy_version, status, proposed_payload, requested_by, idempotency_key, requested_at) \
         VALUES (gen_random_uuid(), 'RATE_PUBLICATION', $1, $2, $3, '1', 'APPROVED', '{}', $4, $5, now())",
    )
    .bind(branch)
    .bind(draft_id.to_string())
    .bind(&eval_hash)
    .bind(who_id)
    .bind(key())
    .execute(&db.pool)
    .await
    .expect("seed approval_requests for draft");

    draft_id
}

async fn branch_price_source_column(db: &TestDb, branch: &str) -> String {
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

// ---------------------------------------------------------------------------
// Behaviour Table (§9) Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn row_1_rate_manager_can_create_a_rate_draft_on_main() {
    let db = TestDb::start().await;
    let e = db.engine();
    let lines = vec![manual_line("USD", "32.10", "33.00")];

    let mut tx = db.begin().await;
    let got = e
        .drafts
        .create(&mut tx, &main_branch(), &rates_user(), lines)
        .await;
    let draft = settle(tx, got)
        .await
        .expect("RATE_MANAGER creates rate draft on MAIN");

    assert_eq!(draft.status, DraftStatus::Draft);
    assert_eq!(draft.version, 1);

    let created_by: String = sqlx::query_scalar("SELECT created_by FROM rate_drafts WHERE id = $1")
        .bind(draft.id.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("read created_by");
    assert_eq!(created_by, rates_user().user_id);

    db.finish().await;
}

#[tokio::test]
async fn row_2_rate_manager_can_edit_a_rate_draft_on_main() {
    let db = TestDb::start().await;
    let draft_id = seed_draft_in_db(&db, MAIN, &rates_user().user_id).await;
    let e = db.engine();
    let new_lines = vec![manual_line("USD", "32.50", "33.50")];

    let mut tx = db.begin().await;
    let got = e
        .drafts
        .edit(
            &mut tx,
            &main_branch(),
            &rates_user(),
            &draft_id,
            1,
            new_lines,
        )
        .await;
    let edited = settle(tx, got)
        .await
        .expect("RATE_MANAGER edits rate draft on MAIN");

    assert_eq!(edited.version, 2);
    assert_eq!(edited.status, DraftStatus::Draft);

    db.finish().await;
}

#[tokio::test]
async fn row_3_rate_manager_can_copy_a_rate_draft_on_main() {
    let db = TestDb::start().await;
    let draft_id = seed_draft_in_db(&db, MAIN, &rates_user().user_id).await;
    let e = db.engine();

    let mut tx = db.begin().await;
    let got = e
        .drafts
        .copy(&mut tx, &main_branch(), &rates_user(), &draft_id)
        .await;
    let copied = settle(tx, got)
        .await
        .expect("RATE_MANAGER copies rate draft on MAIN");

    assert_ne!(copied.id, draft_id);
    assert_eq!(copied.version, 1);
    assert_eq!(copied.status, DraftStatus::Draft);

    let created_by: String = sqlx::query_scalar("SELECT created_by FROM rate_drafts WHERE id = $1")
        .bind(copied.id.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("read created_by");
    assert_eq!(created_by, rates_user().user_id);

    db.finish().await;
}

#[tokio::test]
async fn row_4_rate_manager_can_publish_an_approved_rate_draft_on_main() {
    let db = TestDb::start().await;
    let draft_id = seed_approved_draft_in_db(&db, MAIN, &rates_user().user_id).await;
    let e = db.engine();

    let mut tx = db.begin().await;
    let got = e
        .publisher
        .publish(&mut tx, &main_branch(), &rates_user(), &draft_id, None)
        .await;
    let pub_ref = settle(tx, got)
        .await
        .expect("RATE_MANAGER publishes approved draft on MAIN");

    assert_eq!(pub_ref.version, 1);

    let published_by: String =
        sqlx::query_scalar("SELECT published_by FROM rate_publications WHERE id = $1")
            .bind(pub_ref.id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("read published_by");
    assert_eq!(published_by, rates_user().user_id);

    let current_pub: Option<Uuid> =
        sqlx::query_scalar("SELECT current_publication_id FROM branches WHERE id = $1")
            .bind(MAIN)
            .fetch_one(&db.pool)
            .await
            .expect("read current_publication_id");
    assert_eq!(current_pub, Some(pub_ref.id.as_uuid()));

    db.finish().await;
}

#[tokio::test]
async fn row_5_executive_with_admin_can_create_edit_copy_and_publish_a_rate_draft() {
    let db = TestDb::start().await;
    let e = db.engine();
    let owner = owner_user();

    // 1. Create
    let mut tx = db.begin().await;
    let lines = vec![manual_line("USD", "32.10", "33.00")];
    let got = e
        .drafts
        .create(&mut tx, &main_branch(), &owner, lines)
        .await;
    let draft = settle(tx, got)
        .await
        .expect("EXECUTIVE with ADMIN creates draft");
    assert_eq!(draft.version, 1);

    let created_by: String = sqlx::query_scalar("SELECT created_by FROM rate_drafts WHERE id = $1")
        .bind(draft.id.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("read created_by");
    assert_eq!(created_by, owner.user_id);

    // 2. Edit
    let mut tx = db.begin().await;
    let new_lines = vec![manual_line("USD", "32.40", "33.20")];
    let got = e
        .drafts
        .edit(&mut tx, &main_branch(), &owner, &draft.id, 1, new_lines)
        .await;
    let edited = settle(tx, got)
        .await
        .expect("EXECUTIVE with ADMIN edits draft");
    assert_eq!(edited.version, 2);

    // 3. Copy
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .copy(&mut tx, &main_branch(), &owner, &draft.id)
        .await;
    let copied = settle(tx, got)
        .await
        .expect("EXECUTIVE with ADMIN copies draft");
    assert_ne!(copied.id, draft.id);

    // 4. Publish approved draft
    let approved_id = seed_approved_draft_in_db(&db, MAIN, &owner.user_id).await;
    let mut tx = db.begin().await;
    let got = e
        .publisher
        .publish(&mut tx, &main_branch(), &owner, &approved_id, None)
        .await;
    let pub_ref = settle(tx, got)
        .await
        .expect("EXECUTIVE with ADMIN publishes approved draft");
    assert_eq!(pub_ref.version, 1);

    let published_by: String =
        sqlx::query_scalar("SELECT published_by FROM rate_publications WHERE id = $1")
            .bind(pub_ref.id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("read published_by");
    assert_eq!(published_by, owner.user_id);

    db.finish().await;
}

#[tokio::test]
async fn row_6_executive_without_admin_can_create_edit_copy_and_publish_a_rate_draft() {
    // Ruling Q4: EXECUTIVE without ADMIN was refused by legacy_actor bridge with roleNotAllowedHere;
    // in OR-7.2 it is now ALLOWED per ruling 3 and permission table.
    let db = TestDb::start().await;
    let e = db.engine();
    let exec = executive_user();

    // 1. Create
    let mut tx = db.begin().await;
    let lines = vec![manual_line("USD", "32.10", "33.00")];
    let got = e.drafts.create(&mut tx, &main_branch(), &exec, lines).await;
    let draft = settle(tx, got)
        .await
        .expect("EXECUTIVE without ADMIN creates draft");
    assert_eq!(draft.version, 1);

    let created_by: String = sqlx::query_scalar("SELECT created_by FROM rate_drafts WHERE id = $1")
        .bind(draft.id.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("read created_by");
    assert_eq!(created_by, exec.user_id);

    // 2. Edit
    let mut tx = db.begin().await;
    let new_lines = vec![manual_line("USD", "32.40", "33.20")];
    let got = e
        .drafts
        .edit(&mut tx, &main_branch(), &exec, &draft.id, 1, new_lines)
        .await;
    let edited = settle(tx, got)
        .await
        .expect("EXECUTIVE without ADMIN edits draft");
    assert_eq!(edited.version, 2);

    // 3. Copy
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .copy(&mut tx, &main_branch(), &exec, &draft.id)
        .await;
    let copied = settle(tx, got)
        .await
        .expect("EXECUTIVE without ADMIN copies draft");
    assert_ne!(copied.id, draft.id);

    // 4. Publish approved draft
    let approved_id = seed_approved_draft_in_db(&db, MAIN, &exec.user_id).await;
    let mut tx = db.begin().await;
    let got = e
        .publisher
        .publish(&mut tx, &main_branch(), &exec, &approved_id, None)
        .await;
    let pub_ref = settle(tx, got)
        .await
        .expect("EXECUTIVE without ADMIN publishes approved draft");
    assert_eq!(pub_ref.version, 1);

    let published_by: String =
        sqlx::query_scalar("SELECT published_by FROM rate_publications WHERE id = $1")
            .bind(pub_ref.id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("read published_by");
    assert_eq!(published_by, exec.user_id);

    db.finish().await;
}

#[tokio::test]
async fn row_11_system_rate_feed_actor_records_system_rate_feed_bot() {
    let db = TestDb::start().await;
    let e = db.engine();
    let system = system_feed_user();

    // 1. Create with system actor
    let mut tx = db.begin().await;
    let lines = vec![manual_line("USD", "32.10", "33.00")];
    let got = e
        .drafts
        .create(&mut tx, &main_branch(), &system, lines)
        .await;
    let draft = settle(tx, got).await.expect("system creates draft");
    let created_by: String = sqlx::query_scalar("SELECT created_by FROM rate_drafts WHERE id = $1")
        .bind(draft.id.as_uuid())
        .fetch_one(&db.pool)
        .await
        .expect("read created_by");
    assert_eq!(created_by, "SYSTEM:rate-feed:BOT");

    // 2. Copy with system actor
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .copy(&mut tx, &main_branch(), &system, &draft.id)
        .await;
    let copied = settle(tx, got).await.expect("system copies draft");
    let copied_created_by: String =
        sqlx::query_scalar("SELECT created_by FROM rate_drafts WHERE id = $1")
            .bind(copied.id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("read created_by");
    assert_eq!(copied_created_by, "SYSTEM:rate-feed:BOT");

    // 3. Publish with system actor
    let approved_id = seed_approved_draft_in_db(&db, MAIN, "SYSTEM:rate-feed:BOT").await;
    let mut tx = db.begin().await;
    let got = e
        .publisher
        .publish(&mut tx, &main_branch(), &system, &approved_id, None)
        .await;
    let pub_ref = settle(tx, got).await.expect("system publishes draft");
    let published_by: String =
        sqlx::query_scalar("SELECT published_by FROM rate_publications WHERE id = $1")
            .bind(pub_ref.id.as_uuid())
            .fetch_one(&db.pool)
            .await
            .expect("read published_by");
    assert_eq!(published_by, "SYSTEM:rate-feed:BOT");

    db.finish().await;
}

#[tokio::test]
async fn row_12_branch_price_source_executive_org_wide_with_admin_can_set_rate_board() {
    let db = TestDb::start().await;
    let s = sources(&db);
    let owner = owner_user();

    let mut tx = db.begin().await;
    let got = s
        .set(&mut tx, &owner, &main_branch(), PriceSource::RateBoard)
        .await;
    settle(tx, got)
        .await
        .expect("EXECUTIVE org-wide with ADMIN sets RATE_BOARD");

    assert_eq!(branch_price_source_column(&db, MAIN).await, "RATE_BOARD");

    let rows = audit_rows(&db).await;
    assert_eq!(rows.len(), 1, "one audit event recorded");
    let (role, branch, before, after) = &rows[0];
    assert_eq!(role, "EXECUTIVE");
    assert_eq!(branch, MAIN);
    assert!(
        before
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default()
            .contains("BOT_DRAFT")
    );
    assert!(
        after
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default()
            .contains("RATE_BOARD")
    );

    db.finish().await;
}

#[tokio::test]
async fn row_13_branch_price_source_executive_without_admin_can_set_rate_board() {
    // Ruling Q3: EXECUTIVE without ADMIN org-wide can set branch price source (mirrors #1020 ruling 3).
    let db = TestDb::start().await;
    let s = sources(&db);
    let exec = executive_user();

    let mut tx = db.begin().await;
    let got = s
        .set(&mut tx, &exec, &main_branch(), PriceSource::RateBoard)
        .await;
    settle(tx, got)
        .await
        .expect("EXECUTIVE without ADMIN sets RATE_BOARD");

    assert_eq!(branch_price_source_column(&db, MAIN).await, "RATE_BOARD");

    let rows = audit_rows(&db).await;
    assert_eq!(rows.len(), 1, "one audit event recorded");
    let (role, branch, before, after) = &rows[0];
    assert_eq!(role, "EXECUTIVE");
    assert_eq!(branch, MAIN);
    assert!(
        before
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default()
            .contains("BOT_DRAFT")
    );
    assert!(
        after
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default()
            .contains("RATE_BOARD")
    );

    db.finish().await;
}

#[tokio::test]
async fn row_14_branch_price_source_non_executive_roles_are_refused_out_of_scope_on_set() {
    let db = TestDb::start().await;
    let s = sources(&db);

    for (who, label) in [
        (rates_user(), "RATE_MANAGER"),
        (teller_user(), "TELLER"),
        (finance_user(), "FINANCE"),
        (branch_manager_user(), "BRANCH_MANAGER"),
        (branch_scoped_executive(), "branch-scoped EXECUTIVE"),
        (org_wide_admin_user(), "org-wide ADMIN"),
    ] {
        let mut tx = db.begin().await;
        let got = s
            .set(&mut tx, &who, &main_branch(), PriceSource::RateBoard)
            .await;
        assert_eq!(
            settle(tx, got).await.map(|_| ()),
            Err("outOfScope"),
            "{label} must be refused OutOfScope on set"
        );
        assert_eq!(branch_price_source_column(&db, MAIN).await, "BOT_DRAFT");
    }

    assert!(
        audit_rows(&db).await.is_empty(),
        "nothing audited on refused set"
    );
    db.finish().await;
}

#[tokio::test]
async fn row_15_branch_price_source_executive_org_wide_can_switch_back_to_bot_draft() {
    let db = TestDb::start().await;
    let s = sources(&db);
    let exec = executive_user();

    // 1. Put branch on RATE_BOARD
    sqlx::query("UPDATE branches SET price_source = 'RATE_BOARD' WHERE id = $1")
        .bind(MAIN)
        .execute(&db.pool)
        .await
        .expect("set branch to RATE_BOARD");

    let approved_draft_id = seed_approved_draft_in_db(&db, MAIN, &exec.user_id).await;

    // 2. Switch back with approved draft
    let mut tx = db.begin().await;
    let got = s
        .switch_back(&mut tx, &exec, &main_branch(), &approved_draft_id, None)
        .await;
    let pub_ref = settle(tx, got)
        .await
        .expect("EXECUTIVE org-wide switches back to BOT_DRAFT");

    assert_eq!(branch_price_source_column(&db, MAIN).await, "BOT_DRAFT");
    assert_eq!(pub_ref.version, 1);

    let rows = audit_rows(&db).await;
    assert_eq!(rows.len(), 1, "one audit event recorded for switch_back");
    let (role, branch, before, after) = &rows[0];
    assert_eq!(role, "EXECUTIVE");
    assert_eq!(branch, MAIN);
    assert!(
        before
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default()
            .contains("RATE_BOARD")
    );
    assert!(
        after
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_default()
            .contains("BOT_DRAFT")
    );

    db.finish().await;
}

#[tokio::test]
async fn row_16_branch_price_source_non_executive_roles_are_refused_out_of_scope_on_switch_back() {
    let db = TestDb::start().await;
    let s = sources(&db);

    sqlx::query("UPDATE branches SET price_source = 'RATE_BOARD' WHERE id = $1")
        .bind(MAIN)
        .execute(&db.pool)
        .await
        .expect("set branch to RATE_BOARD");

    let approved_draft_id = seed_approved_draft_in_db(&db, MAIN, &rates_user().user_id).await;

    for (who, label) in [
        (rates_user(), "RATE_MANAGER"),
        (teller_user(), "TELLER"),
        (finance_user(), "FINANCE"),
        (branch_manager_user(), "BRANCH_MANAGER"),
        (branch_scoped_executive(), "branch-scoped EXECUTIVE"),
        (org_wide_admin_user(), "org-wide ADMIN"),
    ] {
        let mut tx = db.begin().await;
        let got = s
            .switch_back(&mut tx, &who, &main_branch(), &approved_draft_id, None)
            .await;
        assert_eq!(
            settle(tx, got).await.map(|_| ()),
            Err("outOfScope"),
            "{label} must be refused OutOfScope on switch_back"
        );
        assert_eq!(branch_price_source_column(&db, MAIN).await, "RATE_BOARD");
    }

    assert!(
        audit_rows(&db).await.is_empty(),
        "nothing audited on refused switch_back"
    );
    db.finish().await;
}

#[tokio::test]
async fn branch_scoped_executive_is_refused_out_of_scope_on_set() {
    let db = TestDb::start().await;
    let s = sources(&db);
    let exec = branch_scoped_executive();

    let mut tx = db.begin().await;
    let got = s
        .set(&mut tx, &exec, &main_branch(), PriceSource::RateBoard)
        .await;
    assert_eq!(
        settle(tx, got).await.map(|_| ()),
        Err("outOfScope"),
        "branch-scoped EXECUTIVE must be refused OutOfScope on set"
    );
    assert_eq!(branch_price_source_column(&db, MAIN).await, "BOT_DRAFT");
    assert!(
        audit_rows(&db).await.is_empty(),
        "nothing audited on refused set"
    );
    db.finish().await;
}

#[tokio::test]
async fn branch_scoped_executive_is_refused_out_of_scope_on_switch_back() {
    let db = TestDb::start().await;
    let s = sources(&db);
    let exec = branch_scoped_executive();

    sqlx::query("UPDATE branches SET price_source = 'RATE_BOARD' WHERE id = $1")
        .bind(MAIN)
        .execute(&db.pool)
        .await
        .expect("set branch to RATE_BOARD");

    let approved_draft_id = seed_approved_draft_in_db(&db, MAIN, &exec.user_id).await;

    let mut tx = db.begin().await;
    let got = s
        .switch_back(&mut tx, &exec, &main_branch(), &approved_draft_id, None)
        .await;
    assert_eq!(
        settle(tx, got).await.map(|_| ()),
        Err("outOfScope"),
        "branch-scoped EXECUTIVE must be refused OutOfScope on switch_back"
    );
    assert_eq!(branch_price_source_column(&db, MAIN).await, "RATE_BOARD");
    assert!(
        audit_rows(&db).await.is_empty(),
        "nothing audited on refused switch_back"
    );
    db.finish().await;
}

#[tokio::test]
async fn org_wide_admin_without_executive_is_refused_out_of_scope_on_set_and_switch_back() {
    let db = TestDb::start().await;
    let s = sources(&db);
    let admin = org_wide_admin_user();

    // 1. set
    let mut tx = db.begin().await;
    let got = s
        .set(&mut tx, &admin, &main_branch(), PriceSource::RateBoard)
        .await;
    assert_eq!(
        settle(tx, got).await.map(|_| ()),
        Err("outOfScope"),
        "org-wide ADMIN must be refused OutOfScope on set"
    );
    assert_eq!(branch_price_source_column(&db, MAIN).await, "BOT_DRAFT");

    // 2. switch_back
    sqlx::query("UPDATE branches SET price_source = 'RATE_BOARD' WHERE id = $1")
        .bind(MAIN)
        .execute(&db.pool)
        .await
        .expect("set branch to RATE_BOARD");

    let approved_draft_id = seed_approved_draft_in_db(&db, MAIN, &admin.user_id).await;

    let mut tx = db.begin().await;
    let got = s
        .switch_back(&mut tx, &admin, &main_branch(), &approved_draft_id, None)
        .await;
    assert_eq!(
        settle(tx, got).await.map(|_| ()),
        Err("outOfScope"),
        "org-wide ADMIN must be refused OutOfScope on switch_back"
    );
    assert_eq!(branch_price_source_column(&db, MAIN).await, "RATE_BOARD");

    assert!(
        audit_rows(&db).await.is_empty(),
        "nothing audited on refused set and switch_back"
    );
    db.finish().await;
}
