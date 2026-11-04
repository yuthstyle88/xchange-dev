//! BC-0 (`docs/design/board-currencies.md` §6, §7): MAIN's seed, and reading it. This holds on the skeleton.

mod rate_engine_common;

use rate_engine_common::*;
use xc_rate::domain::currency::branch_currencies::{BranchCurrencies, Display};
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::infrastructure::currency::branch_currencies::PgBranchCurrencies;

async fn count(db: &TestDb, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

#[tokio::test]
async fn main_has_45_rows_4_show_14_ask_27_hide() {
    let db = TestDb::start().await;
    let mut tx = db.begin().await;
    let rows = PgBranchCurrencies
        .rows_in(&mut tx, &main_branch())
        .await
        .expect("read");
    tx.rollback().await.expect("roll back");
    let of = |d: Display| rows.iter().filter(|r| r.display == d).count();
    assert_eq!(
        (
            rows.len(),
            of(Display::Show),
            of(Display::Ask),
            of(Display::Hide)
        ),
        (45, 4, 14, 27)
    );
    let first: Vec<&str> = rows.iter().take(7).map(|r| r.code.as_str()).collect();
    assert_eq!(
        first,
        ["THB", "USD", "EUR", "GBP", "JPY", "CNY", "AUD"],
        "v2's report look first"
    );
}

#[tokio::test]
async fn display_in_answers_a_row_and_none_without_one() {
    let db = TestDb::start().await;
    let mut tx = db.begin().await;
    let usd = PgBranchCurrencies
        .display_in(&mut tx, &main_branch(), code("USD"), &GroupCode::all())
        .await
        .expect("read");
    let myrs = PgBranchCurrencies
        .display_in(
            &mut tx,
            &main_branch(),
            code("MYR"),
            &GroupCode::new("MYRS").expect("a group"),
        )
        .await
        .expect("read");
    let none = PgBranchCurrencies
        .display_in(&mut tx, &main_branch(), code("PLN"), &GroupCode::all())
        .await
        .expect("read");
    tx.rollback().await.expect("roll back");
    assert_eq!(
        (usd, myrs, none),
        (Some(Display::Show), Some(Display::Ask), None)
    );
}

#[tokio::test]
async fn every_board_currency_has_notes_except_the_two_open_variants() {
    let db = TestDb::start().await;
    let without: Vec<String> = sqlx::query_scalar(
        "SELECT trim(r.currency_code) || '/' || r.group_code FROM branch_currencies r \
         WHERE r.branch_id = 'MAIN' AND NOT EXISTS ( \
             SELECT 1 FROM branch_currency_notes n \
             WHERE n.branch_id = r.branch_id AND n.currency_code = r.currency_code AND n.group_code = r.group_code) \
         ORDER BY 1",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read");
    assert_eq!(
        without,
        ["GBP/SCP", "KRW/KRWS"],
        "the owner has not given these notes yet"
    );
}

#[tokio::test]
async fn a_variant_takes_its_notes_from_the_all_row() {
    let db = TestDb::start().await;
    let faces: Vec<String> = sqlx::query_scalar(
        "SELECT n.group_code || ':' || trim(to_char(d.face_value, 'FM99990')) \
         FROM branch_currency_notes n JOIN denominations d ON d.id = n.denomination_id \
         WHERE n.branch_id = 'MAIN' AND n.currency_code = 'MYR' ORDER BY n.group_code, d.face_value",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read");
    assert_eq!(
        faces,
        ["ALL:1", "ALL:50", "ALL:100", "MYRS:5", "MYRS:10", "MYRS:20"]
    );
}

#[tokio::test]
async fn a_note_of_another_currency_cannot_be_linked() {
    let db = TestDb::start().await;
    let usd_note: i64 = sqlx::query_scalar(
        "SELECT id FROM denominations WHERE currency_code = 'USD' AND face_value = 100",
    )
    .fetch_one(&db.pool)
    .await
    .expect("USD 100");
    let linked = sqlx::query(
        "INSERT INTO branch_currency_notes (branch_id, currency_code, group_code, denomination_id) \
         VALUES ('MAIN', 'EUR', 'ALL', $1)",
    )
    .bind(usd_note)
    .execute(&db.pool)
    .await;
    assert!(linked.is_err(), "the composite FK refuses it");
    assert_eq!(
        count(
            &db,
            "SELECT count(DISTINCT currency_code) FROM denominations WHERE face_value IS NOT NULL"
        )
        .await,
        40
    );
}
