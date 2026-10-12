//! FN-2 (#1069): F01, FINANCE's reconcile-and-certify sheet parts (counts, transfers, integrity, reversals pending).
//! Written before the implementation by the up-front test writer (gemini/studio-16) on the lane controller's order.
//! Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Behaviour table from ticket #1069 §9 and lane controller rulings (issuecomment-6118975534):
//! 1. two counts verified that day, one unverified -> two CountRows; other-branch counts excluded
//! 2. count day by counted_at's business date (timezone); count verified on DAY but counted before excluded
//! 3. a transfer sent 100 USD, received 80 USD -> one short receipt, sent 100 and received 80 (face != 1)
//! 4. transfers touching the day by created/dispatched/received; older still-in-transit appears in in_transit
//! 5. one hand-broken ledger row (LG-7 check 2) -> integrity row 2 ok: false, offenders: 1; other-branch isolated
//! 6. two PENDING reversal requests of the branch, entered newest first -> returned oldest first
//! 7. a part changes after certifying -> the certification reads stale (FN-1 hash covers the parts)

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_cash::application::reconcile::Reconcile;
use xc_cash::domain::transfer::cash_transfer::TransferLine;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, Money};
use xchange::application::finance::reconciliation::{ReconciliationService, SheetPartsPort};
use xchange::application::finance::sheet_parts::SheetParts;
use xchange::infrastructure::executive::wide_transaction_facts::WideTransactionFacts;

const MUST_FINISH: Duration = Duration::from_secs(30);
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const B1: &str = "XC-B01";
const B2: &str = "XC-B02";
const DAY: &str = "2026-10-05";

const CERTIFIER: &str = "FINANCE@XC-B01#1";
const VERIFIER: &str = "FINANCE@XC-B01#2";
const COUNTER: &str = "TELLER@XC-B01#4";
const CALLERS: [&str; 3] = [CERTIFIER, VERIFIER, COUNTER];

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("a date")
}

/// `hh:mm` local time on `day` in Asia/Bangkok (UTC+7), as UTC.
fn bangkok(day: &str, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date(day).and_hms_opt(hour, minute, 0).expect("a time"))
        - chrono::Duration::hours(7)
}

fn user_id(spec: &str) -> String {
    let (_, n) = spec.rsplit_once('#').expect("a spec ends in #n");
    format!(
        "00000000-0000-4000-8000-{:012}",
        n.parse::<u64>().expect("n")
    )
}

