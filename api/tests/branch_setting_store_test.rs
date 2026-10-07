//! Written before the implementation (row SET, design `docs/design/xchange-v3.md` §5.5.1).
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! `BranchSettingStore`: one versioned document per (branch, name), with a history row for
//! every change, written on the caller's branch transaction. It knows nothing of payments — the
//! names used here are made up on purpose. What it promises:
//!
//! - no row reads as nothing (the document's service answers its defaults at version 0);
//! - `write_in` compares the stored version (no row = 0) with the one the caller read, writes
//!   version + 1 and the history row in the same transaction, `before` being the stored body or
//!   the caller's `first_before` when there was no row;
//! - a stale version is refused with `VersionConflict` and writes nothing;
//! - two branches, or two names, never see each other's document or history;
//! - the history is newest first and cut to the limit asked for.
//!
//! The migration and its data-migration section are held by `branch_settings_migration_test.rs`.
//!
//! Needs `TEST_DATABASE_URL` (see `snapshot_common/mod.rs`); nothing here skips.

mod compat_common;
mod snapshot_common;

use chrono::{DateTime, Duration, Utc};
use pos_error::AppResult;
use serde_json::{Value, json};
use snapshot_common::{TestDb, branch, time};
use xc_kernel::branch_tx::BranchTx;
use xchange::infrastructure::settings::branch_setting_store::BranchSettingStore;
use xchange::infrastructure::settings::stored_setting::{
    SettingChange, StoredSetting, VersionConflict,
};

const MAIN: &str = "MAIN";
const AIRPORT: &str = "AIRPORT";
/// A document name nobody uses: the store must not care what it holds.
const NAME: &str = "testDocument";
const OTHER_NAME: &str = "otherDocument";

fn at(n: i64) -> DateTime<Utc> {
    time("2026-10-08T03:00:00.000Z") + Duration::seconds(n)
}

fn body(n: i64) -> Value {
    json!({ "n": n, "rows": [{ "id": "a", "on": n % 2 == 0 }], "text": format!("ฉบับที่ {n}") })
}

/// What the document's service would have answered before the first save.
fn defaults() -> Value {
    json!({ "n": 0, "rows": [], "text": "ค่าเริ่มต้น" })
}

struct Shop {
    db: TestDb,
    branch_tx: BranchTx,
    store: BranchSettingStore,
}

impl Shop {
    async fn open() -> Shop {
        let db = TestDb::start().await;
        db.seed_branch(AIRPORT, "Airport").await;
        let branch_tx = db.branch_tx();
        let store = BranchSettingStore::new(db.pool.clone());
        Shop {
            db,
            branch_tx,
            store,
        }
    }

    /// One `write_in` on a `BranchTx::run` of `branch` of its own.
    async fn write(
        &self,
        branch_id: &str,
        name: &str,
        expected: i64,
        body: Value,
        actor: &str,
        when: DateTime<Utc>,
    ) -> AppResult<Result<StoredSetting, VersionConflict>> {
        let (store, b, name, actor) = (
            self.store.clone(),
            branch(branch_id),
            name.to_string(),
            actor.to_string(),
        );
        self.branch_tx
            .run(&branch(branch_id), move |tx| {
                Box::pin(async move {
                    store
                        .write_in(tx, &b, &name, expected, &defaults(), &body, &actor, when)
                        .await
                })
            })
            .await
    }

    async fn written(
        &self,
        branch_id: &str,
        name: &str,
        expected: i64,
        body: Value,
        when: DateTime<Utc>,
    ) -> StoredSetting {
        self.write(branch_id, name, expected, body, "owner", when)
            .await
            .unwrap_or_else(|e| panic!("write {branch_id}/{name}: {e:?}"))
            .unwrap_or_else(|_| panic!("write {branch_id}/{name}: version {expected} is current"))
    }

    async fn read(&self, branch_id: &str, name: &str) -> Option<StoredSetting> {
        self.store
            .read(&branch(branch_id), name)
            .await
            .expect("read a document")
    }

    async fn history(&self, branch_id: &str, name: &str, limit: i64) -> Vec<SettingChange> {
        self.store
            .history(&branch(branch_id), name, limit)
            .await
            .expect("read the history")
    }

    async fn finish(self) {
        self.db.finish().await;
    }
}

