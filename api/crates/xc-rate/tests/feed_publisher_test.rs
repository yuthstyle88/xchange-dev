//! Rate feed F5 — publishing the rate board's batch on a branch (`docs/design/rate-feed-connector.md`
//! §2, §3.1, §3.3, §3.8; owner's rulings R-2, R-4, R-5, R-6, R-9 on #986).
//! Written before the implementation by the ticket author (claude/c954feda). Do not edit, delete
//! or weaken; add tests in a file of your own.
//!
//! Needs `TEST_DATABASE_URL` (see `rate_engine_common/mod.rs`).
//!
//! What this pins:
//! - `xc_rate::application::publication::feed_publisher::FeedPublisher::new(pool,
//!   groups: Arc<dyn DenominationGroups>, approvals: Arc<ApprovalService>)` and
//!   `publish_latest(tx, branch, provider) -> Result<Option<PublicationRef>, RateEngineError>`.
//! - `xc_rate::application::approval::rate_feed_publication::{register, RateFeedPublicationPolicy}`;
//!   the policy lets only `Role::System` decide its own request.
//! - One published line per mapped `(currency, group)` and per tier of its `instrument_key`, at the
//!   board's shop price converted to the currency's `unit_size`, with no `adjust_pct` (R-2); the band
//!   and the base rates copied; `NULL` sides kept `NULL` (R-6).
//! - The publication is approved through the approval core by `SYSTEM:rate-feed:<provider>`
//!   (`RATE_FEED_PUBLICATION`, APPROVED, one decision), with `draft_id NULL`.
//! - The same batch twice writes nothing (`None`); no batch → `rateUnavailable`; bands that overlap
//!   or leave a gap → `validationFailed`, nothing written.
//! - `source_snapshot` records the source, and carries the branch's current stale policy (R-9):
//!   `fetched_at` is the board's announcement time when it has one.

mod rate_engine_common;

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use rate_engine_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use std::str::FromStr;
use xc_approval::application::approval_service::ApprovalService;
use xc_approval::application::registry::ApprovalRegistry;
use xc_approval::domain::approval_repository::ApprovalRepository;
use xc_approval::domain::audit_log::AuditLog;
use xc_approval::domain::policy::ApprovalPolicy;
use xc_approval::infrastructure::approval_repository::PgApprovalRepository;
use xc_approval::infrastructure::audit_log::PgAuditLog;
use xc_auth::domain::access;
use xc_rate::application::approval::rate_feed_publication::RateFeedPublicationPolicy;
use xc_rate::application::approval::{rate_feed_publication, rate_publication};
use xc_rate::application::currency::denomination_group_service::DenominationGroupService;
use xc_rate::application::publication::feed_publisher::FeedPublisher;
use xc_rate::domain::currency::denomination_groups::{DenominationGroups, GroupCode};
use xc_rate::domain::draft::rate_engine_error::RateEngineError;
use xc_rate::domain::publication::published_rates::PublicationRef;
use xc_rate::infrastructure::currency::denomination_groups::PgDenominationGroups;

const BOARD: &str = "RATE_BOARD";
const SYSTEM_ID: &str = "SYSTEM:rate-feed:RATE_BOARD";

/// One board row: `(currency, instrument_key, tier, lower_exclusive, upper_inclusive, base_buy,
/// base_sell, buy, sell)`, all rates THB per 1 unit of the currency (BP §08).
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

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal")
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

/// Maps `(currency, group)` to `instrument_key` of the board, creating the group if needed.
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