struct Room {
    pool: PgPool,
    #[allow(dead_code)]
    schema: String,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").expect(
            "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip",
        );
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("fn2");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.clone())
            .await
            .expect("connect to the test database");
        sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin)
            .await
            .expect("create the test schema");
        admin.close().await;
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .acquire_timeout(MUST_FINISH)
            .connect_with(options.options([("search_path", schema.as_str())]))
            .await
            .expect("connect to the test schema");
        let migrations = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        Migrator::new(migrations.as_path())
            .await
            .expect("read api/migrations")
            .run(&pool)
            .await
            .expect("apply api/migrations");

        let room = Room { pool, schema };
        for spec in CALLERS {
            sqlx::query(
                "INSERT INTO users (id, organization_id, name, status, created_by) \
                 VALUES ($1::uuid, $2::uuid, $3, 'ACTIVE', 'test')",
            )
            .bind(user_id(spec))
            .bind(ORG)
            .bind(spec)
            .execute(&room.pool)
            .await
            .expect("a user");
        }
        for branch in [B1, B2] {
            sqlx::query(
                "INSERT INTO branches (id, name, organization_id, timezone) \
                 VALUES ($1, $1, $2::uuid, 'Asia/Bangkok')",
            )
            .bind(branch)
            .bind(ORG)
            .execute(&room.pool)
            .await
            .expect("a branch");
            sqlx::query(
                "INSERT INTO branch_versions (branch_id, version, organization_id, changed_at) \
                 VALUES ($1, 1, $2::uuid, now()) ON CONFLICT (branch_id) DO NOTHING",
            )
            .bind(branch)
            .bind(ORG)
            .execute(&room.pool)
            .await
            .expect("a branch version");
            sqlx::query(
                "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, organization_id, teller_id) \
                 VALUES ('SH-' || $1, $1, 'OPEN', now(), 'test', 'open-' || $1, $2::uuid, $3::uuid)",
            )
            .bind(branch)
            .bind(ORG)
            .bind(user_id(COUNTER))
            .execute(&room.pool)
            .await
            .expect("an open shift");
        }
        room
    }

    async fn denom_id(&self, currency: &str, face_value: Decimal) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2",
        )
        .bind(currency)
        .bind(face_value)
        .fetch_one(&self.pool)
        .await
        .expect("denomination exists")
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_count(
        &self,
        count_id: Uuid,
        branch: &str,
        kind: &str,
        counted_by: &str,
        counted_at: DateTime<Utc>,
        verified_by: Option<&str>,
        verified_at: Option<DateTime<Utc>>,
    ) {
        sqlx::query(
            "INSERT INTO cash_counts (id, organization_id, branch_id, shift_id, kind, counted_by, counted_at, verified_by, verified_at, idempotency_key) \
             VALUES ($1, $2::uuid, $3, 'SH-' || $3, $4, $5::uuid, $6, $7, $8, $9)",
        )
        .bind(count_id)
        .bind(ORG)
        .bind(branch)
        .bind(kind)
        .bind(counted_by)
        .bind(counted_at)
        .bind(verified_by.map(|v| Uuid::parse_str(v).expect("uuid")))
        .bind(verified_at)
        .bind(unique("count-idem"))
        .execute(&self.pool)
        .await
        .expect("insert count");
    }

    async fn insert_count_total(
        &self,
        count_id: Uuid,
        currency: &str,
        counted: Decimal,
        expected: Option<Decimal>,
        difference: Option<Decimal>,
    ) {
        sqlx::query(
            "INSERT INTO cash_count_totals (count_id, organization_id, currency, counted, expected, difference) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6)",
        )
        .bind(count_id)
        .bind(ORG)
        .bind(currency)
        .bind(counted)
        .bind(expected)
        .bind(difference)
        .execute(&self.pool)
        .await
        .expect("insert count total");
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_transfer(
        &self,
        transfer_id: Uuid,
        kind: &str,
        from_branch: &str,
        to_branch: Option<&str>,
        status: &str,
        created_by: &str,
        created_at: DateTime<Utc>,
        dispatched_at: Option<DateTime<Utc>>,
        received_at: Option<DateTime<Utc>>,
    ) {
        sqlx::query(
            "INSERT INTO cash_transfers (id, organization_id, kind, from_branch_id, to_branch_id, status, created_by, idempotency_key, created_at, dispatched_at, received_at) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6, $7::uuid, $8, $9, $10, $11)",
        )
        .bind(transfer_id)
        .bind(ORG)
        .bind(kind)
        .bind(from_branch)
        .bind(to_branch)
        .bind(status)
        .bind(created_by)
        .bind(unique("xfer-idem"))
        .bind(created_at)
        .bind(dispatched_at)
        .bind(received_at)
        .execute(&self.pool)
        .await
        .expect("insert transfer");
    }

    async fn insert_transfer_line(
        &self,
        transfer_id: Uuid,
        currency: &str,
        denomination_id: i64,
        qty_sent: i32,
        qty_received: Option<i32>,
    ) {
        sqlx::query(
            "INSERT INTO cash_transfer_lines (transfer_id, organization_id, currency, denomination_id, qty_sent, qty_received) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6)",
        )
        .bind(transfer_id)
        .bind(ORG)
        .bind(currency)
        .bind(denomination_id)
        .bind(qty_sent)
        .bind(qty_received)
        .execute(&self.pool)
        .await
        .expect("insert transfer line");
    }

    async fn insert_reversal_request(
        &self,
        id: Uuid,
        branch_id: &str,
        transaction_id: &str,
        requested_by: &str,
        requested_at: DateTime<Utc>,
        status: &str,
    ) {
        sqlx::query(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, proposed_payload, requested_by, requested_at, status, idempotency_key) \
             VALUES ($1, 'TRANSACTION_REVERSAL', $2, $3, 'hash', 'v1', '{}'::jsonb, $4, $5, $6, $7)",
        )
        .bind(id)
        .bind(branch_id)
        .bind(transaction_id)
        .bind(requested_by)
        .bind(requested_at)
        .bind(status)
        .bind(unique("rev-idem"))
        .execute(&self.pool)
        .await
        .expect("insert approval request");
    }
}

// ------------------------------------------------------------ Row 1
// two counts verified that day, one unverified -> two CountRows; other-branch excluded (F2)

