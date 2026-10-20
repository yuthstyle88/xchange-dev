//! Up-front tests for RBAC §11 C6d: the session read path (`session_service.rs`, `read_session_in` behind
//! `SessionService::active_session`) and the user repository's roles (`user_repository.rs`, `find_by_id`,
//! `find_roles_by_user_id`, `add_role`) carry groups (design `docs/design/blueprint/user-management-rbac.md` §11.1,
//! §11.4 "`ScopedRole` carries the group code", "`read_session_in` checks `role_grant_version`").
//! Written before the implementation by the sub-controller claude/65357afb on `feat/rbac-groups`.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Each test gets a schema of its own with every migration in `api/migrations/` applied (`account_common::TestDb`).
//! Users, `user_roles` rows and sessions are written in SQL, as `rbac_f11a_session_test.rs` writes users; a session
//! row is `user_sessions.hash = sha256_hex(token)` at the user's version, so `active_session(token, now)` reads it.
//!
//! `RoleGrants::install` is process-global, so every test holds `LOCK` from start to end and first installs
//! `RoleGrantsSnapshot::seeded()` (version 0), so no test sees another's snapshot.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Without it these
//! tests FAIL; none skips.
//!
//! Names these tests assume:
//! - migration M2 (`api/migrations/<version>_role_groups.sql`): `roles.name`, `roles.base_role`, `roles.built_in`,
//!   `role_permission_grants`, `role_grant_version` (one row, 0);
//! - a `user_roles` row naming a custom group reads back as `ScopedRole { role: <its base>, group: <its code> }`.
//!   The base is read from the database (`roles.base_role`): the custom group here is inserted in SQL without
//!   moving `role_grant_version`, so the process snapshot does not know it;
//! - `ScopedRole::{new, in_group, group_code}`; `RoleGrants::{install, current, holds, of, base_of, seed}`;
//!   `RoleGrantsSnapshot::{seeded, version}`;
//! - `PgRoleGrantStore::new()` with `seed_missing`, `save_set` and `add_group`, each bumping `role_grant_version`;
//! - `read_session_in` reloads the snapshot (`RoleGrants::install`) when the stored `role_grant_version` differs
//!   from `RoleGrants::current().version()`, and nothing else in these tests reloads it;
//! - `UserRepository::add_role` writes `role_code = ScopedRole::group_code()`.

mod account_common;

use std::collections::BTreeSet;

use account_common::{TestDb, at, unique};
use chrono::Utc;
use tokio::sync::Mutex;
use xc_auth::application::session::{SessionService, sha256_hex};
use xc_auth::domain::access::{
    Group, Permission, Role, RoleGrantStore, RoleGrants, RoleGrantsSnapshot, ScopedRole,
};
use xc_auth::domain::user::UserRepository;
use xc_auth::infrastructure::access::PgRoleGrantStore;
use xc_auth::infrastructure::user::PgUserRepository;
use xc_kernel::ids::BranchId;

/// Held for the whole of every test in this file: `RoleGrants::install` is process-global.
static LOCK: Mutex<()> = Mutex::const_new(());

const NIGHT_TELLER: &str = "NIGHT_TELLER";

/// A test schema with every migration applied, refusing to go on unless M2 is among them; the process snapshot is
/// reset to the seed at version 0.
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
    RoleGrants::install(RoleGrantsSnapshot::seeded());
    db
}

fn service(db: &TestDb) -> SessionService {
    SessionService::new(db.pool.clone(), None, None, None)
}

async fn xc_org(db: &TestDb) -> String {
    sqlx::query_scalar("SELECT id::text FROM organizations WHERE code = 'XC'")
        .fetch_one(&db.pool)
        .await
        .expect("the XC organisation")
}

/// The custom group NIGHT_TELLER (base TELLER), written in SQL as a `roles` row only.
async fn insert_night_teller_row(db: &TestDb) {
    sqlx::query(
        "INSERT INTO roles (code, name, base_role, built_in) VALUES ($1, 'พนักงานกะดึก', 'TELLER', false)",
    )
    .bind(NIGHT_TELLER)
    .execute(&db.pool)
    .await
    .expect("insert the NIGHT_TELLER roles row");
}

async fn insert_user(db: &TestDb, org: &str) -> String {
    sqlx::query_scalar(
        "INSERT INTO users (organization_id, auth_subject, name, status, created_by) \
         VALUES ($1::uuid, $2, 'Somchai', 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(org)
    .bind(unique("sub"))
    .fetch_one(&db.pool)
    .await
    .expect("insert user")
}

/// One `user_roles` row naming `group`, open-ended from `at(from_seconds)`.
async fn grant(
    db: &TestDb,
    user: &str,
    org: &str,
    group: &str,
    branch: Option<&str>,
    from_seconds: i64,
) {
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
         VALUES ($1::uuid, $2, $3::uuid, $4, $5)",
    )
    .bind(user)
    .bind(group)
    .bind(org)
    .bind(branch)
    .bind(at(from_seconds))
    .execute(&db.pool)
    .await
    .unwrap_or_else(|e| panic!("insert the user_roles row for {group}: {e}"));
}

