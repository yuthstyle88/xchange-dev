//! pricing-formula wave 3 (docs/design/blueprint/pricing-formula.md §3, §6a; B1, B15, B16): the draft path prices
//! AUTO lines through the resolved formula and records which formula priced each line.
//!
//! Written before the implementation. Do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.
//!
//! Contract pinned here: `RateDraftService::evaluate` resolves the formula once per draft with
//! `FormulaResolver::resolve(tx, branch, <the draft's policy version>)` and prices through `RateProposal::of_with`,
//! `TierProposal::of_with` and `DraftEvaluation::of_tiered_with`; `submit` then records the source on every line
//! with an AUTO side (`FormulaRecord::draft_line`). A MANUAL-only line keeps NULL.
//!
//! Restated at review (#5530) by the test author: the proposals are compared in currency order, since a draft's
//! line order is its random line ids' (`evaluate` reads `ORDER BY id`).

mod pricing_formula_db;
mod rate_engine_common;

use pricing_formula_db::*;
use rate_engine_common::{TestDb, key, main_branch, owner_user};
use rust_decimal::Decimal;
use sqlx::Row;
use std::str::FromStr;
use uuid::Uuid;
use xc_kernel::money::{CurrencyCode, Rate};
use xc_rate::domain::draft::rate_draft::{DraftId, DraftLine, DraftStatus, LineSide};

fn line(c: &str) -> DraftLine {
    DraftLine::new(
        CurrencyCode::parse(c).unwrap(),
        LineSide::Auto,
        LineSide::Auto,
        None,
    )
}

fn manual(c: &str, buy: &str, sell: &str) -> DraftLine {
    DraftLine::new(
        CurrencyCode::parse(c).unwrap(),
        LineSide::Manual(Rate::parse(buy).unwrap()),
        LineSide::Manual(Rate::parse(sell).unwrap()),
        Some("desk call".into()),
    )
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

/// Creates and submits a draft at MAIN as the owner; answers its id, the submit result and the evaluation's
/// proposals as `(buy, sell)` values, in currency order (the draft's line order is its line ids', which are random).
async fn draft(
    db: &TestDb,
    lines: Vec<DraftLine>,
) -> (
    DraftId,
    Result<DraftStatus, String>,
    Vec<(Option<Decimal>, Option<Decimal>)>,
) {
    let e = db.engine();
    let mut tx = db.begin().await;
    let created = e
        .drafts
        .create(&mut tx, &main_branch(), &owner_user(), lines)
        .await
        .expect("create");
    tx.commit().await.expect("commit");
    let mut tx = db.begin().await;
    let evaluated = e
        .drafts
        .evaluate(&mut tx, &main_branch(), &created.id)
        .await;
    let proposals = match &evaluated {
        Ok(o) => {
            let mut by_currency: Vec<_> = o
                .lines
                .iter()
                .zip(&o.proposals)
                .map(|(l, p)| {
                    (
                        l.currency.as_str().to_owned(),
                        (p.buy.map(|r| r.value()), p.sell.map(|r| r.value())),
                    )
                })
                .collect();
            by_currency.sort_by(|a, b| a.0.cmp(&b.0));
            by_currency.into_iter().map(|(_, p)| p).collect()
        }
        Err(_) => Vec::new(),
    };
    tx.rollback().await.expect("rollback");
    let mut tx = db.begin().await;
    let submitted = e
        .drafts
        .submit(&mut tx, &main_branch(), &owner_user(), &created.id, &key())
        .await;
    let status = match submitted {
        Ok(o) => {
            tx.commit().await.expect("commit");
            Ok(o.status)
        }
        Err(err) => Err(format!("{}: {err}", err.code())),
    };
    (created.id, status, proposals)
}

async fn sources(db: &TestDb, id: &DraftId) -> Vec<(String, Option<String>, Option<Uuid>)> {
    sqlx::query(
        "SELECT trim(currency_code), formula_source, formula_version_id FROM rate_draft_lines \
         WHERE draft_id = $1 ORDER BY currency_code",
    )
    .bind(id.as_uuid())
    .fetch_all(&db.pool)
    .await
    .expect("the lines")
    .iter()
    .map(|r| (r.get(0), r.get(1), r.get(2)))
    .collect()
}

#[tokio::test]
async fn b1_no_formula_rows_price_as_today_and_record_default() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    policy(&db, "MAIN", "0.3000", "-0.3000", wide_guards()).await;
    let (id, status, proposals) = draft(&db, vec![line("USD")]).await;
    assert_eq!(status, Ok(DraftStatus::Approved));
    assert_eq!(
        proposals,
        vec![(Some(d("33.7491446")), Some(d("33.8651987")))]
    );
    assert_eq!(
        sources(&db, &id).await,
        vec![("USD".into(), Some("DEFAULT".into()), None)]
    );
}