#[actix_web::test]
async fn counts_lists_verified_counts_of_the_day_omitting_unverified() {
    let room = Room::start().await;
    let parts = SheetParts::new(room.pool.clone(), Arc::new(Reconcile::standard()));

    let count_1 = Uuid::new_v4();
    let count_2 = Uuid::new_v4();
    let count_3 = Uuid::new_v4();
    let count_other_branch = Uuid::new_v4();

    // Two verified counts on DAY for B1
    room.insert_count(
        count_1,
        B1,
        "SAFE",
        &user_id(COUNTER),
        bangkok(DAY, 9, 0),
        Some(&user_id(VERIFIER)),
        Some(bangkok(DAY, 10, 0)),
    )
    .await;
    room.insert_count_total(
        count_1,
        "THB",
        Decimal::new(100000, 2),
        Some(Decimal::new(100000, 2)),
        Some(Decimal::ZERO),
    )
    .await;

    room.insert_count(
        count_2,
        B1,
        "CLOSE",
        &user_id(COUNTER),
        bangkok(DAY, 17, 0),
        Some(&user_id(VERIFIER)),
        Some(bangkok(DAY, 18, 0)),
    )
    .await;
    room.insert_count_total(
        count_2,
        "USD",
        Decimal::new(5000, 2),
        Some(Decimal::new(5000, 2)),
        Some(Decimal::ZERO),
    )
    .await;

    // One unverified count on DAY for B1
    room.insert_count(
        count_3,
        B1,
        "OPEN",
        &user_id(COUNTER),
        bangkok(DAY, 8, 0),
        None,
        None,
    )
    .await;
    room.insert_count_total(count_3, "THB", Decimal::new(50000, 2), None, None)
        .await;

    // One verified count on DAY for OTHER branch B2 (kills mutant F2)
    room.insert_count(
        count_other_branch,
        B2,
        "SAFE",
        &user_id(COUNTER),
        bangkok(DAY, 10, 0),
        Some(&user_id(VERIFIER)),
        Some(bangkok(DAY, 11, 0)),
    )
    .await;
    room.insert_count_total(
        count_other_branch,
        "THB",
        Decimal::new(300000, 2),
        Some(Decimal::new(300000, 2)),
        Some(Decimal::ZERO),
    )
    .await;

    let res = parts
        .counts(&BranchId::new(B1), date(DAY))
        .await
        .expect("counts query");

    assert!(res.available, "counts part must be available");
    let rows = res.data.expect("counts data");
    assert_eq!(
        rows.len(),
        2,
        "must contain exactly two verified counts of B1"
    );

    let ids: Vec<&str> = rows.iter().map(|r| r.count_id.as_str()).collect();
    let c1_str = count_1.to_string();
    let c2_str = count_2.to_string();
    let c3_str = count_3.to_string();
    let c_other_str = count_other_branch.to_string();
    assert!(ids.contains(&c1_str.as_str()), "count_1 must be present");
    assert!(ids.contains(&c2_str.as_str()), "count_2 must be present");
    assert!(
        !ids.contains(&c3_str.as_str()),
        "unverified count_3 must be omitted"
    );
    assert!(
        !ids.contains(&c_other_str.as_str()),
        "other branch count must be omitted (kills F2)"
    );

    let row1 = rows.iter().find(|r| r.count_id == c1_str).unwrap();
    assert_eq!(row1.kind, "SAFE");
    assert_eq!(row1.counted_by, user_id(COUNTER));
    assert_eq!(
        row1.verified_by.as_deref(),
        Some(user_id(VERIFIER).as_str())
    );
    assert_eq!(row1.lines.len(), 1);
    assert_eq!(row1.lines[0].currency, "THB");
    assert_eq!(row1.lines[0].counted, "1000.00");
    assert_eq!(row1.lines[0].expected, "1000.00");
    assert_eq!(row1.lines[0].difference, "0.00");
}

// ------------------------------------------------------------ Row 2
// count day by counted_at's business date (timezone); count verified on DAY but counted before excluded (F1)