/// A live session of `user`; its token.
async fn open_session(db: &TestDb, user: &str) -> String {
    let token = unique("session-token");
    sqlx::query(
        "INSERT INTO user_sessions (hash, user_id, version, expires, organization_id) \
         SELECT $1, u.id, u.version, now() + interval '1 hour', u.organization_id FROM users u WHERE u.id = $2::uuid",
    )
    .bind(sha256_hex(&token))
    .bind(user)
    .execute(&db.pool)
    .await
    .expect("insert user_sessions");
    token
}

/// The roles the session read answers; it must find the session.
async fn session_roles(svc: &SessionService, token: &str) -> Vec<ScopedRole> {
    let session = svc
        .active_session(token, Utc::now())
        .await
        .expect("the session read")
        .expect("an active session");
    assert_eq!(
        session.user.roles, session.roles,
        "the session's user carries the same roles"
    );
    session.roles
}

/// The roles `find_by_id` and `find_roles_by_user_id` answer; both must agree.
async fn repository_roles(db: &TestDb, user: &str) -> Vec<ScopedRole> {
    let mut tx = db.pool.begin().await.expect("begin");
    let found = PgUserRepository::new()
        .find_by_id(&mut tx, user)
        .await
        .expect("find_by_id reads every group")
        .expect("the user");
    let direct = PgUserRepository::new()
        .find_roles_by_user_id(&mut tx, user)
        .await
        .expect("find_roles_by_user_id reads every group");
    tx.rollback().await.expect("rollback");
    assert_eq!(
        found.roles, direct,
        "find_by_id carries find_roles_by_user_id's answer"
    );
    found.roles
}

async fn stored_version(db: &TestDb) -> i64 {
    sqlx::query_scalar("SELECT version FROM role_grant_version")
        .fetch_one(&db.pool)
        .await
        .expect("read role_grant_version")
}

fn set_of(set: &[Permission]) -> BTreeSet<Permission> {
    set.iter().copied().collect()
}

/// §11.4: a `user_roles` row naming a custom group reads back, through the session read and through the user
/// repository, as a row of that group with its base as `role`. Today `Role::parse("NIGHT_TELLER")` is `None`: the
/// session read drops the row and the repository refuses the user.
#[tokio::test]
async fn custom_group_assignment_reads_back_as_its_base_with_its_group_code() {
    let _lock = LOCK.lock().await;
    let db = start().await;
    insert_night_teller_row(&db).await;
    let org = xc_org(&db).await;
    let user = insert_user(&db, &org).await;
    grant(&db, &user, &org, NIGHT_TELLER, Some("MAIN"), 0).await;
    let token = open_session(&db, &user).await;
    let expected = vec![ScopedRole::in_group(
        NIGHT_TELLER,
        Role::Teller,
        Some(BranchId::new("MAIN")),
        at(0),
        None,
    )];

    let read = session_roles(&service(&db), &token).await;
    assert_eq!(
        read, expected,
        "the session read keeps the custom-group row"
    );
    assert_eq!(read[0].role, Role::Teller);
    assert_eq!(read[0].group_code(), NIGHT_TELLER);

    let stored = repository_roles(&db, &user).await;
    assert_eq!(
        stored, expected,
        "the repository reads the custom-group row"
    );
    assert_eq!(stored[0].group_code(), NIGHT_TELLER);
    db.drop_schema().await;
}

/// A built-in assignment still reads as `ScopedRole::new(role, ..)`: its group code is its role's code. A user with a
/// built-in row and a custom row gets both, in `valid_from` order.
#[tokio::test]
async fn built_in_assignment_still_reads_as_scoped_role_new() {
    let _lock = LOCK.lock().await;
    let db = start().await;
    insert_night_teller_row(&db).await;
    let org = xc_org(&db).await;
    let user = insert_user(&db, &org).await;
    grant(&db, &user, &org, "TELLER", Some("MAIN"), 0).await;
    grant(&db, &user, &org, "EXECUTIVE", None, 3_600).await;
    grant(&db, &user, &org, NIGHT_TELLER, None, 7_200).await;
    let token = open_session(&db, &user).await;
    let expected = vec![
        ScopedRole::new(Role::Teller, Some(BranchId::new("MAIN")), at(0), None),
        ScopedRole::new(Role::Executive, None, at(3_600), None),
        ScopedRole::in_group(NIGHT_TELLER, Role::Teller, None, at(7_200), None),
    ];

    let read = session_roles(&service(&db), &token).await;
    assert_eq!(read, expected);
    assert_eq!(read[0].group_code(), "TELLER");
    assert_eq!(read[1].group_code(), "EXECUTIVE");

    assert_eq!(repository_roles(&db, &user).await, expected);
    db.drop_schema().await;
}

