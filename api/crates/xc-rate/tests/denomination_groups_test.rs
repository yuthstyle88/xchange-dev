//! Rate feed F3 — banknote groups and their board mappings (`docs/design/rate-feed-connector.md`
//! §2, §3.2; owner's ruling R-3 on #986: the owner/admin creates the groups in v3).
//! Written before the implementation by the ticket author (claude/c954feda). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (see `rate_engine_common/mod.rs`).
//!
//! What this pins:
//! - `xc_rate::domain::currency::denomination_groups::{GroupCode, DenominationGroups}`:
//!   - `GroupCode::new(text) -> Result<GroupCode, String>` accepts `[A-Z0-9_]{1,32}` only;
//!     `GroupCode::all()` is `"ALL"`; `as_str()`.
//!   - the trait's `groups_of(tx, code) -> Result<Vec<GroupCode>, RateEngineError>` (sorted
//!     ascending) and `source_of(tx, code, &group, provider) -> Result<Option<String>, _>`.
//! - `xc_rate::infrastructure::currency::denomination_groups::PgDenominationGroups::new(pool)`.
//! - `xc_rate::application::currency::denomination_group_service::DenominationGroupService::new(
//!   groups: Arc<dyn DenominationGroups>, audit: Arc<dyn AuditLog>)` with `create_group`,
//!   `map_group` and `remove_group`, each `(tx, who, code, &group, …) -> Result<(), RateEngineError>`.
//! - A group is a `denominations` row with `face_value IS NULL` and that `group_code`; its board
//!   mapping is a `denomination_group_sources` row.
//! - OWNER only (`outOfScope`). Every change appends one audit event; a change that changes
//!   nothing appends none.
//! - Removing `ALL`, an unknown group, a group a current publication prices, or a group that face
//!   values belong to is refused (`validationFailed`), and nothing changes.

mod rate_engine_common;

use std::sync::Arc;

use rate_engine_common::*;
use sqlx::{Postgres, Transaction};
use xc_approval::domain::audit_log::AuditLog;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_rate::application::currency::denomination_group_service::DenominationGroupService;
use xc_rate::domain::currency::denomination_groups::{DenominationGroups, GroupCode};
use xc_rate::domain::draft::rate_draft::DraftStatus;
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::infrastructure::currency::denomination_groups::PgDenominationGroups;

const BOARD: &str = "RATE_BOARD";

fn group(text: &str) -> GroupCode {
    GroupCode::new(text).expect("a valid group code")
}

fn reader(db: &TestDb) -> PgDenominationGroups {
    PgDenominationGroups::new(db.pool.clone())
}

fn service(db: &TestDb) -> DenominationGroupService {
    let groups: Arc<dyn DenominationGroups> = Arc::new(PgDenominationGroups::new(db.pool.clone()));
    let audit: Arc<dyn AuditLog> = Arc::new(PgAuditLog::new(db.pool.clone()));
    DenominationGroupService::new(groups, audit)
}

async fn settle<T>(
    tx: Transaction<'static, Postgres>,
    result: Result<T, RateEngineError>,
) -> Result<T, &'static str> {
    match result {
        Ok(v) => {
            tx.commit().await.expect("commit");
            Ok(v)
        }
        Err(e) => {
            tx.rollback().await.expect("roll back");
            Err(e.code())
        }
    }
}

async fn groups_of(db: &TestDb, currency: &str) -> Vec<String> {
    let mut tx = db.begin().await;
    let got = reader(db).groups_of(&mut tx, code(currency)).await;
    settle(tx, got)
        .await
        .expect("groups_of answers")
        .into_iter()
        .map(|g| g.as_str().to_string())
        .collect()
}

async fn source_of(db: &TestDb, currency: &str, g: &str, provider: &str) -> Option<String> {
    let mut tx = db.begin().await;
    let got = reader(db)
        .source_of(&mut tx, code(currency), &group(g), provider)
        .await;
    settle(tx, got).await.expect("source_of answers")
}

async fn create(db: &TestDb, who: &Actor, currency: &str, g: &str) -> Result<(), &'static str> {
    let mut tx = db.begin().await;
    let got = service(db)
        .create_group(&mut tx, who, code(currency), &group(g))
        .await;
    settle(tx, got).await
}

