//! Up-front tests for AP-3 (design `docs/design/blueprint/user-management-rbac.md` §11.9, "Data migration" and row
//! A10): at boot, `seed_additions` also gives every **custom** group the version-4 additions (the nine approval
//! kinds) that its **base**'s seed holds.
//! Written before the implementation on the BE lane controller's order (`feat/approval-perms`).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! §11.9: a custom group's no-module set equals its base's seed (rule 5), so today a group based on BRANCH_MANAGER
//! decides exchanges through its base. To keep that when the decision moves to a permission, the boot inserts the
//! version-4 additions into every custom group whose base's seed holds them. Nothing is removed.
//!
//! What these pin:
//! - A10: a database at `seed_version = 3` with a custom group based on BRANCH_MANAGER; after boot the group holds
//!   `ApproveExchangeTransaction`, `ApproveNegativeStock` and `ApproveCashTransferWithinLimit`, keeps everything it
//!   held, and the built-in groups are at `seed_version = 4`;
//! - a FINANCE-based custom group gains FINANCE's two kinds; an ADMIN-based one gains none (ADMIN's seed holds none,
//!   and rule 6 forbids them);
//! - only the **version-4** additions reach a custom group: a module permission added in an earlier seed version
//!   (`MarketRatesRefresh`, v2) that the custom group was saved without is not given to it;
//! - a second boot inserts nothing, and a kind the owner took off a custom group after the first boot is not given
//!   back.
//!
//! These tests pin only the observable sets, not where the progress is recorded. Two existing tests do: both
//! `rbac_seed_version_test.rs::a_custom_group_is_untouched` and `rbac_4155_seed_removals_test.rs::a_custom_group_is_untouched`
//! assert a custom group stays at `roles.seed_version = 0`, so keying the custom-group inserts off the built-in
//! base's `seed_version` (below 4 ⇒ also insert into the custom groups of that base) keeps both of those lines true.
//!
//! `rbac_seed_version_test.rs::a_custom_group_is_untouched` also pins today's "the set is unchanged" and goes red
//! once AP-3 lands (its custom group is based on EXECUTIVE, whose seed holds four version-4 kinds); AP-3 restates it.
//!
//! "Boot" is what `api/src/bootstrap/mod.rs` does: `seed_missing`, `seed_removals`, `seed_additions` in one
//! committed transaction.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Without it these
//! tests FAIL; none skips. Each test gets a schema of its own (`account_common::TestDb`). No test here installs a
//! `RoleGrants` snapshot.

mod account_common;

use std::collections::BTreeSet;

use account_common::{TestDb, unique};
use xc_auth::domain::access::permission_module::APPROVAL_KINDS;
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
/// Answers what `seed_additions` inserted.
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

async fn save_set(db: &TestDb, code: &str, set: &[Permission]) {
    let mut tx = db.pool.begin().await.expect("begin");
    PgRoleGrantStore::new()
        .save_set(&mut tx, code, set)
        .await
        .expect("save_set");
    tx.commit().await.expect("commit");
}

/// The version-4 additions (`RoleGrants::SEED_ADDITIONS` entries with version 4).
fn v4_additions() -> Vec<Permission> {
    RoleGrants::SEED_ADDITIONS
        .iter()
        .filter(|&&(v, _)| v == 4)
        .map(|&(_, p)| p)
        .collect()
}

/// `role`'s seed as it stood at version 3: its seed without the version-4 additions.
fn seed_at_v3(role: Role) -> Vec<Permission> {
    let v4 = v4_additions();
    RoleGrants::seed(role)
        .into_iter()
        .filter(|p| !v4.contains(p))
        .collect()
}

/// The version-4 additions `role`'s seed holds.
fn v4_in_seed_of(role: Role) -> Vec<Permission> {
    let seed = RoleGrants::seed(role);
    v4_additions()
        .into_iter()
        .filter(|p| seed.contains(p))
        .collect()
}

fn custom(base: Role) -> Group {
    Group {
        code: unique("AP3").to_uppercase(),
        name: "กลุ่มกำหนดเอง".to_owned(),
        base,
        built_in: false,
    }
}

/// A database as it stood before §11.9 shipped: every built-in group holds its version-3 seed at `seed_version = 3`.
async fn database_at_version_3() -> TestDb {
    let db = TestDb::start().await;
    boot(&db).await;
    for p in v4_additions() {
        sqlx::query(
            "DELETE FROM role_permission_grants WHERE permission = $1 \
             AND role_code IN (SELECT code FROM roles WHERE built_in)",
        )
        .bind(serde_name(p))
        .execute(&db.pool)
        .await
        .expect("take a version-4 addition back out");
    }
    sqlx::query("UPDATE roles SET seed_version = 3 WHERE built_in")
        .execute(&db.pool)
        .await
        .expect("set the built-in groups to seed version 3");
    for role in RoleGrants::BUILT_IN {
        assert_eq!(
            stored_set(&db, role.as_str()).await,
            names(&seed_at_v3(role)),
            "{role:?} holds its version-3 seed"
        );
    }
    db
}

/// The sanity of the fixture: §11.9's table seeds exactly these three kinds on BRANCH_MANAGER.
#[test]
fn ap3_branch_manager_s_version_4_additions_are_its_three_kinds() {
    assert_eq!(
        v4_in_seed_of(Role::BranchManager)
            .into_iter()
            .collect::<BTreeSet<_>>(),
        [
            P::ApproveCashTransferWithinLimit,
            P::ApproveNegativeStock,
            P::ApproveExchangeTransaction,
        ]
        .into_iter()
        .collect::<BTreeSet<_>>()
    );
    assert!(v4_additions().iter().all(|p| APPROVAL_KINDS.contains(p)));
}

