//! Regression test for #4791: `POST /api/kiosk` (`action: "quote"`) when `margin_guards` holds
//! a row with `max_spread_share = NULL` (no margin-guard cap) or a non-NULL cap.

mod common;
mod compat_common;
#[macro_use]
mod kiosk_http_common;
#[macro_use]
mod rpc_common;
mod snapshot_common;

use kiosk_http_common::Counter;
use rust_decimal::Decimal;
use serde_json::json;
use uuid::Uuid;

const MAIN: &str = "MAIN";

async fn seed_usd_uplift_campaign(counter: &Counter, org_id: Uuid, user_id: Uuid, delta: &str) {
    let campaign_id = Uuid::new_v4();
    let version_id = Uuid::new_v4();
    let rule_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO campaigns (id, organization_id, name, created_by, created_at) \
         VALUES ($1, $2, $3, $4, now())",
    )
    .bind(campaign_id)
    .bind(org_id)
    .bind(format!("kiosk-mg-{}", Uuid::new_v4()))
    .bind(user_id)
    .execute(&counter.desk.db.pool)
    .await
    .expect("insert campaign");

    sqlx::query(
        "INSERT INTO campaign_versions (id, organization_id, campaign_id, version, status, starts_at, ends_at, \
         budget_thb, stack_group, priority, version_hash, edited_at, created_by, created_at) \
         VALUES ($1, $2, $3, 1, 'APPROVED', '2020-01-01T00:00:00Z'::timestamptz, '2030-01-01T00:00:00Z'::timestamptz, \
         5000.00, 'default', 1, 'hash_4791', now(), $4, now())",
    )
    .bind(version_id)
    .bind(org_id)
    .bind(campaign_id)
    .bind(user_id)
    .execute(&counter.desk.db.pool)
    .await
    .expect("insert campaign_version");

    sqlx::query("UPDATE campaigns SET current_version_id = $1 WHERE id = $2")
        .bind(version_id)
        .bind(campaign_id)
        .execute(&counter.desk.db.pool)
        .await
        .expect("set current_version_id");

    sqlx::query(
        "INSERT INTO campaign_branches (campaign_version_id, branch_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(version_id)
    .bind(MAIN)
    .execute(&counter.desk.db.pool)
    .await
    .expect("insert campaign_branches");

    sqlx::query(
        "INSERT INTO benefit_rules (id, campaign_version_id, benefit_type, benefit_class, eligibility, calculation, caps, requires_coupon, rule_no) \
         VALUES ($1, $2, 'RATE_IMPROVEMENT', 'PRICE', $3, $4, '{}'::jsonb, false, 1)",
    )
    .bind(rule_id)
    .bind(version_id)
    .bind(json!({ "currencies": ["USD"] }))
    .bind(json!({ "type": "FIXED_RATE", "delta": delta }))
    .execute(&counter.desk.db.pool)
    .await
    .expect("insert benefit_rule");

    sqlx::query(
        "INSERT INTO campaign_budgets (campaign_version_id, consumed_thb, used_count) VALUES ($1, 0, 0) \
         ON CONFLICT (campaign_version_id) DO NOTHING",
    )
    .bind(version_id)
    .execute(&counter.desk.db.pool)
    .await
    .expect("insert campaign_budgets");
}

async fn upsert_margin_guard(
    counter: &Counter,
    org_id: Uuid,
    user_id: Uuid,
    max_spread_share: Option<Decimal>,
) {
    sqlx::query(
        "INSERT INTO margin_guards (organization_id, max_spread_share, set_by, set_at) \
         VALUES ($1, $2, $3, now()) \
         ON CONFLICT (organization_id) DO UPDATE SET \
             max_spread_share = EXCLUDED.max_spread_share, \
             set_by = EXCLUDED.set_by, \
             set_at = EXCLUDED.set_at",
    )
    .bind(org_id)
    .bind(max_spread_share)
    .bind(user_id)
    .execute(&counter.desk.db.pool)
    .await
    .expect("upsert margin_guards");
}

async fn quote_final_rate(counter: &Counter, request_id: &str) -> String {
    sqlx::query_scalar(
        "SELECT l.final_rate::text \
         FROM kiosk_requests k \
         JOIN quote_lines l ON l.quote_id = k.quote_id \
         WHERE k.branch_id = $1 AND k.id = $2",
    )
    .bind(MAIN)
    .bind(request_id)
    .fetch_one(&counter.desk.db.pool)
    .await
    .expect("fetch final_rate for kiosk quote")
}

#[actix_web::test]
async fn kiosk_quote_handles_null_and_non_null_margin_guard_max_spread_share() {
    let counter = Counter::open(&[]).await;

    let (org_id, user_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT b.organization_id, u.id \
         FROM branches b \
         JOIN users u ON u.organization_id = b.organization_id \
         WHERE b.id = $1 \
         LIMIT 1",
    )
    .bind(MAIN)
    .fetch_one(&counter.desk.db.pool)
    .await
    .expect("branch org and user");

    // Counter::open publishes USD at BUY 32.10 / SELL 32.60 (spread = 0.50).
    // Seed a +0.40 BUY uplift rule on MAIN so we can observe uncapped vs capped final_rate.
    seed_usd_uplift_campaign(&counter, org_id, user_id, "0.40").await;

    // 1. margin_guards row exists with max_spread_share = NULL (no cap).
    upsert_margin_guard(&counter, org_id, user_id, None).await;

    counter.hands.ids.push("K-MG-NULL");
    let ans_null = counter
        .ask(
            None,
            "quote",
            json!({ "type": "BUY", "currency": "USD", "amount": 100 }),
        )
        .await;
    assert_eq!(
        ans_null.status, 200,
        "quote with margin_guards.max_spread_share = NULL must answer 200: {}",
        ans_null.text
    );
    assert_eq!(
        quote_final_rate(&counter, "K-MG-NULL").await,
        "32.50000000",
        "NULL max_spread_share applies no cap (+0.40 uplift on 32.10 base)"
    );

    // 2. margin_guards row has non-NULL max_spread_share = 0.2000 (caps uplift to 20% of 0.50 = +0.10).
    upsert_margin_guard(
        &counter,
        org_id,
        user_id,
        Some(Decimal::from_str_exact("0.2000").unwrap()),
    )
    .await;

    counter.hands.ids.push("K-MG-CAPPED");
    let ans_capped = counter
        .ask(
            None,
            "quote",
            json!({ "type": "BUY", "currency": "USD", "amount": 100 }),
        )
        .await;
    assert_eq!(
        ans_capped.status, 200,
        "quote with margin_guards.max_spread_share = 0.2000 must answer 200: {}",
        ans_capped.text
    );
    assert_eq!(
        quote_final_rate(&counter, "K-MG-CAPPED").await,
        "32.20000000",
        "non-NULL max_spread_share = 0.2000 caps uplift to +0.10 on 32.10 base"
    );

    counter.finish().await;
}
