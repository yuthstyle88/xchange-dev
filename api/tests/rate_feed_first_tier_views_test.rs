//! Rate feed F11 — every list of published prices shows a tiered currency at its first tier
//! (`docs/design/rate-feed-connector.md` §9.1, §9.4; owner's ruling R-10 on #986:
//! *"แสดงราคาขั้นแรก (แนะนำ)"*).
//! Written before the implementation by the ticket author (claude/c954feda). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! What this pins:
//! - `GET /api/v3/views/published_rates` lists one line per `(currency, group)`: on a tiered
//!   publication, the first tier (`tier_lower_exclusive_thb IS NULL`); an untiered line as before.
//!
//! The source guard moved to rate_lines_tier_guard_test.rs (design §9.6).

#[macro_use]
mod auth_http_common;
mod common;

use actix_web::http::Method;
use auth_http_common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const MAIN: &str = "MAIN";

/// One publication on MAIN, made current, with an untiered JPY line and three USD tiers.
/// Answers the USD tier line ids, T1 first.
async fn publish_tiered(pool: &PgPool) -> Vec<String> {
    let request: String = sqlx::query_scalar(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
         proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
         VALUES (gen_random_uuid(), 'RATE_PUBLICATION', $1, 'f11-seed', 'h', 'p', '{}'::jsonb, 'test', now(), \
         'APPROVED', gen_random_uuid()::text, '00000000-0000-0000-0000-000000000001') RETURNING id::text",
    )
    .bind(MAIN)
    .fetch_one(pool)
    .await
    .expect("an approved request");
    let publication: String = sqlx::query_scalar(
        "INSERT INTO rate_publications (id, branch_id, version, approved_request_id, published_by) \
         VALUES (gen_random_uuid(), $1, \
                 (SELECT coalesce(max(version), 0) + 1 FROM rate_publications WHERE branch_id = $1), \
                 $2::uuid, 'test') RETURNING id::text",
    )
    .bind(MAIN)
    .bind(&request)
    .fetch_one(pool)
    .await
    .expect("a publication");
    let snapshot = json!({ "buy_mode": "MANUAL", "sell_mode": "MANUAL" });
    sqlx::query(
        "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, \
         buy_rate, sell_rate, source_snapshot) VALUES (gen_random_uuid(), $1::uuid, 'JPY', 'ALL', 100, 23.1, 23.5, $2)",
    )
    .bind(&publication)
    .bind(&snapshot)
    .execute(pool)
    .await
    .expect("the JPY line");
    let mut ids = Vec::new();
    for (tier, lo, hi, buy, sell) in [
        ("T1", None, Some("10000.00"), "31.50", "33.00"),
        ("T2", Some("10000.00"), Some("50000.00"), "31.70", "32.80"),
        ("T3", Some("50000.00"), None, "31.90", "32.60"),
    ] {
        let id: String = sqlx::query_scalar(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, \
             buy_rate, sell_rate, source_snapshot, value_tier, tier_lower_exclusive_thb, tier_upper_inclusive_thb, \
             base_buy_rate, base_sell_rate) \
             VALUES (gen_random_uuid(), $1::uuid, 'USD', 'ALL', 1, $2::numeric, $3::numeric, $4, $5, \
                     $6::numeric, $7::numeric, 32.00, 32.50) RETURNING id::text",
        )
        .bind(&publication)
        .bind(buy)
        .bind(sell)
        .bind(&snapshot)
        .bind(tier)
        .bind(lo)
        .bind(hi)
        .fetch_one(pool)
        .await
        .expect("a USD tier line");
        ids.push(id);
    }
    sqlx::query("UPDATE branches SET current_publication_id = $1::uuid WHERE id = $2")
        .bind(&publication)
        .bind(MAIN)
        .execute(pool)
        .await
        .expect("make it current");
    ids
}

async fn screen(app: &TestApp, owner: &str) -> Value {
    let answer = send(
        app,
        Call {
            method: Some(Method::GET),
            path: "/api/v3/views/published_rates".to_owned(),
            proto: Some("https".to_owned()),
            ..Call::default()
        }
        .session(owner),
    )
    .await;
    assert_eq!(answer.status, 200, "published_rates: {}", answer.text);
    answer.json()["data"].clone()
}

#[actix_web::test]
async fn the_rates_screen_lists_a_tiered_currency_once_at_its_first_tier() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("boss").await;
    let usd = publish_tiered(&desk.pool).await;

    let data = screen(&app, &owner).await;
    let lines = data["lines"].as_array().expect("lines");
    let usd_lines: Vec<&Value> = lines.iter().filter(|l| l["currency"] == "USD").collect();
    assert_eq!(
        usd_lines.len(),
        1,
        "one USD row, not one per tier: {lines:?}"
    );
    assert_eq!(usd_lines[0]["lineId"], json!(usd[0]), "the first tier (T1)");
    assert_eq!(usd_lines[0]["buyRate"], json!("31.50000000"));
    assert_eq!(usd_lines[0]["sellRate"], json!("33.00000000"));

    let jpy: Vec<&Value> = lines.iter().filter(|l| l["currency"] == "JPY").collect();
    assert_eq!(jpy.len(), 1, "an untiered line is listed as before");
    assert_eq!(jpy[0]["buyRate"], json!("23.10000000"));
}
