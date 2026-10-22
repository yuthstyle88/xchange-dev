//! XC-PRICING PX-15 (#4607): `pricing_board` does not invent silent zero rates when a published
//! tier-1 line is missing `sell_rate` (or `buy_rate`).

mod pricing_common;

use pricing_common::*;
use serde_json::{Value, json};

#[tokio::test]
async fn enabled_currency_with_null_tier1_sell_rate_emits_no_market_gap_recommendation() {
    let room = Room::start().await;
    let v1 = room.worked_example().await;
    room.tt_batch(&[("USD", "32.5000", "33.1000")], 0).await;
    room.sr_batch(&[("USD", "32.6000", "33.0000")], 0).await;

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
    sqlx::query(
        "INSERT INTO published_rate_lines (id, publication_id, currency_code, denomination_group, value_tier, unit_size, \
         buy_rate, sell_rate, source_snapshot) VALUES (gen_random_uuid(), $1, 'USD', 'ALL', 'T1', 1, 30.0000, NULL, '{}')",
    )
    .bind(publication)
    .execute(&room.pool)
    .await
    .expect("published line with sell_rate NULL");
    sqlx::query("UPDATE branches SET current_publication_id = $1 WHERE id = $2")
        .bind(publication)
        .bind(B01)
        .execute(&room.pool)
        .await
        .expect("current publication");

    let data = view(&room, &owner(), "pricing_board", B01)
        .await
        .ok("pricing_board")["data"]
        .clone();
    let usd = data["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .find(|r| r["currency"] == json!("USD"))
        .expect("USD row");
    assert_eq!(usd["enabled"], json!(true));
    assert_eq!(usd["published"][0]["sell"], Value::Null);

    let usd_recs: Vec<&Value> = data["recommendations"]
        .as_array()
        .expect("recommendations")
        .iter()
        .filter(|r| r["currency"] == json!("USD"))
        .collect();
    assert!(
        usd_recs.is_empty(),
        "expected no recommendation for USD when tier-1 sell_rate is NULL, got {usd_recs:?}"
    );

    room.finish().await;
}