/// One COMPLETE `RATE_BOARD` batch with one reference line per row (unit 1, as received).
async fn board_batch(
    db: &TestDb,
    fetched_at: DateTime<Utc>,
    source_published_at: Option<DateTime<Utc>>,
    rows: &[BoardRow],
) -> (String, String) {
    let checksum = unique("board");
    let batch: String = sqlx::query_scalar(
        "INSERT INTO rate_reference_batches \
           (id, provider, source_date, fetched_at, raw_checksum, status, source_published_at) \
         VALUES (gen_random_uuid(), 'RATE_BOARD', ($1::timestamptz AT TIME ZONE 'UTC')::date, $1, $2, \
                 'COMPLETE', $3) \
         RETURNING id::text",
    )
    .bind(fetched_at)
    .bind(&checksum)
    .bind(source_published_at)
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
        .bind(json!({"status": if buy.is_none() && sell.is_none() { "PAUSED" } else { "ACTIVE" }}))
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

/// A published line of `MAIN`'s current publication.
#[derive(Debug, PartialEq)]
struct Line {
    currency: String,
    group: String,
    tier: String,
    unit_size: Decimal,
    buy: Option<Decimal>,
    sell: Option<Decimal>,
    lo: Option<Decimal>,
    hi: Option<Decimal>,
    base_buy: Option<Decimal>,
    base_sell: Option<Decimal>,
}

/// One row of `current_lines`' query.
type LineRow = (
    String,
    String,
    String,
    Decimal,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
    Option<Decimal>,
);

async fn current_lines(db: &TestDb) -> Vec<Line> {
    let rows: Vec<LineRow> = sqlx::query_as(
        "SELECT l.currency_code::text, l.denomination_group, l.value_tier, l.unit_size, \
                l.buy_rate, l.sell_rate, l.tier_lower_exclusive_thb, l.tier_upper_inclusive_thb, \
                l.base_buy_rate, l.base_sell_rate \
         FROM branches b JOIN published_rate_lines l ON l.publication_id = b.current_publication_id \
         WHERE b.id = $1 ORDER BY 1, 2, 3",
    )
    .bind(MAIN)
    .fetch_all(&db.pool)
    .await
    .expect("read the current publication's lines");
    rows.into_iter()
        .map(
            |(currency, group, tier, unit_size, buy, sell, lo, hi, base_buy, base_sell)| Line {
                currency,
                group,
                tier,
                unit_size,
                buy,
                sell,
                lo,
                hi,
                base_buy,
                base_sell,
            },
        )
        .collect()
}

async fn snapshot_of(db: &TestDb, currency: &str, group: &str, tier: &str) -> Value {
    sqlx::query_scalar(
        "SELECT l.source_snapshot FROM branches b \
         JOIN published_rate_lines l ON l.publication_id = b.current_publication_id \
         WHERE b.id = $1 AND l.currency_code = $2 AND l.denomination_group = $3 AND l.value_tier = $4",
    )
    .bind(MAIN)
    .bind(currency)
    .bind(group)
    .bind(tier)
    .fetch_one(&db.pool)
    .await
    .expect("the line exists")
}

#[allow(clippy::too_many_arguments)]
fn line(
    currency: &str,
    group: &str,
    tier: &str,
    unit: &str,
    buy: Option<&str>,
    sell: Option<&str>,
    lo: Option<&str>,
    hi: Option<&str>,
    base: (Option<&str>, Option<&str>),
) -> Line {
    Line {
        currency: currency.to_string(),
        group: group.to_string(),
        tier: tier.to_string(),
        unit_size: dec(unit),
        buy: buy.map(dec),
        sell: sell.map(dec),
        lo: lo.map(dec),
        hi: hi.map(dec),
        base_buy: base.0.map(dec),
        base_sell: base.1.map(dec),
    }
}

/// Compares decimals by value (`32.10` = `32.1000`).
fn normalised(lines: Vec<Line>) -> Vec<Line> {
    let n = |d: Decimal| d.normalize();
    lines
        .into_iter()
        .map(|l| Line {
            unit_size: n(l.unit_size),
            buy: l.buy.map(n),
            sell: l.sell.map(n),
            lo: l.lo.map(n),
            hi: l.hi.map(n),
            base_buy: l.base_buy.map(n),
            base_sell: l.base_sell.map(n),
            ..l
        })
        .collect()
}

#[tokio::test]
async fn every_tier_of_every_mapped_group_is_published_at_the_board_price() {
    let db = TestDb::start().await;
    // A policy that would add 1% if it were applied: board prices must ignore it (R-2).
    db.seed_policy(MAIN, "1.0", "1.0", json!({}), Some(24), None, None)
        .await;
    map(&db, "USD", "ALL", "USD").await;
    map(&db, "USD", "LARGE", "USD1").await;
    let mut rows = usd_rows("USD");
    rows.extend(usd_rows("USD1").into_iter().map(|mut r| {
        r.7 = Some("31.60"); // LARGE notes buy a little higher
        r
    }));
    rows.push((
        "EUR",
        "EUR",
        "T1",
        None,
        None,
        Some("37.00"),
        Some("37.50"),
        Some("36.80"),
        Some("37.90"),
    ));
    board_batch(&db, Utc::now(), Some(Utc::now()), &rows).await;

    let published = publish(&db)
        .await
        .expect("published")
        .expect("a new publication");
    assert_eq!(
        db.current_publication(MAIN).await.as_deref(),
        Some(published.id.to_string().as_str())
    );

    let t1 = (None, Some("10000.00"));
    let t2 = (Some("10000.00"), None);
    let base = (Some("32.00"), Some("32.50"));
    assert_eq!(
        normalised(current_lines(&db).await),
        normalised(vec![
            line(
                "USD",
                "ALL",
                "T1",
                "1",
                Some("31.50"),
                Some("33.00"),
                t1.0,
                t1.1,
                base
            ),
            line(
                "USD",
                "ALL",
                "T2",
                "1",
                Some("31.70"),
                Some("32.80"),
                t2.0,
                t2.1,
                base
            ),
            line(
                "USD",
                "LARGE",
                "T1",
                "1",
                Some("31.60"),
                Some("33.00"),
                t1.0,
                t1.1,
                base
            ),
            line(
                "USD",
                "LARGE",
                "T2",
                "1",
                Some("31.60"),
                Some("32.80"),
                t2.0,
                t2.1,
                base
            ),
        ]),
        "every tier of each mapped group, at the board's shop price; EUR is not mapped"
    );
    db.finish().await;
}

#[tokio::test]
async fn the_publication_is_approved_by_the_system_through_the_approval_core() {
    let db = TestDb::start().await;
    map(&db, "USD", "ALL", "USD").await;
    let (batch, checksum) = board_batch(&db, Utc::now(), None, &usd_rows("USD")).await;
    let published = publish(&db)
        .await
        .expect("published")
        .expect("a new publication");

    let (published_by, draft_id, request_id): (String, Option<String>, String) = sqlx::query_as(
        "SELECT published_by, draft_id::text, approved_request_id::text FROM rate_publications WHERE id = $1",
    )
    .bind(published.id.as_uuid())
    .fetch_one(&db.pool)
    .await
    .expect("the publication");
    assert_eq!(published_by, SYSTEM_ID);
    assert_eq!(draft_id, None, "no draft");

    let (kind, object_id, status, requested_by, hash): (String, String, String, String, String) =
        sqlx::query_as(
            "SELECT type, object_id, status, requested_by, object_version_hash \
             FROM approval_requests WHERE id::text = $1",
        )
        .bind(&request_id)
        .fetch_one(&db.pool)
        .await
        .expect("the approval request");
    assert_eq!(kind, "RATE_FEED_PUBLICATION");
    assert_eq!(object_id, format!("{MAIN}:{batch}"));
    assert_eq!(status, "APPROVED");
    assert_eq!(requested_by, SYSTEM_ID);
    assert_eq!(hash, checksum, "bound to exactly the batch published");

    let deciders: Vec<(String, String)> = sqlx::query_as(
        "SELECT approver_id, outcome FROM approval_decisions WHERE request_id::text = $1",
    )
    .bind(&request_id)
    .fetch_all(&db.pool)
    .await
    .expect("its decision");
    assert_eq!(
        deciders,
        vec![(SYSTEM_ID.to_string(), "APPROVED".to_string())]
    );
    db.finish().await;
}

#[test]
fn only_the_system_may_decide_its_own_feed_publication() {
    let policy = RateFeedPublicationPolicy::new();
    // OR-7.1 #1020 (§3.3; ruling 2: ADMIN and not EXECUTIVE): SYSTEM is the B1-mapped ADMIN of MAIN.
    let system = user_on(SYSTEM_ID, access::Role::Admin, MAIN);
    assert!(policy.allows_self_decision(&system));
    assert!(
        !policy.allows_self_decision(&owner_user()),
        "not even OWNER"
    );
    assert!(!policy.allows_self_decision(&rates_user()));
    assert_eq!(policy.policy_version(), "1");
}

#[tokio::test]
async fn the_same_batch_twice_writes_nothing_and_a_new_batch_publishes_again() {
    let db = TestDb::start().await;
    map(&db, "USD", "ALL", "USD").await;
    board_batch(
        &db,
        Utc::now() - Duration::minutes(2),
        None,
        &usd_rows("USD"),
    )
    .await;
    let first = publish(&db)
        .await
        .expect("published")
        .expect("a new publication");
    let count = db.publications(MAIN).await.len();

    assert_eq!(publish(&db).await, Ok(None), "nothing new");
    assert_eq!(db.publications(MAIN).await.len(), count, "nothing written");

    let mut rows = usd_rows("USD");
    rows[0].7 = Some("31.55");
    board_batch(&db, Utc::now(), None, &rows).await;
    let second = publish(&db)
        .await
        .expect("published")
        .expect("the new batch is published");
    assert_eq!(second.version, first.version + 1);
    let lines = normalised(current_lines(&db).await);
    assert_eq!(lines[0].buy, Some(dec("31.55")));
    db.finish().await;
}

#[tokio::test]
async fn no_batch_is_rate_unavailable() {
    let db = TestDb::start().await;
    map(&db, "USD", "ALL", "USD").await;
    // A BoT batch is not the board's.
    db.seed_batch(Utc::now(), &[("USD", "1", Some("32.0"), Some("33.0"))])
        .await;
    assert_eq!(publish(&db).await, Err("rateUnavailable"));
    assert!(db.publications(MAIN).await.is_empty());
    db.finish().await;
}

#[tokio::test]
async fn rates_are_converted_to_the_currency_unit_size() {
    let db = TestDb::start().await;
    map(&db, "JPY", "ALL", "JPY").await;
    let rows: Vec<BoardRow> = vec![(
        "JPY",
        "JPY",
        "T1",
        None,
        None,
        Some("0.22"),
        Some("0.2350"),
        Some("0.2301"),
        Some("0.2400"),
    )];
    board_batch(&db, Utc::now(), None, &rows).await;
    publish(&db)
        .await
        .expect("published")
        .expect("a new publication");
    assert_eq!(
        normalised(current_lines(&db).await),
        normalised(vec![line(
            "JPY",
            "ALL",
            "T1",
            "100",
            Some("23.01"),
            Some("24.00"),
            None,
            None,
            (Some("22.00"), Some("23.50")),
        )]),
        "board 0.2301 per 1 is 23.01 per 100 (design §3.8)"
    );
    db.finish().await;
}

#[tokio::test]
async fn a_paused_board_publishes_every_side_null_and_a_resumed_board_prices_again() {
    let db = TestDb::start().await;
    map(&db, "USD", "ALL", "USD").await;
    let paused: Vec<BoardRow> = usd_rows("USD")
        .into_iter()
        .map(|mut r| {
            r.7 = None;
            r.8 = None;
            r
        })
        .collect();
    board_batch(&db, Utc::now() - Duration::minutes(2), None, &paused).await;
    publish(&db)
        .await
        .expect("published")
        .expect("a publication");
    let lines = current_lines(&db).await;
    assert_eq!(lines.len(), 2, "the tiers are still published");
    assert!(
        lines.iter().all(|l| l.buy.is_none() && l.sell.is_none()),
        "every side NULL, never a kept old price (R-6): {lines:?}"
    );

    board_batch(&db, Utc::now(), None, &usd_rows("USD")).await;
    publish(&db).await.expect("published").expect("resumed");
    assert!(current_lines(&db).await.iter().all(|l| l.buy.is_some()));
    db.finish().await;
}

#[tokio::test]
async fn a_group_mapped_to_a_key_the_batch_lacks_gets_no_line() {
    let db = TestDb::start().await;
    map(&db, "USD", "ALL", "USD").await;
    map(&db, "USD", "LARGE", "USD9").await;
    board_batch(&db, Utc::now(), None, &usd_rows("USD")).await;
    publish(&db)
        .await
        .expect("published")
        .expect("a publication");
    let groups: Vec<String> = current_lines(&db)
        .await
        .into_iter()
        .map(|l| l.group)
        .collect();
    assert_eq!(groups, vec!["ALL", "ALL"]);
    db.finish().await;
}

#[tokio::test]
async fn bands_that_overlap_or_leave_a_gap_are_refused() {
    for (what, second) in [
        ("overlap", (Some("5000.00"), None::<&str>)),
        ("gap", (Some("50000.00"), None::<&str>)),
    ] {
        let db = TestDb::start().await;
        map(&db, "USD", "ALL", "USD").await;
        let mut rows = usd_rows("USD");
        rows[1].3 = second.0;
        rows[1].4 = second.1;
        board_batch(&db, Utc::now(), None, &rows).await;
        assert_eq!(publish(&db).await, Err("validationFailed"), "{what}");
        assert!(
            db.publications(MAIN).await.is_empty(),
            "{what}: nothing written"
        );
        let requests: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM approval_requests WHERE type = 'RATE_FEED_PUBLICATION'",
        )
        .fetch_one(&db.pool)
        .await
        .expect("count requests");
        assert_eq!(requests, 0, "{what}: no request left behind");
        db.finish().await;
    }
}