#[tokio::test]
async fn no_row_reads_as_nothing_and_the_first_write_is_version_one() {
    let shop = Shop::open().await;
    assert_eq!(shop.read(MAIN, NAME).await, None);
    assert_eq!(shop.history(MAIN, NAME, 20).await, Vec::new());

    let stored = shop.written(MAIN, NAME, 0, body(1), at(0)).await;
    let expected = StoredSetting {
        version: 1,
        body: body(1),
        updated_at: at(0),
        updated_by: "owner".to_string(),
    };
    assert_eq!(stored, expected);
    assert_eq!(shop.read(MAIN, NAME).await, Some(expected));

    let next = shop.written(MAIN, NAME, 1, body(2), at(5)).await;
    assert_eq!(next.version, 2);
    assert_eq!(next.body, body(2));
    assert_eq!(next.updated_at, at(5));
    assert_eq!(shop.read(MAIN, NAME).await, Some(next));
    shop.finish().await;
}

#[tokio::test]
async fn every_write_leaves_one_history_row_with_what_was_there_before() {
    let shop = Shop::open().await;
    shop.written(MAIN, NAME, 0, body(1), at(0)).await;
    shop.write(MAIN, NAME, 1, body(2), "manager.on.duty", at(7))
        .await
        .expect("write")
        .expect("version 1 is current");

    let history = shop.history(MAIN, NAME, 20).await;
    assert_eq!(
        history,
        vec![
            SettingChange {
                version: 2,
                actor: "manager.on.duty".to_string(),
                at: at(7),
                before: body(1),
                after: body(2),
            },
            SettingChange {
                version: 1,
                actor: "owner".to_string(),
                at: at(0),
                // no row before the first write: `before` is what the caller says it read
                before: defaults(),
                after: body(1),
            },
        ]
    );
    shop.finish().await;
}

#[tokio::test]
async fn a_stale_version_is_refused_and_writes_nothing() {
    let shop = Shop::open().await;
    let first = shop.written(MAIN, NAME, 0, body(1), at(0)).await;

    for stale in [0, 2, 7, -1] {
        let answer = shop
            .write(MAIN, NAME, stale, body(99), "late.reader", at(30))
            .await
            .expect("a refusal is not a database error");
        assert!(
            matches!(answer, Err(VersionConflict { .. })),
            "version {stale} against stored 1: {answer:?}"
        );
    }
    assert_eq!(shop.read(MAIN, NAME).await, Some(first));
    assert_eq!(shop.history(MAIN, NAME, 20).await.len(), 1);

    // a branch with no row is at version 0: anything else is stale there too
    let answer = shop
        .write(AIRPORT, NAME, 1, body(1), "owner", at(31))
        .await
        .expect("a refusal is not a database error");
    assert!(matches!(answer, Err(VersionConflict { .. })), "{answer:?}");
    assert_eq!(shop.read(AIRPORT, NAME).await, None);
    assert_eq!(shop.history(AIRPORT, NAME, 20).await, Vec::new());
    shop.finish().await;
}

/// The document and its history row commit together or not at all: a transaction that fails
/// after `write_in` leaves neither.
#[tokio::test]
async fn the_document_and_its_history_row_are_one_transaction() {
    let shop = Shop::open().await;
    let (store, b) = (shop.store.clone(), branch(MAIN));
    let failed: AppResult<()> = shop
        .branch_tx
        .run(&branch(MAIN), move |tx| {
            Box::pin(async move {
                store
                    .write_in(tx, &b, NAME, 0, &defaults(), &body(1), "owner", at(0))
                    .await?
                    .expect("version 0 is current");
                Err(pos_error::AppError::Domain(
                    "a later step refused".to_string(),
                ))
            })
        })
        .await;
    assert!(failed.is_err());
    assert_eq!(shop.read(MAIN, NAME).await, None);
    assert_eq!(shop.history(MAIN, NAME, 20).await, Vec::new());

    // and the caller's transaction sees its own write before it commits
    let (store, b) = (shop.store.clone(), branch(MAIN));
    let seen = shop
        .branch_tx
        .run(&branch(MAIN), move |tx| {
            Box::pin(async move {
                let before = store.read_in(tx, &b, NAME).await?;
                store
                    .write_in(tx, &b, NAME, 0, &defaults(), &body(1), "owner", at(1))
                    .await?
                    .expect("version 0 is current");
                let after = store.read_in(tx, &b, NAME).await?;
                Ok((before, after))
            })
        })
        .await
        .expect("the run commits");
    assert_eq!(seen.0, None);
    assert_eq!(seen.1.map(|s| s.version), Some(1));
    shop.finish().await;
}

