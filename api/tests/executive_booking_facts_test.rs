//! Ticket #1190 (EX-8): up-front tests for `BookingFacts` (booking conversion KPI,
//! design `executive-finance.md` §3, §9).
//! Written before the implementation by the test-writer gemini/warut-2, on the BE lane controller
//! claude/ad05bf8d's order.
//!
//! Covers every row of #1190 §9 ("Edge cases"):
//! - A booking whose day is tomorrow is left out of both counts.
//! - A CANCELLED, NO_SHOW or EXPIRED booking that had been CONFIRMED counts in the denominator only.
//! - A booking that never reached CONFIRMED (PENDING_CUSTOMER_CONFIRMATION → EXPIRED) counts in neither.
//! - A rescheduled booking counts once, on its final service_date.
//! - Branch timezone at the midnight edge.
//! - DEMO excluded.
//!
//! Plus general and filter tests (period, empty, scope, currency).
//!
//! Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips. Each test works in a schema of its own.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{Duration as ChronoDuration, NaiveDate, NaiveTime};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_kernel::ids::BranchId;
use xc_kernel::money::CurrencyCode;
use xchange::application::executive::metrics::{BOOKING_CONVERSION, MetricUnavailable};
use xchange::application::executive::overview::{BranchHead, ExecutiveOverviewService};
use xchange::application::executive::ports::{BrokerNotConnected, ExchangeFacts, ExecutiveFilter};
use xchange::infrastructure::executive::alert_facts::PgAlertFacts;
use xchange::infrastructure::executive::booking_facts::BookingFacts;
use xchange::infrastructure::executive::ledger_cash_facts::LedgerCashFacts;
use xchange::infrastructure::executive::wide_transaction_facts::WideTransactionFacts;

const MUST_FINISH: Duration = Duration::from_secs(30);
const ORG_1: &str = "00000000-0000-0000-0000-000000000001";
const B1: &str = "XC-B01";
const B2: &str = "XC-B02";
const B3: &str = "XC-B03";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

struct Room {
    pool: PgPool,
    schema: String,
}

