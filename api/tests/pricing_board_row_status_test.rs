//! G8 (`pricing_board` row status, V.9 `statusOf`):
//! - a currency published at its suggested tiers reads `"Current"`;
//! - one never published reads `"Unpriced"` (or `"Manual"` when it has no anchor).

mod pricing_common;

use pricing_common::*;
use serde_json::{Value, json};

async fn board(room: &Room) -> Value {
    view(room, &owner(), "pricing_board", B01)
        .await
        .ok("pricing_board")["data"]
        .clone()
}

fn row<'a>(data: &'a Value, currency: &str) -> &'a Value {
    data["rows"]
        .as_array()
        .expect("rows is an array")
        .iter()
        .find(|r| r["currency"] == json!(currency))
        .unwrap_or_else(|| panic!("no {currency} row"))
}

#[tokio::test]
async fn an_unpriced_currency_reads_unpriced_and_one_without_anchor_reads_manual() {
    let room = Room::start().await;
    room.worked_example().await;
    room.tt_batch(&[("USD", "32.5000", "33.1000")], 0).await;

    let data = board(&room).await;
    assert_eq!(row(&data, "USD")["status"], json!("Unpriced"));
    assert_eq!(row(&data, "EUR")["status"], json!("Manual"));

    room.finish().await;
}

#[tokio::test]
async fn a_currency_published_at_its_suggested_tiers_reads_current() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    room.tt_batch(&[("USD", "32.5000", "33.1000")], 0).await;

    let approval: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
         proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
         VALUES (gen_random_uuid(), 'RATE_PUBLISH', $1, 'x', 'h', $2, '{}', 'test', now(), 'APPROVED', $3, '00000000-0000-0000-0000-000000000001') RETURNING id",
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
    for (tier, buy, sell) in [
        ("T1", "32.53", "33.07"),
        ("T2", "32.54", "33.06"),
        ("T3", "32.56", "33.04"),
        ("T4", "32.59", "33.01"),
    ] {
        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, value_tier, unit_size, \
             buy_rate, sell_rate, source_snapshot) VALUES (gen_random_uuid(), $1, 'USD', 'ALL', $2, 1, $3::numeric, $4::numeric, '{}')",
        )
        .bind(publication)
        .bind(tier)
        .bind(buy)
        .bind(sell)
        .execute(&room.pool)
        .await
        .expect("a published line");
    }
    sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
        .bind(publication)
        .bind(B01)
        .execute(&room.pool)
        .await
        .expect("current");

    let data = board(&room).await;
    assert_eq!(row(&data, "USD")["status"], json!("Current"));

    room.finish().await;
}
