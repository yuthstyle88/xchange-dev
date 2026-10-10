//! Ticket #603 (RE-6): a rate publication written straight into the tables, for the kiosk tests.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit, delete or
//! weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.1 (`rate_publications`, `published_rate_lines`,
//! `branches.current_publication_id`), §4.8, §9 RE-6.
//!
//! A publication here is what RE-1's publish leaves behind (`publish_service.rs`): an approved
//! `approval_requests` row, a `rate_publications` row at the branch's next version, one
//! `published_rate_lines` row per line (rate per `unit_size`, scale 8, NULL for an unavailable
//! side, `source_snapshot` with `partner` `{"name", "cost"}` or `null`), and the branch's
//! `current_publication_id` pointing at it. No route and no service of the rate context is used,
//! because `xc-kiosk` does not depend on `xc-rate`.
//!
//! Signatures/fields this module assumes: the tables of migration `20261010120000_rate_engine.sql`
//! and `20261010110000_approvals.sql` as merged in #598.
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;

/// One line of a seeded publication.
#[derive(Debug, Clone)]
pub struct Line {
    pub code: String,
    /// The line's `unit_size`; `None` copies the currency's register unit, as a publish does.
    pub unit_size: Option<String>,
    pub buy: Option<String>,
    pub sell: Option<String>,
}

/// A line with both sides, at the currency's register unit.
pub fn both(code: &str, buy: &str, sell: &str) -> Line {
    Line {
        code: code.to_owned(),
        unit_size: None,
        buy: Some(buy.to_owned()),
        sell: Some(sell.to_owned()),
    }
}

/// A line with the buy side unavailable (`buy_rate` NULL).
pub fn sell_only(code: &str, sell: &str) -> Line {
    Line {
        code: code.to_owned(),
        unit_size: None,
        buy: None,
        sell: Some(sell.to_owned()),
    }
}

/// A line with the sell side unavailable (`sell_rate` NULL).
pub fn buy_only(code: &str, buy: &str) -> Line {
    Line {
        code: code.to_owned(),
        unit_size: None,
        buy: Some(buy.to_owned()),
        sell: None,
    }
}

fn snapshot(code: &str) -> Value {
    json!({
        "currency": code,
        "buy_mode": "MANUAL",
        "sell_mode": "MANUAL",
        "reason": "seeded by the test",
        "fetched_at": null,
        "source_date": null,
        "max_reference_age_hours": null,
        "reference_line_id": null,
        "policy_version_id": null,
        "buy_adjust_pct": null,
        "sell_adjust_pct": null,
        "partner": null,
    })
}

/// Publishes `lines` for `branch` at `at` and makes it the branch's current publication.
/// Answers the publication's id as text.
pub async fn publish(pool: &PgPool, branch: &str, at: DateTime<Utc>, lines: &[Line]) -> String {
    let mut tx = pool.begin().await.expect("begin the seeded publication");
    let request: String = sqlx::query_scalar(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, \
         policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
         VALUES (gen_random_uuid(), 'RATE_PUBLICATION', $1, 'seeded', 'seeded', '1', '{}'::jsonb, \
         'test', $2, 'APPROVED', gen_random_uuid()::text) RETURNING id::text",
    )
    .bind(branch)
    .bind(at)
    .fetch_one(&mut *tx)
    .await
    .expect("seed the approval request");
    let publication: String = sqlx::query_scalar(
        "INSERT INTO rate_publications (id, branch_id, version, published_at, \
         approved_request_id, draft_id, published_by) \
         SELECT gen_random_uuid(), $1, COALESCE(MAX(version), 0) + 1, $2, $3::uuid, NULL, 'test' \
         FROM rate_publications WHERE branch_id = $1 RETURNING id::text",
    )
    .bind(branch)
    .bind(at)
    .bind(&request)
    .fetch_one(&mut *tx)
    .await
    .expect("seed the publication");
    for line in lines {
        sqlx::query(
            "INSERT INTO published_rate_lines (id, publication_id, currency_code, \
             denomination_group, unit_size, buy_rate, sell_rate, source_snapshot) \
             VALUES (gen_random_uuid(), $1::uuid, $2, 'ALL', \
             COALESCE($3::numeric, (SELECT unit_size FROM currencies WHERE code = $2)), \
             $4::numeric, $5::numeric, $6)",
        )
        .bind(&publication)
        .bind(&line.code)
        .bind(&line.unit_size)
        .bind(&line.buy)
        .bind(&line.sell)
        .bind(snapshot(&line.code))
        .execute(&mut *tx)
        .await
        .unwrap_or_else(|e| panic!("seed the line {line:?}: {e}"));
    }
    sqlx::query("UPDATE branches SET current_publication_id = $1::uuid WHERE id = $2")
        .bind(&publication)
        .bind(branch)
        .execute(&mut *tx)
        .await
        .expect("point the branch at the seeded publication");
    tx.commit().await.expect("commit the seeded publication");
    publication
}

/// The branch's `current_publication_id` as text.
pub async fn current(pool: &PgPool, branch: &str) -> Option<String> {
    sqlx::query_scalar("SELECT current_publication_id::text FROM branches WHERE id = $1")
        .bind(branch)
        .fetch_one(pool)
        .await
        .expect("read branches.current_publication_id")
}
