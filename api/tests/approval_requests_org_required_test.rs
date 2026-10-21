//! #4093 (owner decision A, issuecomment-6306957726): `approval_requests.organization_id` has no DEFAULT, so a
//! writer that forgets the organisation fails loudly (23502) instead of filing the request under the default
//! organisation. The DEFAULT came from `api/migrations/20261011100000_organizations.sql`.
//!
//! Written before the implementation by AG controller 4093 (claude/studio-claude-7) on claude/553782's order.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; it fails without it, it does not skip.

mod quote_common;

use quote_common::TestDb;

const NOT_NULL_VIOLATION: &str = "23502";
const ORG: &str = "00000000-0000-0000-0000-000000000001";

const COLUMNS: &str = "id, type, branch_id, object_id, object_version_hash, policy_version, proposed_payload, \
                       requested_by, requested_at, status, idempotency_key";
const VALUES: &str = "gen_random_uuid(), 'RATE_PUBLICATION', 'MAIN', 'obj-4093', 'h', '1', '{}', 'proposer-1', \
                      now(), 'PENDING', 'k-4093'";

#[tokio::test]
async fn an_insert_without_the_organisation_is_refused() {
    let db = TestDb::start().await;
    let outcome = db
        .outcome(&format!(
            "INSERT INTO approval_requests ({COLUMNS}) VALUES ({VALUES})"
        ))
        .await;
    assert_eq!(
        outcome, NOT_NULL_VIOLATION,
        "a writer that forgets organization_id must fail, not land under the default organisation"
    );
    db.finish().await;
}

#[tokio::test]
async fn an_insert_naming_the_organisation_is_kept() {
    let db = TestDb::start().await;
    let outcome = db
        .outcome(&format!(
            "INSERT INTO approval_requests ({COLUMNS}, organization_id) VALUES ({VALUES}, '{ORG}')"
        ))
        .await;
    assert_eq!(outcome, "ok");
    db.finish().await;
}

#[tokio::test]
async fn the_column_has_no_default_and_stays_not_null() {
    let db = TestDb::start().await;
    let default = db
        .text(
            "SELECT column_default::text FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = 'approval_requests' \
               AND column_name = 'organization_id'",
        )
        .await;
    assert_eq!(
        default, None,
        "approval_requests.organization_id has no DEFAULT"
    );
    let nullable = db
        .text(
            "SELECT is_nullable::text FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = 'approval_requests' \
               AND column_name = 'organization_id'",
        )
        .await;
    assert_eq!(
        nullable.as_deref(),
        Some("NO"),
        "only the DEFAULT goes; NOT NULL stays"
    );
    db.finish().await;
}
