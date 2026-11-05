//! Board currencies BC-13 (`docs/design/board-currencies.md` §6, §9, #6266): a new branch copies the currency set
//! (including `display`) and note links of `from_branch` in the same transaction.

mod common;

use common::*;
use xc_rate::domain::currency::branch_currencies::{BranchCurrencies, Display};
use xc_rate::infrastructure::currency::branch_currencies::PgBranchCurrencies;

fn create_from(name: &str, from: &str) -> CreateBranch {
    CreateBranch {
        request_id: request_id(),
        name: name.to_owned(),
        from_branch: branch(from),
    }
}

async fn note_links(db: &TestDb, branch_id: &str) -> Vec<(String, String, i64)> {
    sqlx::query_as(
        "SELECT trim(currency_code), group_code, denomination_id \
         FROM branch_currency_notes \
         WHERE branch_id = $1 \
         ORDER BY currency_code, group_code, denomination_id",
    )
    .bind(branch_id)
    .fetch_all(&db.pool)
    .await
    .expect("read note links")
}

#[tokio::test]
async fn new_branch_copies_from_branch_display() {
    let db = TestDb::start().await;
    let made = db
        .branches()
        .create(create_from("Silom", MAIN), ACTOR, at(0))
        .await
        .expect("create");

    let mut tx = db.pool.begin().await.expect("begin");
    let main_rows = PgBranchCurrencies
        .rows_in(&mut tx, &branch(MAIN))
        .await
        .expect("main rows");
    let new_rows = PgBranchCurrencies
        .rows_in(&mut tx, &made.id)
        .await
        .expect("new rows");
    tx.rollback().await.expect("rollback");

    assert!(main_rows.iter().any(|r| r.display == Display::Show));
    assert!(main_rows.iter().any(|r| r.display == Display::Ask));
    assert!(main_rows.iter().any(|r| r.display == Display::Hide));
    assert_eq!(new_rows, main_rows);
    assert_eq!(
        note_links(&db, made.id.as_str()).await,
        note_links(&db, MAIN).await
    );
    db.finish().await;
}

#[tokio::test]
async fn new_branch_from_branch_with_no_rows_creates_no_rows() {
    let db = TestDb::start().await;
    db.seed_branch("BEMPTY", "Empty").await;

    let made = db
        .branches()
        .create(create_from("Silom", "BEMPTY"), ACTOR, at(0))
        .await
        .expect("create");

    let mut tx = db.pool.begin().await.expect("begin");
    let new_rows = PgBranchCurrencies
        .rows_in(&mut tx, &made.id)
        .await
        .expect("new rows");
    tx.rollback().await.expect("rollback");

    assert!(new_rows.is_empty());
    assert!(note_links(&db, made.id.as_str()).await.is_empty());
    db.finish().await;
}

#[tokio::test]
async fn create_failing_later_writes_no_rows() {
    let db = TestDb::start().await;
    sqlx::query(
        "CREATE FUNCTION fail_note_copy() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'boom'; END $$",
    )
    .execute(&db.pool)
    .await
    .expect("create trigger fn");
    sqlx::query(
        "CREATE TRIGGER fail_after_currencies BEFORE INSERT ON branch_currency_notes \
         FOR EACH STATEMENT EXECUTE FUNCTION fail_note_copy()",
    )
    .execute(&db.pool)
    .await
    .expect("create trigger");

    let cmd = create_from("Silom", MAIN);
    let would_be_id = format!("B{}", cmd.request_id);
    let res = db.branches().create(cmd, ACTOR, at(0)).await;
    assert!(
        res.is_err(),
        "later failure in create transaction must abort"
    );

    let branch_count: i64 = sqlx::query_scalar("SELECT count(*) FROM branches WHERE id = $1")
        .bind(&would_be_id)
        .fetch_one(&db.pool)
        .await
        .expect("count branches");
    let currency_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM branch_currencies WHERE branch_id = $1")
            .bind(&would_be_id)
            .fetch_one(&db.pool)
            .await
            .expect("count currencies");
    assert_eq!((branch_count, currency_count), (0, 0));
    db.finish().await;
}