impl Room {
    async fn start() -> Self {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            panic!(
                "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip"
            )
        });
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("ex8_booking_facts");
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
            .max_connections(5)
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

        Room { pool, schema }
    }

    async fn finish(self) {
        let Room { pool, schema } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }

    async fn seed_org(&self, id: &str, code: &str, timezone: &str) {
        sqlx::query(
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ($1::uuid, $2, $3, now()) \
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(code)
        .bind(timezone)
        .execute(&self.pool)
        .await
        .expect("seed org");
    }

    async fn seed_branch(&self, org_id: &str, branch_id: &str, timezone: &str) {
        self.seed_org(org_id, &unique("ORG"), timezone).await;
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id, timezone) \
             VALUES ($1, $1, $2::uuid, $3) \
             ON CONFLICT (id) DO UPDATE SET organization_id = EXCLUDED.organization_id, timezone = EXCLUDED.timezone",
        )
        .bind(branch_id)
        .bind(org_id)
        .bind(timezone)
        .execute(&self.pool)
        .await
        .expect("seed branch");

        sqlx::query(
            "INSERT INTO branch_versions (branch_id, version, organization_id, changed_at) \
             VALUES ($1, 1, $2::uuid, now()) \
             ON CONFLICT (branch_id) DO NOTHING",
        )
        .bind(branch_id)
        .bind(org_id)
        .execute(&self.pool)
        .await
        .expect("seed branch version");
    }

    async fn seed_customer(&self, org_id: &str, id: &str, mode: &str) {
        sqlx::query(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, status, created_at) \
             VALUES ($1, $2::uuid, $1, 'Test Customer', $3, 'ACTIVE', now())",
        )
        .bind(id)
        .bind(org_id)
        .bind(mode)
        .execute(&self.pool)
        .await
        .expect("seed customer");
    }

    async fn seed_slot(
        &self,
        org_id: &str,
        branch_id: &str,
        service_date: NaiveDate,
        start_time: NaiveTime,
        duration_mins: i64,
    ) -> Uuid {
        let slot_id = Uuid::new_v4();
        let start_at = service_date.and_time(start_time).and_utc();
        let end_at = start_at + ChronoDuration::minutes(duration_mins);
        sqlx::query(
            "INSERT INTO booking_slots (id, organization_id, branch_id, service_date, start_at, end_at, capacity, reserved, status) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6, 10, 0, 'OPEN')",
        )
        .bind(slot_id)
        .bind(org_id)
        .bind(branch_id)
        .bind(service_date)
        .bind(start_at)
        .bind(end_at)
        .execute(&self.pool)
        .await
        .expect("seed slot");
        slot_id
    }

    async fn seed_transaction(
        &self,
        org_id: &str,
        branch_id: &str,
        customer_id: &str,
        mode: &str,
        value_date: NaiveDate,
    ) -> Uuid {
        let tx_id = Uuid::new_v4();
        let doc_ref = unique("DOC");
        let shift_id = if mode == "LIVE" {
            let existing_shift: Option<String> = sqlx::query_scalar(
                "SELECT id FROM shifts WHERE branch_id = $1 AND status = 'OPEN' LIMIT 1",
            )
            .bind(branch_id)
            .fetch_optional(&self.pool)
            .await
            .expect("check existing shift");

            if let Some(sid) = existing_shift {
                Some(sid)
            } else {
                let teller_id = Uuid::new_v4();
                sqlx::query(
                    "INSERT INTO users (id, organization_id, name, status, created_by) \
                     VALUES ($1, $2::uuid, 'Test Teller', 'ACTIVE', 'tester') \
                     ON CONFLICT (id) DO NOTHING",
                )
                .bind(teller_id)
                .bind(org_id)
                .execute(&self.pool)
                .await
                .expect("seed user");

                let sid = unique("SHIFT");
                sqlx::query(
                    "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
                     VALUES ($1, $2, 'OPEN', now(), 'tester', $1, $3)",
                )
                .bind(&sid)
                .bind(branch_id)
                .bind(teller_id)
                .execute(&self.pool)
                .await
                .expect("seed shift");
                Some(sid)
            }
        } else {
            None
        };

        sqlx::query(
            "INSERT INTO transactions (id, document_ref, branch_id, mode, shift_id, submission_id, payload_hash, \
                 customer_id, customer_name, nationality, type, code, amount, rate, principal, fee, \
                 bank_fee, received, paid, method, payment_ref, account, staff_id, booking_no, notes, \
                 value_date, paid_at, recorded_at, recorded_by, status, completed_at) \
             VALUES ($1, $2, $3, $4, $5, $6, 'hash', \
                 $7, 'Customer', 'THA', 'SELL', 'USD', 100, 32.5, 3250, 0, 0, 3250, 0, 'Cash', '', '', 'staff', '', '', \
                 $8, now(), now(), 'staff', 'COMPLETED', now())",
        )
        .bind(tx_id)
        .bind(&doc_ref)
        .bind(branch_id)
        .bind(mode)
        .bind(shift_id)
        .bind(unique("sub"))
        .bind(customer_id)
        .bind(value_date)
        .execute(&self.pool)
        .await
        .expect("seed transaction");

        tx_id
    }

    #[allow(clippy::too_many_arguments)]
    async fn seed_booking(
        &self,
        org_id: &str,
        branch_id: &str,
        customer_id: &str,
        slot_id: Uuid,
        currency: &str,
        status: &str,
        transaction_id: Option<Uuid>,
    ) -> String {
        let booking_id = format!("BK-{}", Uuid::new_v4());
        let booking_no = unique("B");
        let qr = Uuid::new_v4().as_u128();
        let qr_bytes = qr.to_be_bytes();

        sqlx::query(
            "INSERT INTO bookings (id, organization_id, branch_id, booking_no, customer_id, slot_id, shop_side, \
                 requested_currency, requested_quantity, source_channel, status, qr_token_hash, confirm_expires_at, \
                 transaction_id, created_by) \
             VALUES ($1, $2::uuid, $3, $4, $5, $6, 'BUY', $7, 100, 'APP', $8, $9, \
                 CASE WHEN $8 = 'PENDING_CUSTOMER_CONFIRMATION' THEN now() + interval '1 hour' ELSE NULL END, \
                 $10, 'tester')",
        )
        .bind(&booking_id)
        .bind(org_id)
        .bind(branch_id)
        .bind(&booking_no)
        .bind(customer_id)
        .bind(slot_id)
        .bind(currency)
        .bind(status)
        .bind(&qr_bytes[..])
        .bind(transaction_id)
        .execute(&self.pool)
        .await
        .expect("seed booking");

        booking_id
    }

    async fn seed_event(
        &self,
        org_id: &str,
        booking_id: &str,
        kind: &str,
        from_status: Option<&str>,
        to_status: &str,
    ) {
        sqlx::query(
            "INSERT INTO booking_events (id, organization_id, booking_id, kind, from_status, to_status, actor_id, at) \
             VALUES (gen_random_uuid(), $1::uuid, $2, $3, $4, $5, 'tester', now())",
        )
        .bind(org_id)
        .bind(booking_id)
        .bind(kind)
        .bind(from_status)
        .bind(to_status)
        .execute(&self.pool)
        .await
        .expect("seed booking event");
    }

    async fn reschedule_booking(&self, org_id: &str, booking_id: &str, to_slot: Uuid) {
        sqlx::query("UPDATE bookings SET slot_id = $1 WHERE id = $2")
            .bind(to_slot)
            .bind(booking_id)
            .execute(&self.pool)
            .await
            .expect("update booking slot");

        sqlx::query(
            "INSERT INTO booking_events (id, organization_id, booking_id, kind, from_status, to_status, actor_id, at, detail) \
             VALUES (gen_random_uuid(), $1::uuid, $2, 'RESCHEDULE', 'CONFIRMED', 'CONFIRMED', 'tester', now(), $3::jsonb)",
        )
        .bind(org_id)
        .bind(booking_id)
        .bind(serde_json::json!({ "toSlot": to_slot.to_string() }))
        .execute(&self.pool)
        .await
        .expect("seed reschedule event");
    }

    async fn branch_today(&self, branch_id: &str) -> NaiveDate {
        sqlx::query_scalar("SELECT (now() AT TIME ZONE timezone)::date FROM branches WHERE id = $1")
            .bind(branch_id)
            .fetch_one(&self.pool)
            .await
            .expect("branch today")
    }
}

// ── Edge cases (§9) ──────────────────────────────────────────────────────────

