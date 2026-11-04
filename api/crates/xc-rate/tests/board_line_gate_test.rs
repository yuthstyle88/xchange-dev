//! Board currencies BC-3 (`docs/design/board-currencies.md` D-7, §3): `line_for` is the one gate for `HIDE`, and it
//! refuses a currency with no row. Written before the implementation by the sub-controller (claude/94389a6b).
//! Do not edit, delete or weaken; add tests in a file of your own.

mod rate_engine_common;

use rate_engine_common::*;
use xc_kernel::money::Side;
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::domain::draft::rate_draft::DraftStatus;
use xc_rate::domain::publication::published_rates::{LineSelector, PublishedRates};

async fn publish_eur(db: &TestDb, e: &Engine) {
    let lines = vec![manual_line("EUR", "37.4", "38.0")];
    let mut tx = db.begin().await;
    let d = e
        .drafts
        .create(&mut tx, &main_branch(), &owner_user(), lines)
        .await
        .expect("create");
    tx.commit().await.expect("commit");
    let mut tx = db.begin().await;
    let outcome = e
        .drafts
        .submit(&mut tx, &main_branch(), &owner_user(), &d.id, &key())
        .await
        .expect("submit");
    tx.commit().await.expect("commit");
    assert_eq!(outcome.status, DraftStatus::Approved);
    let mut tx = db.begin().await;
    e.publisher
        .publish(&mut tx, &main_branch(), &owner_user(), &d.id, None)
        .await
        .expect("publish");
    tx.commit().await.expect("commit");
}

async fn has_line(db: &TestDb, e: &Engine) -> bool {
    let mut tx = db.begin().await;
    let got = e
        .published
        .line_for(
            &mut tx,
            &main_branch(),
            LineSelector {
                code: code("EUR"),
                side: Side::Buy,
                group: GroupCode::all(),
                quantity: None,
            },
        )
        .await
        .expect("read");
    tx.rollback().await.expect("roll back");
    got.is_some()
}

async fn set_display(db: &TestDb, display: &str) {
    sqlx::query(
        "UPDATE branch_currencies SET display = $1 \
         WHERE branch_id = 'MAIN' AND currency_code = 'EUR' AND group_code = 'ALL'",
    )
    .bind(display)
    .execute(&db.pool)
    .await
    .expect("set display");
}

#[tokio::test]
async fn bc3_show_and_ask_read_the_line() {
    let db = TestDb::start().await;
    let e = db.engine();
    publish_eur(&db, &e).await;
    assert!(has_line(&db, &e).await, "SHOW");
    set_display(&db, "ASK").await;
    assert!(has_line(&db, &e).await, "ASK: the desk can still quote it");
}

#[tokio::test]
async fn bc3_hide_refuses_the_line() {
    let db = TestDb::start().await;
    let e = db.engine();
    publish_eur(&db, &e).await;
    set_display(&db, "HIDE").await;
    assert!(!has_line(&db, &e).await);
}

#[tokio::test]
async fn bc3_no_row_refuses_the_line() {
    let db = TestDb::start().await;
    let e = db.engine();
    publish_eur(&db, &e).await;
    sqlx::query("DELETE FROM branch_currency_notes WHERE branch_id = 'MAIN' AND currency_code = 'EUR'")
        .execute(&db.pool)
        .await
        .expect("delete links");
    sqlx::query("DELETE FROM branch_currencies WHERE branch_id = 'MAIN' AND currency_code = 'EUR'")
        .execute(&db.pool)
        .await
        .expect("delete the row");
    assert!(!has_line(&db, &e).await);
}
