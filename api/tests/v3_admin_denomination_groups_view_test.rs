//! Rate feed F3w — the view `admin_denomination_groups` (`docs/design/rate-feed-connector.md`
//! §10.2; owner's ruling R-3 on #986).
//! Written before the implementation by the ticket author (claude/c954feda). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! What this pins: `GET /api/v3/views/admin_denomination_groups` (owner) answers
//! `{provider: "RATE_BOARD", currencies: [{currency, groups: [{group, instrumentKey, removable}]}]}`:
//! - every currency with a `denominations` row, by code; its groups `'ALL'` first, then ascending;
//! - `instrumentKey` = the group's `RATE_BOARD` mapping, else `null`;
//! - `removable` is false for `'ALL'`, for a group that face values belong to, and for a group a
//!   current publication prices — the three refusals of `DenominationGroupService::remove_group`.

#[macro_use]
mod auth_http_common;
mod common;

use actix_web::http::Method;
use auth_http_common::*;
use serde_json::{Value, json};
use sqlx::PgPool;

const VIEW: &str = "/api/v3/views/admin_denomination_groups";

async fn exec(pool: &PgPool, sql: &str) {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn view(app: &TestApp, owner: &str) -> Value {
    let answer = send(
        app,
        Call {
            method: Some(Method::GET),
            path: VIEW.to_owned(),
            proto: Some("https".to_owned()),
            ..Call::default()
        }
        .session(owner),
    )
    .await;
    assert_eq!(answer.status, 200, "{VIEW}: {}", answer.text);
    answer.json()["data"].clone()
}

fn currency<'a>(data: &'a Value, code: &str) -> &'a Value {
    data["currencies"]
        .as_array()
        .expect("currencies")
        .iter()
        .find(|c| c["currency"] == code)
        .unwrap_or_else(|| panic!("{code} is listed"))
}

/// USD: `ALL` mapped to `USD`; `LARGE` mapped to `USD1` and removable; `COINS` with a face value;
/// `PRICED` priced by MAIN's current publication. EUR: only its seeded `ALL`.
async fn seed(pool: &PgPool) {
    exec(
        pool,
        "INSERT INTO denominations (currency_code, face_value, group_code) VALUES \
           ('USD', NULL, 'LARGE'), ('USD', NULL, 'COINS'), ('USD', 0.25, 'COINS'), ('USD', NULL, 'PRICED')",
    )
    .await;
    exec(
        pool,
        "INSERT INTO denomination_group_sources (currency_code, group_code, provider, instrument_key, updated_by) VALUES \
           ('USD', 'ALL', 'RATE_BOARD', 'USD', 'owner'), ('USD', 'LARGE', 'RATE_BOARD', 'USD1', 'owner'), \
           ('USD', 'LARGE', 'OTHER', 'X', 'owner')",
    )
    .await;
    exec(
        pool,
        "WITH r AS (INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, \
             policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES (gen_random_uuid(), 'RATE_PUBLICATION', 'MAIN', 'f3w-seed', 'h', 'p', '{}'::jsonb, 'test', \
             now(), 'APPROVED', gen_random_uuid()::text) RETURNING id), \
         p AS (INSERT INTO rate_publications (id, branch_id, version, approved_request_id, published_by) \
             SELECT gen_random_uuid(), 'MAIN', 1, r.id, 'test' FROM r RETURNING id) \
         INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, unit_size, \
             buy_rate, sell_rate, source_snapshot) \
         SELECT gen_random_uuid(), p.id, 'USD', 'PRICED', 1, 32.1, 32.5, '{}'::jsonb FROM p",
    )
    .await;
    exec(
        pool,
        "UPDATE branches SET current_publication_id = \
           (SELECT id FROM rate_publications WHERE branch_id = 'MAIN' AND version = 1) WHERE id = 'MAIN'",
    )
    .await;
}

#[actix_web::test]
async fn the_owner_sees_every_currency_with_its_groups_keys_and_removability() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("boss").await;
    seed(&desk.pool).await;

    let data = view(&app, &owner).await;
    assert_eq!(data["provider"], json!("RATE_BOARD"));

    let codes: Vec<String> = data["currencies"]
        .as_array()
        .expect("currencies")
        .iter()
        .map(|c| c["currency"].as_str().expect("a code").to_owned())
        .collect();
    let mut sorted = codes.clone();
    sorted.sort();
    assert_eq!(codes, sorted, "currencies by code");
    assert!(codes.contains(&"EUR".to_owned()) && codes.contains(&"USD".to_owned()));

    assert_eq!(
        currency(&data, "USD")["groups"],
        json!([
            { "group": "ALL", "instrumentKey": "USD", "removable": false },
            { "group": "COINS", "instrumentKey": null, "removable": false },
            { "group": "LARGE", "instrumentKey": "USD1", "removable": true },
            { "group": "PRICED", "instrumentKey": null, "removable": false },
        ]),
        "ALL first; the RATE_BOARD key only; face values and a current publication block removal"
    );
    assert_eq!(
        currency(&data, "EUR")["groups"],
        json!([{ "group": "ALL", "instrumentKey": null, "removable": false }])
    );
}

#[actix_web::test]
async fn a_group_priced_only_by_an_old_publication_is_removable() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("boss").await;
    seed(&desk.pool).await;
    // MAIN moves to a newer publication that does not price PRICED.
    exec(
        &desk.pool,
        "WITH r AS (INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, \
             policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES (gen_random_uuid(), 'RATE_PUBLICATION', 'MAIN', 'f3w-seed-2', 'h', 'p', '{}'::jsonb, 'test', \
             now(), 'APPROVED', gen_random_uuid()::text) RETURNING id) \
         INSERT INTO rate_publications (id, branch_id, version, approved_request_id, published_by) \
         SELECT gen_random_uuid(), 'MAIN', 2, r.id, 'test' FROM r",
    )
    .await;
    exec(
        &desk.pool,
        "UPDATE branches SET current_publication_id = \
           (SELECT id FROM rate_publications WHERE branch_id = 'MAIN' AND version = 2) WHERE id = 'MAIN'",
    )
    .await;

    let data = view(&app, &owner).await;
    let priced = currency(&data, "USD")["groups"]
        .as_array()
        .expect("groups")
        .iter()
        .find(|g| g["group"] == "PRICED")
        .cloned()
        .expect("PRICED is listed");
    assert_eq!(
        priced["removable"],
        json!(true),
        "only a current publication blocks removal"
    );
}
