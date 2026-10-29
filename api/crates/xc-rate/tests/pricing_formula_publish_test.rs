//! pricing-formula wave 3 (docs/design/blueprint/pricing-formula.md §3 "Audit", §6a; B1, B20): publishing re-prices
//! each AUTO line through the resolved formula and puts the formula into `source_snapshot`.
//!
//! Written before the implementation. Do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.
//!
//! Contract pinned here: `PublishService::publish` resolves the formula like the draft path
//! (`FormulaResolver::resolve(tx, branch, <the line's policy version>)`), prices through `RateProposal::of_with` /
//! `TierProposal::of_with`, and a line with an AUTO side gets `source_snapshot.formula =
//! {"source", "formula_version_id", "buy", "sell"}` (`FormulaSource::as_db`, the version id or null, the two
//! texts). A MANUAL-only line's snapshot has no `formula` key.

mod pricing_formula_db;
mod rate_engine_common;

use pricing_formula_db::*;
use rate_engine_common::{TestDb, key, main_branch, owner_user};
use serde_json::{Value, json};
use sqlx::Row;
use xc_kernel::money::{CurrencyCode, Rate};
use xc_rate::domain::draft::rate_draft::{DraftLine, DraftStatus, LineSide};
use xc_rate::domain::formula::price_formula::{DEFAULT_BUY, DEFAULT_SELL};

async fn publish(db: &TestDb, lines: Vec<DraftLine>) -> String {
    let e = db.engine();
    let mut tx = db.begin().await;
    let created = e.drafts.create(&mut tx, &main_branch(), &owner_user(), lines).await.expect("create");
    tx.commit().await.expect("commit");
    let mut tx = db.begin().await;
    let s = e
        .drafts
        .submit(&mut tx, &main_branch(), &owner_user(), &created.id, &key())
        .await
        .expect("submit");
    tx.commit().await.expect("commit");
    assert_eq!(s.status, DraftStatus::Approved);
    let mut tx = db.begin().await;
    let p = e
        .publisher
        .publish(&mut tx, &main_branch(), &owner_user(), &created.id, None)
        .await
        .expect("publish");
    tx.commit().await.expect("commit");
    p.id.to_string()
}

/// `(buy_rate, sell_rate, source_snapshot)` of `currency`'s single-price line.
async fn published(db: &TestDb, publication: &str, currency: &str) -> (String, String, Value) {
    let r = sqlx::query(
        "SELECT buy_rate::text, sell_rate::text, source_snapshot FROM published_rate_lines \
         WHERE publication_id = $1::uuid AND trim(currency_code) = $2",
    )
    .bind(publication)
    .bind(currency)
    .fetch_one(&db.pool)
    .await
    .expect("the published line");
    (r.get(0), r.get(1), r.get(2))
}

#[tokio::test]
async fn b20_an_organisation_formula_is_in_the_snapshot() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    policy(&db, "MAIN", "0.3000", "-0.3000", wide_guards()).await;
    let v2 = org_formula(&db, 2, "ref_mid - 0.05", "ref_mid + 0.05", true).await;
    let pubid = publish(
        &db,
        vec![DraftLine::new(CurrencyCode::parse("USD").unwrap(), LineSide::Auto, LineSide::Auto, None)],
    )
    .await;
    let (buy, sell, snap) = published(&db, &pubid, "USD").await;
    assert_eq!((buy.as_str(), sell.as_str()), ("33.75765000", "33.85765000"));
    assert_eq!(
        snap["formula"],
        json!({
            "source": "ORGANISATION",
            "formula_version_id": v2.to_string(),
            "buy": "ref_mid - 0.05",
            "sell": "ref_mid + 0.05",
        })
    );
    // today's keys stay
    assert_eq!(snap["buy_adjust_pct"], json!("0.3000"));
}

#[tokio::test]
async fn b1_no_formula_rows_publish_todays_price_with_the_default_in_the_snapshot() {
    let db = TestDb::start().await;
    bot_usd_eur(&db).await;
    policy(&db, "MAIN", "0.3000", "-0.3000", wide_guards()).await;
    let pubid = publish(
        &db,
        vec![
            DraftLine::new(CurrencyCode::parse("USD").unwrap(), LineSide::Auto, LineSide::Auto, None),
            DraftLine::new(
                CurrencyCode::parse("EUR").unwrap(),
                LineSide::Manual(Rate::parse("38.10").unwrap()),
                LineSide::Manual(Rate::parse("38.50").unwrap()),
                Some("desk call".into()),
            ),
        ],
    )
    .await;
    let (buy, sell, snap) = published(&db, &pubid, "USD").await;
    assert_eq!((buy.as_str(), sell.as_str()), ("33.74914460", "33.86519870"));
    assert_eq!(
        snap["formula"],
        json!({ "source": "DEFAULT", "formula_version_id": null, "buy": DEFAULT_BUY, "sell": DEFAULT_SELL })
    );
    let (_, _, eur) = published(&db, &pubid, "EUR").await;
    assert!(eur.get("formula").is_none(), "a MANUAL-only line has no formula key: {eur}");
}
