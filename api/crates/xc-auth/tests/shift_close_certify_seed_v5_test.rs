//! Up-front tests: an existing database picks up #5095's grants at boot (seed version 5).
//!
//! Written before the implementation by the ticket author (claude/bc8fd29a, xchange BE lane controller).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The defect at origin/main: #5095 seeded `ShiftCloseCertify` on EXECUTIVE and ADMIN (`role_grants.rs`) but left
//! `RoleGrants::SEED_VERSION` at 4 with nothing recording the change. `seed_additions` (`role_grant_store.rs:209-211`)
//! skips a built-in group already at `SEED_VERSION`, so a database seeded before #5095 never gets the grant. Measured on
//! xc-v3 build `e13f9163` by claude/studio-claude-1 (#564 issuecomment-6397633949): EXECUTIVE 52 grants, ADMIN 9, no
//! `ShiftCloseCertify` for OWN01.
//!
//! Why a new list and not `SEED_ADDITIONS`: `ShiftCloseCertify` was in the version-1 seed (BRANCH_MANAGER), and the
//! anti-forget gate (`rbac_seed_version_test.rs`) forbids a version-1 permission in `SEED_ADDITIONS`, which records
//! permissions new to the seed. What #5095 did is different: an existing permission seeded on more built-in roles. That
//! is recorded per role in `RoleGrants::SEED_ROLE_ADDITIONS: &[(i32, Role, Permission)]`.
//!
//! Why ADMIN-based custom groups gain it too: guardrail 5 (`matrix_guardrails.rs:24`) refuses a set whose no-module
//! permissions differ from its base's seed, so an ADMIN-based custom group made before #5095 can no longer be saved
//! until it holds `ShiftCloseCertify` — the same reason version 4's no-module kinds reach custom groups (AP-3).
//!
//! What this file pins ([required]):
//! - V1: a database at seed version 4 whose EXECUTIVE and ADMIN lack `ShiftCloseCertify`: after boot both hold it, every
//!   built-in group holds its full seed, and every one is at `RoleGrants::SEED_VERSION`, which is 5.
//! - V2: the version-5 entries of `SEED_ROLE_ADDITIONS` are EXECUTIVE's and ADMIN's `ShiftCloseCertify`, and
//!   `SEED_ADDITIONS` does not list `ShiftCloseCertify`.
//! - V3: a second boot inserts nothing.
//! - V4: a grant removed after the version-5 boot is not given back by a later boot.
//! - V5: an ADMIN-based custom group made before #5095 gains `ShiftCloseCertify` at boot and nothing else; a
//!   BRANCH_MANAGER-based one is unchanged.
//! - V6: every `SEED_ROLE_ADDITIONS` entry is in its role's seed, at a version in `2..=SEED_VERSION`, listed once.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.
mod account_common;

use std::collections::BTreeSet;

use account_common::{TestDb, unique};
use xc_auth::domain::access::{Group, Permission, Role, RoleGrantStore, RoleGrants};
use xc_auth::infrastructure::access::PgRoleGrantStore;

use Permission as P;

fn serde_name(p: Permission) -> String {
    match serde_json::to_value(p).expect("a Permission serialises") {
        serde_json::Value::String(s) => s,
        other => panic!("a Permission serialises to a string, got {other}"),
    }
}

fn names(set: &[Permission]) -> BTreeSet<String> {
    set.iter().copied().map(serde_name).collect()
}

async fn stored_set(db: &TestDb, code: &str) -> BTreeSet<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT permission FROM role_permission_grants WHERE role_code = $1",
    )
    .bind(code)
    .fetch_all(&db.pool)
    .await
    .expect("read role_permission_grants")
    .into_iter()
    .collect()
}

async fn seed_version(db: &TestDb, code: &str) -> i32 {
    sqlx::query_scalar("SELECT seed_version FROM roles WHERE code = $1")
        .bind(code)
        .fetch_one(&db.pool)
        .await
        .expect("read roles.seed_version")
}

/// What startup does (`bootstrap/mod.rs`): `seed_missing`, `seed_removals`, `seed_additions`, committed together.
async fn boot(db: &TestDb) -> u64 {
    let store = PgRoleGrantStore::new();
    let mut tx = db.pool.begin().await.expect("begin");
    store.seed_missing(&mut tx).await.expect("seed_missing");
    store.seed_removals(&mut tx).await.expect("seed_removals");
    let added = store.seed_additions(&mut tx).await.expect("seed_additions");
    tx.commit().await.expect("commit");
    added
}

async fn add_group(db: &TestDb, group: &Group, set: &[Permission]) {
    let mut tx = db.pool.begin().await.expect("begin");
    PgRoleGrantStore::new()
        .add_group(&mut tx, group, set)
        .await
        .expect("add_group");
    tx.commit().await.expect("commit");
}

fn custom(base: Role) -> Group {
    Group {
        code: unique("SCC5").to_uppercase(),
        name: "กลุ่มกำหนดเอง".to_owned(),
        base,
        built_in: false,
    }
}

/// `role`'s seed without `ShiftCloseCertify`: what an EXECUTIVE or ADMIN set looked like before #5095.
fn seed_without_scc(role: Role) -> Vec<Permission> {
    RoleGrants::seed(role)
        .into_iter()
        .filter(|&p| p != P::ShiftCloseCertify)
        .collect()
}