#[actix_web::test]
async fn counts_verified_at_late_branch_time_belongs_to_that_business_date() {
    let room = Room::start().await;
    let parts = SheetParts::new(room.pool.clone(), Arc::new(Reconcile::standard()));

    let late_count = Uuid::new_v4();
    let next_day_count = Uuid::new_v4();
    let counted_yesterday_count = Uuid::new_v4();

    // 23:30 Bangkok on DAY: counted_at 23:00 Bangkok on DAY
    room.insert_count(
        late_count,
        B1,
        "CLOSE",
        &user_id(COUNTER),
        bangkok(DAY, 23, 0),
        Some(&user_id(VERIFIER)),
        Some(bangkok(DAY, 23, 30)),
    )
    .await;
    room.insert_count_total(
        late_count,
        "THB",
        Decimal::new(100000, 2),
        Some(Decimal::new(100000, 2)),
        Some(Decimal::ZERO),
    )
    .await;

    // Next day: counted_at 00:15 Bangkok on next day (2026-10-06)
    let next_day = "2026-10-06";
    room.insert_count(
        next_day_count,
        B1,
        "OPEN",
        &user_id(COUNTER),
        bangkok(next_day, 0, 15),
        Some(&user_id(VERIFIER)),
        Some(bangkok(next_day, 0, 30)),
    )
    .await;
    room.insert_count_total(
        next_day_count,
        "THB",
        Decimal::new(100000, 2),
        Some(Decimal::new(100000, 2)),
        Some(Decimal::ZERO),
    )
    .await;

    // Counted on day_before at 23:50 Bangkok, verified on DAY at 00:10 Bangkok (Ruling 1, kills mutant F1)
    let day_before = "2026-10-04";
    room.insert_count(
        counted_yesterday_count,
        B1,
        "CLOSE",
        &user_id(COUNTER),
        bangkok(day_before, 23, 50),
        Some(&user_id(VERIFIER)),
        Some(bangkok(DAY, 0, 10)),
    )
    .await;
    room.insert_count_total(
        counted_yesterday_count,
        "THB",
        Decimal::new(150000, 2),
        Some(Decimal::new(150000, 2)),
        Some(Decimal::ZERO),
    )
    .await;

    // Query DAY: late_count (counted on DAY) must be on DAY;
    // counted_yesterday_count must NOT be on DAY (it was counted on day_before, kills F1)
    let res_day = parts
        .counts(&BranchId::new(B1), date(DAY))
        .await
        .expect("counts on DAY");
    let rows_day = res_day.data.expect("data on DAY");
    let day_ids: Vec<&str> = rows_day.iter().map(|r| r.count_id.as_str()).collect();
    let late_str = late_count.to_string();
    let next_str = next_day_count.to_string();
    let yesterday_str = counted_yesterday_count.to_string();

    assert!(
        day_ids.contains(&late_str.as_str()),
        "late count (counted on DAY) must belong to DAY in branch timezone"
    );
    assert!(
        !day_ids.contains(&next_str.as_str()),
        "next day count must not be on DAY"
    );
    assert!(
        !day_ids.contains(&yesterday_str.as_str()),
        "count counted on day_before must not be on DAY even though verified on DAY (kills F1)"
    );

    // Query NEXT DAY: next_day_count must be on next_day, and late_count must not
    let res_next = parts
        .counts(&BranchId::new(B1), date(next_day))
        .await
        .expect("counts on next_day");
    let rows_next = res_next.data.expect("data on next_day");
    let next_ids: Vec<&str> = rows_next.iter().map(|r| r.count_id.as_str()).collect();
    assert!(
        next_ids.contains(&next_str.as_str()),
        "next day count must belong to next_day"
    );
    assert!(
        !next_ids.contains(&late_str.as_str()),
        "late count must not belong to next_day"
    );

    // Query DAY BEFORE: counted_yesterday_count must be on day_before
    let res_before = parts
        .counts(&BranchId::new(B1), date(day_before))
        .await
        .expect("counts on day_before");
    let rows_before = res_before.data.expect("data on day_before");
    let before_ids: Vec<&str> = rows_before.iter().map(|r| r.count_id.as_str()).collect();
    assert!(
        before_ids.contains(&yesterday_str.as_str()),
        "counted_yesterday_count must belong to day_before"
    );
}

// ------------------------------------------------------------ Row 3
// a transfer sent 100 USD, received 80 USD -> one short receipt, sent 100 and received 80 (face != 1, kills F3 & F4)