#[tokio::test]
async fn the_snapshot_records_the_source_and_the_branch_stale_policy() {
    let db = TestDb::start().await;
    let policy = db
        .seed_policy(MAIN, "0", "0", json!({}), Some(24), Some("STOP"), None)
        .await;
    map(&db, "USD", "ALL", "USD").await;
    let announced = Utc::now() - Duration::hours(30);
    let (batch, checksum) = board_batch(&db, Utc::now(), Some(announced), &usd_rows("USD")).await;
    publish(&db)
        .await
        .expect("published")
        .expect("a publication");

    let s = snapshot_of(&db, "USD", "ALL", "T2").await;
    assert_eq!(s["provider"], json!(BOARD));
    assert_eq!(s["batch_id"], json!(batch));
    assert_eq!(s["batch_checksum"], json!(checksum));
    assert_eq!(s["instrument_key"], json!("USD"));
    assert_eq!(s["value_tier"], json!("T2"));
    assert_eq!(
        s["policy_version_id"],
        json!(policy),
        "the branch's current policy (R-9)"
    );
    assert_eq!(s["max_reference_age_hours"], json!(24));
    let fetched_at = s["fetched_at"]
        .as_str()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .expect("fetched_at is an RFC 3339 time");
    assert_eq!(
        fetched_at.timestamp(),
        announced.timestamp(),
        "freshness is measured from the board's announcement, not the fetch (design §3.6)"
    );
    db.finish().await;
}
