//! Up-front tests for #3976 (option B, owner 2026-10-20): the seed version per built-in group.
//! Written before the implementation by claude/studio-claude-4 on `tests/3976-seed-version`, on the BE lane
//! controller claude/a90ab16c's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! The design (#3976 §5): `roles.seed_version` (integer, 0 for custom groups, 1 on every built-in row after the
//! migration) records the seed version a built-in group has received. `RoleGrantStore::seed_additions` grants each
//! built-in group the permissions of `RoleGrants::SEED_ADDITIONS` newer than its `seed_version` that its
//! `RoleGrants::seed` holds, then raises it to `RoleGrants::SEED_VERSION`. A permission the owner removed is never
//! re-added. `seed_missing` also sets `seed_version = SEED_VERSION` on every group it seeds.
//!
//! Each test gets a schema of its own with every migration in `api/migrations/` applied (`account_common::TestDb`),
//! writes through the store in a committed transaction, and reads back with plain SQL. Expected sets come from
//! `RoleGrants::seed` and `RoleGrants::SEED_ADDITIONS`, never from hand-typed group lists. The one hand-typed list is
//! [`BASELINE_V1`], the version-1 permissions, which is the anti-forget gate's record.
//!
//! Needs a real PostgreSQL: `TEST_DATABASE_URL` naming a database whose name ends in `_test`. Without it these
//! tests FAIL; none skips.
//!
//! Names these tests assume (they do not exist before #3976, so this file does not compile until then):
//! - `RoleGrants::SEED_VERSION: i32` and `RoleGrants::SEED_ADDITIONS: &[(i32, Permission)]`;
//! - `RoleGrantStore::seed_additions(&mut Transaction<'_, Postgres>) -> Result<u64, RoleGrantStoreError>`, answering
//!   how many grant rows it inserted;
//! - `roles.seed_version integer NOT NULL DEFAULT 0` (migration `api/migrations/<slot>_role_seed_version.sql`).

mod account_common;

use std::collections::BTreeSet;

use account_common::{TestDb, unique};
use xc_auth::domain::access::{Group, Permission, Role, RoleGrantStore, RoleGrants};
use xc_auth::infrastructure::access::PgRoleGrantStore;

/// The seed's permissions at version 1: `Permission::ALL` on `main` before PT-14 (#3968, PR 3975, `de5c4147^`),
/// 51 entries. A permission added after that must be listed in `RoleGrants::SEED_ADDITIONS`, never here.
const BASELINE_V1: [Permission; 51] = [
    Permission::OverviewRead,
    Permission::RatesDraft,
    Permission::RatesHistoryRead,
    Permission::RatePolicySave,
    Permission::BranchesManage,
    Permission::UsersManage,
    Permission::ApprovalsDecide,
    Permission::ShiftOperate,
    Permission::ShiftCloseCertify,
    Permission::BookingsDesk,
    Permission::ExchangeTransact,
    Permission::TransactionRead,
    Permission::ReceiptReprint,
    Permission::ReversalRequest,
    Permission::CashManage,
    Permission::QueueOperate,
    Permission::SystemSettings,
    Permission::CompatRead,
    Permission::RequestAbandon,
    Permission::DocumentsList,
    Permission::FileRead,
    Permission::ProofUpload,
    Permission::DeviceQueue,
    Permission::DeviceProcess,
    Permission::DevicePrinterOnline,
    Permission::DrawerPassport,
    Permission::FinanceReconcile,
    Permission::VerificationReview,
    Permission::IdentityDocumentRead,
    Permission::MarketingCampaigns,
    Permission::MarketingMembers,
    Permission::MarketingPerformance,
    Permission::CustomerSelf,
    Permission::AttendanceClock,
    Permission::CustomerHoldPlace,
    Permission::CashTargetSet,
    Permission::BranchDayOperate,
    Permission::SignedIn,
    Permission::ReferenceRatesRead,
    Permission::ConsentPolicyManage,
    Permission::ReferralAttach,
    Permission::ReferralSupersede,
    Permission::ReferralRead,
    Permission::CashLimitsManage,
    Permission::ReportsExport,
    Permission::AlertSettingsSave,
    Permission::CashTransferPolicySet,
    Permission::RateAlertSettingsSave,
    Permission::OrgStartingPricesManage,
    Permission::LoyaltyRulesManage,
    Permission::AuditLogsRead,
];

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