#[actix_web::test]
async fn transfers_short_receipt_reports_sent_and_received_amounts() {
    let room = Room::start().await;
    let parts = SheetParts::new(room.pool.clone(), Arc::new(Reconcile::standard()));

    let short_xfer = Uuid::new_v4();
    let full_xfer = Uuid::new_v4();
    let older_short_xfer = Uuid::new_v4();

    // Use 20 USD note (face_value = 20): sent 5 notes (= 100 USD), received 4 notes (= 80 USD) (kills F3)
    let usd_20_denom = room.denom_id("USD", Decimal::from(20)).await;

    let day_before = "2026-10-04";

    // Short transfer: created and dispatched the day BEFORE, received short on DAY (Ruling 2a, kills G1)
    room.insert_transfer(
        short_xfer,
        "BRANCH",
        B2,
        Some(B1),
        "RECEIVED",
        &user_id(COUNTER),
        bangkok(day_before, 14, 0),
        Some(bangkok(day_before, 15, 0)),
        Some(bangkok(DAY, 10, 0)),
    )
    .await;
    room.insert_transfer_line(short_xfer, "USD", usd_20_denom, 5, Some(4))
        .await;

    // Full transfer on DAY: sent 2 notes (40 USD), received 2 notes (40 USD) (not short)
    room.insert_transfer(
        full_xfer,
        "BRANCH",
        B2,
        Some(B1),
        "RECEIVED",
        &user_id(COUNTER),
        bangkok(DAY, 9, 0),
        Some(bangkok(DAY, 10, 0)),
        Some(bangkok(DAY, 11, 0)),
    )
    .await;
    room.insert_transfer_line(full_xfer, "USD", usd_20_denom, 2, Some(2))
        .await;

    // Short transfer on the DAY BEFORE: wholly on day_before, must NOT appear on DAY (kills F4)
    room.insert_transfer(
        older_short_xfer,
        "BRANCH",
        B2,
        Some(B1),
        "RECEIVED",
        &user_id(COUNTER),
        bangkok(day_before, 8, 0),
        Some(bangkok(day_before, 9, 0)),
        Some(bangkok(day_before, 10, 0)),
    )
    .await;
    room.insert_transfer_line(older_short_xfer, "USD", usd_20_denom, 5, Some(3))
        .await;

    let res = parts
        .transfers(&BranchId::new(B1), date(DAY))
        .await
        .expect("transfers query");

    assert!(res.available, "transfers part must be available");
    let summary = res.data.expect("transfers data");

    assert_eq!(
        summary.short_receipts.len(),
        1,
        "must report exactly one short receipt for DAY (kills F4 & G1)"
    );
    let row = &summary.short_receipts[0];
    assert_eq!(row.transfer_id, short_xfer.to_string());
    assert_eq!(row.currency, "USD");
    assert_eq!(
        row.sent, "100.00",
        "sent amount must be face * qty_sent (100.00, not 5; kills F3)"
    );
    assert_eq!(
        row.received, "80.00",
        "received amount must be face * qty_received (80.00, not 4; kills F3)"
    );

    // by_status counts transfers that touch B1 on DAY (Ruling 2a, kills G1)
    let received = summary
        .by_status
        .iter()
        .find(|s| s.status == "RECEIVED")
        .expect("RECEIVED status present");
    assert_eq!(
        received.count, 2,
        "must count both transfers received on DAY in by_status, even when created/dispatched before (kills G1)"
    );
}

// ------------------------------------------------------------ Row 4
// a transfer DISPATCHED, not received -> in transit, with its quantity; older still in transit included (Ruling 2)