/// §11.4 freshness: every save bumps `role_grant_version`, and the next session read reloads the snapshot. After the
/// first read the process holds the stored version; after `save_set` and a second read it holds the new version and
/// `RoleGrants::holds` / `of` answer the edited set; a group added later is known after the next read.
#[tokio::test]
async fn session_read_reloads_role_grants_when_the_version_moves() {
    let _lock = LOCK.lock().await;
    let db = start().await;
    let store = PgRoleGrantStore::new();
    let mut tx = db.pool.begin().await.expect("begin");
    store.seed_missing(&mut tx).await.expect("seed_missing");
    tx.commit().await.expect("commit");
    let org = xc_org(&db).await;
    let user = insert_user(&db, &org).await;
    grant(&db, &user, &org, "TELLER", Some("MAIN"), 0).await;
    let token = open_session(&db, &user).await;
    let svc = service(&db);

    let seeded = stored_version(&db).await;
    assert_ne!(
        seeded, 0,
        "seed_missing moved the version off the installed snapshot's 0"
    );
    session_roles(&svc, &token).await;
    assert_eq!(
        RoleGrants::current().version(),
        seeded,
        "the first read loads the stored version"
    );

    let teller_seed = RoleGrants::seed(Role::Teller);
    assert!(
        teller_seed.len() >= 2,
        "TELLER's seed has at least two permissions"
    );
    let removed = teller_seed[0];
    let added = Permission::ALL
        .iter()
        .copied()
        .find(|p| !teller_seed.contains(p))
        .expect("a permission TELLER's seed lacks");
    assert!(RoleGrants::holds("TELLER", removed));
    assert!(!RoleGrants::holds("TELLER", added));

    let mut edited: Vec<Permission> = teller_seed[1..].to_vec();
    edited.push(added);
    let mut tx = db.pool.begin().await.expect("begin");
    store
        .save_set(&mut tx, "TELLER", &edited)
        .await
        .expect("save_set TELLER");
    tx.commit().await.expect("commit");
    let saved = stored_version(&db).await;
    assert_ne!(saved, seeded, "save_set moved the version");
    assert_eq!(
        RoleGrants::current().version(),
        seeded,
        "nothing but the session read reloads the snapshot"
    );

    session_roles(&svc, &token).await;

    assert_eq!(RoleGrants::current().version(), saved);
    assert!(
        !RoleGrants::holds("TELLER", removed),
        "the edited set no longer holds {removed:?}"
    );
    assert!(
        RoleGrants::holds("TELLER", added),
        "the edited set holds {added:?}"
    );
    assert_eq!(set_of(&RoleGrants::of("TELLER")), set_of(&edited));

    let night = Group {
        code: NIGHT_TELLER.to_owned(),
        name: "พนักงานกะดึก".to_owned(),
        base: Role::Teller,
        built_in: false,
    };
    let mut tx = db.pool.begin().await.expect("begin");
    store
        .add_group(&mut tx, &night, &[added])
        .await
        .expect("add NIGHT_TELLER");
    tx.commit().await.expect("commit");
    let grown = stored_version(&db).await;

    session_roles(&svc, &token).await;

    assert_eq!(RoleGrants::current().version(), grown);
    assert!(RoleGrants::holds(NIGHT_TELLER, added));
    assert!(!RoleGrants::holds(NIGHT_TELLER, removed));
    assert_eq!(RoleGrants::base_of(NIGHT_TELLER), Some(Role::Teller));
    db.drop_schema().await;
}

/// `add_role` with a row of a custom group (`ScopedRole::in_group`) writes `role_code` = the group code, not its
/// base; a built-in row still writes its role's code. Both read back as written.
#[tokio::test]
async fn add_role_writes_the_group_code() {
    let _lock = LOCK.lock().await;
    let db = start().await;
    insert_night_teller_row(&db).await;
    let org = xc_org(&db).await;
    let user = insert_user(&db, &org).await;
    let rows = vec![
        ScopedRole::in_group(NIGHT_TELLER, Role::Teller, None, at(0), None),
        ScopedRole::new(Role::Finance, None, at(3_600), None),
    ];

    let mut tx = db.pool.begin().await.expect("begin");
    for row in &rows {
        PgUserRepository::new()
            .add_role(&mut tx, &user, &org, row)
            .await
            .expect("add_role");
    }
    tx.commit().await.expect("commit");

    let codes: Vec<String> = sqlx::query_scalar(
        "SELECT role_code FROM user_roles WHERE user_id = $1::uuid ORDER BY valid_from",
    )
    .bind(&user)
    .fetch_all(&db.pool)
    .await
    .expect("read user_roles");
    assert_eq!(codes, vec![NIGHT_TELLER.to_owned(), "FINANCE".to_owned()]);
    assert_eq!(repository_roles(&db, &user).await, rows);
    db.drop_schema().await;
}
