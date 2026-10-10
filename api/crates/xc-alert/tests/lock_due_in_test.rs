//! AL-2 (#1002, rate alerts RA-2 of #561) — `RateAlertRepository::lock_due_in`, delivered here for AL-4 and tested
//! here (design §2 R13, §9, §9.1 row "RA-2").
//!
//! Written before the implementation by the ticket's author (claude/5edf7bb5). Do not edit, delete or weaken;
//! add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (see `common/mod.rs`); nothing here skips.
//!
//! Signature this test assumes (ticket §7, `xc-alert/src/domain/repository.rs`):
//! `lock_due_in<'a>(&'a self, tx: &'a mut Transaction<'_, Postgres>, branch: &'a BranchId, version: i32)
//!  -> BoxFuture<'a, AppResult<Vec<RateAlert>>>`. "Unexpired" is judged by the database clock (`now()`).

mod common;

use std::collections::BTreeSet;

use common::*;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{Side, UnitSize};

fn ids(alerts: &[RateAlert]) -> BTreeSet<String> {
    alerts.iter().map(|a| a.id.to_string()).collect()
}

fn set(of: &[RateAlertId]) -> BTreeSet<String> {
    of.iter().map(|id| id.to_string()).collect()
}

#[tokio::test]
async fn returns_only_enabled_unexpired_undeleted_alerts_not_yet_at_the_version() {
    let db = TestDb::world().await;
    let never = db.insert_alert(CX1, MAIN, &[]).await;
    let older = db
        .insert_alert(CX1, MAIN, &[("last_publication_version", "1")])
        .await;
    let no_expiry = db.insert_alert(CX2, MAIN, &[("expires_at", "NULL")]).await;
    // Not due:
    db.insert_alert(CX1, MAIN, &[("last_publication_version", "2")])
        .await;
    db.insert_alert(CX1, MAIN, &[("last_publication_version", "3")])
        .await;
    db.insert_alert(CX1, MAIN, &[("enabled", "false")]).await;
    db.insert_alert(CX1, MAIN, &[("expires_at", "now() - interval '1 second'")])
        .await;
    db.insert_alert(CX1, MAIN, &[("deleted_at", "now()")]).await;
    db.insert_alert(CX1, NORTH, &[]).await;

    let repo = db.repository();
    let mut tx = db.begin().await;
    let due = repo
        .lock_due_in(&mut tx, &BranchId::new(MAIN), 2)
        .await
        .expect("lock_due_in reads");
    tx.rollback().await.expect("roll back");

    assert_eq!(
        ids(&due),
        set(&[never, older, no_expiry]),
        "only enabled, unexpired, undeleted alerts of the branch with last_publication_version < 2 or NULL (R13)"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_returned_alert_carries_its_row() {
    let db = TestDb::world().await;
    let id = db
        .insert_alert(
            CX2,
            MAIN,
            &[
                ("currency_code", "'JPY'"),
                ("shop_side", "'BUY'"),
                ("unit_size", "100"),
                ("condition", "'BELOW'"),
                ("target", "21.12345678"),
                ("cooldown_seconds", "3600"),
                ("relation", "'NOT_SATISFIED'"),
                ("episode_no", "4"),
                ("last_publication_version", "1"),
                ("version", "7"),
            ],
        )
        .await;

    let repo = db.repository();
    let mut tx = db.begin().await;
    let due = repo
        .lock_due_in(&mut tx, &BranchId::new(MAIN), 2)
        .await
        .expect("lock_due_in reads");
    tx.rollback().await.expect("roll back");

    assert_eq!(due.len(), 1);
    let a = &due[0];
    assert_eq!(a.id, id);
    assert_eq!(a.organization_id, org());
    assert_eq!(a.customer_id, CX2);
    assert_eq!(a.branch, BranchId::new(MAIN));
    assert_eq!(a.currency, currency("JPY"));
    assert_eq!(a.shop_side, Side::Buy);
    assert_eq!(a.unit_size, UnitSize::parse("100").expect("a unit size"));
    assert_eq!(a.condition, AlertCondition::Below);
    assert_eq!(a.target, dec("21.12345678"));
    assert!(a.enabled);
    assert_eq!(a.cooldown_seconds, 3600);
    assert_eq!(a.relation, Relation::NotSatisfied);
    assert_eq!(a.episode_no, 4);
    assert_eq!(a.last_publication_version, Some(1));
    assert_eq!(a.deleted_at, None);
    assert_eq!(a.version, 7);
    db.finish().await;
}

#[tokio::test]
async fn a_second_transaction_at_the_same_time_gets_none_of_the_first_ones_rows() {
    let db = TestDb::world().await;
    let a = db.insert_alert(CX1, MAIN, &[]).await;
    let b = db.insert_alert(CX2, MAIN, &[]).await;
    let repo = db.repository();
    let branch = BranchId::new(MAIN);

    let mut first = db.begin().await;
    let held = repo
        .lock_due_in(&mut first, &branch, 1)
        .await
        .expect("the first transaction locks");
    assert_eq!(ids(&held), set(&[a, b]));

    let mut second = db.begin().await;
    let got = within(
        MUST_FINISH,
        "the second lock_due_in must not wait for the first (SKIP LOCKED)",
        repo.lock_due_in(&mut second, &branch, 1),
    )
    .await
    .expect("the second transaction reads");
    assert!(
        got.is_empty(),
        "rows the first transaction holds are skipped, not returned twice: {:?}",
        ids(&got)
    );
    second.rollback().await.expect("roll back the second");

    first.rollback().await.expect("release the first");
    let mut third = db.begin().await;
    let again = repo
        .lock_due_in(&mut third, &branch, 1)
        .await
        .expect("the third transaction locks");
    assert_eq!(
        ids(&again),
        set(&[a, b]),
        "once released, the rows are due again"
    );
    third.rollback().await.expect("roll back the third");
    db.finish().await;
}