/// §9 row 1: "A booking whose day is tomorrow is left out of both counts."
#[tokio::test]
async fn test_tomorrow_is_excluded() {
    let room = Room::start().await;
    room.seed_branch(ORG_1, B1, "Asia/Bangkok").await;
    let today = room.branch_today(B1).await;
    let tomorrow = today + ChronoDuration::days(1);

    let customer = unique("CU");
    room.seed_customer(ORG_1, &customer, "LIVE").await;

    let slot = room
        .seed_slot(
            ORG_1,
            B1,
            tomorrow,
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            30,
        )
        .await;

    let booking = room
        .seed_booking(ORG_1, B1, &customer, slot, "USD", "CONFIRMED", None)
        .await;
    room.seed_event(ORG_1, &booking, "CREATE", None, "CONFIRMED")
        .await;

    let facts = BookingFacts::new(room.pool.clone());
    let scope = [BranchId::new(B1)];
    let filter = ExecutiveFilter {
        from: Some(today),
        to: Some(tomorrow),
        ..Default::default()
    };

    let conversion = facts
        .booking_conversion(&scope, &filter)
        .await
        .expect("booking conversion succeeds")
        .expect("available");

    assert_eq!(
        conversion.completed(),
        0,
        "tomorrow must be excluded from completed"
    );
    assert_eq!(
        conversion.confirmed(),
        0,
        "tomorrow must be excluded from confirmed"
    );
    assert_eq!(conversion.numerator(), 0);
    assert_eq!(conversion.denominator(), 0);
    assert_eq!(conversion.total, (0, 0));
    assert_eq!(
        conversion
            .by_branch
            .get(&BranchId::new(B1))
            .copied()
            .unwrap_or((0, 0)),
        (0, 0)
    );

    room.finish().await;
}

/// §9 row 2: "A CANCELLED, NO_SHOW or EXPIRED booking that had been CONFIRMED counts in the denominator only."
#[tokio::test]
async fn test_cancelled_no_show_or_expired_that_had_been_confirmed_counts_in_denominator_only() {
    let room = Room::start().await;
    room.seed_branch(ORG_1, B1, "Asia/Bangkok").await;
    let today = room.branch_today(B1).await;

    let c1 = unique("CU1");
    let c2 = unique("CU2");
    let c3 = unique("CU3");
    room.seed_customer(ORG_1, &c1, "LIVE").await;
    room.seed_customer(ORG_1, &c2, "LIVE").await;
    room.seed_customer(ORG_1, &c3, "LIVE").await;

    let s1 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            30,
        )
        .await;
    let s2 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            30,
        )
        .await;
    let s3 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(11, 0, 0).unwrap(),
            30,
        )
        .await;

    // Booking 1: CANCELLED, was CONFIRMED
    let b1 = room
        .seed_booking(ORG_1, B1, &c1, s1, "USD", "CANCELLED", None)
        .await;
    room.seed_event(ORG_1, &b1, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(ORG_1, &b1, "CANCEL", Some("CONFIRMED"), "CANCELLED")
        .await;

    // Booking 2: NO_SHOW, was CONFIRMED
    let b2 = room
        .seed_booking(ORG_1, B1, &c2, s2, "USD", "NO_SHOW", None)
        .await;
    room.seed_event(ORG_1, &b2, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(ORG_1, &b2, "NO_SHOW", Some("CONFIRMED"), "NO_SHOW")
        .await;

    // Booking 3: EXPIRED, was CONFIRMED
    let b3 = room
        .seed_booking(ORG_1, B1, &c3, s3, "USD", "EXPIRED", None)
        .await;
    room.seed_event(ORG_1, &b3, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(ORG_1, &b3, "EXPIRE", Some("CONFIRMED"), "EXPIRED")
        .await;

    let facts = BookingFacts::new(room.pool.clone());
    let scope = [BranchId::new(B1)];
    let filter = ExecutiveFilter {
        from: Some(today),
        to: Some(today),
        ..Default::default()
    };

    let conversion = facts
        .booking_conversion(&scope, &filter)
        .await
        .expect("booking conversion succeeds")
        .expect("available");

    assert_eq!(
        conversion.completed(),
        0,
        "cancelled/no_show/expired cannot count as completed"
    );
    assert_eq!(
        conversion.confirmed(),
        3,
        "all three that had reached CONFIRMED must count in denominator"
    );
    assert_eq!(conversion.numerator(), 0);
    assert_eq!(conversion.denominator(), 3);
    assert_eq!(conversion.total, (0, 3));
    assert_eq!(conversion.by_branch.get(&BranchId::new(B1)), Some(&(0, 3)));

    room.finish().await;
}

/// §9 row 3: "A booking that never reached CONFIRMED (PENDING_CUSTOMER_CONFIRMATION → EXPIRED) counts in neither."
#[tokio::test]
async fn test_never_reached_confirmed_counts_in_neither() {
    let room = Room::start().await;
    room.seed_branch(ORG_1, B1, "Asia/Bangkok").await;
    let today = room.branch_today(B1).await;

    let c1 = unique("CU1");
    let c2 = unique("CU2");
    let c3 = unique("CU3");
    room.seed_customer(ORG_1, &c1, "LIVE").await;
    room.seed_customer(ORG_1, &c2, "LIVE").await;
    room.seed_customer(ORG_1, &c3, "LIVE").await;

    let s1 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            30,
        )
        .await;
    let s2 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            30,
        )
        .await;
    let s3 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(11, 0, 0).unwrap(),
            30,
        )
        .await;

    // Booking 1: PENDING_CUSTOMER_CONFIRMATION → EXPIRED
    let b1 = room
        .seed_booking(ORG_1, B1, &c1, s1, "USD", "EXPIRED", None)
        .await;
    room.seed_event(ORG_1, &b1, "CREATE", None, "PENDING_CUSTOMER_CONFIRMATION")
        .await;
    room.seed_event(
        ORG_1,
        &b1,
        "EXPIRE",
        Some("PENDING_CUSTOMER_CONFIRMATION"),
        "EXPIRED",
    )
    .await;

    // Booking 2: PENDING_CUSTOMER_CONFIRMATION → CANCELLED
    let b2 = room
        .seed_booking(ORG_1, B1, &c2, s2, "USD", "CANCELLED", None)
        .await;
    room.seed_event(ORG_1, &b2, "CREATE", None, "PENDING_CUSTOMER_CONFIRMATION")
        .await;
    room.seed_event(
        ORG_1,
        &b2,
        "CANCEL",
        Some("PENDING_CUSTOMER_CONFIRMATION"),
        "CANCELLED",
    )
    .await;

    // Booking 3: still PENDING_CUSTOMER_CONFIRMATION
    let b3 = room
        .seed_booking(
            ORG_1,
            B1,
            &c3,
            s3,
            "USD",
            "PENDING_CUSTOMER_CONFIRMATION",
            None,
        )
        .await;
    room.seed_event(ORG_1, &b3, "CREATE", None, "PENDING_CUSTOMER_CONFIRMATION")
        .await;

    let facts = BookingFacts::new(room.pool.clone());
    let scope = [BranchId::new(B1)];
    let filter = ExecutiveFilter {
        from: Some(today),
        to: Some(today),
        ..Default::default()
    };

    let conversion = facts
        .booking_conversion(&scope, &filter)
        .await
        .expect("booking conversion succeeds")
        .expect("available");

    assert_eq!(
        conversion.completed(),
        0,
        "unconfirmed bookings cannot count in completed"
    );
    assert_eq!(
        conversion.confirmed(),
        0,
        "unconfirmed bookings cannot count in confirmed"
    );
    assert_eq!(conversion.numerator(), 0);
    assert_eq!(conversion.denominator(), 0);
    assert_eq!(conversion.total, (0, 0));
    assert_eq!(conversion.by_branch.get(&BranchId::new(B1)), Some(&(0, 0)));

    room.finish().await;
}

