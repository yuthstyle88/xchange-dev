//! pricing-formula wave 3 (docs/design/blueprint/pricing-formula.md §3a, §6a; B23): the pricing board prices the
//! suggestion through the branch's formula with the FOLLOW binding and names the formula's source.
//!
//! Written before the implementation. Do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.
//!
//! Contract pinned here: `views/pricing_board.rs` resolves the branch's formula (`FormulaResolver::resolve` with
//! the branch's current version), calls `PricingSuggestion::of_with`, and answers `formulaSource`.

mod common;
mod pricing_common;
mod pricing_formula_api;

use pricing_common::{Room, owner, view};
use pricing_formula_api::*;
use serde_json::{Value, json};

async fn room() -> Room {
    let room = Room::start().await;
    bot_1028(&room).await;
    room.tt_batch(&[("USD", "33.2", "33.9")], 1).await;
    policy(&room, "MAIN", "0", "0", wide(), true).await;
    room
}

fn usd(data: &Value) -> Value {
    data["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["currency"] == json!("USD"))
        .expect("a USD row")
        .clone()
}

fn prices(row: &Value) -> Vec<(String, String)> {
    row["suggestion"]
        .as_array()
        .expect("a suggestion")
        .iter()
        .map(|t| (t["buy"].as_str().unwrap().to_owned(), t["sell"].as_str().unwrap().to_owned()))
        .collect()
}

#[actix_web::test]
async fn b23_the_board_prices_through_the_organisation_formula() {
    let room = room().await;
    org_formula(&room, 1, "ref_mid - 0.05", "ref_mid + 0.05", true).await;
    let data = view(&room, &owner(), "pricing_board", "MAIN").await.ok("board")["data"].clone();
    assert_eq!(data["formulaSource"], json!("ORGANISATION"));
    let row = usd(&data);
    let p: Vec<(String, String)> = prices(&row);
    assert_eq!(p.len(), 4);
    for (b, s) in &p {
        assert_eq!(
            (rust_decimal::Decimal::from_str_exact(b).unwrap(), rust_decimal::Decimal::from_str_exact(s).unwrap()),
            (rust_decimal::Decimal::new(3349, 2), rust_decimal::Decimal::new(3361, 2)),
            "every tier is 33.49 / 33.61: {p:?}"
        );
    }
    room.finish().await;
}

#[actix_web::test]
async fn with_no_formula_rows_the_board_is_todays_and_says_default() {
    let room = room().await;
    let data = view(&room, &owner(), "pricing_board", "MAIN").await.ok("board")["data"].clone();
    assert_eq!(data["formulaSource"], json!("DEFAULT"));
    let p = prices(&usd(&data));
    let first = (
        rust_decimal::Decimal::from_str_exact(&p[0].0).unwrap(),
        rust_decimal::Decimal::from_str_exact(&p[0].1).unwrap(),
    );
    // V.9 tier 1 on TT 33.2 / 33.9 with better 0.1 %: floor(33.2332) / ceil(33.8661) at 2 places
    assert_eq!(first, (rust_decimal::Decimal::new(3323, 2), rust_decimal::Decimal::new(3387, 2)));
    room.finish().await;
}
