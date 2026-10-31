//! Staff corrections SC-D2 (`docs/design/staff-corrections.md`): `AttendanceCloseFor` enters the seed at version 7.
//!
//! Written before the implementation by the sub-controller (SC-0, claude/studio-claude-4). Do not edit, delete or
//! weaken these tests; add your own in another file.
//!
//! Version 6 is binding quote's (#5281); the boot sync applies `SEED_ADDITIONS` entries with `v > seed_version`.
//! Binding quote's `seed_additions` also gives a new no-module permission to every custom group of a holder base.
//!
//! What this file pins ([required]):
//! - S1: `AttendanceCloseFor` is in the seed of BRANCH_MANAGER, EXECUTIVE and ADMIN, and of no other built-in group.
//! - S2: `SEED_ADDITIONS` lists it once, at version 7, and `SEED_VERSION` is 7.
//! - S3: a database at seed version 5 gains it on those three groups at boot; every built-in group ends with its full
//!   seed at `SEED_VERSION`.
//! - S4: a second boot inserts nothing.
//! - S5: a BRANCH_MANAGER-based custom group made before gains `AttendanceCloseFor` at boot and nothing else; a
//!   TELLER-based one is unchanged.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.
mod account_common;

use std::collections::BTreeSet;

use account_common::{TestDb, unique};
use xc_auth::domain::access::{Group, Permission, Role, RoleGrantStore, RoleGrants};
use xc_auth::infrastructure::access::PgRoleGrantStore;

use Permission as P;

const HOLDERS: [Role; 3] = [Role::BranchManager, Role::Executive, Role::Admin];

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

/// A database seeded before staff corrections: built-in groups at seed version 5, nobody holds `AttendanceCloseFor`.
async fn database_at_version_5() -> TestDb {
    let db = TestDb::start().await;
    boot(&db).await;
    sqlx::query("DELETE FROM role_permission_grants WHERE permission = $1")
        .bind(serde_name(P::AttendanceCloseFor))
        .execute(&db.pool)
        .await
        .expect("take AttendanceCloseFor off every group");
    sqlx::query("UPDATE roles SET seed_version = 5 WHERE built_in")
        .execute(&db.pool)
        .await
        .expect("set the built-in groups to seed version 5");
    db
}

#[test]
fn s1_only_branch_manager_executive_and_admin_are_seeded_with_attendance_close_for() {
    for role in RoleGrants::BUILT_IN {
        assert_eq!(
            RoleGrants::seed(role).contains(&P::AttendanceCloseFor),
            HOLDERS.contains(&role),
            "{role:?}"
        );
    }
}

#[test]
fn s2_attendance_close_for_enters_the_seed_at_version_7() {
    let entries: Vec<i32> = RoleGrants::SEED_ADDITIONS
        .iter()
        .filter(|&&(_, p)| p == P::AttendanceCloseFor)
        .map(|&(v, _)| v)
        .collect();
    assert_eq!(entries, vec![7]);
    // LA-1 (ledger adjustment, ledger-inventory.md §13.3) raised it to 8 after SC-D2 set 7, #5733 (PricingFormulaSave) to 9;
    // (7, AttendanceCloseFor) is unchanged.
    assert_eq!(RoleGrants::SEED_VERSION, 9);
}

#[tokio::test]
async fn s3_a_version_5_database_gains_attendance_close_for_at_boot() {
    let db = database_at_version_5().await;
    boot(&db).await;
    for role in RoleGrants::BUILT_IN {
        assert_eq!(
            stored_set(&db, role.as_str())
                .await
                .contains(&serde_name(P::AttendanceCloseFor)),
            HOLDERS.contains(&role),
            "{role:?} holds AttendanceCloseFor after boot exactly when it is a holder"
        );
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

#[tokio::test]
async fn s4_a_second_boot_inserts_nothing() {
    let db = database_at_version_5().await;
    boot(&db).await;
    assert_eq!(boot(&db).await, 0, "the second boot inserts nothing");
    db.drop_schema().await;
}

async fn add_group(db: &TestDb, base: Role, set: &[Permission]) -> String {
    let group = Group {
        code: unique("SCACF").to_uppercase(),
        name: "กลุ่มกำหนดเอง".to_owned(),
        base,
        built_in: false,
    };
    let mut tx = db.pool.begin().await.expect("begin");
    PgRoleGrantStore::new()
        .add_group(&mut tx, &group, set)
        .await
        .expect("add_group");
    tx.commit().await.expect("commit");
    group.code
}

#[tokio::test]
async fn s5_a_branch_manager_based_custom_group_gains_it_and_a_teller_based_one_does_not() {
    let db = database_at_version_5().await;
    let bm_held: Vec<Permission> = RoleGrants::seed(Role::BranchManager)
        .into_iter()
        .filter(|&p| p != P::AttendanceCloseFor)
        .collect();
    let bm_based = add_group(&db, Role::BranchManager, &bm_held).await;
    let teller_held = RoleGrants::seed(Role::Teller);
    let teller_based = add_group(&db, Role::Teller, &teller_held).await;

    boot(&db).await;

    let mut want = bm_held.clone();
    want.push(P::AttendanceCloseFor);
    assert_eq!(
        stored_set(&db, &bm_based).await,
        names(&want),
        "a BRANCH_MANAGER-based custom group gains AttendanceCloseFor and nothing else"
    );
    assert_eq!(
        stored_set(&db, &teller_based).await,
        names(&teller_held),
        "a TELLER-based custom group is unchanged"
    );
    db.drop_schema().await;
}
