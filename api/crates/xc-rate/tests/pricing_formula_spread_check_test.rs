//! pricing-formula wave 2 (docs/design/blueprint/pricing-formula.md §4, §6a; B4, B5, B7, B8, B9, B14, B18, B19, B25):
//! `SpreadCheck::run`.
//!
//! Written before the implementation. Do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.
//!
//! The contract the tests pin, beyond the doc comment on `SpreadCheck::run`:
//! - The currencies checked at a branch are every currency with at least one line in `rate_reference_lines`. One with
//!   no line from the branch's anchor (`ReferenceBase::newest_in`) is listed in `not_checked`.
//! - A branch with no current policy version prices nothing (no AUTO price today) and is neither checked nor listed.
//! - A currency in FOLLOW (`pricing_modes`, MAIN only) is checked at MAIN with the §3a binding on its market anchor
//!   (`MarketQuote::anchor_for` over its newest `MARKET_TT` / `MARKET_SR` lines), tiers 1-4 with MAIN's `better`
//!   (`PricingSuggestionSettingsStore`), before V.9's rounding; not with the AUTO binding.
//! - The value of a FOLLOW row is the formula's result at the currency's unit size (as AUTO rows are).

mod pricing_formula_db;
mod pricing_formula_shared;
mod rate_engine_common;

use pricing_formula_db::*;
use pricing_formula_shared::{d, example_tiers, formula, mid_formula, pct};
use rate_engine_common::TestDb;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xc_rate::application::formula::spread_check::{
    BranchCandidate, SpreadCandidate, SpreadCheck, SpreadMode, SpreadReason, SpreadReport,
};
use xc_rate::domain::formula::error::FormulaError;
use xc_rate::domain::formula::price_formula::PriceFormula;
use xc_rate::domain::policy::rate_policy::RatePolicy;

fn rate_policy(buy: &str, sell: &str, guards: serde_json::Value) -> RatePolicy {
    RatePolicy::new("candidate", pct(buy), pct(sell), &guards).expect("a policy")
}

fn branch(id: &str, formula: Option<PriceFormula>, policy: RatePolicy) -> SpreadCandidate {
    SpreadCandidate::Branch {
        branch: BranchId::new(id),
        version: BranchCandidate {
            formula,
            policy,
            tiers: None,
            overrides: Vec::new(),
        },
    }
}

fn organisation(f: PriceFormula) -> SpreadCandidate {
    SpreadCandidate::Organisation {
        organization_id: org(),
        formula: f,
    }
}

async fn run(db: &TestDb, c: &SpreadCandidate) -> SpreadReport {
    let mut tx = db.begin().await;
    let r = SpreadCheck::run(&mut tx, c).await.expect("the check runs");
    tx.rollback().await.expect("rollback");
    r
}

fn described(r: &SpreadReport) -> Vec<String> {
    r.failures.iter().map(|f| f.describe()).collect()
}

#[tokio::test]
async fn b4_plus_minus_0_7_inverts_usd_on_the_10_28_line() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    policy(&db, "MAIN", "0.3000", "-0.3000", wide_guards()).await;
    let r = run(&db, &branch("MAIN", None, rate_policy("0.7000", "-0.7000", wide_guards()))).await;
    assert!(!r.is_ok());
    assert_eq!(described(&r), vec!["MAIN USD buy 33.8837374 > sell 33.7293303".to_owned()]);
    let f = &r.failures[0];
    assert_eq!(
        (f.tier, f.mode, f.reason.clone()),
        (None, SpreadMode::Auto, SpreadReason::BuyAboveSell)
    );
    assert_eq!((f.buy, f.sell), (Some(d("33.8837374")), Some(d("33.7293303"))));
}

#[tokio::test]
async fn b5_plus_minus_0_3_passes_and_every_row_is_priced() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    policy(&db, "MAIN", "0.7000", "-0.7000", wide_guards()).await;
    let r = run(&db, &branch("MAIN", None, rate_policy("0.3000", "-0.3000", wide_guards()))).await;
    assert!(r.is_ok(), "{:?}", described(&r));
    let priced: Vec<_> = r
        .priced
        .iter()
        .map(|p| (p.branch.as_str().to_owned(), p.currency.as_str().to_owned(), p.tier, p.buy, p.sell))
        .collect();
    assert_eq!(
        priced,
        vec![
            ("MAIN".into(), "EUR".into(), None, Some(d("38.114")), Some(d("38.4842"))),
            ("MAIN".into(), "USD".into(), None, Some(d("33.7491446")), Some(d("33.8651987"))),
        ]
    );
}

