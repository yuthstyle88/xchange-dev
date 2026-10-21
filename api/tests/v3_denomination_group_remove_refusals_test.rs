//! Written before the implementation by the ticket author's test writer (gemini/studio-15) for #1747. Do not edit, delete or weaken
//!
//! Up-front tests for denomination group remove refusals (#1747).
//! Drives `denominationGroupRemove` (`POST /api/denomination-groups/remove`) over HTTP.
//!
//! Behaviour table:
//! - group `ALL`: 409, code `groupAllNotRemovable`, Thai sentence
//! - group not found for currency: 404, code `notFound`, Thai sentence (the existing one)
//! - face values belong to group: 409, code `groupHasFaceValues`, Thai sentence
//! - a current publication prices group: 409, code `groupPricedByPublication`, Thai sentence
//! - caller is not OWNER: 403, code `outOfScope`, unchanged
//! - removable group: 200, success, rows deleted, unchanged
//!
//! "Thai sentence" means the message contains Thai script and no English word of 3 or more letters.

#[macro_use]
mod auth_http_common;
mod common;

use auth_http_common::*;
use serde_json::json;
use sqlx::PgPool;

const ROUTE: &str = "/api/denomination-groups/remove";

async fn exec(pool: &PgPool, sql: &str) {
    sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

/// USD: `ALL` mapped to `USD`; `LARGE` mapped to `USD1` and removable; `COINS` with a face value;
/// `PRICED` priced by MAIN's current publication.
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
             policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
             VALUES (gen_random_uuid(), 'RATE_PUBLICATION', 'MAIN', 'f3w-seed', 'h', 'p', '{}'::jsonb, 'test', \
             now(), 'APPROVED', gen_random_uuid()::text, '00000000-0000-0000-0000-000000000001') RETURNING id), \
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

async fn remove_group(app: &TestApp, owner_token: &str, currency: &str, group: &str) -> Answer {
    send(
        app,
        Call::post(
            ROUTE,
            json!({
                "currency": currency,
                "group": group,
            }),
        )
        .session(owner_token)
        .header("idempotency-key", &format!("idem-{}", uuid::Uuid::new_v4())),
    )
    .await
}

async fn remove_group_as_staff(
    app: &TestApp,
    staff_token: &str,
    currency: &str,
    group: &str,
) -> Answer {
    send(
        app,
        Call::post(
            ROUTE,
            json!({
                "currency": currency,
                "group": group,
            }),
        )
        .session(staff_token)
        .header("idempotency-key", &format!("idem-{}", uuid::Uuid::new_v4())),
    )
    .await
}

fn assert_thai_sentence(message: &str) {
    let has_thai = message
        .chars()
        .any(|c| ('\u{0E00}'..='\u{0E7F}').contains(&c));
    assert!(has_thai, "message must contain Thai script: {message:?}");
    let words: Vec<&str> = message
        .split(|c: char| !c.is_ascii_alphabetic())
        .filter(|w| !w.is_empty())
        .collect();
    for word in words {
        assert!(
            word.len() < 3,
            "message must contain no English word of 3 or more letters ({word:?}): {message:?}"
        );
    }
}

fn assert_refusal(answer: &Answer, expected_status: u16, expected_code: &str) {
    assert_eq!(
        answer.status, expected_status,
        "HTTP status for {expected_code}: {}",
        answer.text
    );
    let json = answer.json();
    let error = &json["error"];
    assert_eq!(
        error["code"], expected_code,
        "error code for {expected_code}: {}",
        answer.text
    );
    let message = error["message"]
        .as_str()
        .unwrap_or_else(|| panic!("message in error response: {}", answer.text));
    assert_thai_sentence(message);
}

#[actix_web::test]
async fn the_all_group_gives_409_and_group_all_not_removable() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("boss").await;
    seed(&desk.pool).await;

    let answer = remove_group(&app, &owner, "USD", "ALL").await;
    assert_refusal(&answer, 409, "groupAllNotRemovable");
}

#[actix_web::test]
async fn an_unknown_group_gives_404_and_not_found() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("boss").await;
    seed(&desk.pool).await;

    let answer = remove_group(&app, &owner, "USD", "NONEXISTENT").await;
    assert_refusal(&answer, 404, "notFound");
}

#[actix_web::test]
async fn a_group_with_face_values_gives_409_and_group_has_face_values() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("boss").await;
    seed(&desk.pool).await;

    let answer = remove_group(&app, &owner, "USD", "COINS").await;
    assert_refusal(&answer, 409, "groupHasFaceValues");
}

#[actix_web::test]
async fn a_group_priced_by_a_current_publication_gives_409_and_group_priced_by_publication() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("boss").await;
    seed(&desk.pool).await;

    let answer = remove_group(&app, &owner, "USD", "PRICED").await;
    assert_refusal(&answer, 409, "groupPricedByPublication");
}

#[actix_web::test]
async fn a_non_owner_caller_still_gets_403_out_of_scope() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    seed(&desk.pool).await;

    let staff = desk
        .staff_session("rates_staff", "RATE_MANAGER", "MAIN")
        .await;
    let answer = remove_group_as_staff(&app, &staff, "USD", "LARGE").await;

    assert_eq!(answer.status, 403, "status: {}", answer.text);
    assert_eq!(
        answer.json()["error"]["code"],
        "outOfScope",
        "code: {}",
        answer.text
    );
}

#[actix_web::test]
async fn a_removable_group_still_succeeds_and_deletes_rows() {
    let desk = Desk::start().await;
    let app = desk_app!(desk);
    let owner = desk.owner_session("boss").await;
    seed(&desk.pool).await;

    let before_denom: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM denominations WHERE currency_code = 'USD' AND group_code = 'LARGE'",
    )
    .fetch_one(&desk.pool)
    .await
    .expect("count denominations before");
    assert!(
        before_denom.0 > 0,
        "LARGE denomination rows must exist before removal"
    );

    let before_source: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM denomination_group_sources WHERE currency_code = 'USD' AND group_code = 'LARGE'",
    )
    .fetch_one(&desk.pool)
    .await
    .expect("count sources before");
    assert!(
        before_source.0 > 0,
        "LARGE group source rows must exist before removal"
    );

    let answer = remove_group(&app, &owner, "USD", "LARGE").await;
    assert_eq!(answer.status, 200, "status: {}", answer.text);
    assert_eq!(
        answer.json(),
        json!({
            "currency": "USD",
            "group": "LARGE",
        }),
        "command output: {}",
        answer.text
    );

    let after_denom: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM denominations WHERE currency_code = 'USD' AND group_code = 'LARGE'",
    )
    .fetch_one(&desk.pool)
    .await
    .expect("count denominations after");
    assert_eq!(after_denom.0, 0, "LARGE denomination rows must be deleted");

    let after_source: (i64,) = sqlx::query_as(
        "SELECT count(*) FROM denomination_group_sources WHERE currency_code = 'USD' AND group_code = 'LARGE'",
    )
    .fetch_one(&desk.pool)
    .await
    .expect("count sources after");
    assert!(
        after_source.0 == 0,
        "LARGE group source rows must be deleted"
    );
}
