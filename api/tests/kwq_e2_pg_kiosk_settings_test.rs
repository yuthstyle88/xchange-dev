//! KWQ-E2 — `PgKioskSettingsRepository` against a real PostgreSQL (design
//! `docs/design/blueprint/kiosk-shared-phone.md` §4.2 D14, §4.6): no row reads `None`; the first
//! save inserts version 1, a later one replaces it and bumps the version; each branch has its own
//! row; the table's CHECK refuses an out-of-range value even past the service.
//! Written before the implementation by the slice controller (claude/mbp-xc-ks). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Seeds: `quote_common::TestDb` (all migrations, the seeded branch `MAIN`).
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

mod quote_common;

use chrono::{TimeZone, Utc};
use quote_common::TestDb;
use xc_kernel::ids::BranchId;
use xc_kiosk::domain::kiosk::entities::kiosk_settings::KioskSettings;
use xc_kiosk::domain::kiosk::repositories::kiosk_settings_repository::KioskSettingsRepository;
use xchange::infrastructure::kiosk::pg_kiosk_settings_repository::PgKioskSettingsRepository;

fn main_branch() -> BranchId {
    BranchId::new("MAIN")
}

async fn save(
    db: &TestDb,
    repo: &PgKioskSettingsRepository,
    branch: &BranchId,
    seconds: i32,
    by: &str,
    minute: u32,
) -> KioskSettings {
    let mut tx = db.pool.begin().await.expect("begin");
    let row = repo
        .upsert_in(
            &mut tx,
            branch,
            seconds,
            by,
            Utc.with_ymd_and_hms(2026, 10, 22, 1, minute, 0).unwrap(),
        )
        .await
        .expect("save");
    tx.commit().await.expect("commit");
    row
}

#[tokio::test(flavor = "multi_thread")]
async fn no_row_reads_none_then_saves_insert_and_replace_with_a_bumped_version() {
    let db = TestDb::start().await;
    let repo = PgKioskSettingsRepository::new(db.pool.clone());
    let main = main_branch();

    assert_eq!(repo.get(&main).await.expect("read"), None);

    let first = save(&db, &repo, &main, 15, "owner-1", 0).await;
    let expected_first = KioskSettings {
        branch_id: main.clone(),
        result_seconds: 15,
        updated_by: Some("owner-1".to_owned()),
        updated_at: Some(Utc.with_ymd_and_hms(2026, 10, 22, 1, 0, 0).unwrap()),
        version: 1,
    };
    assert_eq!(first, expected_first);
    assert_eq!(repo.get(&main).await.expect("read"), Some(expected_first));

    let second = save(&db, &repo, &main, 30, "owner-2", 5).await;
    assert_eq!(second.result_seconds, 30);
    assert_eq!(second.updated_by.as_deref(), Some("owner-2"));
    assert_eq!(second.version, 2);
    assert_eq!(repo.get(&main).await.expect("read"), Some(second));
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn each_branch_has_its_own_row() {
    let db = TestDb::start().await;
    db.exec("INSERT INTO branches (id, name, code) VALUES ('KS2', 'Branch KS2', 'KS2')")
        .await;
    let repo = PgKioskSettingsRepository::new(db.pool.clone());
    save(&db, &repo, &main_branch(), 20, "owner-1", 0).await;
    assert_eq!(repo.get(&BranchId::new("KS2")).await.expect("read"), None);
    let other = save(&db, &repo, &BranchId::new("KS2"), 5, "owner-1", 1).await;
    assert_eq!(other.version, 1);
    assert_eq!(
        repo.get(&main_branch())
            .await
            .expect("read")
            .map(|s| s.result_seconds),
        Some(20)
    );
    db.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_table_refuses_an_out_of_range_value_and_keeps_the_row() {
    let db = TestDb::start().await;
    let repo = PgKioskSettingsRepository::new(db.pool.clone());
    let main = main_branch();
    save(&db, &repo, &main, 12, "owner-1", 0).await;

    for bad in [2, 121] {
        let mut tx = db.pool.begin().await.expect("begin");
        let refused = repo
            .upsert_in(&mut tx, &main, bad, "owner-1", Utc::now())
            .await;
        assert!(refused.is_err(), "{bad} must be refused by the CHECK");
        tx.rollback().await.expect("rollback");
    }
    assert_eq!(
        repo.get(&main).await.expect("read").map(|s| (s.result_seconds, s.version)),
        Some((12, 1))
    );
    db.finish().await;
}
