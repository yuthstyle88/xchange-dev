//! pricing-formula wave 3 (docs/design/blueprint/pricing-formula.md §3, §4, §5; B7, B8, B9, B13): `pricingFormulaSave`.
//!
//! Written before the implementation. Do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.

mod common;
mod pricing_common;
mod pricing_formula_api;

use pricing_common::{Room, owner, rate_manager};
use pricing_formula_api::*;
use serde_json::json;

/// MAIN (0 / 0), PFB (+0.3 / −0.3), the 10-28 BOT line.
async fn room() -> Room {
    let room = Room::start().await;
    bot_1028(&room).await;
    policy(&room, "MAIN", "0", "0", wide(), false).await;
    policy(&room, PFB, "0.3000", "-0.3000", wide(), false).await;
    room
}

/// Inverts where the percentages are ±0.3 (PFB), not at 0 (MAIN).
const B7_BUY: &str = "ref_mid - 0.05 + buy_pct";
const B7_SELL: &str = "ref_mid + 0.05 + sell_pct";

#[actix_web::test]
async fn b7_a_formula_failing_at_one_branch_is_refused_and_nothing_is_written() {
    let room = room().await;
    let a = save(&room, &owner(), B7_BUY, B7_SELL).await;
    a.refused(400, "validationFailed", "B7");
    let (field, message) = error_of(&a);
    assert_eq!(field, json!("buyFormula"));
    for line in ["XC-PFB EUR buy 38.55 > sell 38.05", "XC-PFB USD buy 34.05765 > sell 33.55765"] {
        assert!(message.contains(line), "{line:?} in {message:?}");
    }
    assert!(!message.contains("MAIN "), "MAIN passes: {message:?}");
    assert_eq!(room.count("pricing_formula_versions").await, 0);
    room.finish().await;
}

#[actix_web::test]
async fn b8_saved_when_the_failing_branch_has_its_own_formula() {
    let room = room().await;
    let pfb = sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM rate_policy_versions WHERE branch_id = $1")
        .bind(PFB)
        .fetch_one(&room.pool)
        .await
        .unwrap();
    set_branch_formula(&room, pfb, "ref_buy", "ref_sell").await;
    let body = save(&room, &owner(), B7_BUY, B7_SELL).await.ok("B8");
    assert_eq!(body["version"], json!(1));
    assert!(body["id"].is_string());
    assert_eq!(body["notChecked"], json!([]));
    assert_eq!(org_versions(&room).await, vec![(1, B7_BUY.into(), B7_SELL.into(), true, None)]);
    room.finish().await;
}

#[actix_web::test]
async fn each_save_is_the_next_version_and_closes_the_current_one() {
    let room = room().await;
    save(&room, &owner(), "ref_mid - 0.05", "ref_mid + 0.05").await.ok("v1");
    let v2 = save(&room, &owner(), "ref_buy", "ref_sell").await.ok("v2");
    assert_eq!(v2["version"], json!(2));
    assert_eq!(
        org_versions(&room).await,
        vec![
            (1, "ref_mid - 0.05".into(), "ref_mid + 0.05".into(), false, None),
            (2, "ref_buy".into(), "ref_sell".into(), true, Some(1)),
        ]
    );
    room.finish().await;
}

#[actix_web::test]
async fn b9_a_currency_with_no_reference_line_is_saved_and_listed() {
    let room = room().await;
    room.tt_batch(&[("GBP", "44.10", "44.90")], 1).await;
    let body = save(&room, &owner(), "ref_mid - 0.05", "ref_mid + 0.05").await.ok("B9");
    let listed = body["notChecked"].as_array().expect("a list").clone();
    assert!(listed.contains(&json!({ "branch": "MAIN", "currency": "GBP" })), "{listed:?}");
    assert!(listed.contains(&json!({ "branch": PFB, "currency": "GBP" })), "{listed:?}");
    room.finish().await;
}

#[actix_web::test]
async fn b13_only_an_organisation_wide_executive_saves() {
    let room = room().await;
    for (name, who) in [("RATE_MANAGER", rate_manager()), ("FINANCE", finance()), ("BRANCH_MANAGER", branch_manager())] {
        let a = save(&room, &who, "ref_mid - 0.05", "ref_mid + 0.05").await;
        refused_role(&a, name);
    }
    assert_eq!(room.count("pricing_formula_versions").await, 0);
    room.finish().await;
}

#[actix_web::test]
async fn a_text_that_does_not_parse_names_its_field() {
    let room = room().await;
    for (buy, sell, field) in [
        ("ref_buy +", "ref_sell", "buyFormula"),
        ("ref_buy", "foo * 2", "sellFormula"),
        ("ref_buy", &"1+".repeat(300)[..], "sellFormula"),
    ] {
        let a = save(&room, &owner(), buy, sell).await;
        a.refused(400, "validationFailed", field);
        assert_eq!(error_of(&a).0, json!(field), "{buy} / {sell}");
    }
    let mut body = save_body("ref_buy", "ref_sell");
    body["reason"] = json!("  ");
    let a = pricing_common::command(&room, &owner(), SAVE, "MAIN", body, None).await;
    a.refused(400, "validationFailed", "blank reason");
    assert_eq!(error_of(&a).0, json!("reason"));
    assert_eq!(room.count("pricing_formula_versions").await, 0);
    room.finish().await;
}
