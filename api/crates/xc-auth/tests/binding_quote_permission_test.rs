//! Up-front tests for BQ-7 (design `docs/design/blueprint/binding-quote.md` D-13, D-18; §4.2 Q14, Q15 at the seed
//! level, Q28, Q29, Q30). Written by the sub-controller claude/644b3958 on `feat/binding-quote` before the
//! implementation. **Do not edit, delete or weaken.**
//!
//! `Permission::BindingQuoteIssue` (the skeleton added the variant and its no-module arm) is the one permission that
//! lets a group issue a binding quote. BQ-7 lists it in `Permission::ALL`, seeds it on the built-in TELLER and
//! BRANCH_MANAGER groups only (O-1 A), and records it as `(6, BindingQuoteIssue)` in `RoleGrants::SEED_ADDITIONS`
//! with `RoleGrants::SEED_VERSION` raised 5 → 6, so the boot step `seed_additions` (#3976) grants it to existing
//! TELLER and BRANCH_MANAGER groups once.
//!
//! What these pin:
//! - P1: `Permission::ALL` lists `BindingQuoteIssue` once; it has no module (so Customers at Create, which EXECUTIVE
//!   holds at Full, never implies it).
//! - P2 (Q14/Q15 at the seed): TELLER's and BRANCH_MANAGER's seeds hold it; EXECUTIVE, ADMIN, RATE_MANAGER, FINANCE,
//!   MARKETING, VERIFICATION_REVIEWER and CUSTOMER's do not.
//! - P3 (D-18): `SEED_ADDITIONS` has `(6, BindingQuoteIssue)` exactly once and `SEED_VERSION` is 6.
//! - Q28: built-in groups at seed version 5 without it ⇒ after boot TELLER and BRANCH_MANAGER each gained exactly it,
//!   no other built-in group gained it, every built-in group is at version 6; an EXECUTIVE-based custom group gains
//!   nothing.
//! - Q28b (sub-controller's ruling, design §10): a TELLER- or BRANCH_MANAGER-based custom group saved before BQ-7
//!   gains it too, at boot. Guardrail 5 (`matrix_guardrails.rs`) asks a custom group's no-module set to equal its
//!   base's seed, so without it such a group could no longer be saved. AP-3 gave the version-4 kinds to custom
//!   groups for the same reason; BQ-7 widens that rule from "version 4" to "every no-module addition".
//! - Q29: removed from TELLER after the version-6 boot ⇒ a later boot does not give it back.
//! - Q30: a fresh database ⇒ TELLER and BRANCH_MANAGER hold it once each, at version 6.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.
mod account_common;

use std::collections::BTreeSet;

use account_common::{TestDb, unique};
use xc_auth::domain::access::permission_module::PermissionModule;
use xc_auth::domain::access::{Group, Permission, Role, RoleGrantStore, RoleGrants};
use xc_auth::infrastructure::access::PgRoleGrantStore;

use Permission as P;

const HOLDERS: [Role; 2] = [Role::Teller, Role::BranchManager];

fn serde_name(p: Permission) -> String {
    match serde_json::to_value(p).expect("a Permission serialises") {
        serde_json::Value::String(s) => s,
        other => panic!("a Permission serialises to a string, got {other}"),
    }
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

async fn holders(db: &TestDb) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT role_code FROM role_permission_grants WHERE permission = $1 ORDER BY role_code",
    )
    .bind(serde_name(P::BindingQuoteIssue))
    .fetch_all(&db.pool)
    .await
    .expect("read role_permission_grants")
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

/// A database as main stood before BQ-7: built-in groups at seed version 5, nobody holding `BindingQuoteIssue`.
async fn database_at_version_5() -> TestDb {
    let db = TestDb::start().await;
    boot(&db).await;
    sqlx::query("DELETE FROM role_permission_grants WHERE permission = $1")
        .bind(serde_name(P::BindingQuoteIssue))
        .execute(&db.pool)
        .await
        .expect("take BindingQuoteIssue off every group");
    sqlx::query("UPDATE roles SET seed_version = 5 WHERE built_in")
        .execute(&db.pool)
        .await
        .expect("set the built-in groups to seed version 5");
    assert!(
        holders(&db).await.is_empty(),
        "the fixture: nobody holds it"
    );
    db
}

#[test]
fn p1_all_lists_binding_quote_issue_once_and_it_has_no_module() {
    assert_eq!(
        Permission::ALL
            .iter()
            .filter(|&&p| p == P::BindingQuoteIssue)
            .count(),
        1,
        "Permission::ALL lists BindingQuoteIssue exactly once"
    );
    assert_eq!(
        PermissionModule::module(P::BindingQuoteIssue),
        None,
        "BindingQuoteIssue is a no-module permission (D-13)"
    );
}

#[test]
fn p2_only_teller_and_branch_manager_seeds_hold_it() {
    for role in HOLDERS {
        assert!(
            RoleGrants::seed(role).contains(&P::BindingQuoteIssue),
            "{role:?}'s seed holds BindingQuoteIssue (O-1 A)"
        );
    }
    for role in [
        Role::Executive,
        Role::Admin,
        Role::RateManager,
        Role::Finance,
        Role::Marketing,
        Role::VerificationReviewer,
        Role::Customer,
    ] {
        assert!(
            !RoleGrants::seed(role).contains(&P::BindingQuoteIssue),
            "{role:?}'s seed does not hold BindingQuoteIssue (O-1 A refuses it)"
        );
    }
}

