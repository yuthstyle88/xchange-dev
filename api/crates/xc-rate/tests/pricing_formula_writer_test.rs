//! pricing-formula wave 2 (docs/design/blueprint/pricing-formula.md §3): `PolicyVersionWriter::next_with_formula`
//! writes the branch formula pair, and `next` carries it forward.
//!
//! Written before the implementation. Do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.

mod pricing_formula_db;
mod pricing_formula_shared;
mod rate_engine_common;

use pricing_formula_db::*;
use pricing_formula_shared::formula;
use rate_engine_common::TestDb;
use rust_decimal::Decimal;
use sqlx::Row;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_rate::application::policy::policy_version_writer::{
    FormulaChange, PolicyVersionChange, PolicyVersionWriter,
};

fn change() -> PolicyVersionChange {
    PolicyVersionChange {
        buy_adjust_pct: Decimal::new(3, 1),
        sell_adjust_pct: Decimal::new(-3, 1),
        guards: wide_guards(),
        max_reference_age_hours: None,
        on_stale: None,
        stale_limit_thb: None,
        value_tiers_on: false,
        sharp_move_alert_pct: None,
        value_tiers: Vec::new(),
        anchor_source: None,
        overrides: None,
        approved_by: "exec-1".into(),
        reason: "writer test".into(),
    }
}

async fn formulas(db: &TestDb, id: Uuid) -> (Option<String>, Option<String>) {
    let r = sqlx::query("SELECT buy_formula, sell_formula FROM rate_policy_versions WHERE id = $1")
        .bind(id)
        .fetch_one(&db.pool)
        .await
        .expect("the version");
    (r.get(0), r.get(1))
}

#[tokio::test]
async fn set_keep_clear_and_next_carries_the_formula_forward() {
    let db = TestDb::start().await;
    db.seed_branch("PFW").await;
    let branch = BranchId::new("PFW");
    let own = formula("ref_mid - 0.05", "ref_mid + 0.05");
    let some = (
        Some("ref_mid - 0.05".to_owned()),
        Some("ref_mid + 0.05".to_owned()),
    );

    let mut tx = db.begin().await;
    let v1 = PolicyVersionWriter::next_with_formula(
        &mut tx,
        &branch,
        change(),
        FormulaChange::Set(own.clone()),
    )
    .await
    .expect("v1");
    tx.commit().await.expect("commit");
    assert_eq!(formulas(&db, v1.id).await, some, "Set writes both texts");

    let mut tx = db.begin().await;
    let v2 = PolicyVersionWriter::next(&mut tx, &branch, change())
        .await
        .expect("v2");
    tx.commit().await.expect("commit");
    assert_eq!(
        formulas(&db, v2.id).await,
        some,
        "next keeps the current formula"
    );

    let mut tx = db.begin().await;
    let v3 =
        PolicyVersionWriter::next_with_formula(&mut tx, &branch, change(), FormulaChange::Keep)
            .await
            .expect("v3");
    tx.commit().await.expect("commit");
    assert_eq!(formulas(&db, v3.id).await, some, "Keep keeps it");

    let mut tx = db.begin().await;
    let v4 =
        PolicyVersionWriter::next_with_formula(&mut tx, &branch, change(), FormulaChange::Clear)
            .await
            .expect("v4");
    tx.commit().await.expect("commit");
    assert_eq!(
        formulas(&db, v4.id).await,
        (None, None),
        "Clear follows the organisation again"
    );

    let mut tx = db.begin().await;
    let v5 = PolicyVersionWriter::next(&mut tx, &branch, change())
        .await
        .expect("v5");
    tx.commit().await.expect("commit");
    assert_eq!(
        formulas(&db, v5.id).await,
        (None, None),
        "and next keeps none"
    );
    assert_eq!(v5.version, 5);
}

#[tokio::test]
async fn the_first_version_with_keep_has_no_formula() {
    let db = TestDb::start().await;
    db.seed_branch("PFW1").await;
    let mut tx = db.begin().await;
    let v1 = PolicyVersionWriter::next_with_formula(
        &mut tx,
        &BranchId::new("PFW1"),
        change(),
        FormulaChange::Keep,
    )
    .await
    .expect("v1");
    tx.commit().await.expect("commit");
    assert_eq!(formulas(&db, v1.id).await, (None, None));
}
