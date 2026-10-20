//! Up-front tests for RBAC §11 C6c: `PgRoleGrantStore` (design `docs/design/blueprint/user-management-rbac.md`
//! §11.4, §11.5 item 2, §11.7).
//! Written before the implementation by the sub-controller claude/65357afb on `feat/rbac-groups`.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Each test gets a schema of its own with every migration in `api/migrations/` applied (`account_common::TestDb`),
//! writes through the store in a committed transaction (`pool.begin()` / `tx.commit()`), and reads back with a pooled
//! connection or plain SQL. Expected sets come from `RoleGrants::seed`, never from hand-typed lists.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Without it these
//! tests FAIL; none skips.
//!
//! Names these tests assume:
//! - migration M2 (`api/migrations/<version>_role_groups.sql`): `roles.name text NOT NULL`,
//!   `roles.base_role text NOT NULL REFERENCES roles(code)`, `roles.built_in boolean NOT NULL DEFAULT false`
//!   (the nine existing rows backfilled to `name = code`, `base_role = code`, `built_in = true`);
//!   `role_permission_grants (role_code text REFERENCES roles(code) ON DELETE CASCADE, permission text,
//!   PRIMARY KEY (role_code, permission))`; `role_grant_version (version bigint NOT NULL)` holding one row, 0;
//! - a permission is stored by its serde name (`serde_json::to_value(p)`, e.g. `"ExchangeTransact"`);
//! - `xc_auth::infrastructure::access::PgRoleGrantStore::new()` implementing
//!   `xc_auth::domain::access::RoleGrantStore` (`load`, `version`, `seed_missing`, `save_set`, `add_group`,
//!   `delete_group`) with `RoleGrantStoreError::{NotFound, CodeTaken, BuiltIn, InUse}`;
//! - `xc_auth::domain::access::{Group, RoleGrants, RoleGrantsSnapshot}`: `Group::built_in`, `RoleGrants::BUILT_IN`,
//!   `RoleGrants::seed`, `RoleGrantsSnapshot::{version, group, permissions, groups}`.

mod account_common;

use std::collections::BTreeSet;

use account_common::{TestDb, unique};
use xc_auth::domain::access::{
    Group, Permission, Role, RoleGrantStore, RoleGrantStoreError, RoleGrants, RoleGrantsSnapshot,
};
use xc_auth::infrastructure::access::PgRoleGrantStore;

const NIGHT_TELLER: &str = "NIGHT_TELLER";

/// A test schema with every migration applied, refusing to go on unless M2 is among them.
async fn start() -> TestDb {
    let db = TestDb::start().await;
    let m2: bool = sqlx::query_scalar(
        "SELECT (to_regclass('role_permission_grants') IS NOT NULL) \
            AND (to_regclass('role_grant_version') IS NOT NULL) \
            AND (SELECT count(*) FROM information_schema.columns \
                 WHERE table_schema = current_schema() AND table_name = 'roles' \
                   AND column_name IN ('name', 'base_role', 'built_in')) = 3",
    )
    .fetch_one(&db.pool)
    .await
    .expect("inspect the migrated schema");
    assert!(
        m2,
        "migration M2 (api/migrations/<version>_role_groups.sql) must add roles.name, roles.base_role, \
         roles.built_in and create role_permission_grants and role_grant_version"
    );
    db
}

/// The name a permission is stored by: its serde name.
fn serde_name(p: Permission) -> String {
    match serde_json::to_value(p).expect("a Permission serialises") {
        serde_json::Value::String(s) => s,
        other => panic!("a Permission serialises to a string, got {other}"),
    }
}

fn names(set: &[Permission]) -> BTreeSet<String> {
    set.iter().copied().map(serde_name).collect()
}

fn set_of(set: &[Permission]) -> BTreeSet<Permission> {
    set.iter().copied().collect()
}

/// A set that differs from `role`'s seed both ways: the seed's first permission, and the first two the seed lacks.
fn other_than_seed(role: Role) -> Vec<Permission> {
    let seed = RoleGrants::seed(role);
    assert!(!seed.is_empty(), "the seed of {role:?} is not empty");
    let mut set = vec![seed[0]];
    set.extend(
        Permission::ALL
            .iter()
            .copied()
            .filter(|p| !seed.contains(p))
            .take(2),
    );
    assert_eq!(
        set.len(),
        3,
        "the seed of {role:?} leaves at least two permissions out"
    );
    set
}

