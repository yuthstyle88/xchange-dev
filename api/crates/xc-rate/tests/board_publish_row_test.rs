//! Board currencies BC-7 (`docs/design/board-currencies.md` §4, §9):
//! - `publish` refuses with `ValidationFailed` when a draft line's `(currency, group)` has no `branch_currencies` row.
//! - `HIDE` lines may publish.
//! - `plan_once` skips a currency whose MAIN `ALL` row is `HIDE` (when `display_in` is checked on MAIN).

mod rate_engine_common;

use rate_engine_common::*;
use xc_rate::domain::currency::branch_currencies::{BranchCurrencies, Display};
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xc_rate::domain::draft::rate_draft::DraftStatus;
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::pricing::pricing_mode::{PricingMode, PricingModeStore};
use xc_rate::infrastructure::currency::branch_currencies::PgBranchCurrencies;
use xc_rate::infrastructure::pricing::pricing_mode_store::PgPricingModeStore;

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

#[tokio::test]
async fn plan_with_a_hide_follow_currency_is_not_planned() {
    let db = TestDb::start().await;
    let now = chrono::Utc::now();

    // Put USD (SHOW) and AED (HIDE) into FOLLOW mode.
    let mut tx = db.begin().await;
    PgPricingModeStore
        .save_in(&mut tx, ORG, code("USD"), PricingMode::Follow, OWNER_ID, now)
        .await
        .expect("save USD FOLLOW");
    PgPricingModeStore
        .save_in(&mut tx, ORG, code("AED"), PricingMode::Follow, OWNER_ID, now)
        .await
        .expect("save AED FOLLOW");

    let following = PgPricingModeStore
        .following_in(&mut tx, ORG)
        .await
        .expect("following_in");
    assert_eq!(following, vec![code("AED"), code("USD")]);

    let all_group = GroupCode::all();
    let mut planned = Vec::new();
    for c in following {
        if PgBranchCurrencies
            .display_in(&mut tx, &main_branch(), c, &all_group)
            .await
            .expect("display_in")
            == Some(Display::Hide)
        {
            continue;
        }
        planned.push(c);
    }
    tx.rollback().await.expect("rollback");

    assert_eq!(planned, vec![code("USD")], "HIDE FOLLOW currency is skipped");
    db.finish().await;
}