/// §9 row 4: "A rescheduled booking counts once, on its final service_date."
#[tokio::test]
async fn test_rescheduled_booking_counts_once_on_final_service_date() {
    let room = Room::start().await;
    room.seed_branch(ORG_1, B1, "Asia/Bangkok").await;
    let today = room.branch_today(B1).await;
    let d1 = today - ChronoDuration::days(2);
    let d2 = today - ChronoDuration::days(1);

    let customer = unique("CU");
    room.seed_customer(ORG_1, &customer, "LIVE").await;

    let s1 = room
        .seed_slot(ORG_1, B1, d1, NaiveTime::from_hms_opt(9, 0, 0).unwrap(), 30)
        .await;
    let s2 = room
        .seed_slot(ORG_1, B1, d2, NaiveTime::from_hms_opt(9, 0, 0).unwrap(), 30)
        .await;

    let tx = room
        .seed_transaction(ORG_1, B1, &customer, "LIVE", d2)
        .await;

    // Booking initially on slot s1, then rescheduled to s2, and COMPLETED
    let b = room
        .seed_booking(ORG_1, B1, &customer, s1, "USD", "COMPLETED", Some(tx))
        .await;
    room.seed_event(ORG_1, &b, "CREATE", None, "CONFIRMED")
        .await;
    room.reschedule_booking(ORG_1, &b, s2).await;
    room.seed_event(ORG_1, &b, "COMPLETE", Some("IN_SERVICE"), "COMPLETED")
        .await;

    let facts = BookingFacts::new(room.pool.clone());
    let scope = [BranchId::new(B1)];

    // Query on original date d1: should not count
    let filter_d1 = ExecutiveFilter {
        from: Some(d1),
        to: Some(d1),
        ..Default::default()
    };
    let conv_d1 = facts
        .booking_conversion(&scope, &filter_d1)
        .await
        .expect("query d1 succeeds")
        .expect("available");
    assert_eq!(
        conv_d1.completed(),
        0,
        "rescheduled booking should not count on original date d1"
    );
    assert_eq!(
        conv_d1.confirmed(),
        0,
        "rescheduled booking should not count on original date d1"
    );
    assert_eq!(conv_d1.total, (0, 0));

    // Query on final date d2: counts once
    let filter_d2 = ExecutiveFilter {
        from: Some(d2),
        to: Some(d2),
        ..Default::default()
    };
    let conv_d2 = facts
        .booking_conversion(&scope, &filter_d2)
        .await
        .expect("query d2 succeeds")
        .expect("available");
    assert_eq!(
        conv_d2.completed(),
        1,
        "rescheduled booking must count on final service_date d2"
    );
    assert_eq!(
        conv_d2.confirmed(),
        1,
        "rescheduled booking must count on final service_date d2"
    );
    assert_eq!(conv_d2.total, (1, 1));

    // Query across both dates d1..d2: counts once, not twice!
    let filter_both = ExecutiveFilter {
        from: Some(d1),
        to: Some(d2),
        ..Default::default()
    };
    let conv_both = facts
        .booking_conversion(&scope, &filter_both)
        .await
        .expect("query both succeeds")
        .expect("available");
    assert_eq!(
        conv_both.completed(),
        1,
        "rescheduled booking must count exactly once across both dates"
    );
    assert_eq!(
        conv_both.confirmed(),
        1,
        "rescheduled booking must count exactly once across both dates"
    );
    assert_eq!(conv_both.total, (1, 1));

    room.finish().await;
}

