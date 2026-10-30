//! #5618 — an organisation-wide EXECUTIVE manages denomination groups, as the owner does
//! (design `docs/design/org-wide-actor-branch.md` §12, rows D-1..D-5).
//! Written before the implementation by the sub-controller (claude/studio-claude-17). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! The rule: "only OWNER may manage denomination groups" (`rate-feed-connector.md` §3.2, R-3) is
//! restated as "an organisation-wide EXECUTIVE", the rule #1020 ruling 3 and #1021 ruling 3 already
//! wrote for `BranchPriceSource` (`price_source.rs`). The owner (EXECUTIVE + ADMIN) still passes;
//! a RATE_MANAGER is still refused (`denomination_groups_test::only_the_owner_may_change_groups`).
//!
//! Needs `TEST_DATABASE_URL` (see `rate_engine_common/mod.rs`).

mod rate_engine_common;

use std::sync::Arc;

use chrono::{DateTime, Utc};
use rate_engine_common::*;
use sqlx::{Postgres, Transaction};
use xc_approval::domain::audit_log::AuditLog;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access::{self, Actor, ScopedRole};
use xc_kernel::ids::BranchId;
use xc_rate::application::currency::denomination_group_service::DenominationGroupService;
use xc_rate::domain::currency::denomination_groups::{DenominationGroups, GroupCode};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::infrastructure::currency::denomination_groups::PgDenominationGroups;

const BOARD: &str = "RATE_BOARD";

fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

/// EXECUTIVE organisation-wide, with no ADMIN: `Standing::of` answers `None` for it.
fn executive_alone() -> Actor {
    Actor::new(
        "exec-alone",
        ORG,
        vec![ScopedRole::new(
            access::Role::Executive,
            None,
            epoch(),
            None,
        )],
        None,
    )
}

/// EXECUTIVE at one branch only. `ScopeRule` refuses to grant it, but the service must not read
/// it as organisation-wide either.
fn executive_at_main() -> Actor {
    Actor::new(
        "exec-main",
        ORG,
        vec![ScopedRole::new(
            access::Role::Executive,
            Some(BranchId::new(MAIN)),
            epoch(),
            None,
        )],
        None,
    )
}

fn group(text: &str) -> GroupCode {
    GroupCode::new(text).expect("a valid group code")
}

fn service(db: &TestDb) -> DenominationGroupService {
    let groups: Arc<dyn DenominationGroups> = Arc::new(PgDenominationGroups::new(db.pool.clone()));
    let audit: Arc<dyn AuditLog> = Arc::new(PgAuditLog::new(db.pool.clone()));
    DenominationGroupService::new(groups, audit)
}

async fn settle(
    tx: Transaction<'static, Postgres>,
    result: Result<(), RateEngineError>,
) -> Result<(), &'static str> {
    match result {
        Ok(()) => {
            tx.commit().await.expect("commit");
            Ok(())
        }
        Err(e) => {
            tx.rollback().await.expect("roll back");
            Err(e.code())
        }
    }
}

async fn create(db: &TestDb, who: &Actor, g: &str) -> Result<(), &'static str> {
    let mut tx = db.begin().await;
    let got = service(db)
        .create_group(&mut tx, who, code("USD"), &group(g))
        .await;
    settle(tx, got).await
}

async fn map(db: &TestDb, who: &Actor, g: &str, key: &str) -> Result<(), &'static str> {
    let mut tx = db.begin().await;
    let got = service(db)
        .map_group(&mut tx, who, code("USD"), &group(g), BOARD, key)
        .await;
    settle(tx, got).await
}

async fn remove(db: &TestDb, who: &Actor, g: &str) -> Result<(), &'static str> {
    let mut tx = db.begin().await;
    let got = service(db)
        .remove_group(&mut tx, who, code("USD"), &group(g))
        .await;
    settle(tx, got).await
}

/// `(action, actor_id, actor_role)` of every group audit event, oldest first.
async fn audits(db: &TestDb) -> Vec<(String, String, String)> {
    sqlx::query_as(
        "SELECT action, actor_id, actor_role FROM audit_events \
         WHERE action IN ('denominationGroupCreate', 'denominationGroupMap', 'denominationGroupRemove') \
         ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read audit_events")
}

fn row(action: &str, id: &str, role: &str) -> (String, String, String) {
    (action.to_owned(), id.to_owned(), role.to_owned())
}

/// D-1: an organisation-wide EXECUTIVE alone creates, maps and removes a group; each change is
/// audited as `EXECUTIVE`.
#[tokio::test]
async fn d1_an_executive_alone_manages_groups_audited_as_executive() {
    let db = TestDb::start().await;
    let exec = executive_alone();

    assert_eq!(create(&db, &exec, "LARGE").await, Ok(()));
    assert_eq!(map(&db, &exec, "LARGE", "USD1").await, Ok(()));
    assert_eq!(remove(&db, &exec, "LARGE").await, Ok(()));

    assert_eq!(
        audits(&db).await,
        vec![
            row("denominationGroupCreate", "exec-alone", "EXECUTIVE"),
            row("denominationGroupMap", "exec-alone", "EXECUTIVE"),
            row("denominationGroupRemove", "exec-alone", "EXECUTIVE"),
        ]
    );
    db.finish().await;
}

/// D-2: the owner (EXECUTIVE + ADMIN) is unchanged: allowed, audited as `EXECUTIVE` (`OWNER` until #5630).
#[tokio::test]
async fn d2_the_owner_is_unchanged_and_audited_as_owner() {
    let db = TestDb::start().await;
    assert_eq!(create(&db, &owner(), "LARGE").await, Ok(()));
    let got = audits(&db).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, "denominationGroupCreate");
    // #5630 (§14.5, rulings on #5661), restated by claude/studio-claude-17: was V.9's `OWNER`.
    assert_eq!(got[0].2, "EXECUTIVE");
    db.finish().await;
}

/// D-3: a branch-scoped EXECUTIVE is not organisation-wide: `outOfScope`, nothing written.
#[tokio::test]
async fn d3_a_branch_scoped_executive_is_out_of_scope() {
    let db = TestDb::start().await;
    assert_eq!(
        create(&db, &executive_at_main(), "LARGE").await,
        Err("outOfScope")
    );
    assert!(audits(&db).await.is_empty());
    db.finish().await;
}

/// D-4: a RATE_MANAGER is still refused on all three (today's answer, kept).
#[tokio::test]
async fn d4_a_rate_manager_is_still_out_of_scope() {
    let db = TestDb::start().await;
    create(&db, &owner(), "LARGE").await.expect("owner creates");
    let before = audits(&db).await.len();
    assert_eq!(create(&db, &rates(), "SMALL").await, Err("outOfScope"));
    assert_eq!(map(&db, &rates(), "LARGE", "USD1").await, Err("outOfScope"));
    assert_eq!(remove(&db, &rates(), "LARGE").await, Err("outOfScope"));
    assert_eq!(audits(&db).await.len(), before);
    db.finish().await;
}

/// D-5: an org-wide ADMIN without EXECUTIVE is refused (kills an "any org-wide role" mutant).
#[tokio::test]
async fn d5_an_admin_without_executive_is_out_of_scope() {
    let db = TestDb::start().await;
    let admin = Actor::new(
        "admin-alone",
        ORG,
        vec![ScopedRole::new(access::Role::Admin, None, epoch(), None)],
        None,
    );
    assert_eq!(create(&db, &admin, "LARGE").await, Err("outOfScope"));
    db.finish().await;
}