#[tokio::test]
async fn b7_an_organisation_formula_failing_at_one_branch_names_it() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    db.seed_branch("PFB").await;
    policy(&db, "MAIN", "0", "0", wide_guards()).await;
    policy(&db, "PFB", "0.3000", "-0.3000", wide_guards()).await;
    // buy = mid − 0.05 + buy_pct, sell = mid + 0.05 + sell_pct: inverts where the percentages are ±0.3, not at 0
    let f = formula("ref_mid - 0.05 + buy_pct", "ref_mid + 0.05 + sell_pct");
    let r = run(&db, &organisation(f)).await;
    assert!(!r.is_ok());
    assert!(r.failures.iter().all(|f| f.branch.as_str() == "PFB"), "{:?}", described(&r));
    assert_eq!(
        described(&r),
        vec![
            "PFB EUR buy 38.55 > sell 38.05".to_owned(),
            "PFB USD buy 34.05765 > sell 33.55765".to_owned(),
        ]
    );
}

#[tokio::test]
async fn b8_a_branch_with_its_own_formula_is_not_checked_against_the_organisations() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    db.seed_branch("PFB").await;
    policy(&db, "MAIN", "0", "0", wide_guards()).await;
    let pfb = policy(&db, "PFB", "0.3000", "-0.3000", wide_guards()).await;
    set_branch_formula(&db, pfb, "ref_buy", "ref_sell").await;
    let f = formula("ref_mid - 0.05 + buy_pct", "ref_mid + 0.05 + sell_pct");
    let r = run(&db, &organisation(f)).await;
    assert!(r.is_ok(), "{:?}", described(&r));
    assert!(r.priced.iter().all(|p| p.branch.as_str() == "MAIN"), "PFB follows its own formula");
}

#[tokio::test]
async fn b9_a_currency_without_a_line_from_the_anchor_is_listed_not_checked() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    market_line(&db, "GBP", "MARKET_TT", "44.10", "44.90").await;
    policy(&db, "MAIN", "0", "0", wide_guards()).await;
    let r = run(&db, &organisation(mid_formula())).await;
    assert!(r.is_ok(), "{:?}", described(&r));
    assert_eq!(
        r.not_checked
            .iter()
            .map(|(b, c)| (b.as_str().to_owned(), c.as_str().to_owned()))
            .collect::<Vec<_>>(),
        vec![("MAIN".to_owned(), "GBP".to_owned())],
        "GBP has a market line but no BOT line; MAIN's anchor is BOT"
    );
}

#[tokio::test]
async fn b14_a_tier_that_inverts_is_named() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    policy(&db, "MAIN", "0", "0", wide_guards()).await;
    let candidate = SpreadCandidate::Branch {
        branch: BranchId::new("MAIN"),
        version: BranchCandidate {
            // T2 (−0.1): sell = mid − 0.01 ≥ buy; T3 (−0.2): sell = mid − 0.07 < buy = mid − 0.05
            formula: Some(formula("ref_mid - 0.05", "ref_mid + 0.05 + tier_sell_pct * 0.6")),
            policy: rate_policy("0", "0", wide_guards()),
            tiers: Some(example_tiers()),
            overrides: Vec::new(),
        },
    };
    let r = run(&db, &candidate).await;
    let usd: Vec<Option<u8>> = r
        .failures
        .iter()
        .filter(|f| f.currency.as_str() == "USD")
        .map(|f| f.tier)
        .collect();
    assert_eq!(usd, vec![Some(3), Some(4)], "{:?}", described(&r));
    assert!(described(&r).contains(&"MAIN USD tier 3 buy 33.75765 > sell 33.73765".to_owned()));
}