#[tokio::test]
async fn b15_a_line_priced_by_the_organisation_formula_names_its_version() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    policy(&db, "MAIN", "0.3000", "-0.3000", wide_guards()).await;
    org_formula(&db, 1, "ref_buy", "ref_sell", false).await;
    let v2 = org_formula(&db, 2, "ref_mid - 0.05", "ref_mid + 0.05", true).await;
    let (id, status, proposals) =
        draft(&db, vec![line("USD"), manual("EUR", "38.10", "38.50")]).await;
    assert_eq!(status, Ok(DraftStatus::Approved));
    assert_eq!(
        proposals,
        vec![
            (Some(d("38.10")), Some(d("38.50"))),
            (Some(d("33.75765")), Some(d("33.85765"))),
        ],
        "EUR (MANUAL) then USD (organisation formula)"
    );
    assert_eq!(
        sources(&db, &id).await,
        vec![
            ("EUR".into(), None, None),
            ("USD".into(), Some("ORGANISATION".into()), Some(v2)),
        ],
        "the AUTO line names version 2; the MANUAL-only line records nothing"
    );
}

#[tokio::test]
async fn a_branch_formula_prices_and_is_recorded_as_branch() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    let pv = policy(&db, "MAIN", "0", "0", wide_guards()).await;
    org_formula(&db, 1, "ref_mid - 0.05", "ref_mid + 0.05", true).await;
    set_branch_formula(&db, pv, "ref_buy - 0.01", "ref_sell + 0.01").await;
    let (id, status, proposals) = draft(&db, vec![line("USD")]).await;
    assert_eq!(status, Ok(DraftStatus::Approved));
    assert_eq!(proposals, vec![(Some(d("33.6382")), Some(d("33.9771")))]);
    assert_eq!(
        sources(&db, &id).await,
        vec![("USD".into(), Some("BRANCH".into()), None)]
    );
}

#[tokio::test]
async fn b16_a_narrowed_reference_is_still_refused_when_the_draft_is_priced() {
    let db = TestDb::start().await;
    policy(&db, "MAIN", "0.3000", "-0.3000", wide_guards()).await;
    // the newest BOT line has a 0.1 % spread: ±0.3 % inverts it
    db.seed_batch(
        chrono::Utc::now() - chrono::Duration::minutes(5),
        &[("USD", "1", Some("33.8000"), Some("33.8338"))],
    )
    .await;
    let (_, status, _) = draft(&db, vec![line("USD")]).await;
    let refused = status.expect_err("refused");
    assert!(
        refused.starts_with("validationFailed")
            && refused.contains("buy rate must be less than or equal to sell rate"),
        "{refused}"
    );
}

#[tokio::test]
async fn g10_draft_guard_reads_unit_size_from_currencies_column() {
    let db = TestDb::start().await;
    sqlx::query("UPDATE currencies SET unit_size = 10 WHERE code = 'USD'")
        .execute(&db.pool)
        .await
        .expect("update USD unit_size to 10");
    policy(&db, "MAIN", "0", "0", guards("-2.0000", "0.5000")).await;
    db.seed_batch(
        chrono::Utc::now() - chrono::Duration::minutes(5),
        &[
            ("JPY", "1", Some("0.2350"), Some("0.2350")),
            ("USD", "1", Some("4.0000"), Some("4.0000")),
        ],
    )
    .await;

    let e = db.engine();
    let mut tx = db.begin().await;
    let created = e
        .drafts
        .create(
            &mut tx,
            &main_branch(),
            &rate_engine_common::rates_user(),
            vec![
                manual("JPY", "23.20", "23.60"),
                manual("USD", "39.50", "40.10"),
            ],
        )
        .await
        .expect("create");
    let evaluated = e
        .drafts
        .evaluate(&mut tx, &main_branch(), &created.id)
        .await
        .expect("evaluate");
    assert!(evaluated.evaluation.is_in_bound());
    assert!(evaluated.evaluation.out_of_bound().is_empty());
    let submitted = e
        .drafts
        .submit(
            &mut tx,
            &main_branch(),
            &rate_engine_common::rates_user(),
            &created.id,
            &key(),
        )
        .await
        .expect("submit");
    tx.commit().await.expect("commit");
    assert_eq!(submitted.status, DraftStatus::Approved);
    db.finish().await;
}
