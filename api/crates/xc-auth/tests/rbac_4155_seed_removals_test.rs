//! Up-front tests for #4155, the owner's SEED_REMOVALS choice (relayed by claude/553782): groups already seeded on a
//! host lose the DASHBOARD grants that ruling W + A1 takes out of `RoleGrants::seed`, at boot, with no migration.
//! Written before the implementation by AG controller 4093 (claude/studio-claude-7). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Names these tests assume ([required], they do not exist on `main`, so this file does not compile until then):
//! - `RoleGrants::SEED_VERSION == 3`;
//! - `RoleGrants::SEED_REMOVALS: &[(i32, Permission)]`, the mirror of `SEED_ADDITIONS`:
//!   `[(3, OverviewRead), (3, AlertSettingsSave)]`;
//! - `RoleGrantStore::seed_removals(&mut Transaction<'_, Postgres>) -> Result<u64, RoleGrantStoreError>`: for each
//!   built-in group with `seed_version < SEED_VERSION`, delete every `(v, p)` of `SEED_REMOVALS` with
//!   `v > seed_version` and `p ∉ RoleGrants::seed(role)`; it does not change `seed_version` (`seed_additions` raises
//!   it afterwards); bump `role_grant_version` once if anything was deleted; answer how many rows it deleted.
//! - Boot (`api/src/bootstrap/mod.rs`) runs `seed_missing`, then `seed_removals`, then `seed_additions`, in one
//!   transaction.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Without it these tests
//! FAIL; none skips.

mod account_common;

use std::collections::BTreeSet;

use account_common::{TestDb, unique};
use xc_auth::domain::access::{Group, Permission, Role, RoleGrantStore, RoleGrants};
use xc_auth::infrastructure::access::PgRoleGrantStore;

/// The DASHBOARD grants of seed version 2 that W + A1 takes away: measured on xc-v3 2026-10-21T04:08Z
/// (#4155 issuecomment-6307814002), less EXECUTIVE's two, which stay. 10 rows.
const VERSION_2_DASHBOARD_EXTRAS: [(Role, Permission); 10] = [
    (Role::Admin, Permission::OverviewRead),
    (Role::RateManager, Permission::OverviewRead),
    (Role::RateManager, Permission::AlertSettingsSave),
    (Role::BranchManager, Permission::OverviewRead),
    (Role::BranchManager, Permission::AlertSettingsSave),
    (Role::Teller, Permission::OverviewRead),
    (Role::Finance, Permission::OverviewRead),
    (Role::Finance, Permission::AlertSettingsSave),
    (Role::VerificationReviewer, Permission::OverviewRead),
    (Role::VerificationReviewer, Permission::AlertSettingsSave),
];

fn serde_name(p: Permission) -> String {
    match serde_json::to_value(p).expect("a Permission serialises") {
        serde_json::Value::String(s) => s,
        other => panic!("a Permission serialises to a string, got {other}"),
    }
}

fn names(set: &[Permission]) -> BTreeSet<String> {
    set.iter().copied().map(serde_name).collect()
}

async fn grant_version(db: &TestDb) -> i64 {
    sqlx::query_scalar("SELECT version FROM role_grant_version")
        .fetch_one(&db.pool)
        .await
        .expect("read role_grant_version")
}

