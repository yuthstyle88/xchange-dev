//! pricing-formula wave 3 (docs/design/blueprint/pricing-formula.md §6): the `pricing_formula` view.
//!
//! Written before the implementation. Do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.

mod common;
mod pricing_common;
mod pricing_formula_api;

use pricing_common::{Room, owner, view};
use pricing_formula_api::*;
use serde_json::{Value, json};
use xc_rate::domain::formula::price_formula::{DEFAULT_BUY, DEFAULT_SELL};

#[actix_web::test]
async fn with_no_formula_rows_every_branch_follows_the_default() {
    let room = Room::start().await;
    policy(&room, "MAIN", "0", "0", wide(), false).await;
    policy(&room, PFB, "0.3000", "-0.3000", wide(), false).await;
    let data = view(&room, &owner(), "pricing_formula", "MAIN").await.ok("view")["data"].clone();
    assert_eq!(data["organisation"], Value::Null);
    assert_eq!(data["default"], json!({ "buyFormula": DEFAULT_BUY, "sellFormula": DEFAULT_SELL }));
    assert_eq!(
        data["inputs"],
        json!([
            {"name": "ref_buy"}, {"name": "ref_sell"}, {"name": "ref_mid"}, {"name": "buy_pct"},
            {"name": "sell_pct"}, {"name": "tier_buy_pct"}, {"name": "tier_sell_pct"}
        ])
    );
    assert_eq!(
        data["branches"],
        json!([
            {"branch": "MAIN", "source": "DEFAULT", "own": null},
            {"branch": PFB, "source": "DEFAULT", "own": null},
        ])
    );
    room.finish().await;
}

#[actix_web::test]
async fn the_current_organisation_formula_and_each_branchs_source() {
    let room = Room::start().await;
    policy(&room, "MAIN", "0", "0", wide(), false).await;
    let pfb = policy(&room, PFB, "0.3000", "-0.3000", wide(), false).await;
    org_formula(&room, 1, "ref_buy", "ref_sell", false).await;
    let v2 = org_formula(&room, 2, "ref_mid - 0.05", "ref_mid + 0.05", true).await;
    set_branch_formula(&room, pfb, "ref_buy - 0.01", "ref_sell + 0.01").await;
    let data = view(&room, &owner(), "pricing_formula", "MAIN").await.ok("view")["data"].clone();
    let o = &data["organisation"];
    assert_eq!(
        (o["id"].clone(), o["version"].clone(), o["buyFormula"].clone(), o["sellFormula"].clone(), o["approvedBy"].clone()),
        (json!(v2.to_string()), json!(2), json!("ref_mid - 0.05"), json!("ref_mid + 0.05"), json!("owner-1"))
    );
    assert_eq!(
        data["branches"],
        json!([
            {"branch": "MAIN", "source": "ORGANISATION", "own": null},
            {"branch": PFB, "source": "BRANCH", "own": {"buyFormula": "ref_buy - 0.01", "sellFormula": "ref_sell + 0.01"}},
        ])
    );
    room.finish().await;
}