async fn map(
    db: &TestDb,
    who: &Actor,
    currency: &str,
    g: &str,
    provider: &str,
    instrument_key: &str,
) -> Result<(), &'static str> {
    let mut tx = db.begin().await;
    let got = service(db)
        .map_group(
            &mut tx,
            who,
            code(currency),
            &group(g),
            provider,
            instrument_key,
        )
        .await;
    settle(tx, got).await
}

async fn remove(db: &TestDb, who: &Actor, currency: &str, g: &str) -> Result<(), &'static str> {
    let mut tx = db.begin().await;
    let got = service(db)
        .remove_group(&mut tx, who, code(currency), &group(g))
        .await;
    settle(tx, got).await
}

/// `(action, actor_role, after_redacted as text)` of every group audit event, oldest first.
async fn audits(db: &TestDb) -> Vec<(String, String, String)> {
    sqlx::query_as(
        "SELECT action, actor_role, COALESCE(after_redacted::text, '') FROM audit_events \
         WHERE action IN ('denominationGroupCreate', 'denominationGroupMap', 'denominationGroupRemove') \
         ORDER BY id",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read audit_events")
}

async fn group_rows(db: &TestDb, currency: &str, g: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM denominations \
         WHERE currency_code = $1 AND group_code = $2 AND face_value IS NULL",
    )
    .bind(currency)
    .bind(g)
    .fetch_one(&db.pool)
    .await
    .expect("count denominations")
}

async fn mapping_rows(db: &TestDb) -> Vec<(String, String, String, String, String)> {
    sqlx::query_as(
        "SELECT currency_code::text, group_code, provider, instrument_key, updated_by \
         FROM denomination_group_sources ORDER BY currency_code, group_code, provider",
    )
    .fetch_all(&db.pool)
    .await
    .expect("read denomination_group_sources")
}

/// Publishes an OWNER BoT draft for USD on `MAIN`, so `MAIN` has a current publication with a
/// `(USD, 'ALL')` line.
async fn publish_usd(db: &TestDb) {
    let e = db.engine();
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .create(
            &mut tx,
            &main_branch(),
            &owner(),
            vec![manual_line("USD", "32.10", "33.00")],
        )
        .await;
    let d = settle(tx, got).await.expect("the draft is created");
    let mut tx = db.begin().await;
    let got = e
        .drafts
        .submit(&mut tx, &main_branch(), &owner(), &d.id, &key())
        .await;
    let outcome = settle(tx, got).await.expect("the submit is accepted");
    assert_eq!(outcome.status, DraftStatus::Approved);
    let mut tx = db.begin().await;
    let got = e
        .publisher
        .publish(&mut tx, &main_branch(), &owner(), &d.id, None)
        .await;
    settle(tx, got).await.expect("the draft is published");
}

#[test]
fn a_group_code_is_upper_case_letters_digits_and_underscores() {
    assert_eq!(GroupCode::all().as_str(), "ALL");
    assert_eq!(group("LARGE_100").as_str(), "LARGE_100");
    assert!(GroupCode::new("").is_err(), "empty");
    assert!(GroupCode::new("   ").is_err(), "blank");
    assert!(GroupCode::new("large").is_err(), "lower case");
    assert!(GroupCode::new(" LARGE").is_err(), "surrounding space");
    assert!(GroupCode::new("LAR-GE").is_err(), "a dash");
    assert!(GroupCode::new("A".repeat(32)).is_ok(), "32 characters");
    assert!(GroupCode::new("A".repeat(33)).is_err(), "33 characters");
}

#[tokio::test]
async fn a_currency_starts_with_its_all_group_and_no_mapping() {
    let db = TestDb::start().await;
    assert_eq!(groups_of(&db, "USD").await, vec!["ALL"]);
    assert_eq!(source_of(&db, "USD", "ALL", BOARD).await, None);
    db.finish().await;
}