#[test]
fn p3_it_enters_the_seed_in_version_6() {
    let entries: Vec<i32> = RoleGrants::SEED_ADDITIONS
        .iter()
        .filter(|&&(_, p)| p == P::BindingQuoteIssue)
        .map(|&(v, _)| v)
        .collect();
    assert_eq!(
        entries,
        vec![6],
        "SEED_ADDITIONS has (6, BindingQuoteIssue) once"
    );
    // Staff corrections SC-D2 raised it to 7 after D-18 set 6; LA-1 (ledger adjustment) to 8.
    assert_eq!(
        RoleGrants::SEED_VERSION,
        8,
        "SEED_VERSION is 8 (D-18 set 6; SC-D2 7; LA-1 8)"
    );
}

#[tokio::test]
async fn q28_a_version_5_database_grants_it_to_teller_and_branch_manager_only() {
    let db = database_at_version_5().await;
    let executive_based = Group {
        code: unique("BQ7").to_uppercase(),
        name: "กลุ่มกำหนดเอง".to_owned(),
        base: Role::Executive,
        built_in: false,
    };
    let executive_held = RoleGrants::seed(Role::Executive);
    let mut tx = db.pool.begin().await.expect("begin");
    PgRoleGrantStore::new()
        .add_group(&mut tx, &executive_based, &executive_held)
        .await
        .expect("add_group");
    tx.commit().await.expect("commit");
    let mut before = std::collections::HashMap::new();
    for role in RoleGrants::BUILT_IN {
        before.insert(role, stored_set(&db, role.as_str()).await);
    }
    let custom_before = stored_set(&db, &executive_based.code).await;

    boot(&db).await;

    for role in RoleGrants::BUILT_IN {
        let mut want = before[&role].clone();
        if HOLDERS.contains(&role) {
            want.insert(serde_name(P::BindingQuoteIssue));
        }
        assert_eq!(
            stored_set(&db, role.as_str()).await,
            want,
            "{role:?}: gains exactly BindingQuoteIssue if TELLER or BRANCH_MANAGER, else nothing"
        );
        assert_eq!(
            seed_version(&db, role.as_str()).await,
            RoleGrants::SEED_VERSION,
            "{role:?} ends at the current seed version (6 at D-18; 7 at SC-D2; 8 since LA-1)"
        );
    }
    assert_eq!(
        stored_set(&db, &executive_based.code).await,
        custom_before,
        "an EXECUTIVE-based custom group gains nothing"
    );
    let mut want_holders: Vec<String> = HOLDERS.iter().map(|r| r.as_str().to_owned()).collect();
    want_holders.sort();
    assert_eq!(holders(&db).await, want_holders, "the holders after boot");
    assert_eq!(boot(&db).await, 0, "a second boot inserts nothing");
    db.drop_schema().await;
}

#[tokio::test]
async fn q28b_a_teller_or_branch_manager_based_custom_group_gains_it_too() {
    let db = database_at_version_5().await;
    let mut customs = Vec::new();
    for base in HOLDERS {
        let group = Group {
            code: unique("BQ7").to_uppercase(),
            name: "กลุ่มกำหนดเอง".to_owned(),
            base,
            built_in: false,
        };
        let held: Vec<Permission> = RoleGrants::seed(base)
            .into_iter()
            .filter(|&p| p != P::BindingQuoteIssue)
            .collect();
        let mut tx = db.pool.begin().await.expect("begin");
        PgRoleGrantStore::new()
            .add_group(&mut tx, &group, &held)
            .await
            .expect("add_group");
        tx.commit().await.expect("commit");
        let before = stored_set(&db, &group.code).await;
        customs.push((group, before));
    }

    boot(&db).await;

    for (group, before) in &customs {
        let mut want = before.clone();
        want.insert(serde_name(P::BindingQuoteIssue));
        assert_eq!(
            stored_set(&db, &group.code).await,
            want,
            "a {:?}-based custom group gains exactly BindingQuoteIssue",
            group.base
        );
    }
    assert_eq!(boot(&db).await, 0, "a second boot inserts nothing");
    db.drop_schema().await;
}

#[tokio::test]
async fn q29_removed_from_teller_after_the_grant_is_not_given_back() {
    let db = database_at_version_5().await;
    boot(&db).await;
    assert!(
        stored_set(&db, Role::Teller.as_str())
            .await
            .contains(&serde_name(P::BindingQuoteIssue)),
        "the fixture: TELLER was granted it by the version-6 boot"
    );
    sqlx::query("DELETE FROM role_permission_grants WHERE role_code = $1 AND permission = $2")
        .bind(Role::Teller.as_str())
        .bind(serde_name(P::BindingQuoteIssue))
        .execute(&db.pool)
        .await
        .expect("the owner takes it off TELLER");
    boot(&db).await;
    assert!(
        !stored_set(&db, Role::Teller.as_str())
            .await
            .contains(&serde_name(P::BindingQuoteIssue)),
        "a later boot does not give it back"
    );
    db.drop_schema().await;
}

#[tokio::test]
async fn q30_a_fresh_database_seeds_it_on_teller_and_branch_manager_at_version_6() {
    let db = TestDb::start().await;
    boot(&db).await;
    let mut want: Vec<String> = HOLDERS.iter().map(|r| r.as_str().to_owned()).collect();
    want.sort();
    assert_eq!(
        holders(&db).await,
        want,
        "TELLER and BRANCH_MANAGER hold it, once each"
    );
    for role in HOLDERS {
        // 6 at D-18; 7 at staff corrections SC-D2; 8 since LA-1 (ledger adjustment).
        assert_eq!(
            seed_version(&db, role.as_str()).await,
            RoleGrants::SEED_VERSION,
            "{role:?}"
        );
    }
    db.drop_schema().await;
}
