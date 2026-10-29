//! pricing-formula wave 3 (docs/design/blueprint/pricing-formula.md §3, §4, §5, §6a; B4, B5, B6, B9, B13, B18):
//! `ratePolicySave` runs the §4 check before it writes, and saves the branch formula.
//!
//! Written before the implementation. Do not edit, delete or weaken. Needs `TEST_DATABASE_URL`.
//!
//! Contract pinned here (beyond the doc comment on `RatePolicySaveInput::buy_formula`): the check is
//! `SpreadCheck::run(Branch)` with the version as it would be written; a failure answers `400 validationFailed`
//! with field `buyFormula` when the request carries formula texts, else `buyAdjustPct`, and the message is one
//! `SpreadFailure::describe()` per line. A passing save answers `notChecked` (a list, empty when everything was
//! checked). Sending formula fields needs `may_change_the_frame` (EXECUTIVE), else `roleNotAllowedHere`, as any frame change.

mod common;
mod pricing_common;
mod pricing_formula_api;

use pricing_common::{Room, command, owner, rate_manager};
use pricing_formula_api::*;
use serde_json::{Value, json};

async fn room(guards: Value) -> Room {
    let room = Room::start().await;
    bot_1028(&room).await;
    policy(&room, "MAIN", "0.3000", "-0.3000", guards, false).await;
    room
}

async fn post(room: &Room, who: &pricing_common::Who, body: Value) -> pricing_common::Answer {
    command(room, who, POLICY, "MAIN", body, None).await
}

#[actix_web::test]
async fn b4_plus_minus_0_7_on_the_10_28_line_is_refused_and_nothing_written() {
    let room = room(wide()).await;
    let a = post(&room, &owner(), policy_body("0.7000", "-0.7000", wide())).await;
    a.refused(400, "validationFailed", "B4");
    let (field, message) = error_of(&a);
    assert_eq!(field, json!("buyAdjustPct"));
    assert_eq!(message, "MAIN USD buy 33.8837374 > sell 33.7293303");
    assert_eq!(
        branch_formulas(&room, "MAIN").await.len(),
        1,
        "no new version"
    );
    room.finish().await;
}

#[actix_web::test]
async fn b5_plus_minus_0_3_is_saved_with_an_empty_not_checked() {
    let room = room(wide()).await;
    let body = post(&room, &owner(), policy_body("0.3000", "-0.3000", wide()))
        .await
        .ok("B5");
    assert_eq!(body["version"], json!(2));
    assert_eq!(body["notChecked"], json!([]));
    room.finish().await;
}

#[actix_web::test]
async fn b9_a_currency_without_a_reference_line_is_listed() {
    let room = room(wide()).await;
    room.tt_batch(&[("GBP", "44.10", "44.90")], 1).await;
    let body = post(&room, &owner(), policy_body("0.3000", "-0.3000", wide()))
        .await
        .ok("B9");
    assert_eq!(
        body["notChecked"],
        json!([{ "branch": "MAIN", "currency": "GBP" }])
    );
    room.finish().await;
}

#[actix_web::test]
async fn b6_the_two_formula_texts_come_together() {
    let room = room(wide()).await;
    for (buy, sell) in [
        (json!("ref_mid - 0.05"), Value::Null),
        (json!("ref_mid - 0.05"), json!("__absent__")),
        (Value::Null, json!("ref_mid + 0.05")),
    ] {
        let mut body = policy_body("0.3000", "-0.3000", wide());
        body["buyFormula"] = buy.clone();
        if sell != json!("__absent__") {
            body["sellFormula"] = sell.clone();
        }
        let a = post(&room, &owner(), body).await;
        a.refused(400, "validationFailed", "B6");
        assert_eq!(error_of(&a).0, json!("sellFormula"), "{buy} / {sell}");
    }
    assert_eq!(branch_formulas(&room, "MAIN").await.len(), 1);
    room.finish().await;
}

#[actix_web::test]
async fn a_branch_formula_is_set_kept_and_cleared() {
    let room = room(wide()).await;
    let mut set = policy_body("0.3000", "-0.3000", wide());
    set["buyFormula"] = json!("ref_mid - 0.05");
    set["sellFormula"] = json!("ref_mid + 0.05");
    post(&room, &owner(), set).await.ok("set");
    post(&room, &owner(), policy_body("0.2000", "-0.2000", wide()))
        .await
        .ok("keep");
    let mut clear = policy_body("0.3000", "-0.3000", wide());
    clear["buyFormula"] = Value::Null;
    clear["sellFormula"] = Value::Null;
    post(&room, &owner(), clear).await.ok("clear");
    let mid = (
        Some("ref_mid - 0.05".to_owned()),
        Some("ref_mid + 0.05".to_owned()),
    );
    assert_eq!(
        branch_formulas(&room, "MAIN").await,
        vec![
            (1, None, None),
            (2, mid.0.clone(), mid.1.clone()),
            (3, mid.0, mid.1),
            (4, None, None)
        ]
    );
    room.finish().await;
}

#[actix_web::test]
async fn a_branch_formula_that_inverts_is_refused_on_buy_formula() {
    let room = room(wide()).await;
    let mut body = policy_body("0.3000", "-0.3000", wide());
    body["buyFormula"] = json!("ref_mid + 0.10");
    body["sellFormula"] = json!("ref_mid - 0.10");
    let a = post(&room, &owner(), body).await;
    a.refused(400, "validationFailed", "inverting branch formula");
    let (field, message) = error_of(&a);
    assert_eq!(field, json!("buyFormula"));
    assert!(
        message.contains("MAIN USD buy 33.90765 > sell 33.70765"),
        "{message}"
    );
    room.finish().await;
}

#[actix_web::test]
async fn b13_a_rate_manager_cannot_send_a_formula_but_still_saves_percentages() {
    let room = room(wide()).await;
    let mut body = policy_body("0.3000", "-0.3000", wide());
    body["buyFormula"] = json!("ref_mid - 0.05");
    body["sellFormula"] = json!("ref_mid + 0.05");
    let a = post(&room, &rate_manager(), body).await;
    refused_role(&a, "RATE_MANAGER with a formula");
    post(
        &room,
        &rate_manager(),
        policy_body("0.2000", "-0.2000", wide()),
    )
    .await
    .ok("percentages only");
    room.finish().await;
}

#[actix_web::test]
async fn b18_the_guard_is_checked_on_the_formulas_price() {
    let g = guards("-1.0000", "1.0000");
    let room = room(g.clone()).await;
    let mut body = policy_body("0", "0", g);
    body["buyFormula"] = json!("ref_buy * 0.988");
    body["sellFormula"] = json!("ref_sell");
    let a = post(&room, &owner(), body).await;
    a.refused(400, "validationFailed", "B18");
    let (field, message) = error_of(&a);
    assert_eq!(field, json!("buyFormula"));
    assert!(
        message.contains("MAIN USD buy 33.2444216 outside guard [-1, 1] (-1.2 %)"),
        "{message}"
    );
    room.finish().await;
}