#[tokio::test]
async fn the_owner_creates_a_group_and_it_is_audited() {
    let db = TestDb::start().await;
    create(&db, &owner(), "USD", "LARGE")
        .await
        .expect("OWNER may create");
    assert_eq!(groups_of(&db, "USD").await, vec!["ALL", "LARGE"]);
    assert_eq!(group_rows(&db, "USD", "LARGE").await, 1);
    assert_eq!(
        groups_of(&db, "EUR").await,
        vec!["ALL"],
        "groups are per currency"
    );

    let rows = audits(&db).await;
    assert_eq!(rows.len(), 1, "one audit event");
    assert_eq!(rows[0].0, "denominationGroupCreate");
    assert_eq!(rows[0].1, "OWNER");
    assert!(rows[0].2.contains("LARGE"), "after: {}", rows[0].2);
    db.finish().await;
}

#[tokio::test]
async fn creating_a_group_that_exists_writes_nothing() {
    let db = TestDb::start().await;
    create(&db, &owner(), "USD", "ALL")
        .await
        .expect("an existing group is accepted");
    create(&db, &owner(), "USD", "LARGE").await.expect("create");
    create(&db, &owner(), "USD", "LARGE")
        .await
        .expect("again is accepted");
    assert_eq!(group_rows(&db, "USD", "ALL").await, 1);
    assert_eq!(group_rows(&db, "USD", "LARGE").await, 1);
    assert_eq!(
        audits(&db).await.len(),
        1,
        "only the first create is audited"
    );
    db.finish().await;
}

#[tokio::test]
async fn only_the_owner_may_change_groups() {
    let db = TestDb::start().await;
    create(&db, &owner(), "USD", "LARGE").await.expect("create");
    let before = audits(&db).await.len();

    assert_eq!(
        create(&db, &rates(), "USD", "SMALL").await,
        Err("outOfScope")
    );
    assert_eq!(
        map(&db, &rates(), "USD", "LARGE", BOARD, "USD1").await,
        Err("outOfScope")
    );
    assert_eq!(
        remove(&db, &rates(), "USD", "LARGE").await,
        Err("outOfScope")
    );

    assert_eq!(groups_of(&db, "USD").await, vec!["ALL", "LARGE"]);
    assert!(mapping_rows(&db).await.is_empty());
    assert_eq!(audits(&db).await.len(), before, "nothing audited");
    db.finish().await;
}

#[tokio::test]
async fn the_owner_maps_a_group_to_a_board_row() {
    let db = TestDb::start().await;
    map(&db, &owner(), "USD", "ALL", BOARD, "USD")
        .await
        .expect("OWNER may map");
    assert_eq!(
        source_of(&db, "USD", "ALL", BOARD).await.as_deref(),
        Some("USD")
    );
    assert_eq!(
        source_of(&db, "USD", "ALL", "BOT").await,
        None,
        "per provider"
    );
    assert_eq!(
        source_of(&db, "EUR", "ALL", BOARD).await,
        None,
        "per currency"
    );
    assert_eq!(
        mapping_rows(&db).await,
        vec![(
            "USD".to_string(),
            "ALL".to_string(),
            BOARD.to_string(),
            "USD".to_string(),
            OWNER_ID.to_string()
        )]
    );
    let rows = audits(&db).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "denominationGroupMap");
    assert!(rows[0].2.contains("USD"), "after: {}", rows[0].2);
    db.finish().await;
}

#[tokio::test]
async fn mapping_again_replaces_the_key_and_the_same_key_writes_nothing() {
    let db = TestDb::start().await;
    create(&db, &owner(), "USD", "LARGE").await.expect("create");
    map(&db, &owner(), "USD", "LARGE", BOARD, "USD1")
        .await
        .expect("map");
    map(&db, &owner(), "USD", "LARGE", BOARD, "USD2")
        .await
        .expect("re-map");
    assert_eq!(
        source_of(&db, "USD", "LARGE", BOARD).await.as_deref(),
        Some("USD2")
    );
    assert_eq!(
        mapping_rows(&db).await.len(),
        1,
        "one row per (currency, group, provider)"
    );

    map(&db, &owner(), "USD", "LARGE", BOARD, "USD2")
        .await
        .expect("the same key is accepted");
    let maps = audits(&db)
        .await
        .into_iter()
        .filter(|r| r.0 == "denominationGroupMap")
        .count();
    assert_eq!(maps, 2, "the unchanged mapping is not audited");
    db.finish().await;
}

