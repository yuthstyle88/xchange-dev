//! #2063 K-b hardening: `PgPlannerRepository` amounts that overflow `i64` hundredths.
//!
//! A `Money` whose hundredths exceed `i64::MAX` cannot be stored in `planner_drafts.budget_thb`.
//! It must return `CustomerAppError::ValidationFailed { field: "budgetThb" }`, never wrap silently
//! to a negative/corrupt figure or cause a database error.

mod common;

use common::*;
use uuid::Uuid;
use xc_app::domain::planner::repository::PlannerRepository;
use xc_app::infrastructure::planner_repository::PgPlannerRepository;
use xc_kernel::money::CustomerSide::*;
use xc_kernel::money::Money;

fn overflowing_budget() -> Money {
    Money::try_from_minor_i128((i64::MAX as i128) + 1).expect("valid Money > i64::MAX minor")
}

#[tokio::test]
async fn insert_with_overflowing_budget_thb_refused_as_validation_failed() {
    let db = TestDb::start().await;
    db.seed_customer("cust-overflow-insert").await;
    let repo = PgPlannerRepository::new(db.pool.clone());

    let mut plan_item = plan("JPY", CustomerSell, "1");
    plan_item.amount = DraftAmount::BudgetThb(overflowing_budget());

    let draft = PlannerDraft {
        id: Uuid::new_v4(),
        organization_id: org(),
        owner: DraftOwner::Customer("cust-overflow-insert".to_string()),
        plan: plan_item,
        version: 1,
        expires_at: at(3600),
        created_at: at(0),
        updated_at: at(0),
    };

    let mut tx = db.begin().await;
    let err = repo
        .insert_in(&mut tx, &draft)
        .await
        .expect_err("inserting an overflowing budget_thb must fail");
    tx.rollback().await.expect("rollback");

    assert_eq!(
        err,
        CustomerAppError::ValidationFailed { field: "budgetThb" },
        "must report validationFailed on budgetThb, got: {err:?}"
    );

    // Verify nothing was written to planner_drafts
    assert_eq!(
        db.draft_row(draft.id).await,
        None,
        "refused insert must write nothing"
    );

    db.finish().await;
}

#[tokio::test]
async fn update_with_overflowing_budget_thb_refused_as_validation_failed() {
    let db = TestDb::start().await;
    db.seed_customer("cust-overflow-update").await;
    let repo = PgPlannerRepository::new(db.pool.clone());

    // First insert a valid draft
    let mut initial_plan = plan("JPY", CustomerSell, "1");
    initial_plan.amount = DraftAmount::BudgetThb(money("2350.00"));

    let draft_id = Uuid::new_v4();
    let draft = PlannerDraft {
        id: draft_id,
        organization_id: org(),
        owner: DraftOwner::Customer("cust-overflow-update".to_string()),
        plan: initial_plan,
        version: 1,
        expires_at: at(3600),
        created_at: at(0),
        updated_at: at(0),
    };

    let mut tx = db.begin().await;
    repo.insert_in(&mut tx, &draft)
        .await
        .expect("valid draft inserted");
    tx.commit().await.expect("commit initial draft");

    // Now attempt to update with overflowing budget_thb
    let mut updated_plan = plan("JPY", CustomerSell, "1");
    updated_plan.amount = DraftAmount::BudgetThb(overflowing_budget());

    let mut update_draft = draft.clone();
    update_draft.plan = updated_plan;

    let mut tx = db.begin().await;
    let err = repo
        .update_in(&mut tx, &update_draft, 1)
        .await
        .expect_err("updating with an overflowing budget_thb must fail");
    tx.rollback().await.expect("rollback");

    assert_eq!(
        err,
        CustomerAppError::ValidationFailed { field: "budgetThb" },
        "must report validationFailed on budgetThb, got: {err:?}"
    );

    // Verify draft was untouched
    let row = db.draft_row(draft_id).await.expect("draft still exists");
    assert_eq!(
        row.budget_thb.as_deref(),
        Some("2350.00"),
        "budget_thb must remain untouched"
    );
    assert_eq!(row.version, 1, "version must remain 1");

    db.finish().await;
}

#[tokio::test]
async fn ordinary_budget_thb_is_inserted_and_read_normally() {
    let db = TestDb::start().await;
    db.seed_customer("cust-ordinary-budget").await;
    let repo = PgPlannerRepository::new(db.pool.clone());

    let mut normal_plan = plan("JPY", CustomerSell, "1");
    normal_plan.amount = DraftAmount::BudgetThb(money("2350.00"));

    let draft_id = Uuid::new_v4();
    let draft = PlannerDraft {
        id: draft_id,
        organization_id: org(),
        owner: DraftOwner::Customer("cust-ordinary-budget".to_string()),
        plan: normal_plan,
        version: 1,
        expires_at: at(3600),
        created_at: at(0),
        updated_at: at(0),
    };

    let mut tx = db.begin().await;
    repo.insert_in(&mut tx, &draft)
        .await
        .expect("insert ordinary budget succeeds");
    tx.commit().await.expect("commit");

    let row = db.draft_row(draft_id).await.expect("row stored");
    assert_eq!(row.budget_thb.as_deref(), Some("2350.00"));

    db.finish().await;
}