#[actix_web::test]
async fn transfers_dispatched_and_not_received_reports_in_transit_quantity() {
    let room = Room::start().await;
    let parts = SheetParts::new(room.pool.clone(), Arc::new(Reconcile::standard()));

    let in_transit_today = Uuid::new_v4();
    let in_transit_yesterday = Uuid::new_v4();
    let in_transit_tomorrow = Uuid::new_v4();
    let received_today = Uuid::new_v4();
    let older_received = Uuid::new_v4();

    let usd_20_denom = room.denom_id("USD", Decimal::from(20)).await;
    let usd_50_denom = room.denom_id("USD", Decimal::from(50)).await;

    // DISPATCHED on DAY and not received: 3 notes of 20 USD = 60.00 USD
    room.insert_transfer(
        in_transit_today,
        "BRANCH",
        B1,
        Some(B2),
        "DISPATCHED",
        &user_id(COUNTER),
        bangkok(DAY, 13, 0),
        Some(bangkok(DAY, 14, 0)),
        None,
    )
    .await;
    room.insert_transfer_line(in_transit_today, "USD", usd_20_denom, 3, None)
        .await;

    // DISPATCHED the DAY BEFORE and still in transit (Ruling 2b): 1 note of 50 USD = 50.00 USD
    let day_before = "2026-10-04";
    room.insert_transfer(
        in_transit_yesterday,
        "BRANCH",
        B1,
        Some(B2),
        "DISPATCHED",
        &user_id(COUNTER),
        bangkok(day_before, 15, 0),
        Some(bangkok(day_before, 16, 0)),
        None,
    )
    .await;
    room.insert_transfer_line(in_transit_yesterday, "USD", usd_50_denom, 1, None)
        .await;

    // DISPATCHED the day AFTER DAY: must NOT appear in DAY's in_transit (kills G2)
    let next_day = "2026-10-06";
    room.insert_transfer(
        in_transit_tomorrow,
        "BRANCH",
        B1,
        Some(B2),
        "DISPATCHED",
        &user_id(COUNTER),
        bangkok(next_day, 10, 0),
        Some(bangkok(next_day, 11, 0)),
        None,
    )
    .await;
    room.insert_transfer_line(in_transit_tomorrow, "USD", usd_20_denom, 2, None)
        .await;

    // RECEIVED on DAY: not in transit
    room.insert_transfer(
        received_today,
        "BRANCH",
        B1,
        Some(B2),
        "RECEIVED",
        &user_id(COUNTER),
        bangkok(DAY, 9, 0),
        Some(bangkok(DAY, 9, 30)),
        Some(bangkok(DAY, 10, 0)),
    )
    .await;
    room.insert_transfer_line(received_today, "USD", usd_50_denom, 1, Some(1))
        .await;

    // RECEIVED on DAY BEFORE: neither in transit nor touching DAY (kills F4)
    room.insert_transfer(
        older_received,
        "BRANCH",
        B1,
        Some(B2),
        "RECEIVED",
        &user_id(COUNTER),
        bangkok(day_before, 9, 0),
        Some(bangkok(day_before, 9, 30)),
        Some(bangkok(day_before, 10, 0)),
    )
    .await;
    room.insert_transfer_line(older_received, "USD", usd_50_denom, 1, Some(1))
        .await;

    let res = parts
        .transfers(&BranchId::new(B1), date(DAY))
        .await
        .expect("transfers query");

    assert!(res.available, "transfers part must be available");
    let summary = res.data.expect("transfers data");

    // In-transit list must include both today's and yesterday's still-dispatched transfers (Ruling 2b, kills G2)
    assert_eq!(
        summary.in_transit.len(),
        2,
        "must report transfers currently in transit dispatched on or before DAY (Ruling 2b, kills G2)"
    );

    let t_today_str = in_transit_today.to_string();
    let t_yest_str = in_transit_yesterday.to_string();
    let t_tomorrow_str = in_transit_tomorrow.to_string();
    let in_transit_ids: Vec<&str> = summary
        .in_transit
        .iter()
        .map(|r| r.transfer_id.as_str())
        .collect();
    assert!(in_transit_ids.contains(&t_today_str.as_str()));
    assert!(in_transit_ids.contains(&t_yest_str.as_str()));
    assert!(
        !in_transit_ids.contains(&t_tomorrow_str.as_str()),
        "transfer dispatched after DAY must not appear in in_transit (upper bound, kills G2)"
    );

    let row_today = summary
        .in_transit
        .iter()
        .find(|r| r.transfer_id == t_today_str)
        .unwrap();
    assert_eq!(row_today.currency, "USD");
    assert_eq!(row_today.quantity, "60.00");

    let row_yest = summary
        .in_transit
        .iter()
        .find(|r| r.transfer_id == t_yest_str)
        .unwrap();
    assert_eq!(row_yest.currency, "USD");
    assert_eq!(row_yest.quantity, "50.00");

    // by_status counts transfers that touch B1 on DAY (created, dispatched, or received on DAY; Ruling 2a)
    let dispatched = summary
        .by_status
        .iter()
        .find(|s| s.status == "DISPATCHED")
        .expect("DISPATCHED status present");
    assert_eq!(dispatched.count, 1, "only 1 transfer was dispatched on DAY");

    let received = summary
        .by_status
        .iter()
        .find(|s| s.status == "RECEIVED")
        .expect("RECEIVED status present");
    assert_eq!(
        received.count, 1,
        "only 1 transfer was received on DAY; older received transfer excluded"
    );
}

// ------------------------------------------------------------ Row 5
// one hand-broken ledger row (LG-7 check 2) -> integrity row 2 ok: false, offenders: 1; other branch isolated (F5)