/// §11.9 A10: a database at `seed_version = 3` with a custom group based on BRANCH_MANAGER. After boot the group
/// holds `ApproveExchangeTransaction`, `ApproveNegativeStock` and `ApproveCashTransferWithinLimit`, loses nothing,
/// and the built-in groups are at `seed_version = 4`.
#[tokio::test]
async fn ap3_a10_a_branch_manager_based_custom_group_gains_its_base_s_kinds_at_boot() {
    let db = database_at_version_3().await;
    let group = custom(Role::BranchManager);
    let held = seed_at_v3(Role::BranchManager);
    add_group(&db, &group, &held).await;
    let before = stored_set(&db, &group.code).await;
    assert_eq!(before, names(&held));

    boot(&db).await;

    let after = stored_set(&db, &group.code).await;
    let gained: BTreeSet<String> = after.difference(&before).cloned().collect();
    assert_eq!(
        gained,
        names(&[
            P::ApproveExchangeTransaction,
            P::ApproveNegativeStock,
            P::ApproveCashTransferWithinLimit,
        ]),
        "the custom group gains exactly BRANCH_MANAGER's three version-4 kinds"
    );
    assert!(before.is_subset(&after), "it loses nothing it held");
    for role in RoleGrants::BUILT_IN {
        assert_eq!(
            seed_version(&db, role.as_str()).await,
            4,
            "{role:?} ends at seed version 4"
        );
        assert_eq!(
            stored_set(&db, role.as_str()).await,
            names(&RoleGrants::seed(role)),
            "{role:?} ends with its full seed"
        );
    }
    db.drop_schema().await;
}

/// §11.9: the same holds for every base whose seed holds a kind: a FINANCE-based custom group gains
/// `ApproveCountDifference` and `ApproveTransactionReversal`.
#[tokio::test]
async fn ap3_a_finance_based_custom_group_gains_finance_s_two_kinds() {
    let db = database_at_version_3().await;
    let group = custom(Role::Finance);
    let held = seed_at_v3(Role::Finance);
    add_group(&db, &group, &held).await;

    boot(&db).await;

    let mut want = held.clone();
    want.extend([P::ApproveCountDifference, P::ApproveTransactionReversal]);
    assert_eq!(stored_set(&db, &group.code).await, names(&want));
    db.drop_schema().await;
}

/// §11.9 A10 (other half) and rule 6: an ADMIN-based custom group gains no approval kind.
#[tokio::test]
async fn ap3_an_admin_based_custom_group_gains_no_kind() {
    let db = database_at_version_3().await;
    let group = custom(Role::Admin);
    let held = seed_at_v3(Role::Admin);
    add_group(&db, &group, &held).await;

    boot(&db).await;

    assert_eq!(
        stored_set(&db, &group.code).await,
        names(&held),
        "an ADMIN-based custom group is unchanged"
    );
    db.drop_schema().await;
}

/// §11.9 "inserts version-4 additions": a custom group gets only the version-4 additions of its base's seed. A
/// module permission added in an earlier seed version that the group was saved without stays off it: a custom group's
/// module set is the admin's choice, not its base's seed.
#[tokio::test]
async fn ap3_a_custom_group_gains_only_the_version_4_additions() {
    let db = database_at_version_3().await;
    // A base a custom group may have (not EXECUTIVE, not CUSTOMER) whose seed holds an addition older than v4.
    let (base, older) = RoleGrants::BUILT_IN
        .into_iter()
        .filter(|r| !matches!(r, Role::Executive | Role::Customer))
        .find_map(|r| {
            let seed = RoleGrants::seed(r);
            RoleGrants::SEED_ADDITIONS
                .iter()
                .find(|&&(v, p)| v < 4 && seed.contains(&p))
                .map(|&(_, p)| (r, p))
        })
        .expect("some custom-group base's seed holds a pre-v4 addition (MarketRatesRefresh, v2)");
    let group = custom(base);
    let held: Vec<Permission> = seed_at_v3(base)
        .into_iter()
        .filter(|p| *p != older)
        .collect();
    add_group(&db, &group, &held).await;

    boot(&db).await;

    let after = stored_set(&db, &group.code).await;
    assert!(
        !after.contains(&serde_name(older)),
        "{older:?} (added before v4) is not given to a custom group saved without it"
    );
    let mut want = held.clone();
    want.extend(v4_in_seed_of(base));
    assert_eq!(after, names(&want), "{base:?}-based: the v4 kinds only");
    db.drop_schema().await;
}

/// A second boot inserts nothing; a kind the owner took off the custom group after the first boot is not given back.
#[tokio::test]
async fn ap3_a_second_boot_gives_nothing_back() {
    let db = database_at_version_3().await;
    let group = custom(Role::BranchManager);
    add_group(&db, &group, &seed_at_v3(Role::BranchManager)).await;

    let first = boot(&db).await;
    assert!(first > 0, "the first boot grants the version-4 additions");
    let without_exchange: Vec<Permission> = seed_at_v3(Role::BranchManager)
        .into_iter()
        .chain([P::ApproveNegativeStock, P::ApproveCashTransferWithinLimit])
        .collect();
    save_set(&db, &group.code, &without_exchange).await;

    let second = boot(&db).await;

    assert_eq!(second, 0, "the second boot inserts nothing");
    assert_eq!(
        stored_set(&db, &group.code).await,
        names(&without_exchange),
        "the owner's removal of ApproveExchangeTransaction from the custom group stands"
    );
    db.drop_schema().await;
}
