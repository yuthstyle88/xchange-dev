//! #2063 K-b hardening: `PgPlannerRepository` amounts that overflow `i64` hundredths.
//!
//! K-b site that cannot overflow: `planner_repository.rs:77` uses `try_minor()`.
//! Every production path that builds `DraftAmount::BudgetThb` goes through `Money::parse`,
//! which caps input at `MAX_INPUT_UNITS` = 100,000,000 THB (`xc-kernel/src/money/money.rs:31, 36-39`).
//! Its only constructors are `src/presentation/dto/v3/planner.rs:48-52` and `planner_repository.rs:50-52`.
//!
//! Per the #2063 rule (issuecomment-6231704691), no test claims to reach this site by bypassing
//! the earlier `Money::parse` bound. This test verifies normal storage and retrieval of a budget plan.

mod common;

use common::*;
use uuid::Uuid;
use xc_app::domain::planner::repository::PlannerRepository;
use xc_app::infrastructure::planner_repository::PgPlannerRepository;
use xc_kernel::money::CustomerSide::*;

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
