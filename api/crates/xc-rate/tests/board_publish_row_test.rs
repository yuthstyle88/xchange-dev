//! Board currencies BC-7 (`docs/design/board-currencies.md` §4, §9):
//! - `publish` refuses with `ValidationFailed` when a draft line's `(currency, group)` has no `branch_currencies` row.
//! - `HIDE` lines may publish.

mod rate_engine_common;

use rate_engine_common::*;
use xc_rate::domain::draft::rate_draft::DraftStatus;
use xc_rate::domain::draft::rate_engine_error::RateEngineError;

#[tokio::test]
async fn publish_a_line_with_no_row_is_refused_validation_failed() {
    let db = TestDb::start().await;
    let e = db.engine();
    let lines = vec![manual_line("PLN", "8.5", "9.0")];

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
    let err = e
        .publisher
        .publish(&mut tx, &main_branch(), &owner_user(), &d.id, None)
        .await
        .expect_err("publish with no branch_currencies row must fail");
    tx.rollback().await.expect("rollback");

    assert!(
        matches!(err, RateEngineError::ValidationFailed(_)),
        "expected ValidationFailed, got {err:?}"
    );
    assert_eq!(db.current_publication(MAIN).await, None);
    db.finish().await;
}

#[tokio::test]
async fn publish_a_hide_line_is_published() {
    let db = TestDb::start().await;
    let e = db.engine();
    // AED is seeded as HIDE on MAIN in 20261226200000_branch_currencies.sql
    let lines = vec![manual_line("AED", "9.1", "9.5")];

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
    let pub_ref = e
        .publisher
        .publish(&mut tx, &main_branch(), &owner_user(), &d.id, None)
        .await
        .expect("HIDE line may publish");
    tx.commit().await.expect("commit");

    assert_eq!(
        db.current_publication(MAIN).await,
        Some(pub_ref.id.to_string())
    );
    db.finish().await;
}