#[actix_web::test]
async fn integrity_reports_offenders_when_ledger_check_fails() {
    let room = Room::start().await;
    let parts = SheetParts::new(room.pool.clone(), Arc::new(Reconcile::standard()));

    // Seed broken inventory_positions row for B1: Check 2 fails with 1 offender
    sqlx::query(
        "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb, version) \
         VALUES ($1::uuid, $2, 'USD', 0::numeric, 3200::numeric, 1)",
    )
    .bind(ORG)
    .bind(B1)
    .execute(&room.pool)
    .await
    .expect("broken inventory position for B1");

    // Also seed a broken inventory_positions row for OTHER branch B2 (kills mutant F5)
    sqlx::query(
        "INSERT INTO inventory_positions (organization_id, branch_id, currency, quantity, cost_thb, version) \
         VALUES ($1::uuid, $2, 'USD', 0::numeric, 5000::numeric, 1)",
    )
    .bind(ORG)
    .bind(B2)
    .execute(&room.pool)
    .await
    .expect("broken inventory position for B2");

    let res = parts
        .integrity(&BranchId::new(B1), date(DAY))
        .await
        .expect("integrity query");

    assert!(res.available, "integrity part must be available");
    let rows = res.data.expect("integrity data");
    assert_eq!(rows.len(), 5, "must contain exactly 5 reconcile checks");

    let check2 = rows
        .iter()
        .find(|r| r.check == 2)
        .expect("check 2 must be present");
    assert!(!check2.ok, "check 2 must fail");
    assert_eq!(
        check2.offenders, 1,
        "check 2 must have exactly 1 offender for B1; B2 must be excluded (kills F5)"
    );

    for check in rows.iter().filter(|r| r.check != 2) {
        assert!(check.ok, "check {} must pass", check.check);
        assert_eq!(
            check.offenders, 0,
            "check {} must have 0 offenders",
            check.check
        );
    }
}

// ------------------------------------------------------------ Row 6
// two PENDING requests, entered newest first -> returned oldest first (Ruling 4 / Must-fix 5, kills F6)

#[actix_web::test]
async fn reversals_pending_lists_pending_requests_oldest_first() {
    let room = Room::start().await;
    let parts = SheetParts::new(room.pool.clone(), Arc::new(Reconcile::standard()));

    let tx_id_older = "00000000-0000-0000-0000-000000000091";
    let tx_id_newer = "00000000-0000-0000-0000-000000000092";
    let pending_older = Uuid::new_v4();
    let pending_newer = Uuid::new_v4();
    let approved_req = Uuid::new_v4();
    let other_branch_req = Uuid::new_v4();

    // Insert NEWER request first (11:00)
    room.insert_reversal_request(
        pending_newer,
        B1,
        tx_id_newer,
        &user_id(COUNTER),
        bangkok(DAY, 11, 0),
        "PENDING",
    )
    .await;

    // Insert OLDER request second (09:00)
    room.insert_reversal_request(
        pending_older,
        B1,
        tx_id_older,
        &user_id(COUNTER),
        bangkok(DAY, 9, 0),
        "PENDING",
    )
    .await;

    // 1 APPROVED reversal request for B1 (should NOT be in pending)
    room.insert_reversal_request(
        approved_req,
        B1,
        "00000000-0000-0000-0000-000000000098",
        &user_id(COUNTER),
        bangkok(DAY, 8, 30),
        "APPROVED",
    )
    .await;

    // 1 PENDING reversal request for OTHER branch B2 (should NOT be in B1 pending)
    room.insert_reversal_request(
        other_branch_req,
        B2,
        "00000000-0000-0000-0000-000000000097",
        &user_id(COUNTER),
        bangkok(DAY, 10, 0),
        "PENDING",
    )
    .await;

    let res = parts
        .reversals_pending(&BranchId::new(B1), date(DAY))
        .await
        .expect("reversals_pending query");

    assert!(res.available, "reversals_pending part must be available");
    let rows = res.data.expect("reversals_pending data");
    assert_eq!(
        rows.len(),
        2,
        "must report exactly 2 pending reversal requests of B1"
    );

    // Oldest first: pending_older (09:00) must be first, pending_newer (11:00) second (kills F6)
    assert_eq!(
        rows[0].request_id,
        pending_older.to_string(),
        "first row must be the older request (kills F6)"
    );
    assert_eq!(rows[0].transaction_id, tx_id_older);
    assert_eq!(rows[0].requested_by, user_id(COUNTER));

    assert_eq!(
        rows[1].request_id,
        pending_newer.to_string(),
        "second row must be the newer request"
    );
    assert_eq!(rows[1].transaction_id, tx_id_newer);
    assert_eq!(rows[1].requested_by, user_id(COUNTER));
}