/// The permissions added after version 1 that `role`'s seed holds.
fn additions_in_seed_of(role: Role) -> Vec<Permission> {
    let seed = RoleGrants::seed(role);
    RoleGrants::SEED_ADDITIONS
        .iter()
        .map(|&(_, p)| p)
        .filter(|p| seed.contains(p))
        .collect()
}

/// A built-in group whose seed holds at least one addition, and one whose seed holds none.
fn group_with_additions() -> Role {
    RoleGrants::BUILT_IN
        .into_iter()
        .find(|&r| !additions_in_seed_of(r).is_empty())
        .expect("some built-in group's seed holds a SEED_ADDITIONS permission (MarketRatesRefresh: RATES_STOCK Create)")
}

fn group_without_additions() -> Role {
    RoleGrants::BUILT_IN
        .into_iter()
        .find(|&r| additions_in_seed_of(r).is_empty())
        .expect("some built-in group's seed holds no SEED_ADDITIONS permission")
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

async fn row_count(db: &TestDb, code: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM role_permission_grants WHERE role_code = $1")
        .bind(code)
        .fetch_one(&db.pool)
        .await
        .expect("count role_permission_grants")
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

async fn seed_additions(db: &TestDb) -> u64 {
    let mut tx = db.pool.begin().await.expect("begin");
    let inserted = PgRoleGrantStore::new()
        .seed_additions(&mut tx)
        .await
        .expect("seed_additions");
    tx.commit().await.expect("commit");
    inserted
}

async fn save_set(db: &TestDb, code: &str, set: &[Permission]) {
    let mut tx = db.pool.begin().await.expect("begin");
    PgRoleGrantStore::new()
        .save_set(&mut tx, code, set)
        .await
        .expect("save_set");
    tx.commit().await.expect("commit");
}

async fn add_group(db: &TestDb, group: &Group, set: &[Permission]) {
    let mut tx = db.pool.begin().await.expect("begin");
    PgRoleGrantStore::new()
        .add_group(&mut tx, group, set)
        .await
        .expect("add_group");
    tx.commit().await.expect("commit");
}

/// A database as it stands right after the #3976 migration on a database that booted since M2: every built-in group
/// holds its version-1 seed (its seed without the additions) and is at `seed_version = 1`.
async fn database_at_version_1() -> TestDb {
    let db = TestDb::start().await;
    seed_missing(&db).await;
    for &(_, p) in RoleGrants::SEED_ADDITIONS {
        sqlx::query(
            "DELETE FROM role_permission_grants WHERE permission = $1 \
             AND role_code IN (SELECT code FROM roles WHERE built_in)",
        )
        .bind(serde_name(p))
        .execute(&db.pool)
        .await
        .expect("take an addition back out");
    }
    sqlx::query("UPDATE roles SET seed_version = 1 WHERE built_in")
        .execute(&db.pool)
        .await
        .expect("set the built-in groups to seed version 1");
    db
}

/// #3976 §7 row 1: a built-in group at seed version 1 whose seed holds an addition gains exactly the additions in its
/// seed, nothing else, ends at `SEED_VERSION`, and `role_grant_version` is bumped once.
#[tokio::test]
async fn a_version_1_group_gains_exactly_the_additions_in_its_seed() {
    let db = database_at_version_1().await;
    let role = group_with_additions();
    let code = role.as_str();
    let before = stored_set(&db, code).await;
    let version_before = grant_version(&db).await;
    let expected_inserted: usize = RoleGrants::BUILT_IN
        .into_iter()
        .map(|r| additions_in_seed_of(r).len())
        .sum();

    let inserted = seed_additions(&db).await;

    let after = stored_set(&db, code).await;
    let gained: BTreeSet<String> = after.difference(&before).cloned().collect();
    assert_eq!(
        gained,
        names(&additions_in_seed_of(role)),
        "{code} gains exactly the SEED_ADDITIONS its seed holds"
    );
    assert!(before.is_subset(&after), "{code} loses nothing it held");
    assert_eq!(
        after,
        names(&RoleGrants::seed(role)),
        "{code} ends with its full seed"
    );
    assert_eq!(
        inserted, expected_inserted as u64,
        "seed_additions answers the grant rows it inserted, over every built-in group"
    );
    assert_eq!(seed_version(&db, code).await, RoleGrants::SEED_VERSION);
    assert_eq!(
        grant_version(&db).await,
        version_before + 1,
        "role_grant_version is bumped exactly once"
    );
}

/// #3976 §7 row 1, the other half: a built-in group whose seed holds no addition gains nothing and still ends at
/// `SEED_VERSION`.
#[tokio::test]
async fn a_group_whose_seed_holds_no_addition_gains_nothing_and_ends_at_seed_version() {
    let db = database_at_version_1().await;
    let role = group_without_additions();
    let code = role.as_str();
    let before = stored_set(&db, code).await;

    seed_additions(&db).await;

    assert_eq!(stored_set(&db, code).await, before, "{code} gains nothing");
    assert_eq!(seed_version(&db, code).await, RoleGrants::SEED_VERSION);
}

/// #3976 §7 row 2: the owner removed an addition from a built-in group that had already received it; the next
/// `seed_additions` does not give it back.
#[tokio::test]
async fn an_addition_the_owner_removed_is_not_given_back() {
    let db = TestDb::start().await;
    seed_missing(&db).await;
    let role = group_with_additions();
    let code = role.as_str();
    assert_eq!(seed_version(&db, code).await, RoleGrants::SEED_VERSION);
    let removed = additions_in_seed_of(role);
    let kept: Vec<Permission> = RoleGrants::seed(role)
        .into_iter()
        .filter(|p| !removed.contains(p))
        .collect();
    save_set(&db, code, &kept).await;
    let version_before = grant_version(&db).await;

    let inserted = seed_additions(&db).await;

    assert_eq!(inserted, 0, "nothing is inserted");
    assert_eq!(
        stored_set(&db, code).await,
        names(&kept),
        "{code} keeps the owner's removal of {removed:?}"
    );
    assert_eq!(seed_version(&db, code).await, RoleGrants::SEED_VERSION);
    assert_eq!(
        grant_version(&db).await,
        version_before,
        "no bump when nothing is inserted"
    );
}

/// #3976 §7 row 3: a custom group (`built_in = false`) is untouched, even one based on a group whose seed holds an
/// addition.
#[tokio::test]
async fn a_custom_group_is_untouched() {
    let db = database_at_version_1().await;
    let base = group_with_additions();
    let custom = Group {
        code: unique("CUSTOM").to_uppercase(),
        name: "กลุ่มกำหนดเอง".to_owned(),
        base,
        built_in: false,
    };
    let set: Vec<Permission> = RoleGrants::seed(base)
        .into_iter()
        .filter(|p| !additions_in_seed_of(base).contains(p))
        .collect();
    add_group(&db, &custom, &set).await;
    let version_of_custom = seed_version(&db, &custom.code).await;

    seed_additions(&db).await;

    assert_eq!(
        stored_set(&db, &custom.code).await,
        names(&set),
        "the custom group's set is unchanged"
    );
    assert_eq!(
        seed_version(&db, &custom.code).await,
        version_of_custom,
        "the custom group's seed_version is unchanged"
    );
    assert_eq!(
        version_of_custom, 0,
        "a custom group stays at seed version 0"
    );
}

/// #3976 §7 row 4: on a fresh database, `seed_missing` then `seed_additions` leaves every built-in group at
/// `SEED_VERSION` with its full seed and no duplicate rows; `seed_additions` inserts nothing there.
#[tokio::test]
async fn a_fresh_database_ends_at_seed_version_with_the_full_seed_and_no_duplicates() {
    let db = TestDb::start().await;
    seed_missing(&db).await;
    let version_after_seed = grant_version(&db).await;

    let inserted = seed_additions(&db).await;

    assert_eq!(
        inserted, 0,
        "a freshly seeded group already holds every addition in its seed"
    );
    assert_eq!(
        grant_version(&db).await,
        version_after_seed,
        "no second bump"
    );
    for role in RoleGrants::BUILT_IN {
        let code = role.as_str();
        let seed = RoleGrants::seed(role);
        assert_eq!(
            stored_set(&db, code).await,
            names(&seed),
            "{code} holds its full seed"
        );
        assert_eq!(
            row_count(&db, code).await,
            seed.len() as i64,
            "{code} has no duplicate rows"
        );
        assert_eq!(
            seed_version(&db, code).await,
            RoleGrants::SEED_VERSION,
            "{code}"
        );
    }
}

/// #3976 §7 row 5: running `seed_additions` twice inserts nothing the second time and bumps the version once.
#[tokio::test]
async fn running_seed_additions_twice_inserts_nothing_the_second_time() {
    let db = database_at_version_1().await;
    let version_before = grant_version(&db).await;

    let first = seed_additions(&db).await;
    let second = seed_additions(&db).await;

    assert!(first > 0, "the first run grants the additions");
    assert_eq!(second, 0, "the second run inserts nothing");
    assert_eq!(
        grant_version(&db).await,
        version_before + 1,
        "role_grant_version is bumped once over both runs"
    );
}

/// #3976 §7 row 6, the anti-forget gate (no database): every permission is in the version-1 baseline or in
/// `SEED_ADDITIONS`, never both; every addition's version is in `2..=SEED_VERSION`; and `SEED_VERSION` is the newest.
#[test]
fn every_permission_is_in_the_version_1_baseline_or_in_seed_additions() {
    let baseline: BTreeSet<Permission> = BASELINE_V1.into_iter().collect();
    assert_eq!(
        baseline.len(),
        BASELINE_V1.len(),
        "BASELINE_V1 has no duplicates"
    );
    let added: BTreeSet<Permission> = RoleGrants::SEED_ADDITIONS.iter().map(|&(_, p)| p).collect();
    assert_eq!(
        added.len(),
        RoleGrants::SEED_ADDITIONS.len(),
        "SEED_ADDITIONS lists each permission once"
    );
    for p in Permission::ALL {
        assert!(
            baseline.contains(&p) || added.contains(&p),
            "{p:?} is in Permission::ALL but not in SEED_ADDITIONS: register it there with the next seed version, \
             or no existing database's built-in groups will ever receive it (#3976)"
        );
    }
    for p in &added {
        assert!(
            !baseline.contains(p),
            "{p:?} is in BASELINE_V1 and SEED_ADDITIONS"
        );
    }
    for &(v, p) in RoleGrants::SEED_ADDITIONS {
        assert!(
            (2..=RoleGrants::SEED_VERSION).contains(&v),
            "{p:?} is added in version {v}, outside 2..={}",
            RoleGrants::SEED_VERSION
        );
    }
    // #4155 W/A1: SEED_VERSION is the newest of SEED_ADDITIONS and SEED_REMOVALS.
    assert_eq!(
        RoleGrants::SEED_ADDITIONS
            .iter()
            .chain(RoleGrants::SEED_REMOVALS.iter())
            .map(|&(v, _)| v)
            .max()
            .unwrap_or(1),
        RoleGrants::SEED_VERSION,
        "SEED_VERSION is the newest of SEED_ADDITIONS and SEED_REMOVALS"
    );
}
