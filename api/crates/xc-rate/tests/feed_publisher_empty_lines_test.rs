//! Rate feed F5 rework test — deduplication when publication has no lines (#1101, reviewer claude/c954feda).
//!
//! Verifies that `publish_latest` does not republish the same batch when the current publication
//! has no lines (e.g. mapped group key is missing from the batch).

mod rate_engine_common;

use std::sync::Arc;

use rate_engine_common::*;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::approval_repository::ApprovalRepository;
use xc_approval::domain::audit_log::AuditLog;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_rate::application::approval::{rate_feed_publication, rate_publication};
use xc_rate::application::currency::denomination_group_service::DenominationGroupService;
use xc_rate::application::publication::feed_publisher::FeedPublisher;
use xc_rate::domain::currency::denomination_groups::{DenominationGroups, GroupCode};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::publication::published_rates::PublicationRef;
use xc_rate::infrastructure::currency::denomination_groups::PgDenominationGroups;

const BOARD: &str = "RATE_BOARD";

type BoardRow = (
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
);

fn usd_rows(key: &'static str) -> Vec<BoardRow> {
    vec![
        (
            "USD",
            key,
            "T1",
            None,
            Some("10000.00"),
            Some("32.00"),
            Some("32.50"),
            Some("31.50"),
            Some("33.00"),
        ),
        (
            "USD",
            key,
            "T2",
            Some("10000.00"),
            None,
            Some("32.00"),
            Some("32.50"),
            Some("31.70"),
            Some("32.80"),
        ),
    ]
}

fn publisher(db: &TestDb) -> FeedPublisher {
    let mut registry = ApprovalRegistry::new();
    rate_publication::register(&mut registry, db.pool.clone());
    rate_feed_publication::register(&mut registry, db.pool.clone());
    let repository: Arc<dyn ApprovalRepository> =
        Arc::new(PgApprovalRepository::new(db.pool.clone()));
    let audit: Arc<dyn AuditLog> = Arc::new(PgAuditLog::new(db.pool.clone()));
    let approvals = Arc::new(ApprovalService::new(registry, repository, audit));
    let groups: Arc<dyn DenominationGroups> = Arc::new(PgDenominationGroups::new(db.pool.clone()));
    FeedPublisher::new(db.pool.clone(), groups, approvals)
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

async fn publish(db: &TestDb) -> Result<Option<PublicationRef>, &'static str> {
    let mut tx = db.begin().await;
    let got = publisher(db)
        .publish_latest(&mut tx, &main_branch(), BOARD)
        .await;
    settle(tx, got).await
}

async fn map(db: &TestDb, currency: &str, group: &str, instrument_key: &str) {
    let groups: Arc<dyn DenominationGroups> = Arc::new(PgDenominationGroups::new(db.pool.clone()));
    let audit: Arc<dyn AuditLog> = Arc::new(PgAuditLog::new(db.pool.clone()));
    let service = DenominationGroupService::new(groups, audit);
    let g = GroupCode::new(group).expect("a group code");
    let mut tx = db.begin().await;
    service
        .create_group(&mut tx, &owner(), code(currency), &g)
        .await
        .expect("create the group");
    service
        .map_group(&mut tx, &owner(), code(currency), &g, BOARD, instrument_key)
        .await
        .expect("map the group");
    tx.commit().await.expect("commit");
}

async fn board_batch(db: &TestDb, rows: &[BoardRow]) -> (String, String) {
    let checksum = unique("board");
    let batch: String = sqlx::query_scalar(
        "INSERT INTO rate_reference_batches \
           (id, provider, source_date, fetched_at, raw_checksum, status) \
         VALUES (gen_random_uuid(), 'RATE_BOARD', (now() AT TIME ZONE 'UTC')::date, now(), $1, \
                 'COMPLETE') \
         RETURNING id::text",
    )
    .bind(&checksum)
    .fetch_one(&db.pool)
    .await
    .expect("insert a board batch");
    for (currency, key, tier, lo, hi, base_buy, base_sell, buy, sell) in rows {
        sqlx::query(
            "INSERT INTO rate_reference_lines \
               (id, batch_id, currency_code, reference_type, unit_size, buy, sell, raw, \
                instrument_key, value_tier, tier_lower_exclusive_thb, tier_upper_inclusive_thb, \
                base_buy, base_sell) \
             VALUES (gen_random_uuid(), $1::uuid, $2, 'RATE_BOARD', 1, $3::numeric, $4::numeric, \
                     $5, $6, $7, $8::numeric, $9::numeric, $10::numeric, $11::numeric)",
        )
        .bind(&batch)
        .bind(*currency)
        .bind(*buy)
        .bind(*sell)
        .bind(json!({"status": "ACTIVE"}))
        .bind(*key)
        .bind(*tier)
        .bind(*lo)
        .bind(*hi)
        .bind(*base_buy)
        .bind(*base_sell)
        .execute(&db.pool)
        .await
        .unwrap_or_else(|e| panic!("insert board row {currency}/{key}/{tier}: {e}"));
    }
    (batch, checksum)
}

#[tokio::test]
async fn same_batch_twice_with_no_published_lines_writes_nothing_on_second_call() {
    let db = TestDb::start().await;
    // Map USD/ALL to a key that the batch lacks (USD9)
    map(&db, "USD", "ALL", "USD9").await;

    // Batch has rows for USD, but not USD9
    board_batch(&db, &usd_rows("USD")).await;

    // First publish creates a publication (with 0 lines)
    let first = publish(&db)
        .await
        .expect("published")
        .expect("a first publication is created");

    let count_after_first = db.publications(MAIN).await.len();
    assert_eq!(
        count_after_first, 1,
        "exactly one publication after first call"
    );

    // Second publish on the same batch must return Ok(None) and write nothing
    let second = publish(&db).await.expect("publish call succeeds");
    assert_eq!(second, None, "second call on same batch returns None");

    let count_after_second = db.publications(MAIN).await.len();
    assert_eq!(
        count_after_second, count_after_first,
        "no new publication written"
    );

    // Third call should still return None
    let third = publish(&db).await.expect("publish call succeeds");
    assert_eq!(third, None, "third call on same batch returns None");
    assert_eq!(
        db.publications(MAIN).await.len(),
        1,
        "still exactly one publication"
    );

    assert_eq!(first.version, 1);
    db.finish().await;
}
