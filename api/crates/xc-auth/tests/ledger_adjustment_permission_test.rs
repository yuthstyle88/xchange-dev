//! Up-front tests for LA-1 (design `docs/design/blueprint/ledger-inventory.md` §13.3, §13.9 rows LA-T1 to LA-T4).
//! Written by the sub-controller claude/f32b85db on `feat/ledger-adjustment` before the implementation.
//! **Do not edit, delete or weaken.**
//!
//! `Permission::ApproveLedgerAdjustment` (the skeleton added the variant and its no-module arm) is the approval kind
//! that lets a group decide a `LEDGER_ADJUSTMENT`. LA-1 lists it in `Permission::ALL` and `APPROVAL_KINDS` (9 -> 10),
//! seeds it on the built-in FINANCE group only, and records it as `(8, ApproveLedgerAdjustment)` in
//! `RoleGrants::SEED_ADDITIONS` with `RoleGrants::SEED_VERSION` raised to 8 (lane controller 2026-10-28: main is at
//! 6, staff corrections takes 7), so the boot step `seed_additions` (#3976) grants it to existing FINANCE groups once.
//!
//! What these pin:
//! - LA-T1: `Permission::ALL` and `APPROVAL_KINDS` list it once each; it has no module; `APPROVAL_KINDS` has 10.
//! - LA-T2: only FINANCE's seed holds it, and FINANCE still holds `ApprovalsDecide`.
//! - LA-T3 (seed): `SEED_ADDITIONS` has `(8, ApproveLedgerAdjustment)` exactly once and `SEED_VERSION` is 8.
//! - LA-T3 (boot): built-in groups at seed version 7 without it ⇒ after boot FINANCE gained exactly it, no other
//!   built-in group gained anything, every built-in group is at version 8; an EXECUTIVE-based custom group gains
//!   nothing. A FINANCE-based custom group gains it too: since BQ-7 every no-module seed addition reaches the custom
//!   groups of its base at boot (`role_grant_store.rs` `seed_additions`), which §13.9's "the custom group gains
//!   nothing" predates (LA-9 corrects the text).
//! - LA-T3 (fresh): a fresh database ⇒ FINANCE holds it once, at version 8.
//! - LA-T4: an ADMIN-based group (custom or built-in) saved holding it is refused with `AdminBaseHoldsApproval`.
//!
//! The boot tests need `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing here skips.
mod account_common;

use std::collections::{BTreeSet, HashMap};

use account_common::{TestDb, unique};
use xc_auth::domain::access::permission_module::{APPROVAL_KINDS, PermissionModule};
use xc_auth::domain::access::{
    Group, GuardrailBreach, MatrixGuardrails, Permission, Role, RoleGrantStore, RoleGrants,
};
use xc_auth::infrastructure::access::PgRoleGrantStore;

use Permission as P;

const KIND: Permission = P::ApproveLedgerAdjustment;

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
    .bind(serde_name(KIND))
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

/// A database as it stands before LA-1: built-in groups at seed version 7, nobody holding the kind.
async fn database_at_version_7() -> TestDb {
    let db = TestDb::start().await;
    boot(&db).await;
    sqlx::query("DELETE FROM role_permission_grants WHERE permission = $1")
        .bind(serde_name(KIND))
        .execute(&db.pool)
        .await
        .expect("take ApproveLedgerAdjustment off every group");
    sqlx::query("UPDATE roles SET seed_version = 7 WHERE built_in")
        .execute(&db.pool)
        .await
        .expect("set the built-in groups to seed version 7");
    assert!(
        holders(&db).await.is_empty(),
        "the fixture: nobody holds it"
    );
    db
}

async fn add_custom(db: &TestDb, base: Role, held: &[Permission]) -> Group {
    let group = Group {
        code: unique("LA1").to_uppercase(),
        name: "กลุ่มกำหนดเอง".to_owned(),
        base,
        built_in: false,
    };
    let mut tx = db.pool.begin().await.expect("begin");
    PgRoleGrantStore::new()
        .add_group(&mut tx, &group, held)
        .await
        .expect("add_group");
    tx.commit().await.expect("commit");
    group
}

// ── LA-T1 ───────────────────────────────────────────────────────────────────

#[test]
fn la_t1_it_is_the_tenth_approval_kind_in_all_with_no_module() {
    assert_eq!(
        Permission::ALL.iter().filter(|&&p| p == KIND).count(),
        1,
        "Permission::ALL lists ApproveLedgerAdjustment exactly once"
    );
    assert_eq!(
        APPROVAL_KINDS.iter().filter(|&&p| p == KIND).count(),
        1,
        "APPROVAL_KINDS lists ApproveLedgerAdjustment exactly once"
    );
    let kinds: BTreeSet<Permission> = APPROVAL_KINDS.into_iter().collect();
    assert_eq!(kinds.len(), 10, "APPROVAL_KINDS has ten distinct kinds");
    assert_eq!(
        PermissionModule::module(KIND),
        None,
        "an approval kind is no-module (§13.3)"
    );
}

// ── LA-T2 ───────────────────────────────────────────────────────────────────

#[test]
fn la_t2_only_finances_seed_holds_it_and_finance_still_decides() {
    let holders: Vec<Role> = RoleGrants::BUILT_IN
        .into_iter()
        .filter(|&r| RoleGrants::seed(r).contains(&KIND))
        .collect();
    assert_eq!(holders, vec![Role::Finance], "seeded on FINANCE only");
    assert!(
        RoleGrants::seed(Role::Finance).contains(&P::ApprovalsDecide),
        "FINANCE still holds ApprovalsDecide"
    );
}

// ── LA-T3 ───────────────────────────────────────────────────────────────────