/// "มันต้องรองรับหลาย สาขา": each branch has its own document, version and history.
#[tokio::test]
async fn two_branches_never_see_each_others_document_or_history() {
    let shop = Shop::open().await;
    shop.written(MAIN, NAME, 0, body(1), at(0)).await;
    shop.written(MAIN, NAME, 1, body(2), at(1)).await;
    assert_eq!(shop.read(AIRPORT, NAME).await, None);
    assert_eq!(shop.history(AIRPORT, NAME, 20).await, Vec::new());

    let airport = shop.written(AIRPORT, NAME, 0, body(10), at(2)).await;
    assert_eq!(airport.version, 1, "the airport counts from its own zero");
    assert_eq!(shop.read(MAIN, NAME).await.map(|s| s.body), Some(body(2)));
    assert_eq!(
        shop.read(AIRPORT, NAME).await.map(|s| s.body),
        Some(body(10))
    );
    let main: Vec<i64> = shop
        .history(MAIN, NAME, 20)
        .await
        .iter()
        .map(|c| c.version)
        .collect();
    assert_eq!(main, vec![2, 1]);
    let airport: Vec<Value> = shop
        .history(AIRPORT, NAME, 20)
        .await
        .into_iter()
        .map(|c| c.after)
        .collect();
    assert_eq!(airport, vec![body(10)]);
    shop.finish().await;
}

#[tokio::test]
async fn two_names_in_one_branch_are_two_documents() {
    let shop = Shop::open().await;
    shop.written(MAIN, NAME, 0, body(1), at(0)).await;
    assert_eq!(shop.read(MAIN, OTHER_NAME).await, None);
    let other = shop.written(MAIN, OTHER_NAME, 0, body(5), at(1)).await;
    assert_eq!(other.version, 1);
    assert_eq!(shop.read(MAIN, NAME).await.map(|s| s.body), Some(body(1)));
    assert_eq!(shop.history(MAIN, OTHER_NAME, 20).await.len(), 1);
    assert_eq!(shop.history(MAIN, NAME, 20).await.len(), 1);
    shop.finish().await;
}

#[tokio::test]
async fn the_history_is_newest_first_and_cut_to_the_limit() {
    let shop = Shop::open().await;
    for n in 0..25 {
        shop.written(MAIN, NAME, n, body(n + 1), at(n)).await;
    }
    let history = shop.history(MAIN, NAME, 20).await;
    let versions: Vec<i64> = history.iter().map(|c| c.version).collect();
    assert_eq!(versions, (6..=25).rev().collect::<Vec<i64>>());
    assert_eq!(history[0].before, body(24));
    assert_eq!(history[0].after, body(25));
    assert_eq!(shop.history(MAIN, NAME, 3).await.len(), 3);
    shop.finish().await;
}

/// Two saves that read the same version race on one branch: the branch lock lines them up, so
/// exactly one is written and the other is told its version is stale.
#[tokio::test]
async fn two_writers_of_the_same_version_one_wins_one_is_refused() {
    let shop = Shop::open().await;
    let one = shop.write(MAIN, NAME, 0, body(1), "first", at(0));
    let two = shop.write(MAIN, NAME, 0, body(2), "second", at(0));
    let (one, two) = tokio::join!(one, two);
    let answers = [
        one.expect("not a database error"),
        two.expect("not a database error"),
    ];
    assert_eq!(
        answers.iter().filter(|a| a.is_ok()).count(),
        1,
        "{answers:?}"
    );
    assert_eq!(
        answers
            .iter()
            .filter(|a| matches!(a, Err(VersionConflict { .. })))
            .count(),
        1,
        "{answers:?}"
    );
    assert_eq!(shop.read(MAIN, NAME).await.map(|s| s.version), Some(1));
    assert_eq!(shop.history(MAIN, NAME, 20).await.len(), 1);
    shop.finish().await;
}

/// The table's own rules: a document of a branch that does not exist, or under a name that is
/// not `^[a-z][A-Za-z0-9]{0,39}$`, is an error — never a conflict, never stored.
#[tokio::test]
async fn a_branch_that_does_not_exist_or_a_bad_name_is_an_error() {
    let shop = Shop::open().await;
    let nowhere = shop
        .write("NOWHERE", NAME, 0, body(1), "owner", at(0))
        .await;
    assert!(nowhere.is_err(), "{nowhere:?}");
    for bad in [
        "",
        "Payment",
        "payment-methods",
        "payment methods",
        "1st",
        &"a".repeat(41),
    ] {
        let answer = shop.write(MAIN, bad, 0, body(1), "owner", at(0)).await;
        assert!(answer.is_err(), "name {bad:?}: {answer:?}");
    }
    let longest = "a".repeat(40);
    assert_eq!(
        shop.written(MAIN, &longest, 0, body(1), at(0))
            .await
            .version,
        1
    );
    let none: Vec<(String, String)> =
        sqlx::query_as("SELECT branch_id, name FROM branch_settings WHERE name <> $1")
            .bind(&longest)
            .fetch_all(&shop.db.pool)
            .await
            .expect("read the table");
    assert_eq!(none, Vec::new());
    shop.finish().await;
}
