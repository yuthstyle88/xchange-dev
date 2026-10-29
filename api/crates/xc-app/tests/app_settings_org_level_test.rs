//! #5514 (design `docs/design/org-wide-actor-branch.md` §9.4 O-1/O-2, lane controller's ruling on #5514,
//! issuecomment-6495289565): `appSettingsSave` changes a setting of the whole organisation, so it needs an
//! organisation-wide `SystemSettings` row. A branch-scoped ADMIN is refused even at its own session branch.
//! Written before the implementation by the ticket author (claude/2ca5874b). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! Uses `common/mod.rs` as `app_settings_test.rs` does; `common::actor` is the organisation-wide one.

mod common;

use common::*;
use xc_auth::domain::access::actor::Actor;
use xc_auth::domain::access::role::Role;
use xc_auth::domain::access::scoped_role::ScopedRole;
use xc_kernel::ids::BranchId;

/// An ADMIN whose only row is on `MAIN`, signed in at `MAIN`.
fn branch_admin() -> Actor {
    Actor::new(
        "branch-admin",
        ORG,
        vec![ScopedRole::new(
            Role::Admin,
            Some(BranchId::new(MAIN)),
            at(-86_400),
            None,
        )],
        None,
    )
    .with_branch(Some(BranchId::new(MAIN)))
}

#[tokio::test]
async fn a_branch_scoped_admin_in_a_session_is_refused_app_settings_save_and_writes_nothing() {
    let db = TestDb::start().await;
    let service = AppSettingsService::new(db.pool.clone());

    let mut tx = db.begin().await;
    let refused = service
        .save_in(&mut tx, &branch_admin(), 60, at(0))
        .await
        .expect_err("a branch row does not reach an organisation setting");
    tx.commit().await.expect("commit after the refusal");

    assert_eq!(refused.code(), "outOfScope");
    assert_eq!(
        db.settings_row().await,
        (1, Some(604_800), Some("migration".to_owned()))
    );
    db.finish().await;
}

#[tokio::test]
async fn an_organisation_wide_admin_in_a_session_saves_app_settings() {
    let db = TestDb::start().await;
    let service = AppSettingsService::new(db.pool.clone());

    let mut tx = db.begin().await;
    let saved = service
        .save_in(&mut tx, &actor("org-admin", Role::Admin), 60, at(0))
        .await
        .expect("an organisation-wide ADMIN saves");
    tx.commit().await.expect("commit");

    assert_eq!(saved.planner_draft_ttl_seconds.map(|t| t.get()), Some(60));
    db.finish().await;
}
