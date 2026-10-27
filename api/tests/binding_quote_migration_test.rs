//! Up-front tests for BQ-0 (design `docs/design/blueprint/binding-quote.md` §3, §6 item 1, D-1, D-10). Written by
//! the sub-controller claude/644b3958 on `feat/binding-quote` before the implementation. **Do not edit, delete or
//! weaken.**
//!
//! The migration is `api/migrations/<slot>_binding_quote.sql` (the slot from `scripts/migration-slot.sh next`, so
//! its version is not pinned here: these tests find the one file named `*_binding_quote.sql`). It
//! - drops the second CHECK on `quotes.type` (`type = 'INDICATIVE'`), keeping `type IN ('INDICATIVE', 'BINDING')`;
//! - adds `exchange_settings.binding_quote_ttl_seconds integer NULL CHECK (binding_quote_ttl_seconds > 0)`, no
//!   default, and **no upper bound** (O-3's range is owner-owed; its CHECK is a later migration of its own).
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`).

mod quote_common;

use std::path::Path;

use quote_common::*;

const CHECK: &str = "23514";

/// The version of the one `*_binding_quote.sql` migration.
fn binding_quote_version() -> i64 {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let found: Vec<i64> = std::fs::read_dir(&dir)
        .expect("read api/migrations")
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix("_binding_quote.sql")
                .and_then(|v| v.parse::<i64>().ok())
        })
        .collect();
    assert_eq!(
        found.len(),
        1,
        "BQ-0 adds exactly one api/migrations/<slot>_binding_quote.sql: found {found:?}"
    );
    found[0]
}

/// An INSERT of an ACTIVE DESK quote of MAIN with `type`.
async fn insert_quote(db: &TestDb, publication: &str, quote_type: &str) -> String {
    db.outcome(&format!(
        "INSERT INTO quotes (id, organization_id, branch_id, customer_id, type, channel, status, \
             publication_id, expires_at, created_by, accepted_by_transaction_id, created_at, updated_at) \
         VALUES (gen_random_uuid(), '{ORG_1}', 'MAIN', NULL, '{quote_type}', 'DESK', 'ACTIVE', \
             '{publication}', now() + interval '300 seconds', 'employee-1', NULL, now(), now())"
    ))
    .await
}

#[tokio::test]
async fn bq0_exactly_one_binding_quote_migration_exists() {
    let v = binding_quote_version();
    assert!(
        v > MIGRATION_1,
        "the binding-quote migration runs after the quotes migration"
    );
}

#[tokio::test]
async fn bq0_binding_is_admitted_and_any_other_type_is_still_refused() {
    let db = TestDb::start().await;
    let p = db.publish("MAIN", &[JPY]).await;
    assert_eq!(
        insert_quote(&db, &p.id, "INDICATIVE").await,
        "ok",
        "INDICATIVE"
    );
    assert_eq!(insert_quote(&db, &p.id, "BINDING").await, "ok", "BINDING");
    assert_eq!(insert_quote(&db, &p.id, "FIRM").await, CHECK, "FIRM");
    assert_eq!(
        insert_quote(&db, &p.id, "binding").await,
        CHECK,
        "lower case"
    );
    db.finish().await;
}

#[tokio::test]
async fn bq0_one_check_on_quotes_type_is_left_and_it_is_the_in_list() {
    let db = TestDb::start().await;
    let defs: Vec<String> = sqlx::query_scalar(
        "SELECT pg_get_constraintdef(c.oid) FROM pg_constraint c \
          WHERE c.conrelid = 'quotes'::regclass AND c.contype = 'c' \
            AND pg_get_constraintdef(c.oid) LIKE '%INDICATIVE%'",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read the CHECKs of quotes");
    assert_eq!(defs.len(), 1, "one CHECK names INDICATIVE: {defs:?}");
    assert!(
        defs[0].contains("BINDING"),
        "the one left admits BINDING: {defs:?}"
    );
    db.finish().await;
}

#[tokio::test]
async fn bq0_binding_quote_ttl_seconds_is_a_nullable_integer_with_no_default() {
    let db = TestDb::start().await;
    let row: (String, String, Option<String>) = sqlx::query_as(
        "SELECT data_type::text, is_nullable::text, column_default::text \
           FROM information_schema.columns \
          WHERE table_schema = current_schema() AND table_name = 'exchange_settings' \
            AND column_name = 'binding_quote_ttl_seconds'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("exchange_settings.binding_quote_ttl_seconds exists");
    assert_eq!(row.0, "integer");
    assert_eq!(row.1, "YES", "NULL = binding quotes off (D-10)");
    assert_eq!(row.2, None, "no default (O-3 A)");
    db.finish().await;
}

#[tokio::test]
async fn bq0_a_lifetime_must_be_positive_and_has_no_upper_bound_yet() {
    let db = TestDb::start().await;
    let set = |v: &str| {
        format!(
            "UPDATE exchange_settings SET binding_quote_ttl_seconds = {v} WHERE organization_id = '{ORG_1}'"
        )
    };
    for (v, want) in [
        ("0", CHECK),
        ("-1", CHECK),
        ("1", "ok"),
        ("600", "ok"),
        ("86400", "ok"),
        ("2147483647", "ok"),
        ("NULL", "ok"),
    ] {
        assert_eq!(
            db.outcome(&set(v)).await,
            want,
            "binding_quote_ttl_seconds {v}"
        );
    }
    db.finish().await;
}

#[tokio::test]
async fn bq0_existing_rows_keep_their_meaning_and_every_organisation_starts_off() {
    let db = TestDb::empty().await;
    let v = binding_quote_version();
    db.migrate_through(Some(v - 1)).await;
    let org2 = "00000000-0000-0000-0000-000000000002";
    db.second_organisation(org2, "XC2", "B2").await;
    db.exec(&format!(
        "INSERT INTO exchange_settings (organization_id, quote_ttl_seconds, updated_by, updated_at) \
         VALUES ('{org2}', 120, 'test', now()) ON CONFLICT (organization_id) DO UPDATE SET quote_ttl_seconds = 120"
    ))
    .await;
    let p = db.publish("MAIN", &[JPY]).await;
    assert_eq!(insert_quote(&db, &p.id, "INDICATIVE").await, "ok", "setup");
    assert_eq!(
        insert_quote(&db, &p.id, "BINDING").await,
        CHECK,
        "before BQ-0"
    );

    db.migrate_through(None).await;

    assert_eq!(
        db.count("SELECT count(*) FROM quotes WHERE type = 'INDICATIVE'")
            .await,
        1,
        "the existing quote is untouched"
    );
    assert_eq!(
        db.count("SELECT count(*) FROM exchange_settings WHERE binding_quote_ttl_seconds IS NOT NULL")
            .await,
        0,
        "every organisation starts with binding quotes off (D-10, §6 item 1)"
    );
    assert_eq!(
        db.count(&format!(
            "SELECT quote_ttl_seconds::bigint FROM exchange_settings WHERE organization_id = '{org2}'"
        ))
        .await,
        120,
        "the indicative lifetime is untouched"
    );
    db.finish().await;
}