fn night_teller() -> Group {
    Group {
        code: NIGHT_TELLER.to_owned(),
        name: "พนักงานกะดึก".to_owned(),
        base: Role::Teller,
        built_in: false,
    }
}

async fn stored_version(db: &TestDb) -> i64 {
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM role_grant_version")
        .fetch_one(&db.pool)
        .await
        .expect("count role_grant_version");
    assert_eq!(rows, 1, "role_grant_version holds exactly one row");
    sqlx::query_scalar("SELECT version FROM role_grant_version")
        .fetch_one(&db.pool)
        .await
        .expect("read role_grant_version")
}

/// The serde names stored for `code`.
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

async fn all_grant_rows(db: &TestDb) -> Vec<(String, String)> {
    sqlx::query_as("SELECT role_code, permission FROM role_permission_grants ORDER BY 1, 2")
        .fetch_all(&db.pool)
        .await
        .expect("read role_permission_grants")
}

async fn roles_row(db: &TestDb, code: &str) -> Option<(String, String, bool)> {
    sqlx::query_as("SELECT name, base_role, built_in FROM roles WHERE code = $1")
        .bind(code)
        .fetch_optional(&db.pool)
        .await
        .expect("read roles")
}

async fn seed_missing(db: &TestDb) -> u64 {
    let mut tx = db.pool.begin().await.expect("begin");
    let seeded = PgRoleGrantStore::new()
        .seed_missing(&mut tx)
        .await
        .expect("seed_missing");
    tx.commit().await.expect("commit");
    seeded
}

async fn save_set(db: &TestDb, code: &str, set: &[Permission]) -> Result<(), RoleGrantStoreError> {
    let mut tx = db.pool.begin().await.expect("begin");
    let result = PgRoleGrantStore::new().save_set(&mut tx, code, set).await;
    match result {
        Ok(()) => tx.commit().await.expect("commit"),
        Err(_) => tx.rollback().await.expect("rollback"),
    }
    result
}

async fn add_group(
    db: &TestDb,
    group: &Group,
    set: &[Permission],
) -> Result<(), RoleGrantStoreError> {
    let mut tx = db.pool.begin().await.expect("begin");
    let result = PgRoleGrantStore::new().add_group(&mut tx, group, set).await;
    match result {
        Ok(()) => tx.commit().await.expect("commit"),
        Err(_) => tx.rollback().await.expect("rollback"),
    }
    result
}

async fn delete_group(db: &TestDb, code: &str) -> Result<(), RoleGrantStoreError> {
    let mut tx = db.pool.begin().await.expect("begin");
    let result = PgRoleGrantStore::new().delete_group(&mut tx, code).await;
    match result {
        Ok(()) => tx.commit().await.expect("commit"),
        Err(_) => tx.rollback().await.expect("rollback"),
    }
    result
}

async fn load(db: &TestDb) -> RoleGrantsSnapshot {
    let mut conn = db.pool.acquire().await.expect("acquire");
    PgRoleGrantStore::new().load(&mut conn).await.expect("load")
}

async fn store_version(db: &TestDb) -> i64 {
    let mut conn = db.pool.acquire().await.expect("acquire");
    PgRoleGrantStore::new()
        .version(&mut conn)
        .await
        .expect("version")
}

/// A user of the XC organisation holding `group` through one `user_roles` row, valid now or ended an hour ago.
async fn user_holding(db: &TestDb, group: &str, ended: bool) {
    let user: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         SELECT id, $1, 'ACTIVE', 'test' FROM organizations WHERE code = 'XC' RETURNING id::text",
    )
    .bind(unique("u"))
    .fetch_one(&db.pool)
    .await
    .expect("insert user");
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, valid_from, valid_to) \
         SELECT $1::uuid, $2, id, now() - interval '2 hours', \
                CASE WHEN $3 THEN now() - interval '1 hour' END \
         FROM organizations WHERE code = 'XC'",
    )
    .bind(&user)
    .bind(group)
    .bind(ended)
    .execute(&db.pool)
    .await
    .expect("insert user_roles");
}

