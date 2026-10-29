//! pricing-formula wave 2 (docs/design/blueprint/pricing-formula.md §3; B1, B15): `FormulaResolver`.
//!
//! Written before the implementation. Do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.

mod pricing_formula_db;
mod pricing_formula_shared;
mod rate_engine_common;

use pricing_formula_db::*;
use pricing_formula_shared::{formula, mid_formula};
use rate_engine_common::TestDb;
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_rate::application::formula::formula_resolver::{FormulaResolver, FormulaSource};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::formula::price_formula::PriceFormula;

#[test]
fn pick_follows_the_branch_then_the_organisation_then_the_default() {
    let (b, o) = (Uuid::new_v4(), Uuid::new_v4());
    let own = formula("ref_buy - 0.01", "ref_sell + 0.01");
    assert_eq!(
        FormulaResolver::pick(None, None),
        (PriceFormula::default_formula(), FormulaSource::Default)
    );
    assert_eq!(
        FormulaResolver::pick(None, Some((o, mid_formula()))),
        (mid_formula(), FormulaSource::Organisation(o))
    );
    assert_eq!(
        FormulaResolver::pick(Some((b, own.clone())), Some((o, mid_formula()))),
        (own.clone(), FormulaSource::Branch(b))
    );
    assert_eq!(
        FormulaResolver::pick(Some((b, own.clone())), None),
        (own, FormulaSource::Branch(b))
    );
}

#[test]
fn the_source_names_its_column_values() {
    let o = Uuid::new_v4();
    assert_eq!(FormulaSource::Default.as_db(), "DEFAULT");
    assert_eq!(FormulaSource::Organisation(o).as_db(), "ORGANISATION");
    assert_eq!(FormulaSource::Branch(o).as_db(), "BRANCH");
    assert_eq!(FormulaSource::Organisation(o).formula_version_id(), Some(o));
    assert_eq!(FormulaSource::Branch(o).formula_version_id(), None);
    assert_eq!(FormulaSource::Default.formula_version_id(), None);
}

#[tokio::test]
async fn b1_no_formula_rows_resolve_to_the_default() {
    let db = TestDb::start().await;
    let pv = policy(&db, "MAIN", "0.3000", "-0.3000", wide_guards()).await;
    let mut tx = db.begin().await;
    let got = FormulaResolver::resolve(&mut tx, &BranchId::new("MAIN"), Some(pv))
        .await
        .expect("resolves");
    assert_eq!(got, (PriceFormula::default_formula(), FormulaSource::Default));
    let none = FormulaResolver::resolve(&mut tx, &BranchId::new("MAIN"), None)
        .await
        .expect("resolves with no policy version");
    assert_eq!(none, (PriceFormula::default_formula(), FormulaSource::Default));
    assert_eq!(
        FormulaResolver::organisation_current(&mut tx, org()).await.expect("reads"),
        None
    );
}

#[tokio::test]
async fn b15_the_current_organisation_formula_prices_a_branch_with_none_of_its_own() {
    let db = TestDb::start().await;
    let pv = policy(&db, "MAIN", "0.3000", "-0.3000", wide_guards()).await;
    org_formula(&db, 1, "ref_buy", "ref_sell", false).await;
    let v2 = org_formula(&db, 2, "ref_mid - 0.05", "ref_mid + 0.05", true).await;
    let mut tx = db.begin().await;
    assert_eq!(
        FormulaResolver::resolve(&mut tx, &BranchId::new("MAIN"), Some(pv))
            .await
            .expect("resolves"),
        (mid_formula(), FormulaSource::Organisation(v2)),
        "the current version (2), not the closed one"
    );
    let cur = FormulaResolver::organisation_current(&mut tx, org())
        .await
        .expect("reads")
        .expect("a current formula");
    assert_eq!((cur.id, cur.version, cur.formula), (v2, 2, mid_formula()));
}

#[tokio::test]
async fn a_branch_formula_wins_over_the_organisations() {
    let db = TestDb::start().await;
    let pv = policy(&db, "MAIN", "0.3000", "-0.3000", wide_guards()).await;
    org_formula(&db, 1, "ref_mid - 0.05", "ref_mid + 0.05", true).await;
    set_branch_formula(&db, pv, "ref_buy - 0.01", "ref_sell + 0.01").await;
    let mut tx = db.begin().await;
    assert_eq!(
        FormulaResolver::resolve(&mut tx, &BranchId::new("MAIN"), Some(pv))
            .await
            .expect("resolves"),
        (formula("ref_buy - 0.01", "ref_sell + 0.01"), FormulaSource::Branch(pv))
    );
}

#[tokio::test]
async fn stored_text_that_does_not_parse_is_an_internal_error() {
    let db = TestDb::start().await;
    let pv = policy(&db, "MAIN", "0", "0", wide_guards()).await;
    set_branch_formula(&db, pv, "ref_buy +", "ref_sell").await;
    let mut tx = db.begin().await;
    let e = FormulaResolver::resolve(&mut tx, &BranchId::new("MAIN"), Some(pv))
        .await
        .expect_err("refused");
    assert!(matches!(e, RateEngineError::Internal(_)), "{e:?}");
}
