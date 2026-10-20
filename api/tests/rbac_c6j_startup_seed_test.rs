//! RBAC §11.5 (C6j): `bootstrap::connect` seeds missing built-in groups and
//! installs the loaded `RoleGrantsSnapshot` at startup.

mod common;
mod routes_common;

use std::collections::BTreeSet;

use routes_common::ScratchDatabase;
use xc_auth::domain::access::{Permission, Role, RoleGrantStore, RoleGrants};
use xc_auth::infrastructure::access::PgRoleGrantStore;
use xchange::bootstrap;

#[actix_web::test]
async fn connect_seeds_built_in_groups_once_and_preserves_edits_on_restart() {
    let db = ScratchDatabase::create("t1b_rbac_c6j").await;
    let settings = db.settings();

    // 1. Fresh database: connect seeds every built-in group and bumps version to 1.
    let pool = bootstrap::connect(&settings)
        .await
        .expect("first connect on fresh database");

    let store = PgRoleGrantStore::new();
    let mut conn = pool.acquire().await.expect("acquire connection");
    let loaded = store.load(&mut conn).await.expect("load snapshot");
    assert_eq!(loaded.version(), 1);
    assert_eq!(RoleGrants::current().version(), 1);

    for role in RoleGrants::BUILT_IN {
        let seed = RoleGrants::seed(role);
        let expected: BTreeSet<Permission> = seed.iter().copied().collect();
        assert_eq!(loaded.permissions(role.as_str()), Some(&expected));
        assert_eq!(RoleGrants::of(role.as_str()), seed);
    }

    // 2. Restart: every built-in group already has rows, so nothing changes and version stays 1.
    let second = bootstrap::connect(&settings)
        .await
        .expect("second connect on seeded database");
    assert_eq!(store.version(&mut conn).await.expect("version"), 1);
    assert_eq!(RoleGrants::current().version(), 1);

    // 3. Edited group: an administrator edits TELLER and restarts; TELLER keeps the edited set.
    let mut tx = second.begin().await.expect("begin edit tx");
    store
        .save_set(&mut tx, Role::Teller.as_str(), &[Permission::SignedIn])
        .await
        .expect("save edited TELLER set");
    tx.commit().await.expect("commit edit tx");

    let third = bootstrap::connect(&settings)
        .await
        .expect("third connect after editing TELLER");
    assert_eq!(store.version(&mut conn).await.expect("version"), 2);
    assert_eq!(RoleGrants::current().version(), 2);
    assert_eq!(
        RoleGrants::of(Role::Teller.as_str()),
        vec![Permission::SignedIn]
    );

    drop(conn);
    third.close().await;
    second.close().await;
    pool.close().await;
    db.drop().await;
}
