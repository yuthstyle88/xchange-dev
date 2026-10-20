//! PT-7 (pricing-tiers.md §3.1, §5, §6, §9, §11): the `pricing_board` view (`GET /api/v3/views/pricing_board`),
//! one branch's board. Written before the implementation by the ticket author (claude/647e1967, PT-0). Do not
//! edit, delete or weaken; add tests in a file of your own.
//!
//! Choices pinned here (the implementer follows them):
//! - `sources` has exactly one entry per source `TT`, `SR`, `BOT`; `fetchedAt` is the newest batch of that source,
//!   `null` when there is none.
//! - `rows`: one per currency that is in the branch's current publication (`enabled: true`) or has any `BOT`,
//!   `MARKET_TT` or `MARKET_SR` line (`enabled: false`); enabled rows first, then by currency code.
//! - A row's `quotes` are the newest `MARKET_TT` and `MARKET_SR` lines of the currency (`source` `TT` / `SR`).
//! - `margins` are the effective margins (the currency's override, else the branch's), four rows when value tiers
//!   are ON and none when OFF; `overridden` says which. `anchorSource` is the effective anchor (§3.1).
//! - `flags` / `selectable` / `status` come from `MarketQuote::anchor_for`, `PricingFlag::blocking` and
//!   `RowStatus::of` (PT-8). Until PT-9, `suggestion` is `null` and `recommendations` is `[]`.
//! - Decimals are strings (compared here as decimals).
//!
//! Runs once PT-2, PT-3 and PT-8 are on `feat/pricing-tiers`.

mod pricing_common;

use pricing_common::*;
use serde_json::{Value, json};

async fn board(room: &Room) -> Value {
    let a = view(room, &owner(), "pricing_board", B01).await;
    a.ok("pricing_board")["data"].clone()
}

fn row<'a>(data: &'a Value, currency: &str) -> &'a Value {
    data["rows"]
        .as_array()
        .expect("rows is an array")
        .iter()
        .find(|r| r["currency"] == json!(currency))
        .unwrap_or_else(|| panic!("a row for {currency}: {}", data["rows"]))
}

fn margins(r: &Value) -> Vec<(i64, rust_decimal::Decimal, rust_decimal::Decimal)> {
    r["margins"]
        .as_array()
        .expect("margins is an array")
        .iter()
        .map(|m| {
            (
                m["tierNo"].as_i64().expect("tierNo"),
                dec_of(&m["buyMarginPct"]),
                dec_of(&m["sellMarginPct"]),
            )
        })
        .collect()
}