// ------------------------------------------------------------ Row 7
// a part changes after certifying -> the certification reads stale (FN-1 hash covers the parts)

#[actix_web::test]
async fn certifying_then_part_change_makes_certification_stale() {
    let room = Room::start().await;
    let parts = Arc::new(SheetParts::new(
        room.pool.clone(),
        Arc::new(Reconcile::standard()),
    ));
    let facts = Arc::new(WideTransactionFacts::new(room.pool.clone()));
    let service = ReconciliationService::new(room.pool.clone(), facts, parts);

    let count_1 = Uuid::new_v4();
    room.insert_count(
        count_1,
        B1,
        "SAFE",
        &user_id(COUNTER),
        bangkok(DAY, 9, 0),
        Some(&user_id(VERIFIER)),
        Some(bangkok(DAY, 10, 0)),
    )
    .await;
    room.insert_count_total(
        count_1,
        "THB",
        Decimal::new(100000, 2),
        Some(Decimal::new(100000, 2)),
        Some(Decimal::ZERO),
    )
    .await;

    let sheet_before = service
        .sheet(&BranchId::new(B1), date(DAY))
        .await
        .expect("read initial sheet");

    assert!(sheet_before.counts.available);
    let original_hash = sheet_before.figures_hash.clone();
    assert!(
        !original_hash.is_empty(),
        "sheet must produce a figures hash"
    );

    // Certify this initial state
    let certifier_uuid = Uuid::parse_str(&user_id(CERTIFIER)).expect("certifier uuid");
    let mut tx = room.pool.begin().await.expect("begin tx");
    let cert_res = service
        .certify_in(
            &mut tx,
            &BranchId::new(B1),
            date(DAY),
            certifier_uuid,
            &original_hash,
            Some("certified before change"),
            bangkok(DAY, 11, 0),
        )
        .await
        .expect("certify_in")
        .expect("not refused");
    tx.commit().await.expect("commit tx");
    assert_eq!(cert_res.figures_hash, original_hash);

    // Reading immediately: certification is NOT stale
    let sheet_certified = service
        .sheet(&BranchId::new(B1), date(DAY))
        .await
        .expect("read sheet after certify");
    assert_eq!(sheet_certified.certifications.len(), 1);
    assert!(
        !sheet_certified.certifications[0].stale,
        "certification must not be stale before any part changes"
    );

    // Now change one of the parts covered by the hash: add a second verified count
    let count_2 = Uuid::new_v4();
    room.insert_count(
        count_2,
        B1,
        "CLOSE",
        &user_id(COUNTER),
        bangkok(DAY, 17, 0),
        Some(&user_id(VERIFIER)),
        Some(bangkok(DAY, 18, 0)),
    )
    .await;
    room.insert_count_total(
        count_2,
        "THB",
        Decimal::new(200000, 2),
        Some(Decimal::new(200000, 2)),
        Some(Decimal::ZERO),
    )
    .await;

    // Read sheet again: the figures hash must have changed, and certification must now be stale
    let sheet_after = service
        .sheet(&BranchId::new(B1), date(DAY))
        .await
        .expect("read sheet after part change");

    assert_ne!(
        sheet_after.figures_hash, original_hash,
        "figures_hash must change when a part changes"
    );
    assert_eq!(sheet_after.certifications.len(), 1);
    assert!(
        sheet_after.certifications[0].stale,
        "certification must read stale after part changed"
    );
}

// ------------------------------------------------------------ TransferLine helpers (Ruling 3, kills G3)

#[test]
fn transfer_line_sent_and_received_minor_computations() {
    let line_received = TransferLine {
        currency: CurrencyCode::parse("USD").expect("valid currency"),
        denomination_id: 1,
        face_value: Money::parse("20.00").expect("valid money"),
        qty_sent: 5,
        qty_received: Some(4),
    };
    assert_eq!(line_received.sent_minor(), 10000);
    assert_eq!(line_received.received_minor(), 8000);

    let line_in_transit = TransferLine {
        currency: CurrencyCode::parse("USD").expect("valid currency"),
        denomination_id: 1,
        face_value: Money::parse("20.00").expect("valid money"),
        qty_sent: 5,
        qty_received: None,
    };
    assert_eq!(line_in_transit.sent_minor(), 10000);
    assert_eq!(line_in_transit.received_minor(), 0);
}