#[tokio::test]
async fn a_mapping_needs_a_known_group_a_provider_and_a_key() {
    let db = TestDb::start().await;
    assert_eq!(
        map(&db, &owner(), "USD", "NO_SUCH", BOARD, "USD1").await,
        Err("validationFailed"),
        "the group does not exist for USD"
    );
    create(&db, &owner(), "EUR", "LARGE").await.expect("create");
    assert_eq!(
        map(&db, &owner(), "USD", "LARGE", BOARD, "USD1").await,
        Err("validationFailed"),
        "LARGE exists for EUR, not for USD"
    );
    assert_eq!(
        map(&db, &owner(), "USD", "ALL", BOARD, "  ").await,
        Err("validationFailed"),
        "blank key"
    );
    assert_eq!(
        map(&db, &owner(), "USD", "ALL", "", "USD").await,
        Err("validationFailed"),
        "empty provider"
    );
    assert!(mapping_rows(&db).await.is_empty(), "nothing written");
    db.finish().await;
}

#[tokio::test]
async fn the_owner_removes_a_group_with_its_mappings() {
    let db = TestDb::start().await;
    create(&db, &owner(), "USD", "LARGE").await.expect("create");
    map(&db, &owner(), "USD", "LARGE", BOARD, "USD1")
        .await
        .expect("map");
    remove(&db, &owner(), "USD", "LARGE")
        .await
        .expect("OWNER may remove");
    assert_eq!(groups_of(&db, "USD").await, vec!["ALL"]);
    assert_eq!(group_rows(&db, "USD", "LARGE").await, 0);
    assert!(
        mapping_rows(&db).await.is_empty(),
        "its mapping goes with it"
    );
    assert_eq!(
        audits(&db).await.last().map(|r| r.0.clone()).as_deref(),
        Some("denominationGroupRemove")
    );
    db.finish().await;
}

#[tokio::test]
async fn the_all_group_and_unknown_groups_cannot_be_removed() {
    let db = TestDb::start().await;
    assert_eq!(
        remove(&db, &owner(), "USD", "ALL").await,
        Err("validationFailed")
    );
    assert_eq!(
        remove(&db, &owner(), "USD", "NO_SUCH").await,
        Err("validationFailed")
    );
    assert_eq!(groups_of(&db, "USD").await, vec!["ALL"]);
    assert!(audits(&db).await.is_empty());
    db.finish().await;
}

#[tokio::test]
async fn a_group_the_current_publication_prices_cannot_be_removed() {
    let db = TestDb::start().await;
    create(&db, &owner(), "USD", "LARGE").await.expect("create");
    publish_usd(&db).await;
    // Make MAIN's current publication price the LARGE group.
    sqlx::query(
        "UPDATE published_rate_lines l SET denomination_group = 'LARGE' \
         FROM branches b WHERE b.id = $1 AND l.publication_id = b.current_publication_id \
           AND l.currency_code = 'USD'",
    )
    .bind(MAIN)
    .execute(&db.pool)
    .await
    .expect("price LARGE in the current publication");

    assert_eq!(
        remove(&db, &owner(), "USD", "LARGE").await,
        Err("validationFailed"),
        "a current publication still prices it"
    );
    assert_eq!(
        groups_of(&db, "USD").await,
        vec!["ALL", "LARGE"],
        "it stays"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_group_that_face_values_belong_to_cannot_be_removed() {
    let db = TestDb::start().await;
    create(&db, &owner(), "USD", "LARGE").await.expect("create");
    sqlx::query(
        "INSERT INTO denominations (currency_code, face_value, group_code) VALUES ('USD', 1000, 'LARGE')",
    )
    .execute(&db.pool)
    .await
    .expect("a face value in LARGE");

    assert_eq!(
        remove(&db, &owner(), "USD", "LARGE").await,
        Err("validationFailed"),
        "cash counts reference denominations rows"
    );
    assert_eq!(group_rows(&db, "USD", "LARGE").await, 1, "it stays");
    db.finish().await;
}