/// §11.5 item 2: on a fresh database `seed_missing` seeds all nine built-in groups, answers 9, writes each group's
/// `RoleGrants::seed` by serde name, and bumps `role_grant_version` exactly once.
#[tokio::test]
async fn seed_missing_on_fresh_db_seeds_every_built_in_group_and_bumps_version_once() {
    let db = start().await;
    assert_eq!(
        stored_version(&db).await,
        0,
        "M2 creates role_grant_version at 0"
    );
    assert!(
        all_grant_rows(&db).await.is_empty(),
        "M2 seeds no grants in SQL"
    );

    let seeded = seed_missing(&db).await;

    assert_eq!(seeded, RoleGrants::BUILT_IN.len() as u64);
    for role in RoleGrants::BUILT_IN {
        assert_eq!(
            stored_set(&db, role.as_str()).await,
            names(&RoleGrants::seed(role)),
            "the stored set of {} is its seed",
            role.as_str()
        );
    }
    assert_eq!(stored_version(&db).await, 1, "one bump for the whole seed");
    db.drop_schema().await;
}

/// §11.5 item 2: a restart changes nothing. The second run answers 0 and leaves the version and every row alone.
#[tokio::test]
async fn seed_missing_second_run_answers_zero_and_changes_nothing() {
    let db = start().await;
    seed_missing(&db).await;
    let version = stored_version(&db).await;
    let rows = all_grant_rows(&db).await;

    assert_eq!(seed_missing(&db).await, 0);

    assert_eq!(stored_version(&db).await, version);
    assert_eq!(all_grant_rows(&db).await, rows);
    db.drop_schema().await;
}

/// §11.5 item 2: an edited group is never overwritten. After TELLER's set is cut down, `seed_missing` answers 0
/// and TELLER keeps the edited set.
#[tokio::test]
async fn seed_missing_never_overwrites_an_edited_group() {
    let db = start().await;
    seed_missing(&db).await;
    let seed = RoleGrants::seed(Role::Teller);
    assert!(
        seed.len() >= 2,
        "TELLER's seed has at least two permissions"
    );
    let smaller = &seed[..seed.len() / 2];
    save_set(&db, "TELLER", smaller)
        .await
        .expect("save_set TELLER");
    let version = stored_version(&db).await;

    assert_eq!(seed_missing(&db).await, 0);

    assert_eq!(stored_set(&db, "TELLER").await, names(smaller));
    assert_eq!(stored_version(&db).await, version);
    db.drop_schema().await;
}

/// §11.5 item 2: only a built-in group with no rows is seeded. Deleting FINANCE's rows by hand makes the next run
/// reseed FINANCE alone (answers 1, one bump) and leave every other group's rows as they were.
#[tokio::test]
async fn seed_missing_reseeds_only_the_group_whose_rows_are_gone() {
    let db = start().await;
    seed_missing(&db).await;
    sqlx::query("DELETE FROM role_permission_grants WHERE role_code = 'FINANCE'")
        .execute(&db.pool)
        .await
        .expect("delete FINANCE's rows");
    let others = all_grant_rows(&db).await;
    let version = stored_version(&db).await;

    assert_eq!(seed_missing(&db).await, 1);

    assert_eq!(
        stored_set(&db, "FINANCE").await,
        names(&RoleGrants::seed(Role::Finance))
    );
    let untouched: Vec<(String, String)> = all_grant_rows(&db)
        .await
        .into_iter()
        .filter(|(code, _)| code != "FINANCE")
        .collect();
    assert_eq!(untouched, others);
    assert_eq!(stored_version(&db).await, version + 1);
    db.drop_schema().await;
}

/// `load` answers the stored version and every group with its name, base, built-in flag and set; a custom group
/// added with `add_group` is there with its own base and set.
#[tokio::test]
async fn load_returns_version_and_every_group_with_its_set() {
    let db = start().await;
    seed_missing(&db).await;
    let night_set = other_than_seed(Role::Teller);
    add_group(&db, &night_teller(), &night_set)
        .await
        .expect("add NIGHT_TELLER");

    let snapshot = load(&db).await;

    assert_eq!(snapshot.version(), stored_version(&db).await);
    assert_eq!(snapshot.groups().count(), RoleGrants::BUILT_IN.len() + 1);
    for role in RoleGrants::BUILT_IN {
        let code = role.as_str();
        assert_eq!(
            snapshot.group(code),
            Some(&Group::built_in(role)),
            "group {code}"
        );
        assert_eq!(
            snapshot.permissions(code),
            Some(&set_of(&RoleGrants::seed(role))),
            "set of {code}"
        );
    }
    assert_eq!(snapshot.group(NIGHT_TELLER), Some(&night_teller()));
    assert_eq!(
        snapshot.permissions(NIGHT_TELLER),
        Some(&set_of(&night_set))
    );
    db.drop_schema().await;
}

