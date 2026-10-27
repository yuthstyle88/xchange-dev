//! XC-V9-PRICING PV-l1 (pricing-tiers.md §15.10 gap 10; `H:` §4.1: OWNER sets minSpreadPct 0–5 %): the branch `'*'`
//! min spread through `pricingSuggestionSettingsSave.minSpreadPct`. Written before the implementation by the
//! sub-controller claude/451f0116 (XC-V9-PRICING skeleton). Do not edit, delete or weaken; add tests in a file of your
//! own.

mod pricing_common;

use pricing_common::*;
use serde_json::{Value, json};

const ROUTE: &str = "/api/pricing/suggestion-settings";

async fn star_row(
    room: &Room,
) -> Option<(Vec<rust_decimal::Decimal>, Option<rust_decimal::Decimal>)> {
    sqlx::query_as::<_, (Vec<rust_decimal::Decimal>, Option<rust_decimal::Decimal>)>(
        "SELECT better_pct, min_spread_pct FROM pricing_suggestion_settings \
         WHERE branch_id = $1 AND scope = '*'",
    )
    .bind(B01)
    .fetch_optional(&room.pool)
    .await
    .unwrap()
}

fn min_spread(currency: &str, value: Value) -> Value {
    json!({ "currency": currency, "minSpreadPct": value })
}

/// PV-l1: the owner sets the `'*'` min spread; a branch with no `'*'` row gets one with V.9's default `better`; a
/// later `minSpreadPct` request keeps the stored `better`; `null` clears only the min spread.
#[tokio::test]
async fn the_owner_sets_and_clears_the_branch_min_spread() {
    let room = Room::start().await;
    room.worked_example().await;

    command(
        &room,
        &owner(),
        ROUTE,
        B01,
        min_spread("*", json!("0.5")),
        None,
    )
    .await
    .ok("set 0.5");
    assert_eq!(
        star_row(&room).await,
        Some((
            vec![dec("0.1"), dec("0.14"), dec("0.2"), dec("0.3")],
            Some(dec("0.5"))
        ))
    );

    command(
        &room,
        &owner(),
        ROUTE,
        B01,
        json!({ "currency": "*", "betterPct": ["0.05", "0.1", "0.1", "0.2"] }),
        None,
    )
    .await
    .ok("save better");
    command(
        &room,
        &owner(),
        ROUTE,
        B01,
        min_spread("*", Value::Null),
        None,
    )
    .await
    .ok("clear");
    assert_eq!(
        star_row(&room).await,
        Some((vec![dec("0.05"), dec("0.1"), dec("0.1"), dec("0.2")], None)),
        "clearing the min spread keeps the stored better"
    );
    room.finish().await;
}

/// PV-l1: outside 0..=5, not a number, or on a currency row ⇒ `validationFailed`, nothing written.
#[tokio::test]
async fn a_min_spread_outside_zero_to_five_or_on_a_currency_is_refused() {
    let room = Room::start().await;
    room.worked_example().await;
    for (currency, value) in [
        ("*", json!("5.01")),
        ("*", json!("-0.1")),
        ("*", json!("abc")),
        ("USD", json!("0.5")),
    ] {
        command(
            &room,
            &owner(),
            ROUTE,
            B01,
            min_spread(currency, value.clone()),
            None,
        )
        .await
        .refused(400, "validationFailed", &format!("{currency} {value}"));
    }
    assert_eq!(star_row(&room).await, None, "nothing written");
    room.finish().await;
}