/// §9 row 5: "Branch timezone at the midnight edge."
///
/// Deterministic timezone straddle: Pacific/Kiritimati (UTC+14) and Etc/GMT+12 (UTC-12).
/// At any instant of every day, one is ahead of UTC and one is behind UTC.
/// Giving each branch a COMPLETED booking on its local today and a CONFIRMED booking on its
/// local tomorrow asserts that both branches' local today dates are correctly resolved and
/// their tomorrow bookings are excluded, yielding 2/2 in total.
#[tokio::test]
async fn test_branch_timezone_at_midnight_edge() {
    let room = Room::start().await;
    room.seed_branch(ORG_1, B1, "Pacific/Kiritimati").await;
    room.seed_branch(ORG_1, B2, "Etc/GMT+12").await;

    let today_b1 = room.branch_today(B1).await;
    let tomorrow_b1 = today_b1 + ChronoDuration::days(1);

    let today_b2 = room.branch_today(B2).await;
    let tomorrow_b2 = today_b2 + ChronoDuration::days(1);

    let customer_b1 = unique("CU_B1");
    let customer_b2 = unique("CU_B2");
    room.seed_customer(ORG_1, &customer_b1, "LIVE").await;
    room.seed_customer(ORG_1, &customer_b2, "LIVE").await;

    // B1: 1 COMPLETED on local today, 1 CONFIRMED on local tomorrow
    let s1_today = room
        .seed_slot(
            ORG_1,
            B1,
            today_b1,
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            30,
        )
        .await;
    let s1_tomorrow = room
        .seed_slot(
            ORG_1,
            B1,
            tomorrow_b1,
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            30,
        )
        .await;

    let tx_b1 = room
        .seed_transaction(ORG_1, B1, &customer_b1, "LIVE", today_b1)
        .await;

    let b1_today = room
        .seed_booking(
            ORG_1,
            B1,
            &customer_b1,
            s1_today,
            "USD",
            "COMPLETED",
            Some(tx_b1),
        )
        .await;
    room.seed_event(ORG_1, &b1_today, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(
        ORG_1,
        &b1_today,
        "COMPLETE",
        Some("IN_SERVICE"),
        "COMPLETED",
    )
    .await;

    let b1_tomorrow = room
        .seed_booking(
            ORG_1,
            B1,
            &customer_b1,
            s1_tomorrow,
            "USD",
            "CONFIRMED",
            None,
        )
        .await;
    room.seed_event(ORG_1, &b1_tomorrow, "CREATE", None, "CONFIRMED")
        .await;

    // B2: 1 COMPLETED on local today, 1 CONFIRMED on local tomorrow
    let s2_today = room
        .seed_slot(
            ORG_1,
            B2,
            today_b2,
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            30,
        )
        .await;
    let s2_tomorrow = room
        .seed_slot(
            ORG_1,
            B2,
            tomorrow_b2,
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            30,
        )
        .await;

    let tx_b2 = room
        .seed_transaction(ORG_1, B2, &customer_b2, "LIVE", today_b2)
        .await;

    let b2_today = room
        .seed_booking(
            ORG_1,
            B2,
            &customer_b2,
            s2_today,
            "USD",
            "COMPLETED",
            Some(tx_b2),
        )
        .await;
    room.seed_event(ORG_1, &b2_today, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(
        ORG_1,
        &b2_today,
        "COMPLETE",
        Some("IN_SERVICE"),
        "COMPLETED",
    )
    .await;

    let b2_tomorrow = room
        .seed_booking(
            ORG_1,
            B2,
            &customer_b2,
            s2_tomorrow,
            "USD",
            "CONFIRMED",
            None,
        )
        .await;
    room.seed_event(ORG_1, &b2_tomorrow, "CREATE", None, "CONFIRMED")
        .await;

    let facts = BookingFacts::new(room.pool.clone());
    let scope = [BranchId::new(B1), BranchId::new(B2)];

    // Query with default period (today in each branch's timezone)
    let filter_today = ExecutiveFilter {
        from: None,
        to: None,
        ..Default::default()
    };
    let conv = facts
        .booking_conversion(&scope, &filter_today)
        .await
        .expect("conversion succeeds")
        .expect("available");

    assert_eq!(
        conv.completed(),
        2,
        "both branches' local today bookings must be included in completed"
    );
    assert_eq!(
        conv.confirmed(),
        2,
        "both branches' local today bookings must be included in confirmed, tomorrow excluded"
    );
    assert_eq!(conv.numerator(), 2);
    assert_eq!(conv.denominator(), 2);
    assert_eq!(conv.by_branch.get(&BranchId::new(B1)), Some(&(1, 1)));
    assert_eq!(conv.by_branch.get(&BranchId::new(B2)), Some(&(1, 1)));

    // Second query with an explicit period spanning both branches' "tomorrow" (Must-fix 1).
    // The "not after the branch's today" bound alone decides to exclude tomorrow bookings at every hour.
    let filter_spanning = ExecutiveFilter {
        from: Some(today_b2 - ChronoDuration::days(1)),
        to: Some(tomorrow_b1 + ChronoDuration::days(1)),
        ..Default::default()
    };
    let conv_spanning = facts
        .booking_conversion(&scope, &filter_spanning)
        .await
        .expect("conversion succeeds")
        .expect("available");

    assert_eq!(
        conv_spanning.completed(),
        2,
        "both branches' local today bookings must be included in completed with spanning filter"
    );
    assert_eq!(
        conv_spanning.confirmed(),
        2,
        "both branches' local today bookings must be included in confirmed with spanning filter, tomorrow excluded"
    );
    assert_eq!(conv_spanning.numerator(), 2);
    assert_eq!(conv_spanning.denominator(), 2);
    assert_eq!(conv_spanning.total, (2, 2));
    assert_eq!(
        conv_spanning.by_branch.get(&BranchId::new(B1)),
        Some(&(1, 1))
    );
    assert_eq!(
        conv_spanning.by_branch.get(&BranchId::new(B2)),
        Some(&(1, 1))
    );

    room.finish().await;
}

/// §9 row 6: "DEMO excluded."
#[tokio::test]
async fn test_demo_excluded() {
    let room = Room::start().await;
    room.seed_branch(ORG_1, B1, "Asia/Bangkok").await;
    let today = room.branch_today(B1).await;

    let c_demo = unique("CU_DEMO");
    let c_live = unique("CU_LIVE");
    room.seed_customer(ORG_1, &c_demo, "DEMO").await;
    room.seed_customer(ORG_1, &c_live, "LIVE").await;

    let s1 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            30,
        )
        .await;
    let s2 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            30,
        )
        .await;
    let s3 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(11, 0, 0).unwrap(),
            30,
        )
        .await;

    let tx_demo = room
        .seed_transaction(ORG_1, B1, &c_demo, "DEMO", today)
        .await;
    let tx_live = room
        .seed_transaction(ORG_1, B1, &c_live, "LIVE", today)
        .await;

    // DEMO booking 1: completed with DEMO transaction
    let b_demo = room
        .seed_booking(ORG_1, B1, &c_demo, s1, "USD", "COMPLETED", Some(tx_demo))
        .await;
    room.seed_event(ORG_1, &b_demo, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(ORG_1, &b_demo, "COMPLETE", Some("IN_SERVICE"), "COMPLETED")
        .await;

    // DEMO booking 2: CONFIRMED with NO transaction (pins customers.mode exclusion, mutant M6)
    let b_demo_confirmed = room
        .seed_booking(ORG_1, B1, &c_demo, s3, "USD", "CONFIRMED", None)
        .await;
    room.seed_event(ORG_1, &b_demo_confirmed, "CREATE", None, "CONFIRMED")
        .await;

    // LIVE booking: completed
    let b_live = room
        .seed_booking(ORG_1, B1, &c_live, s2, "USD", "COMPLETED", Some(tx_live))
        .await;
    room.seed_event(ORG_1, &b_live, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(ORG_1, &b_live, "COMPLETE", Some("IN_SERVICE"), "COMPLETED")
        .await;

    let facts = BookingFacts::new(room.pool.clone());
    let scope = [BranchId::new(B1)];
    let filter = ExecutiveFilter {
        from: Some(today),
        to: Some(today),
        ..Default::default()
    };

    let conv = facts
        .booking_conversion(&scope, &filter)
        .await
        .expect("conversion succeeds")
        .expect("available");

    assert_eq!(
        conv.completed(),
        1,
        "DEMO customer booking must be excluded from completed"
    );
    assert_eq!(
        conv.confirmed(),
        1,
        "DEMO customer booking must be excluded from confirmed"
    );
    assert_eq!(conv.numerator(), 1);
    assert_eq!(conv.denominator(), 1);
    assert_eq!(conv.total, (1, 1));
    assert_eq!(conv.by_branch.get(&BranchId::new(B1)), Some(&(1, 1)));

    room.finish().await;
}

// ── General, Scope and Filter Tests ──────────────────────────────────────────

#[tokio::test]
async fn test_completed_and_confirmed_bookings_in_period() {
    let room = Room::start().await;
    room.seed_branch(ORG_1, B1, "Asia/Bangkok").await;
    let today = room.branch_today(B1).await;

    let c = unique("CU");
    room.seed_customer(ORG_1, &c, "LIVE").await;

    let s1 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            30,
        )
        .await;
    let s2 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            30,
        )
        .await;
    let s3 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(11, 0, 0).unwrap(),
            30,
        )
        .await;

    let tx = room.seed_transaction(ORG_1, B1, &c, "LIVE", today).await;

    // Booking 1: COMPLETED
    let b1 = room
        .seed_booking(ORG_1, B1, &c, s1, "USD", "COMPLETED", Some(tx))
        .await;
    room.seed_event(ORG_1, &b1, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(ORG_1, &b1, "COMPLETE", Some("IN_SERVICE"), "COMPLETED")
        .await;

    // Booking 2: IN_SERVICE
    let b2 = room
        .seed_booking(ORG_1, B1, &c, s2, "USD", "IN_SERVICE", None)
        .await;
    room.seed_event(ORG_1, &b2, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(ORG_1, &b2, "START", Some("CHECKED_IN"), "IN_SERVICE")
        .await;

    // Booking 3: CONFIRMED
    let b3 = room
        .seed_booking(ORG_1, B1, &c, s3, "USD", "CONFIRMED", None)
        .await;
    room.seed_event(ORG_1, &b3, "CREATE", None, "CONFIRMED")
        .await;

    let facts = BookingFacts::new(room.pool.clone());
    let scope = [BranchId::new(B1)];
    let filter = ExecutiveFilter {
        from: Some(today),
        to: Some(today),
        ..Default::default()
    };

    let conv = facts
        .booking_conversion(&scope, &filter)
        .await
        .expect("conversion succeeds")
        .expect("available");

    assert_eq!(conv.completed(), 1, "exactly one booking completed");
    assert_eq!(conv.confirmed(), 3, "three bookings reached confirmed");
    assert_eq!(conv.numerator(), 1);
    assert_eq!(conv.denominator(), 3);
    assert_eq!(conv.total, (1, 3));
    assert_eq!(conv.by_branch.get(&BranchId::new(B1)), Some(&(1, 3)));

    room.finish().await;
}

#[tokio::test]
async fn test_empty_cohort_reads_zero_zero() {
    let room = Room::start().await;
    room.seed_branch(ORG_1, B1, "Asia/Bangkok").await;
    let today = room.branch_today(B1).await;

    let facts = BookingFacts::new(room.pool.clone());
    let scope = [BranchId::new(B1)];
    let filter = ExecutiveFilter {
        from: Some(today),
        to: Some(today),
        ..Default::default()
    };

    let conv = facts
        .booking_conversion(&scope, &filter)
        .await
        .expect("conversion succeeds")
        .expect("available");

    assert_eq!(conv.completed(), 0);
    assert_eq!(conv.confirmed(), 0);
    assert_eq!(conv.numerator(), 0);
    assert_eq!(conv.denominator(), 0);
    assert_eq!(conv.total, (0, 0));
    assert_eq!(
        conv.by_branch
            .get(&BranchId::new(B1))
            .copied()
            .unwrap_or((0, 0)),
        (0, 0)
    );

    room.finish().await;
}

#[tokio::test]
async fn test_scope_filters_by_branch() {
    let room = Room::start().await;
    room.seed_branch(ORG_1, B1, "Asia/Bangkok").await;
    room.seed_branch(ORG_1, B2, "Asia/Bangkok").await;
    room.seed_branch(ORG_1, B3, "Asia/Bangkok").await;
    let today = room.branch_today(B1).await;

    let c = unique("CU");
    room.seed_customer(ORG_1, &c, "LIVE").await;

    // B1 (in scope): 2 completed, 2 confirmed -> (2, 2)
    let s1_1 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            30,
        )
        .await;
    let s1_2 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            30,
        )
        .await;
    let tx1_1 = room.seed_transaction(ORG_1, B1, &c, "LIVE", today).await;
    let tx1_2 = room.seed_transaction(ORG_1, B1, &c, "LIVE", today).await;
    let b1_1 = room
        .seed_booking(ORG_1, B1, &c, s1_1, "USD", "COMPLETED", Some(tx1_1))
        .await;
    room.seed_event(ORG_1, &b1_1, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(ORG_1, &b1_1, "COMPLETE", Some("IN_SERVICE"), "COMPLETED")
        .await;
    let b1_2 = room
        .seed_booking(ORG_1, B1, &c, s1_2, "USD", "COMPLETED", Some(tx1_2))
        .await;
    room.seed_event(ORG_1, &b1_2, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(ORG_1, &b1_2, "COMPLETE", Some("IN_SERVICE"), "COMPLETED")
        .await;

    // B2 (in scope): 1 completed, 2 confirmed -> (1, 2)
    let s2_1 = room
        .seed_slot(
            ORG_1,
            B2,
            today,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            30,
        )
        .await;
    let s2_2 = room
        .seed_slot(
            ORG_1,
            B2,
            today,
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            30,
        )
        .await;
    let tx2_1 = room.seed_transaction(ORG_1, B2, &c, "LIVE", today).await;
    let b2_1 = room
        .seed_booking(ORG_1, B2, &c, s2_1, "USD", "COMPLETED", Some(tx2_1))
        .await;
    room.seed_event(ORG_1, &b2_1, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(ORG_1, &b2_1, "COMPLETE", Some("IN_SERVICE"), "COMPLETED")
        .await;
    let b2_2 = room
        .seed_booking(ORG_1, B2, &c, s2_2, "USD", "CONFIRMED", None)
        .await;
    room.seed_event(ORG_1, &b2_2, "CREATE", None, "CONFIRMED")
        .await;

    // B3 (out of scope): 1 completed, 1 confirmed -> (1, 1)
    let s3_1 = room
        .seed_slot(
            ORG_1,
            B3,
            today,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            30,
        )
        .await;
    let tx3_1 = room.seed_transaction(ORG_1, B3, &c, "LIVE", today).await;
    let b3_1 = room
        .seed_booking(ORG_1, B3, &c, s3_1, "USD", "COMPLETED", Some(tx3_1))
        .await;
    room.seed_event(ORG_1, &b3_1, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(ORG_1, &b3_1, "COMPLETE", Some("IN_SERVICE"), "COMPLETED")
        .await;

    let facts = BookingFacts::new(room.pool.clone());
    let scope = [BranchId::new(B1), BranchId::new(B2)];
    let filter = ExecutiveFilter {
        from: Some(today),
        to: Some(today),
        ..Default::default()
    };

    let conv = facts
        .booking_conversion(&scope, &filter)
        .await
        .expect("conversion succeeds")
        .expect("available");

    assert_eq!(
        conv.total,
        (3, 4),
        "total over in-scope branches B1 (2/2) and B2 (1/2) must be (3, 4)"
    );
    assert_eq!(conv.completed(), 3);
    assert_eq!(conv.confirmed(), 4);
    assert_eq!(conv.numerator(), 3);
    assert_eq!(conv.denominator(), 4);

    assert_eq!(
        conv.by_branch.get(&BranchId::new(B1)),
        Some(&(2, 2)),
        "B1 must have (2, 2)"
    );
    assert_eq!(
        conv.by_branch.get(&BranchId::new(B2)),
        Some(&(1, 2)),
        "B2 must have (1, 2)"
    );
    assert_eq!(
        conv.by_branch.get(&BranchId::new(B3)),
        None,
        "out-of-scope B3 must have no entry"
    );
    assert_eq!(
        conv.by_branch.len(),
        2,
        "by_branch must contain only the in-scope branches"
    );

    room.finish().await;
}

#[tokio::test]
async fn test_currency_filter_applies() {
    let room = Room::start().await;
    room.seed_branch(ORG_1, B1, "Asia/Bangkok").await;
    let today = room.branch_today(B1).await;

    let c = unique("CU");
    room.seed_customer(ORG_1, &c, "LIVE").await;

    let s1 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            30,
        )
        .await;
    let s2 = room
        .seed_slot(
            ORG_1,
            B1,
            today,
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            30,
        )
        .await;

    let tx1 = room.seed_transaction(ORG_1, B1, &c, "LIVE", today).await;
    let tx2 = room.seed_transaction(ORG_1, B1, &c, "LIVE", today).await;

    let b1 = room
        .seed_booking(ORG_1, B1, &c, s1, "USD", "COMPLETED", Some(tx1))
        .await;
    room.seed_event(ORG_1, &b1, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(ORG_1, &b1, "COMPLETE", Some("IN_SERVICE"), "COMPLETED")
        .await;

    let b2 = room
        .seed_booking(ORG_1, B1, &c, s2, "EUR", "COMPLETED", Some(tx2))
        .await;
    room.seed_event(ORG_1, &b2, "CREATE", None, "CONFIRMED")
        .await;
    room.seed_event(ORG_1, &b2, "COMPLETE", Some("IN_SERVICE"), "COMPLETED")
        .await;

    let facts = BookingFacts::new(room.pool.clone());
    let scope = [BranchId::new(B1)];
    let filter_usd = ExecutiveFilter {
        from: Some(today),
        to: Some(today),
        currency: Some(CurrencyCode::parse("USD").unwrap()),
        ..Default::default()
    };

    let conv = facts
        .booking_conversion(&scope, &filter_usd)
        .await
        .expect("conversion succeeds")
        .expect("available");

    assert_eq!(
        conv.completed(),
        1,
        "currency filter must select only USD booking"
    );
    assert_eq!(
        conv.confirmed(),
        1,
        "currency filter must select only USD booking"
    );
    assert_eq!(conv.numerator(), 1);
    assert_eq!(conv.denominator(), 1);
    assert_eq!(conv.total, (1, 1));
    assert_eq!(conv.by_branch.get(&BranchId::new(B1)), Some(&(1, 1)));

    room.finish().await;
}

/// §4 item 2: Service-level test for the overview path (lane controller ruling issuecomment-6123359454).
/// Checks three things:
/// - A01's booking-conversion metric reads through `booking_conversion`.
/// - An empty cohort shows as "no data", never a percentage.
/// - The transaction facts still answer `AwaitsBookingDesign` for it.
#[tokio::test]
async fn test_overview_service_booking_conversion_path() {
    let room = Room::start().await;
    room.seed_branch(ORG_1, B1, "Asia/Bangkok").await;
    let today = room.branch_today(B1).await;

    let scope = [BranchId::new(B1)];
    let filter = ExecutiveFilter {
        from: Some(today),
        to: Some(today),
        ..Default::default()
    };
    let head = BranchHead {
        id: BranchId::new(B1),
        name: B1.to_string(),
        as_of: None,
    };

    // 1. Transaction facts (WideTransactionFacts) still answer AwaitsBookingDesign for booking_conversion.
    let wide = WideTransactionFacts::new(room.pool.clone());
    let wide_conv = wide
        .booking_conversion(&scope, &filter)
        .await
        .expect("wide facts query succeeds");
    assert_eq!(
        wide_conv,
        Err(MetricUnavailable::AwaitsBookingDesign),
        "transaction facts must answer AwaitsBookingDesign for booking_conversion"
    );

    // Overview with transaction facts produces AwaitsBookingDesign for booking_conversion.
    let cash = Arc::new(LedgerCashFacts::new(room.pool.clone()));
    let wide_service = ExecutiveOverviewService::new(
        Arc::new(wide),
        cash.clone(),
        Arc::new(PgAlertFacts::new(room.pool.clone(), cash.clone())),
        Arc::new(BrokerNotConnected),
    );
    let wide_overview = wide_service
        .overview(
            &scope,
            std::slice::from_ref(&head),
            &filter,
            &[BranchId::new(B1)],
        )
        .await
        .expect("overview with wide facts succeeds");
    let wide_metric = wide_overview
        .kpis
        .iter()
        .find(|m| m.key == BOOKING_CONVERSION)
        .expect("booking_conversion KPI must exist");
    assert_eq!(
        wide_metric.unavailable,
        Some(MetricUnavailable::AwaitsBookingDesign),
        "overview with transaction facts must answer awaitsBookingDesign"
    );
    assert!(
        wide_metric.value.is_none(),
        "unavailable metric must have no value"
    );

    // 2. Booking facts path: A01's booking-conversion metric reads through booking_conversion.
    // 3. When no Blueprint booking exists (empty cohort 0/0), the metric reads as "no data", never a percentage.
    let booking_facts = BookingFacts::new(room.pool.clone());
    // Direct call to booking_conversion must succeed and return total (0, 0)
    let booking_conv = booking_facts
        .booking_conversion(&scope, &filter)
        .await
        .expect("booking conversion succeeds")
        .expect("available");
    assert_eq!(booking_conv.total, (0, 0));

    let cash = Arc::new(LedgerCashFacts::new(room.pool.clone()));
    let booking_service = ExecutiveOverviewService::new(
        Arc::new(booking_facts),
        cash.clone(),
        Arc::new(PgAlertFacts::new(room.pool.clone(), cash.clone())),
        Arc::new(BrokerNotConnected),
    );
    let overview = booking_service
        .overview(
            &scope,
            std::slice::from_ref(&head),
            &filter,
            &[BranchId::new(B1)],
        )
        .await
        .expect("overview with booking facts succeeds");
    let metric = overview
        .kpis
        .iter()
        .find(|m| m.key == BOOKING_CONVERSION)
        .expect("booking_conversion KPI must exist");

    assert_ne!(
        metric.unavailable,
        Some(MetricUnavailable::AwaitsBookingDesign),
        "A01's booking-conversion metric must read through booking_conversion"
    );

    if let Some(val) = &metric.value {
        assert_ne!(
            val, "0%",
            "empty cohort must not be a made-up percentage '0%'"
        );
        assert_ne!(
            val, "0.00%",
            "empty cohort must not be a made-up percentage '0.00%'"
        );
        assert_ne!(val, "NaN%", "empty cohort must not be 'NaN%'");
        assert!(
            !val.contains('%'),
            "empty cohort must not be a percentage: {val}"
        );
    }

    room.finish().await;
}
