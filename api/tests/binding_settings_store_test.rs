//! Up-front tests for BQ-1's store `PgBindingSettings` (design `docs/design/blueprint/binding-quote.md` D-10, §5).
//! Written by the sub-controller claude/644b3958 on `feat/binding-quote` before the implementation. **Do not edit,
//! delete or weaken.**
//!
//! `load_in` reads `exchange_settings.binding_quote_ttl_seconds` (BQ-0) of the organisation in the caller's
//! transaction: `NULL`, or no row at all, is `ttl: None` (off); a value is that many seconds.
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`).

mod quote_common;

use chrono::Duration;
use quote_common::*;
use uuid::Uuid;
use xc_exchange::domain::quote::binding_policy::BindingQuotePolicy;
use xc_exchange::domain::quote::binding_settings::BindingSettingsRepository;
use xc_exchange::infrastructure::quote::binding_settings::PgBindingSettings;

async fn load(db: &TestDb, org: &str) -> BindingQuotePolicy {
    let mut tx = db.pool.begin().await.expect("begin");
    let got = PgBindingSettings::new()
        .load_in(&mut tx, Uuid::parse_str(org).expect("an org id"))
        .await
        .unwrap_or_else(|e| panic!("load_in: {e}"));
    tx.rollback().await.expect("rollback");
    got
}

#[tokio::test]
async fn q1_a_null_lifetime_is_off() {
    let db = TestDb::start().await;
    assert_eq!(load(&db, ORG_1).await, BindingQuotePolicy { ttl: None });
    db.finish().await;
}

#[tokio::test]
async fn q2_an_entered_lifetime_is_that_many_seconds() {
    let db = TestDb::start().await;
    db.exec(&format!(
        "UPDATE exchange_settings SET binding_quote_ttl_seconds = 900 WHERE organization_id = '{ORG_1}'"
    ))
    .await;
    assert_eq!(
        load(&db, ORG_1).await,
        BindingQuotePolicy {
            ttl: Some(Duration::seconds(900))
        }
    );
    db.finish().await;
}

#[tokio::test]
async fn an_organisation_with_no_settings_row_is_off() {
    let db = TestDb::start().await;
    let org2 = "00000000-0000-0000-0000-000000000002";
    db.second_organisation(org2, "XC2", "B2").await;
    db.exec(&format!(
        "DELETE FROM exchange_settings WHERE organization_id = '{org2}'"
    ))
    .await;
    assert_eq!(load(&db, org2).await, BindingQuotePolicy { ttl: None });
    db.finish().await;
}

#[tokio::test]
async fn each_organisation_reads_its_own_lifetime() {
    let db = TestDb::start().await;
    let org2 = "00000000-0000-0000-0000-000000000002";
    db.second_organisation(org2, "XC2", "B2").await;
    db.exec(&format!(
        "INSERT INTO exchange_settings (organization_id, updated_by, updated_at, binding_quote_ttl_seconds) \
         VALUES ('{org2}', 'test', now(), 60) \
         ON CONFLICT (organization_id) DO UPDATE SET binding_quote_ttl_seconds = 60"
    ))
    .await;
    assert_eq!(load(&db, ORG_1).await, BindingQuotePolicy { ttl: None });
    assert_eq!(
        load(&db, org2).await,
        BindingQuotePolicy {
            ttl: Some(Duration::seconds(60))
        }
    );
    db.finish().await;
}
