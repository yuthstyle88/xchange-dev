//! Ticket #769 — what the three readers' stale-policy tests share: a published line that reads
//! stale, under a policy version that exists, or naming one that cannot be found.
//! Written before the implementation by the ticket author (claude/e5ab6c). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/rates.md` §4.1 (tables), §4.8 "Stale at read time"
//! (`PgPublishedRates::line_in` marks a line stale when its `source_snapshot` has `fetched_at`
//! older than `max_reference_age_hours`, decided on the wall clock), §4.9 R5 (*a stale line whose
//! policy cannot be found must be an error*). Ticket #769 §2, §9.
//!
//! Publications are seeded with SQL, as `quote_common/mod.rs` and `rates_re4_bookings_test.rs`
//! seed them. `PublishService` writes `fetched_at`, `max_reference_age_hours` and
//! `policy_version_id` together (`publish_service.rs:330-342`), so no route makes a stale line
//! whose policy cannot be found; that data error is what these helpers seed. A reference fetched
//! 30 h ago against 24 h is stale; one fetched 1 h ago is not, so no test depends on timing.
//!
//! Each caller loads this module once, with `mod stale_policy_769_common;`.
//!
//! Needs `TEST_DATABASE_URL`, as the caller's harness does; nothing here skips.
#![allow(dead_code)]

use chrono::{Duration, Utc};
use serde_json::json;
use sqlx::PgPool;

/// What a seeded line's `source_snapshot` says about its policy.
#[derive(Clone)]
pub enum Names {
    /// `policy_version_id` names this id (a row may or may not exist).
    Policy(String),
    /// No `policy_version_id` key at all.
    NoKey,
    /// `policy_version_id` is JSON `null`.
    Null,
}

/// An id no `rate_policy_versions` row has.
pub fn unknown_policy() -> Names {
    Names::Policy(uuid::Uuid::new_v4().to_string())
}

/// A policy version of `branch` with `on_stale` (`None` = NULL) and `stale_limit_thb`, whose
/// references go stale after 24 h. Answers its id.
pub async fn policy(
    pool: &PgPool,
    branch: &str,
    on_stale: Option<&str>,
    stale_limit_thb: Option<&str>,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO rate_policy_versions (id, organization_id, branch_id, version, \
             buy_adjust_pct, sell_adjust_pct, guards, max_reference_age_hours, on_stale, \
             stale_limit_thb, valid_from, valid_to, approved_by, reason, source_version) \
         SELECT $1::uuid, b.organization_id, b.id, \
             COALESCE((SELECT max(version) FROM rate_policy_versions WHERE branch_id = b.id), 0) + 1, \
             0, 0, '{}'::jsonb, 24, $3, $4::numeric, now() - interval '1 day', NULL, \
             'test', 'test', NULL \
         FROM branches b WHERE b.id = $2",
    )
    .bind(&id)
    .bind(branch)
    .bind(on_stale)
    .bind(stale_limit_thb)
    .execute(pool)
    .await
    .expect("insert into rate_policy_versions");
    id
}