/// `save_set` replaces the set (permissions left out are gone, new ones are there) and bumps the version by 1.
#[tokio::test]
async fn save_set_replaces_the_set_and_bumps_version_by_one() {
    let db = start().await;
    seed_missing(&db).await;
    let version = stored_version(&db).await;
    let new_set = other_than_seed(Role::Finance);

    save_set(&db, "FINANCE", &new_set)
        .await
        .expect("save_set FINANCE");

    assert_eq!(stored_set(&db, "FINANCE").await, names(&new_set));
    assert_eq!(stored_version(&db).await, version + 1);
    assert_eq!(
        stored_set(&db, "TELLER").await,
        names(&RoleGrants::seed(Role::Teller)),
        "another group is untouched"
    );
    assert_eq!(
        load(&db).await.permissions("FINANCE"),
        Some(&set_of(&new_set))
    );
    db.drop_schema().await;
}

/// `save_set` on a code no `roles` row has: `NotFound`, and nothing is written.
#[tokio::test]
async fn save_set_on_unknown_code_is_not_found() {
    let db = start().await;
    seed_missing(&db).await;
    let version = stored_version(&db).await;

    let err = save_set(&db, "NO_SUCH_GROUP", &RoleGrants::seed(Role::Teller))
        .await
        .expect_err("an unknown group");

    assert!(
        matches!(err, RoleGrantStoreError::NotFound(_)),
        "got {err:?}"
    );
    assert!(stored_set(&db, "NO_SUCH_GROUP").await.is_empty());
    assert_eq!(stored_version(&db).await, version);
    db.drop_schema().await;
}

/// `add_group` writes the `roles` row (name, base, not built in) and its set, and bumps the version by 1.
#[tokio::test]
async fn add_group_writes_the_group_and_its_set_and_bumps_version_by_one() {
    let db = start().await;
    seed_missing(&db).await;
    let version = stored_version(&db).await;
    let night_set = other_than_seed(Role::Teller);

    add_group(&db, &night_teller(), &night_set)
        .await
        .expect("add NIGHT_TELLER");

    assert_eq!(
        roles_row(&db, NIGHT_TELLER).await,
        Some(("พนักงานกะดึก".to_owned(), "TELLER".to_owned(), false))
    );
    assert_eq!(stored_set(&db, NIGHT_TELLER).await, names(&night_set));
    assert_eq!(stored_version(&db).await, version + 1);
    db.drop_schema().await;
}

/// `add_group` with a code that exists, built in or custom: `CodeTaken`, and the existing group is unchanged.
#[tokio::test]
async fn add_group_with_an_existing_code_is_code_taken() {
    let db = start().await;
    seed_missing(&db).await;
    add_group(&db, &night_teller(), &other_than_seed(Role::Teller))
        .await
        .expect("add NIGHT_TELLER");
    let rows = all_grant_rows(&db).await;
    let version = stored_version(&db).await;

    let over_built_in = Group {
        code: "TELLER".to_owned(),
        name: "x".to_owned(),
        base: Role::Teller,
        built_in: false,
    };
    let err = add_group(&db, &over_built_in, &[])
        .await
        .expect_err("TELLER exists");
    assert!(
        matches!(err, RoleGrantStoreError::CodeTaken(_)),
        "got {err:?}"
    );

    let again = Group {
        name: "อีกกลุ่ม".to_owned(),
        base: Role::Finance,
        ..night_teller()
    };
    let err = add_group(&db, &again, &RoleGrants::seed(Role::Finance))
        .await
        .expect_err("NIGHT_TELLER exists");
    assert!(
        matches!(err, RoleGrantStoreError::CodeTaken(_)),
        "got {err:?}"
    );

    assert_eq!(
        roles_row(&db, "TELLER").await,
        Some(("TELLER".to_owned(), "TELLER".to_owned(), true))
    );
    assert_eq!(
        roles_row(&db, NIGHT_TELLER).await,
        Some(("พนักงานกะดึก".to_owned(), "TELLER".to_owned(), false))
    );
    assert_eq!(all_grant_rows(&db).await, rows);
    assert_eq!(stored_version(&db).await, version);
    db.drop_schema().await;
}

