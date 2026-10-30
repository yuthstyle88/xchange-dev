//! pricing-formula wave 3 (docs/design/blueprint/pricing-formula.md §4; B17): `pricingFormulaPreview` answers the
//! save's table and writes nothing.
//!
//! Written before the implementation. Do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.

mod common;
mod pricing_common;
mod pricing_formula_api;

use pricing_common::{Room, command, owner, rate_manager};
use pricing_formula_api::*;
use serde_json::{Value, json};

async fn room() -> Room {
    let room = Room::start().await;
    bot_1028(&room).await;
    policy(&room, "MAIN", "0", "0", wide(), false).await;
    policy(&room, PFB, "0.3000", "-0.3000", wide(), false).await;
    room
}

async fn preview(
    room: &Room,
    who: &pricing_common::Who,
    branch: &str,
    buy: &str,
    sell: &str,
    scope: &str,
) -> pricing_common::Answer {
    command(
        room,
        who,
        PREVIEW,
        branch,
        json!({ "buyFormula": buy, "sellFormula": sell, "scope": scope }),
        None,
    )
    .await
}

fn row<'a>(rows: &'a [Value], branch: &str, currency: &str) -> &'a Value {
    rows.iter()
        .find(|r| {
            r["branch"] == json!(branch) && r["currency"] == json!(currency) && r["tier"].is_null()
        })
        .unwrap_or_else(|| panic!("a {branch} {currency} row in {rows:?}"))
}

#[actix_web::test]
async fn b17_the_same_table_as_the_save_and_nothing_written() {
    let room = room().await;
    let body = preview(
        &room,
        &owner(),
        "MAIN",
        "ref_mid - 0.05 + buy_pct",
        "ref_mid + 0.05 + sell_pct",
        "ORGANISATION",
    )
    .await
    .ok("B17");
    assert_eq!(body["ok"], json!(false));
    assert_eq!(body["parseError"], Value::Null);
    let rows = body["rows"].as_array().expect("rows").clone();
    let pfb = row(&rows, PFB, "USD");
    assert_eq!(pfb["unitSize"], json!("1"));
    assert_eq!(
        (pfb["mode"].clone(), pfb["buy"].clone(), pfb["sell"].clone()),
        (json!("AUTO"), json!("34.05765"), json!("33.55765"))
    );
    assert_eq!(
        pfb["failure"],
        json!("XC-PFB USD buy 34.05765 > sell 33.55765")
    );
    let main = row(&rows, "MAIN", "USD");
    assert_eq!(main["unitSize"], json!("1"));
    assert_eq!(
        (
            main["buy"].clone(),
            main["sell"].clone(),
            main["failure"].clone()
        ),
        (json!("33.75765"), json!("33.85765"), Value::Null)
    );
    assert_eq!(room.count("pricing_formula_versions").await, 0);
    room.finish().await;
}

#[actix_web::test]
async fn a_formula_that_passes_everywhere_is_ok() {
    let room = room().await;
    room.tt_batch(&[("GBP", "44.10", "44.90")], 1).await;
    let body = preview(
        &room,
        &owner(),
        "MAIN",
        "ref_mid - 0.05",
        "ref_mid + 0.05",
        "ORGANISATION",
    )
    .await
    .ok("ok");
    assert_eq!(body["ok"], json!(true));
    assert!(
        body["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["failure"].is_null())
    );
    assert!(
        body["notChecked"]
            .as_array()
            .unwrap()
            .contains(&json!({ "branch": "MAIN", "currency": "GBP" }))
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_parse_error_is_answered_not_refused() {
    let room = room().await;
    let body = preview(
        &room,
        &owner(),
        "MAIN",
        "ref_buy +",
        "ref_sell",
        "ORGANISATION",
    )
    .await
    .ok("parse error");
    assert_eq!(body["ok"], json!(false));
    assert_eq!(body["parseError"]["field"], json!("buyFormula"));
    assert_eq!(body["parseError"]["code"], json!("syntax"));
    assert_eq!(body["rows"], json!([]));
    room.finish().await;
}

#[actix_web::test]
async fn branch_scope_checks_only_the_request_branch_as_its_own_formula() {
    let room = room().await;
    let body = preview(
        &room,
        &owner(),
        PFB,
        "ref_mid - 0.05 + buy_pct",
        "ref_mid + 0.05 + sell_pct",
        "BRANCH",
    )
    .await
    .ok("branch scope");
    let rows = body["rows"].as_array().unwrap();
    assert!(
        !rows.is_empty() && rows.iter().all(|r| r["branch"] == json!(PFB)),
        "{rows:?}"
    );
    assert_eq!(body["ok"], json!(false));
    room.finish().await;
}

#[actix_web::test]
async fn a_rate_manager_cannot_preview() {
    let room = room().await;
    let a = preview(
        &room,
        &rate_manager(),
        "MAIN",
        "ref_buy",
        "ref_sell",
        "ORGANISATION",
    )
    .await;
    refused_role(&a, "RATE_MANAGER");
    room.finish().await;
}