/// A database as xc-v3 stood before this fix: built-in groups at seed version 4, EXECUTIVE and ADMIN without
/// `ShiftCloseCertify`.
async fn database_at_version_4() -> TestDb {
    let db = TestDb::start().await;
    boot(&db).await;
    sqlx::query(
        "DELETE FROM role_permission_grants WHERE permission = $1 AND role_code IN ('EXECUTIVE', 'ADMIN')",
    )
    .bind(serde_name(P::ShiftCloseCertify))
    .execute(&db.pool)
    .await
    .expect("take ShiftCloseCertify off EXECUTIVE and ADMIN");
    sqlx::query("UPDATE roles SET seed_version = 4 WHERE built_in")
        .execute(&db.pool)
        .await
        .expect("set the built-in groups to seed version 4");
    for role in [Role::Executive, Role::Admin] {
        assert_eq!(
            stored_set(&db, role.as_str()).await,
            names(&seed_without_scc(role)),
            "the fixture: {role:?} holds its seed without ShiftCloseCertify"
        );
    }
    db
}

#[tokio::test]
async fn v1_a_version_4_database_gains_shift_close_certify_for_executive_and_admin_at_boot() {
    let db = database_at_version_4().await;
    boot(&db).await;
    for role in [Role::Executive, Role::Admin] {
        assert!(
            stored_set(&db, role.as_str())
                .await
                .contains(&serde_name(P::ShiftCloseCertify)),
            "{role:?} holds ShiftCloseCertify after boot"
        );
    }
    // BQ-7 (binding quote) raised it to 6, staff corrections SC-D2 to 7, LA-1 (ledger adjustment) to 8, #5733 to 9.
    // Restated for #6004 (user-management-rbac.md §11.11): SEED_VERSION is 10.
    // Restated for backdated keying (backdated-keying.md §3): SEED_VERSION is 11 (ExchangeBackdate).
    // Restated for XC Counter kiosk exit (#6569): SEED_VERSION is 12 (CounterUnlock).
    assert_eq!(RoleGrants::SEED_VERSION, 12);
    for role in RoleGrants::BUILT_IN {
        assert_eq!(
            stored_set(&db, role.as_str()).await,
            names(&RoleGrants::seed(role)),
            "{role:?} ends with its full seed"
        );
        assert_eq!(
            seed_version(&db, role.as_str()).await,
            RoleGrants::SEED_VERSION,
            "{role:?} ends at the current seed version"
        );
    }
    db.drop_schema().await;
}

#[test]
fn v2_the_version_5_role_additions_are_executive_and_admin_shift_close_certify() {
    let v5: Vec<(Role, Permission)> = RoleGrants::SEED_ROLE_ADDITIONS
        .iter()
        .filter(|&&(v, _, _)| v == 5)
        .map(|&(_, r, p)| (r, p))
        .collect();
    assert_eq!(
        v5,
        vec![
            (Role::Executive, P::ShiftCloseCertify),
            (Role::Admin, P::ShiftCloseCertify)
        ]
    );
    assert!(
        !RoleGrants::SEED_ADDITIONS
            .iter()
            .any(|&(_, p)| p == P::ShiftCloseCertify),
        "ShiftCloseCertify was in the version-1 seed, so it is not a SEED_ADDITIONS entry"
    );
}

#[tokio::test]
async fn v3_a_second_boot_inserts_nothing() {
    let db = database_at_version_4().await;
    boot(&db).await;
    assert_eq!(boot(&db).await, 0, "the second boot inserts nothing");
    db.drop_schema().await;
}

#[tokio::test]
async fn v4_a_grant_removed_after_the_version_5_boot_is_not_given_back() {
    let db = database_at_version_4().await;
    boot(&db).await;
    sqlx::query(
        "DELETE FROM role_permission_grants WHERE role_code = 'EXECUTIVE' AND permission = $1",
    )
    .bind(serde_name(P::ShiftCloseCertify))
    .execute(&db.pool)
    .await
    .expect("take ShiftCloseCertify off EXECUTIVE");
    boot(&db).await;
    assert!(
        !stored_set(&db, Role::Executive.as_str())
            .await
            .contains(&serde_name(P::ShiftCloseCertify)),
        "a later boot does not give it back"
    );
    db.drop_schema().await;
}

#[tokio::test]
async fn v5_an_admin_based_custom_group_gains_shift_close_certify_and_a_branch_manager_based_one_is_unchanged()
 {
    let db = database_at_version_4().await;
    let admin_based = custom(Role::Admin);
    let admin_held = seed_without_scc(Role::Admin);
    add_group(&db, &admin_based, &admin_held).await;
    let bm_based = custom(Role::BranchManager);
    let bm_held = RoleGrants::seed(Role::BranchManager);
    add_group(&db, &bm_based, &bm_held).await;

    boot(&db).await;

    let mut want = admin_held.clone();
    want.push(P::ShiftCloseCertify);
    assert_eq!(
        stored_set(&db, &admin_based.code).await,
        names(&want),
        "an ADMIN-based custom group gains ShiftCloseCertify and nothing else"
    );
    assert_eq!(
        stored_set(&db, &bm_based.code).await,
        names(&bm_held),
        "a BRANCH_MANAGER-based custom group is unchanged"
    );
    db.drop_schema().await;
}

#[test]
fn v6_every_role_addition_is_in_its_role_s_seed_at_a_valid_version_once() {
    let mut seen = BTreeSet::new();
    for &(v, role, p) in RoleGrants::SEED_ROLE_ADDITIONS {
        assert!(
            RoleGrants::seed(role).contains(&p),
            "{role:?} / {p:?}: a role addition must be in that role's seed"
        );
        assert!(
            (2..=RoleGrants::SEED_VERSION).contains(&v),
            "{role:?} / {p:?} is added in version {v}, outside 2..={}",
            RoleGrants::SEED_VERSION
        );
        assert!(
            seen.insert((role.as_str(), serde_name(p))),
            "{role:?} / {p:?} is listed twice"
        );
    }
}