#[test]
fn la_t3_it_enters_the_seed_in_version_8() {
    let entries: Vec<i32> = RoleGrants::SEED_ADDITIONS
        .iter()
        .filter(|&&(_, p)| p == KIND)
        .map(|&(v, _)| v)
        .collect();
    assert_eq!(
        entries,
        vec![8],
        "SEED_ADDITIONS has (8, ApproveLedgerAdjustment) once"
    );
    // #5733 (PricingFormulaSave) raised it to 9 after LA-1 set 8; (8, ApproveLedgerAdjustment) is unchanged.
    // Restated for #6004 (user-management-rbac.md §11.11): SEED_VERSION is 10.
    // Restated for backdated keying (backdated-keying.md §3): SEED_VERSION is 11 (ExchangeBackdate).
    assert_eq!(RoleGrants::SEED_VERSION, 11, "SEED_VERSION is 11");
    assert!(
        RoleGrants::SEED_REMOVALS.iter().all(|&(_, p)| p != KIND),
        "it is never removed"
    );
}

#[tokio::test]
async fn la_t3_a_version_7_database_grants_it_to_finance_and_its_custom_groups_only() {
    let db = database_at_version_7().await;
    let executive_based =
        add_custom(&db, Role::Executive, &RoleGrants::seed(Role::Executive)).await;
    let finance_held: Vec<Permission> = RoleGrants::seed(Role::Finance)
        .into_iter()
        .filter(|&p| p != KIND)
        .collect();
    let finance_based = add_custom(&db, Role::Finance, &finance_held).await;
    let mut before = HashMap::new();
    for role in RoleGrants::BUILT_IN {
        before.insert(role, stored_set(&db, role.as_str()).await);
    }
    let executive_before = stored_set(&db, &executive_based.code).await;
    let finance_custom_before = stored_set(&db, &finance_based.code).await;

    boot(&db).await;

    for role in RoleGrants::BUILT_IN {
        let mut want = before[&role].clone();
        if role == Role::Finance {
            want.insert(serde_name(KIND));
        }
        assert_eq!(
            stored_set(&db, role.as_str()).await,
            want,
            "{role:?}: gains exactly ApproveLedgerAdjustment if FINANCE, else nothing"
        );
        // Restated for #6004 (user-management-rbac.md §11.11): SEED_VERSION is 10.
        // Restated for backdated keying (backdated-keying.md §3): SEED_VERSION is 11.
        assert_eq!(
            seed_version(&db, role.as_str()).await,
            11,
            "{role:?} ends at seed version 11 (#5733 raised it to 9, #6004 to 10, backdated keying to 11)"
        );
    }
    assert_eq!(
        stored_set(&db, &executive_based.code).await,
        executive_before,
        "an EXECUTIVE-based custom group gains nothing"
    );
    let mut want = finance_custom_before;
    want.insert(serde_name(KIND));
    assert_eq!(
        stored_set(&db, &finance_based.code).await,
        want,
        "a FINANCE-based custom group gains exactly ApproveLedgerAdjustment"
    );
    let mut want_holders = vec![
        Role::Finance.as_str().to_owned(),
        finance_based.code.clone(),
    ];
    want_holders.sort();
    assert_eq!(holders(&db).await, want_holders, "the holders after boot");
    assert_eq!(boot(&db).await, 0, "a second boot inserts nothing");
    db.drop_schema().await;
}

#[tokio::test]
async fn la_t3_removed_from_finance_after_the_grant_is_not_given_back() {
    let db = database_at_version_7().await;
    boot(&db).await;
    assert!(
        stored_set(&db, Role::Finance.as_str())
            .await
            .contains(&serde_name(KIND)),
        "the fixture: FINANCE was granted it by the version-8 boot"
    );
    sqlx::query("DELETE FROM role_permission_grants WHERE role_code = $1 AND permission = $2")
        .bind(Role::Finance.as_str())
        .bind(serde_name(KIND))
        .execute(&db.pool)
        .await
        .expect("the owner takes it off FINANCE");
    boot(&db).await;
    assert!(
        !stored_set(&db, Role::Finance.as_str())
            .await
            .contains(&serde_name(KIND)),
        "a later boot does not give it back"
    );
    db.drop_schema().await;
}

#[tokio::test]
async fn la_t3_a_fresh_database_seeds_it_on_finance_at_version_8() {
    let db = TestDb::start().await;
    boot(&db).await;
    assert_eq!(
        holders(&db).await,
        vec![Role::Finance.as_str().to_owned()],
        "FINANCE holds it, once"
    );
    // #5733 raised the seed version to 9 after LA-1's 8.
    // Restated for #6004 (user-management-rbac.md §11.11): SEED_VERSION is 10.
    // Restated for backdated keying (backdated-keying.md §3): SEED_VERSION is 11.
    assert_eq!(seed_version(&db, Role::Finance.as_str()).await, 11);
    db.drop_schema().await;
}

// ── LA-T4 ───────────────────────────────────────────────────────────────────

#[test]
fn la_t4_an_admin_based_group_holding_it_is_refused_with_guardrail_6() {
    let mut set = RoleGrants::seed(Role::Admin);
    for p in [KIND, P::ApprovalsDecide] {
        if !set.contains(&p) {
            set.push(p);
        }
    }
    for group in [
        Group {
            code: "ADMIN_PLUS".to_owned(),
            name: "ADMIN_PLUS".to_owned(),
            base: Role::Admin,
            built_in: false,
        },
        Group::built_in(Role::Admin),
    ] {
        assert_eq!(
            MatrixGuardrails::check(&group, &set),
            Err(GuardrailBreach::AdminBaseHoldsApproval),
            "{} holding ApproveLedgerAdjustment",
            group.code
        );
    }
}
