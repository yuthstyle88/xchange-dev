//! Ticket #757 (RE-4c) — what the harnesses of the desk save and the v2 snapshot share once the
//! counter rate is the current publication: publishing the rate set a test (or a recorded v2
//! step) put into `rates`.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` D-3 (the current rate is the line of
//! `branches.current_publication_id`, never the newest `rates` row), §4.8, §4.9 R1/R4/R10, §8
//! migration 4 (version 1 is built from the newest `rates` row of each code, × `unit_size`).
//!
//! Needs `TEST_DATABASE_URL` …; nothing here skips (the callers' harnesses check it).
//!
//! Signatures this module assumes: the rate engine as `rates_re4_common::publish` drives it
//! (RE-4a, merged). The publishing helpers are `publish.rs`, `include!`d here and by `save_common`
//! (see that file); this module is loaded by `snapshot_common` only.
#![allow(dead_code)]

use rust_decimal::Decimal;
use sqlx::PgPool;

include!("publish.rs");

/// `rate ÷ unit_size`, exact and normalised, as a JSON number: the per-1 rate the v2 wire
/// shows for a line (§4.9 R3, `CompatRate::per_one`).
pub fn per_one_json(rate: Decimal, unit_size: Decimal) -> serde_json::Value {
    let text = (rate / unit_size).normalize().to_string();
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{text} is a JSON number: {e}"))
}

/// One currency of `branch`'s current publication as `adminSnapshot.rates[]` shows it (§4.9
/// R10), without `code`, `name` and `symbol`: `{buy, sell, updated, requestId, partnerName,
/// partnerCost, botReference}`. `None` when the publication has no line for `code`, or the
/// branch has no publication.
pub async fn published_entry(pool: &PgPool, branch: &str, code: &str) -> Option<serde_json::Value> {
    type Row = (
        Option<Decimal>,
        Option<Decimal>,
        Decimal,
        serde_json::Value,
        chrono::DateTime<chrono::Utc>,
    );
    let row: Option<Row> = sqlx::query_as(
        "SELECT l.buy_rate, l.sell_rate, l.unit_size, l.source_snapshot, p.published_at \
         FROM branches b JOIN rate_publications p ON p.id = b.current_publication_id \
         JOIN published_rate_lines l ON l.publication_id = p.id \
         WHERE b.id = $1 AND l.currency_code = $2 AND l.denomination_group = 'ALL'",
    )
    .bind(branch)
    .bind(code)
    .fetch_optional(pool)
    .await
    .expect("read the current line");
    let (buy, sell, unit, snapshot, published_at) = row?;
    let partner = snapshot.get("partner").filter(|p| !p.is_null());
    let partner_name = partner
        .and_then(|p| p.get("name"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned();
    let partner_cost = partner
        .and_then(|p| p.get("cost"))
        .and_then(|c| match c {
            serde_json::Value::String(text) => text.parse::<Decimal>().ok(),
            serde_json::Value::Number(n) => n.to_string().parse::<Decimal>().ok(),
            _ => None,
        })
        .map_or(serde_json::Value::Null, |cost| {
            per_one_json(cost, Decimal::ONE)
        });
    Some(serde_json::json!({
        "buy": buy.map_or(serde_json::Value::Null, |r| per_one_json(r, unit)),
        "sell": sell.map_or(serde_json::Value::Null, |r| per_one_json(r, unit)),
        "updated": time_text(published_at),
        "requestId": "",
        "partnerName": partner_name,
        "partnerCost": partner_cost,
        "botReference": "",
    }))
}

/// `published_at` of `branch`'s current publication as v2 prints a time, or `None` before its
/// first publication.
pub async fn published_at(pool: &PgPool, branch: &str) -> Option<String> {
    let at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT p.published_at FROM branches b JOIN rate_publications p \
           ON p.id = b.current_publication_id WHERE b.id = $1",
    )
    .bind(branch)
    .fetch_optional(pool)
    .await
    .expect("read the current publication");
    at.map(time_text)
}

/// A time as `v2_json::time_json` prints it.
fn time_text(at: chrono::DateTime<chrono::Utc>) -> String {
    match xchange::application::compat::v2_json::time_json(at).to_value() {
        serde_json::Value::String(text) => text,
        other => panic!("time_json printed {other}"),
    }
}

/// A recorded v2 answer (`adminSnapshot`, `adminDeskConfig`, or the snapshot a write answers
/// with) re-expected for D-3 and §4.9 R10, for `branch` as it stands now:
///
/// - a rate v2 showed set keeps its `buy`, `sell`, `partnerName` and `partnerCost` (the replay
///   harness published exactly what the recorded direct saves wrote), and shows the current
///   publication's `updated`, `requestId` `""` and `botReference` `""`;
/// - `saved`, the code an `adminSaveRate` step just saved, shows what the publication holds
///   for it, unset when it holds nothing: the answer of a direct save does not show that save
///   (D-3, R10).
///
/// Everything else of the answer is kept, so it is still held to v2.
pub async fn as_published(
    pool: &PgPool,
    branch: &str,
    recorded: &serde_json::Value,
    saved: Option<&str>,
) -> serde_json::Value {
    let mut value = recorded.clone();
    let Some(serde_json::Value::Array(rates)) = value.get_mut("rates") else {
        return value;
    };
    let now = published_at(pool, branch).await;
    for rate in rates.iter_mut() {
        if rate.get("requestId").is_none() {
            // not v2's snapshot entry
            continue;
        }
        let code = rate["code"].as_str().unwrap_or_default().to_owned();
        if Some(code.as_str()) == saved {
            let shown = published_entry(pool, branch, &code)
                .await
                .unwrap_or_else(|| {
                    serde_json::json!({
                        "buy": null, "sell": null, "updated": "", "requestId": "",
                        "partnerName": "", "partnerCost": null, "botReference": null,
                    })
                });
            for (key, field) in shown.as_object().expect("an object") {
                rate[key.as_str()] = field.clone();
            }
        } else if !rate["buy"].is_null() {
            if let Some(now) = &now {
                rate["updated"] = serde_json::Value::String(now.clone());
            }
            rate["requestId"] = serde_json::Value::String(String::new());
            rate["botReference"] = serde_json::Value::String(String::new());
        }
    }
    value
}