#[tokio::test]
async fn the_board_names_the_policy_it_was_built_on_and_the_branch_bands() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    let data = board(&room).await;
    assert_eq!(data["branch"], json!(B01));
    assert_eq!(
        data["policyVersionId"],
        json!(v1.to_string()),
        "the lost-update token (§6)"
    );
    assert_eq!(data["valueTiersOn"], json!(true));
    assert_eq!(data["anchorSource"], json!("BOT"));
    let bands = data["bands"].as_array().expect("bands");
    assert_eq!(
        bands
            .iter()
            .map(|b| b["tierNo"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(bands[0]["lowerExclusiveThb"], Value::Null);
    assert_eq!(dec_of(&bands[0]["upperInclusiveThb"]), dec("20000"));
    assert_eq!(dec_of(&bands[3]["lowerExclusiveThb"]), dec("500000"));
    assert_eq!(bands[3]["upperInclusiveThb"], Value::Null);
    assert_eq!(data["recommendations"], json!([]));
    room.finish().await;
}

#[tokio::test]
async fn sources_report_each_feed_once() {
    let room = Room::start().await;
    room.worked_example().await;
    room.tt_batch(&[("USD", "32.5000", "33.1000")], 2).await;
    let data = board(&room).await;
    let sources = data["sources"].as_array().expect("sources");
    let mut names: Vec<&str> = sources
        .iter()
        .map(|s| s["source"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(names, vec!["BOT", "SR", "TT"]);
    let by = |n: &str| {
        sources
            .iter()
            .find(|s| s["source"] == json!(n))
            .unwrap()
            .clone()
    };
    assert!(by("TT")["fetchedAt"].is_string());
    assert!(by("BOT")["fetchedAt"].is_string());
    assert_eq!(by("SR")["fetchedAt"], Value::Null, "no SR batch yet");
    room.finish().await;
}

#[tokio::test]
async fn a_row_shows_its_market_quotes_and_its_effective_margins() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    room.tt_batch(&[("USD", "32.5000", "33.1000")], 2).await;
    room.sr_batch(&[("USD", "32.6000", "33.0000")], 1).await;
    room.override_margins(
        v1,
        "USD",
        [
            ("0.4000", "-0.4000"),
            ("0.4500", "-0.4500"),
            ("0.5000", "-0.5000"),
            ("0.6000", "-0.6000"),
        ],
    )
    .await;
    room.settings(v1, "USD", Some("1.5000"), Some("MARKET_TT"))
        .await;

    let data = board(&room).await;
    let usd = row(&data, "USD");
    assert_eq!(usd["overridden"], json!(true));
    assert_eq!(
        usd["anchorSource"],
        json!("MARKET_TT"),
        "the currency's anchor (§3.1)"
    );
    assert_eq!(dec_of(&usd["minSpreadPct"]), dec("1.5"));
    assert_eq!(
        margins(usd),
        vec![
            (1, dec("0.4"), dec("-0.4")),
            (2, dec("0.45"), dec("-0.45")),
            (3, dec("0.5"), dec("-0.5")),
            (4, dec("0.6"), dec("-0.6"))
        ]
    );
    let quotes = usd["quotes"].as_array().expect("quotes");
    let tt = quotes
        .iter()
        .find(|q| q["source"] == json!("TT"))
        .expect("a TT quote");
    assert_eq!(
        (dec_of(&tt["buy"]), dec_of(&tt["sell"])),
        (dec("32.5"), dec("33.1"))
    );
    let sr = quotes
        .iter()
        .find(|q| q["source"] == json!("SR"))
        .expect("an SR quote");
    assert_eq!(
        (dec_of(&sr["buy"]), dec_of(&sr["sell"])),
        (dec("32.6"), dec("33"))
    );
    assert!(
        !usd["flags"]
            .as_array()
            .unwrap()
            .contains(&json!("NoAnchor"))
    );

    let eur = row(&data, "EUR");
    assert_eq!(eur["overridden"], json!(false));
    assert_eq!(eur["anchorSource"], json!("BOT"), "the branch's anchor");
    assert_eq!(eur["minSpreadPct"], Value::Null);
    assert_eq!(
        margins(eur),
        vec![
            (1, dec("0"), dec("0")),
            (2, dec("0.1"), dec("-0.1")),
            (3, dec("0.2"), dec("-0.2")),
            (4, dec("0.3"), dec("-0.3"))
        ],
        "the branch's margins"
    );
    assert_eq!(eur["suggestion"], Value::Null, "PT-9 waits for V.9's files");
    room.finish().await;
}

#[tokio::test]
async fn a_currency_with_no_market_quote_is_manual_and_not_selectable() {
    let room = Room::start().await;
    room.worked_example().await;
    let data = board(&room).await;
    let eur = row(&data, "EUR");
    assert!(
        eur["flags"]
            .as_array()
            .unwrap()
            .contains(&json!("NoAnchor")),
        "§11: no market quote ⇒ NoAnchor"
    );
    assert_eq!(eur["status"], json!("Manual"));
    assert_eq!(
        eur["selectable"],
        json!(false),
        "§11: row not bulk-selectable"
    );
    room.finish().await;
}

#[tokio::test]
async fn a_super_rich_only_currency_flags_no_tt_quote_and_stays_selectable() {
    let room = Room::start().await;
    room.worked_example().await;
    room.sr_batch(&[("USD", "32.6000", "33.0000")], 1).await;
    let data = board(&room).await;
    let usd = row(&data, "USD");
    let flags = usd["flags"].as_array().unwrap();
    assert!(
        flags.contains(&json!("NoTtQuote")),
        "§11: no TT quote, SR present ⇒ NoTtQuote: {flags:?}"
    );
    assert!(!flags.contains(&json!("NoAnchor")));
    assert_eq!(usd["selectable"], json!(true), "NoTtQuote warns only");
    room.finish().await;
}

#[tokio::test]
async fn rows_cover_published_and_quoted_currencies_enabled_first() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    // JPY has only a Super Rich line; nothing is published yet.
    room.sr_batch(&[("JPY", "21.0000", "22.0000")], 1).await;
    let data = board(&room).await;
    let mut codes: Vec<&str> = data["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["currency"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        vec!["EUR", "JPY", "USD"],
        "nothing enabled yet: by currency code"
    );
    assert!(
        data["rows"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["enabled"] == json!(false))
    );

    // Publish USD at the branch: it becomes enabled and comes first.
    let approval: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
         proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, 'x', 'h', $2, '{}', 'test', now(), 'APPROVED', $3) RETURNING id",
    )
    .bind(B01)
    .bind(v1.to_string())
    .bind(unique("pub"))
    .fetch_one(&room.pool)
    .await
    .expect("approval");
    let publication: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO rate_publications (id, branch_id, version, published_at, approved_request_id, published_by) \
         VALUES (gen_random_uuid(), $1, 1, now(), $2, 'owner-1') RETURNING id",
    )
    .bind(B01)
    .bind(approval)
    .fetch_one(&room.pool)
    .await
    .expect("publication");
    sqlx::query(
        "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, value_tier, unit_size, \
         buy_rate, sell_rate, source_snapshot) VALUES (gen_random_uuid(), $1, 'USD', 'ALL', 'ALL', 1, 32.28775, 33.0316, '{}')",
    )
    .bind(publication)
    .execute(&room.pool)
    .await
    .expect("a published line");
    sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
        .bind(publication)
        .bind(B01)
        .execute(&room.pool)
        .await
        .expect("current");

    let data = board(&room).await;
    codes = data["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["currency"].as_str().unwrap())
        .collect();
    assert_eq!(codes, vec!["USD", "EUR", "JPY"], "enabled first");
    let usd = row(&data, "USD");
    assert_eq!(usd["enabled"], json!(true));
    let published = usd["published"].as_array().expect("published");
    assert_eq!(published.len(), 1);
    assert_eq!(
        (dec_of(&published[0]["buy"]), dec_of(&published[0]["sell"])),
        (dec("32.28775"), dec("33.0316"))
    );
    room.finish().await;
}

#[tokio::test]
async fn a_branch_with_tiers_off_has_no_margins() {
    let room = Room::start().await;
    room.create_branch(B01).await;
    room.policy_v1(B01, false).await;
    room.bot_batch(&[("USD", "32.4500", "32.9000")], 1).await;
    let data = board(&room).await;
    assert_eq!(data["valueTiersOn"], json!(false));
    assert_eq!(row(&data, "USD")["margins"], json!([]));
    room.finish().await;
}

#[tokio::test]
async fn a_rate_manager_may_read_the_board() {
    let room = Room::start().await;
    room.worked_example().await;
    let a = view(&room, &rate_manager(), "pricing_board", B01).await;
    a.ok("RATE_MANAGER has ReferenceRatesRead (§8)");
    room.finish().await;
}