async fn seed_version(db: &TestDb, code: &str) -> i32 {
    sqlx::query_scalar("SELECT seed_version FROM roles WHERE code = $1")
        .bind(code)
        .fetch_one(&db.pool)
        .await
        .expect("read roles.seed_version")
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

async fn grant(db: &TestDb, code: &str, p: Permission) {
    sqlx::query(
        "INSERT INTO role_permission_grants (role_code, permission) VALUES ($1, $2) \
         ON CONFLICT (role_code, permission) DO NOTHING",
    )
    .bind(code)
    .bind(serde_name(p))
    .execute(&db.pool)
    .await
    .expect("insert a grant");
}

async fn seed_missing(db: &TestDb) {
    let mut tx = db.pool.begin().await.expect("begin");
    PgRoleGrantStore::new()
        .seed_missing(&mut tx)
        .await
        .expect("seed_missing");
    tx.commit().await.expect("commit");
}

/// One boot's upgrade, in boot's order: removals, then additions.
async fn upgrade(db: &TestDb) -> (u64, u64) {
    let mut tx = db.pool.begin().await.expect("begin");
    let store = PgRoleGrantStore::new();
    let removed = store.seed_removals(&mut tx).await.expect("seed_removals");
    let added = store.seed_additions(&mut tx).await.expect("seed_additions");
    tx.commit().await.expect("commit");
    (removed, added)
}

/// A database as xc-v3 stands: every built-in group at seed version 2, holding today's seed plus the version-2
/// DASHBOARD extras.
async fn database_at_version_2() -> TestDb {
    let db = TestDb::start().await;
    seed_missing(&db).await;
    for (role, p) in VERSION_2_DASHBOARD_EXTRAS {
        grant(&db, role.as_str(), p).await;
    }
    sqlx::query("UPDATE roles SET seed_version = 2 WHERE built_in")
        .execute(&db.pool)
        .await
        .expect("set the built-in groups to seed version 2");
    db
}

#[test]
fn seed_removals_holds_exactly_the_two_dashboard_permissions_at_version_3() {
    assert_eq!(RoleGrants::SEED_VERSION, 3);
    assert_eq!(
        RoleGrants::SEED_REMOVALS,
        &[
            (3, Permission::OverviewRead),
            (3, Permission::AlertSettingsSave)
        ]
    );
    for &(_, p) in RoleGrants::SEED_REMOVALS {
        assert!(
            !RoleGrants::SEED_ADDITIONS.iter().any(|&(_, a)| a == p),
            "{p:?} is both added and removed"
        );
    }
}

/// An old-version database loses exactly the version-2 extras, every group ends with today's seed, EXECUTIVE keeps
/// both permissions, and `role_grant_version` moves.
#[tokio::test]
async fn an_old_version_database_loses_exactly_the_removed_pairs() {
    let db = database_at_version_2().await;
    let version_before = grant_version(&db).await;
    let mut before: Vec<(Role, BTreeSet<String>)> = Vec::new();
    for role in RoleGrants::BUILT_IN {
        before.push((role, stored_set(&db, role.as_str()).await));
    }

    let (removed, _) = upgrade(&db).await;

    assert_eq!(
        removed,
        VERSION_2_DASHBOARD_EXTRAS.len() as u64,
        "10 rows, as on xc-v3"
    );
    for role in RoleGrants::BUILT_IN {
        let code = role.as_str();
        let after = stored_set(&db, code).await;
        let lost: BTreeSet<String> = before
            .iter()
            .find(|(r, _)| *r == role)
            .expect("a before set")
            .1
            .difference(&after)
            .cloned()
            .collect();
        let expected: BTreeSet<String> = VERSION_2_DASHBOARD_EXTRAS
            .iter()
            .filter(|(r, _)| *r == role)
            .map(|&(_, p)| serde_name(p))
            .collect();
        assert_eq!(
            lost, expected,
            "{code} loses exactly its version-2 DASHBOARD extras"
        );
        assert_eq!(
            after,
            names(&RoleGrants::seed(role)),
            "{code} ends with today's seed"
        );
        assert_eq!(
            seed_version(&db, code).await,
            RoleGrants::SEED_VERSION,
            "{code}"
        );
    }
    let exec = stored_set(&db, Role::Executive.as_str()).await;
    assert!(exec.contains(&serde_name(Permission::OverviewRead)));
    assert!(exec.contains(&serde_name(Permission::AlertSettingsSave)));
    assert!(
        grant_version(&db).await > version_before,
        "the snapshot version moves"
    );
}

/// The second boot is a no-op: nothing removed, nothing added, no version bump.
#[tokio::test]
async fn a_second_boot_removes_nothing() {
    let db = database_at_version_2().await;
    upgrade(&db).await;
    let version_after_first = grant_version(&db).await;

    let (removed, added) = upgrade(&db).await;

    assert_eq!((removed, added), (0, 0));
    assert_eq!(
        grant_version(&db).await,
        version_after_first,
        "no second bump"
    );
}

/// A built-in group already at the new seed version keeps what a person granted it, even a removed permission.
#[tokio::test]
async fn a_group_at_the_new_seed_version_is_untouched() {
    let db = TestDb::start().await;
    seed_missing(&db).await;
    let code = Role::Teller.as_str();
    assert_eq!(seed_version(&db, code).await, RoleGrants::SEED_VERSION);
    grant(&db, code, Permission::OverviewRead).await;
    let before = stored_set(&db, code).await;

    let (removed, _) = upgrade(&db).await;

    assert_eq!(removed, 0);
    assert_eq!(
        stored_set(&db, code).await,
        before,
        "a person's grant after version 3 stays"
    );
}

/// A custom group (`built_in = false`) is never touched, even one based on TELLER holding OverviewRead.
#[tokio::test]
async fn a_custom_group_is_untouched() {
    let db = database_at_version_2().await;
    let custom = Group {
        code: unique("CUSTOM").to_uppercase(),
        name: "กลุ่มกำหนดเอง".to_owned(),
        base: Role::Teller,
        built_in: false,
    };
    let mut set = RoleGrants::seed(Role::Teller);
    set.push(Permission::OverviewRead);
    let mut tx = db.pool.begin().await.expect("begin");
    PgRoleGrantStore::new()
        .add_group(&mut tx, &custom, &set)
        .await
        .expect("add_group");
    tx.commit().await.expect("commit");
    let before = stored_set(&db, &custom.code).await;

    upgrade(&db).await;

    assert_eq!(stored_set(&db, &custom.code).await, before);
    assert_eq!(seed_version(&db, &custom.code).await, 0);
}