/// The rows of `rate_policy_versions` with `id` (0 for an unknown id).
pub async fn policy_rows(pool: &PgPool, names: &Names) -> i64 {
    let Names::Policy(id) = names else {
        return 0;
    };
    sqlx::query_scalar("SELECT count(*) FROM rate_policy_versions WHERE id::text = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("count rate_policy_versions")
}

/// Publish AUTO lines `(code, unit_size, buy, sell)` whose reference was fetched `age_hours` ago
/// against a 24 h limit, their snapshot naming the policy as `names` says, as `branch`'s next
/// version, and point the branch at it. Answers the line ids by currency.
pub async fn publish_auto(
    pool: &PgPool,
    branch: &str,
    names: &Names,
    age_hours: i64,
    lines: &[(&str, &str, &str, &str)],
) -> Vec<(String, String)> {
    let request = uuid::Uuid::new_v4().to_string();
    let publication = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
             object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
             status, idempotency_key) \
         SELECT $1::uuid, organization_id, 'RATE_PUBLICATION', id, $2, 'test', '1', \
             '{}'::jsonb, 'test', now(), 'APPROVED', $3 \
         FROM branches WHERE id = $4",
    )
    .bind(&request)
    .bind(&publication)
    .bind(format!("s769-{publication}"))
    .bind(branch)
    .execute(pool)
    .await
    .expect("insert into approval_requests");
    sqlx::query(
        "INSERT INTO rate_publications (id, organization_id, branch_id, version, published_at, \
             approved_request_id, draft_id, published_by) \
         SELECT $1::uuid, b.organization_id, b.id, \
             COALESCE((SELECT max(version) FROM rate_publications WHERE branch_id = b.id), 0) + 1, \
             now(), $2::uuid, NULL, 'test' \
         FROM branches b WHERE b.id = $3",
    )
    .bind(&publication)
    .bind(&request)
    .bind(branch)
    .execute(pool)
    .await
    .expect("insert into rate_publications");
    let fetched_at = (Utc::now() - Duration::hours(age_hours)).to_rfc3339();
    let mut ids = Vec::new();
    for (code, unit, buy, sell) in lines {
        let id = uuid::Uuid::new_v4().to_string();
        let mut snapshot = json!({
            "currency": code,
            "buy_mode": "AUTO",
            "sell_mode": "AUTO",
            "fetched_at": fetched_at,
            "source_date": &fetched_at[..10],
            "max_reference_age_hours": 24,
        });
        match names {
            Names::Policy(policy) => snapshot["policy_version_id"] = json!(policy),
            Names::Null => snapshot["policy_version_id"] = serde_json::Value::Null,
            Names::NoKey => {}
        }
        sqlx::query(
            "INSERT INTO published_rate_lines (id, organization_id, publication_id, \
                 currency_code, denomination_group, unit_size, buy_rate, sell_rate, \
                 source_snapshot) \
             SELECT $1::uuid, organization_id, $2::uuid, $3, 'ALL', $4::numeric, $5::numeric, \
                 $6::numeric, $7::jsonb \
             FROM rate_publications WHERE id = $2::uuid",
        )
        .bind(&id)
        .bind(&publication)
        .bind(*code)
        .bind(*unit)
        .bind(*buy)
        .bind(*sell)
        .bind(snapshot.to_string())
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("insert the {code} line: {e}"));
        ids.push(((*code).to_owned(), id));
    }
    let pointed =
        sqlx::query("UPDATE branches SET current_publication_id = $1::uuid WHERE id = $2")
            .bind(&publication)
            .bind(branch)
            .execute(pool)
            .await
            .expect("point the branch at the publication")
            .rows_affected();
    assert_eq!(pointed, 1, "the setup: {branch} exists");
    ids
}

/// The line id of `code` among `ids`.
pub fn id_of(ids: &[(String, String)], code: &str) -> String {
    ids.iter()
        .find(|(c, _)| c == code)
        .map(|(_, id)| id.clone())
        .unwrap_or_else(|| panic!("{code} was published"))
}

/// Make the line's reference `hours` old (`line_in` decides `stale` from
/// `source_snapshot.fetched_at` and the wall clock).
pub async fn reference_age(pool: &PgPool, line: &str, hours: i64) {
    let fetched_at = (Utc::now() - Duration::hours(hours)).to_rfc3339();
    let changed = sqlx::query(
        "UPDATE published_rate_lines \
         SET source_snapshot = jsonb_set(source_snapshot, '{fetched_at}', to_jsonb($2::text)) \
         WHERE id = $1::uuid",
    )
    .bind(line)
    .bind(fetched_at)
    .execute(pool)
    .await
    .expect("age the line's reference")
    .rows_affected();
    assert_eq!(changed, 1, "the setup: line {line} exists");
}

/// The three ways a stale line's policy cannot be found (§9), with a label for messages.
pub fn unfindable() -> Vec<(Names, &'static str)> {
    vec![
        (Names::NoKey, "no policy_version_id key"),
        (Names::Null, "policy_version_id is null"),
        (unknown_policy(), "the policy row is missing"),
    ]
}