#[tokio::test]
async fn b18_the_guard_is_checked_on_the_price() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    policy(&db, "MAIN", "0", "0", guards("-1.0000", "1.0000")).await;
    let candidate = branch(
        "MAIN",
        Some(formula("ref_buy * 1.012", "ref_sell * 1.005")),
        rate_policy("0", "0", guards("-1.0000", "1.0000")),
    );
    let r = run(&db, &candidate).await;
    let usd: Vec<_> = r.failures.iter().filter(|f| f.currency.as_str() == "USD").collect();
    assert_eq!(usd.len(), 1, "{:?}", described(&r));
    assert_eq!(
        usd[0].reason,
        SpreadReason::OutsideGuard {
            side: Side::Buy,
            pct: d("1.2"),
            min: d("-1"),
            max: d("1"),
        }
    );
    assert!(usd[0].describe().starts_with("MAIN USD buy 34.0519784 outside guard [-1, 1]"), "{}", usd[0].describe());
}

#[tokio::test]
async fn a_formula_error_is_a_failure_not_an_error() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    policy(&db, "MAIN", "0", "0", wide_guards()).await;
    let r = run(&db, &organisation(formula("ref_buy / (buy_pct - buy_pct)", "ref_sell"))).await;
    assert_eq!(
        r.failures.iter().map(|f| f.reason.clone()).collect::<Vec<_>>(),
        vec![
            SpreadReason::Formula(FormulaError::DivideByZero),
            SpreadReason::Formula(FormulaError::DivideByZero),
        ]
    );
    assert_eq!(described(&r), vec!["MAIN EUR divideByZero".to_owned(), "MAIN USD divideByZero".to_owned()]);
}

#[tokio::test]
async fn b25_a_follow_currency_is_checked_with_the_follow_binding() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    market_line(&db, "USD", "MARKET_TT", "33.2", "33.9").await;
    policy(&db, "MAIN", "0", "0", wide_guards()).await;
    follow(&db, "USD").await;
    // FOLLOW USD: mid 33.55, tier n gets b = better[n] = 0.1, 0.14, 0.2, 0.3; sell − buy = 0.015 − 0.0671 × b,
    // negative only at tier 4. AUTO rows (EUR) have tier_* = 0 and pass.
    let f = formula("ref_mid * (1 + tier_buy_pct / 1000)", "ref_mid * (1 + tier_sell_pct / 1000) + 0.015");
    let r = run(&db, &organisation(f)).await;
    assert_eq!(r.failures.len(), 1, "{:?}", described(&r));
    let fail = &r.failures[0];
    assert_eq!(
        (fail.branch.as_str(), fail.currency.as_str(), fail.tier, fail.mode, fail.reason.clone()),
        ("MAIN", "USD", Some(4), SpreadMode::Follow, SpreadReason::BuyAboveSell)
    );
    assert!(fail.describe().starts_with("MAIN USD tier 4 FOLLOW buy "), "{}", fail.describe());
}

#[tokio::test]
async fn b19_the_check_holds_the_organisation_row_until_the_transaction_ends() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    policy(&db, "MAIN", "0", "0", wide_guards()).await;
    let mut tx = db.begin().await;
    SpreadCheck::run(&mut tx, &organisation(mid_formula()))
        .await
        .expect("the check runs");
    let mut other = db.begin().await;
    let blocked = sqlx::query("SELECT id FROM organizations WHERE id = $1 FOR UPDATE NOWAIT")
        .bind(org())
        .execute(&mut *other)
        .await
        .expect_err("the organisation row is locked by the check");
    assert_eq!(
        blocked.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("55P03"),
        "lock_not_available"
    );
    drop(other);
    tx.rollback().await.expect("rollback");

    // a branch candidate takes its organisation's row too
    let mut tx = db.begin().await;
    SpreadCheck::run(&mut tx, &branch("MAIN", None, rate_policy("0", "0", wide_guards())))
        .await
        .expect("the check runs");
    let mut other = db.begin().await;
    let blocked = sqlx::query("SELECT id FROM organizations WHERE id = $1 FOR UPDATE NOWAIT")
        .bind(org())
        .execute(&mut *other)
        .await
        .expect_err("locked by the branch check");
    assert_eq!(blocked.as_database_error().and_then(|e| e.code()).as_deref(), Some("55P03"));
}