/// `delete_group` on a built-in group: `BuiltIn`, and the group keeps its row and set.
#[tokio::test]
async fn delete_group_on_a_built_in_group_is_built_in() {
    let db = start().await;
    seed_missing(&db).await;
    let version = stored_version(&db).await;

    let err = delete_group(&db, "MARKETING")
        .await
        .expect_err("MARKETING is built in");

    assert!(
        matches!(err, RoleGrantStoreError::BuiltIn(_)),
        "got {err:?}"
    );
    assert!(roles_row(&db, "MARKETING").await.is_some());
    assert_eq!(
        stored_set(&db, "MARKETING").await,
        names(&RoleGrants::seed(Role::Marketing))
    );
    assert_eq!(stored_version(&db).await, version);
    db.drop_schema().await;
}

/// §11.1: a group that any user holds cannot be deleted. A `user_roles` row naming it, valid now or already ended,
/// makes `delete_group` answer `InUse`, and the group keeps its row and set.
#[tokio::test]
async fn delete_group_held_by_a_user_is_in_use() {
    let db = start().await;
    seed_missing(&db).await;
    let night_set = other_than_seed(Role::Teller);
    add_group(&db, &night_teller(), &night_set)
        .await
        .expect("add NIGHT_TELLER");
    let ended = Group {
        code: "DAY_FINANCE".to_owned(),
        name: "การเงินกะวัน".to_owned(),
        base: Role::Finance,
        built_in: false,
    };
    add_group(&db, &ended, &RoleGrants::seed(Role::Finance))
        .await
        .expect("add DAY_FINANCE");
    user_holding(&db, NIGHT_TELLER, false).await;
    user_holding(&db, "DAY_FINANCE", true).await;
    let version = stored_version(&db).await;

    for code in [NIGHT_TELLER, "DAY_FINANCE"] {
        let err = delete_group(&db, code).await.expect_err("held by a user");
        assert!(
            matches!(err, RoleGrantStoreError::InUse(_)),
            "{code}: got {err:?}"
        );
        assert!(roles_row(&db, code).await.is_some(), "{code} keeps its row");
    }
    assert_eq!(stored_set(&db, NIGHT_TELLER).await, names(&night_set));
    assert_eq!(stored_version(&db).await, version);
    db.drop_schema().await;
}

/// `delete_group` on a code no `roles` row has: `NotFound`.
#[tokio::test]
async fn delete_group_on_unknown_code_is_not_found() {
    let db = start().await;
    seed_missing(&db).await;
    let version = stored_version(&db).await;

    let err = delete_group(&db, "NO_SUCH_GROUP")
        .await
        .expect_err("an unknown group");

    assert!(
        matches!(err, RoleGrantStoreError::NotFound(_)),
        "got {err:?}"
    );
    assert_eq!(stored_version(&db).await, version);
    db.drop_schema().await;
}

/// `delete_group` on a custom group nobody holds: the `roles` row and its grants are gone, the version moves by 1,
/// and `load` no longer has it.
#[tokio::test]
async fn delete_group_on_an_unused_custom_group_removes_it_and_its_grants() {
    let db = start().await;
    seed_missing(&db).await;
    add_group(&db, &night_teller(), &other_than_seed(Role::Teller))
        .await
        .expect("add NIGHT_TELLER");
    let version = stored_version(&db).await;

    delete_group(&db, NIGHT_TELLER)
        .await
        .expect("delete NIGHT_TELLER");

    assert_eq!(roles_row(&db, NIGHT_TELLER).await, None);
    assert!(stored_set(&db, NIGHT_TELLER).await.is_empty());
    assert_eq!(stored_version(&db).await, version + 1);
    let snapshot = load(&db).await;
    assert_eq!(snapshot.group(NIGHT_TELLER), None);
    assert_eq!(snapshot.groups().count(), RoleGrants::BUILT_IN.len());
    db.drop_schema().await;
}

/// `version` answers the stored row, not a counter of its own: after the row is set by hand, it answers that.
#[tokio::test]
async fn version_returns_the_stored_row() {
    let db = start().await;
    assert_eq!(store_version(&db).await, 0);
    seed_missing(&db).await;
    assert_eq!(store_version(&db).await, stored_version(&db).await);

    sqlx::query("UPDATE role_grant_version SET version = 41")
        .execute(&db.pool)
        .await
        .expect("set the version by hand");

    assert_eq!(store_version(&db).await, 41);
    assert_eq!(load(&db).await.version(), 41);
    db.drop_schema().await;
}
