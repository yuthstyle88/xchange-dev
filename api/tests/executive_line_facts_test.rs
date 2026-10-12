//! Ticket #1189 (EX-6, narrowed by the lane controller's ruling on #1189, option A): A01 shows the period's
//! POSTED reversals beside the gross buy/sell volumes (design `executive-finance.md` §3 `buy_volume_thb` /
//! `sell_volume_thb`: "posted reversals shown **separately**", `:97`; F01's `:196`).
//!
//! `[required]` by the ruling: the A01 answer's `data` gains `buyReversedThb` and `sellReversedThb`, decimal THB
//! strings: Σ POSTED reversals whose original's line is BUY, and Σ of those whose line is SELL, by the reversal's
//! posting business date (its `posted_at` in the branch's timezone), in the period and the scope; `"0.00"` when there
//! are none. The volumes stay gross: a reversal never lowers `buy_volume_thb` / `sell_volume_thb`.
//!
//! Every original here has one line, and its reversal's `amount_thb` is that line's `thb_amount` (no fee), so
//! "the reversal's amount by its original's side" and "the reversed lines' THB" give the same number. RV-3 #684
//! (posting) is not built, so `reversals` rows are seeded directly, as #1071's rulings test does.
//!
//! These tests are red on `main` `4bfe4d7d` by result (the two keys are absent), not by a compile error.
//! Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips. Each test works in a schema of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_exchange::domain::customer::value_objects::mode::Mode;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::{StatusMove, TransactionStore};
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const HOST: &str = "xc.test";
/// The organisation the migrations seed.
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const B1: &str = "XC-B01";
const B2: &str = "XC-B02";
const EXECUTIVE: &str = "00000000-0000-0000-0000-0000000000e1";
/// The originals complete on 2026-10-01; the reversals are posted on 2026-10-05 unless a test says otherwise.
const ORIGINAL_DAY: &str = "2026-10-01";
const REVERSAL_DAY: &str = "from=2026-10-05&to=2026-10-05";

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{nanos}_{n}")
}

struct Room {
    pool: PgPool,
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
        let schema = unique("ex6_reversed");
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
        for branch in [B1, B2] {
            sqlx::query(
                "INSERT INTO branches (id, name, organization_id, timezone) \
                 VALUES ($1, $1, $2::uuid, 'Asia/Bangkok')",
            )
            .bind(branch)
            .bind(ORG)
            .execute(&pool)
            .await
            .expect("a branch");
            sqlx::query(
                "INSERT INTO branch_versions (branch_id, version, organization_id, changed_at) \
                 VALUES ($1, 1, $2::uuid, now())",
            )
            .bind(branch)
            .bind(ORG)
            .execute(&pool)
            .await
            .expect("a branch version");
        }
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
}

/// The organisation-wide EXECUTIVE of ORG.
struct Executive;

impl SessionReader for Executive {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async move {
            Sessions {
                owner: Some(Actor::owner(EXECUTIVE)),
                staff: None,
            }
        })
    }

    fn callers<'a>(&'a self, _req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        Box::pin(async move {
            let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
            let user = User::new(
                EXECUTIVE,
                ORG,
                vec![ScopedRole::new(Role::Executive, None, from, None)],
                None,
            );
            Callers {
                owner: Some(Actor::owner(EXECUTIVE)),
                staff: None,
                user: Some(user.clone()),
                staff_user: Some(user),
            }
        })
    }
}

/// A01's `data` for `query`, after checking the answer is 200.
async fn a01(room: &Room, query: &str) -> Value {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(Executive);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri(&format!("/api/executive/overview?{query}"))
        .insert_header((header::HOST, HOST))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert_eq!(status, StatusCode::OK, "{query}: {text}");
    let body: Value = serde_json::from_slice(&bytes).expect("a JSON answer");
    body["data"].clone()
}

/// `data.<key>` as a string; a missing key fails with the whole `data` shown.
fn text<'a>(data: &'a Value, key: &str) -> &'a str {
    data[key]
        .as_str()
        .unwrap_or_else(|| panic!("data.{key} is not a string: {data}"))
}

/// The value of A01's KPI `key` (design §3).
fn kpi<'a>(data: &'a Value, key: &str) -> &'a Value {
    data["kpis"]
        .as_array()
        .unwrap_or_else(|| panic!("data.kpis: {data}"))
        .iter()
        .find(|m| m["key"] == key)
        .map(|m| &m["value"])
        .unwrap_or_else(|| panic!("no KPI {key}: {data}"))
}

/// `hh:mm` local time on `day` in Asia/Bangkok (UTC+7), as UTC.
fn bangkok(day: &str, hour: u32, minute: u32) -> DateTime<Utc> {
    let d = NaiveDate::parse_from_str(day, "%Y-%m-%d").expect("a date");
    Utc.from_utc_datetime(&d.and_hms_opt(hour, minute, 0).expect("a time"))
        - chrono::Duration::hours(7)
}

/// One COMPLETED LIVE cash exchange at `branch` on `ORIGINAL_DAY`: one line on `side` of `thb` THB, no fee,
/// through TX-2's store, which issues its receipt. Returns (the transaction's id, its receipt's id).
async fn completed_exchange(room: &Room, branch: &str, side: Side, thb: &str) -> (Uuid, Uuid) {
    let pool = &room.pool;
    let shift = format!("SH-{branch}");
    let teller: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(ORG)
    .bind(unique("teller"))
    .fetch_one(pool)
    .await
    .expect("a teller");
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, organization_id, teller_id) \
         VALUES ($1, $2, 'OPEN', now(), 'test', $3, $4::uuid, $5::uuid) ON CONFLICT DO NOTHING",
    )
    .bind(&shift)
    .bind(branch)
    .bind(format!("open-{branch}"))
    .bind(ORG)
    .bind(&teller)
    .execute(pool)
    .await
    .expect("an open shift");
    sqlx::query(
        "INSERT INTO customers (id, mode, created_at, organization_id, customer_no, display_name) \
         VALUES ('C-REV', 'LIVE', now(), $1::uuid, 'C-REV', 'C-REV') ON CONFLICT DO NOTHING",
    )
    .bind(ORG)
    .execute(pool)
    .await
    .expect("a customer");
    for kind in ["PASSPORT", "RECEIPT"] {
        sqlx::query(
            "INSERT INTO files (key, organization_id, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ($1, $2::uuid, $3, 'owner', 'application/pdf', 'sha', now()) ON CONFLICT DO NOTHING",
        )
        .bind(format!("{kind}-{branch}"))
        .bind(ORG)
        .bind(branch)
        .execute(pool)
        .await
        .expect("a file");
    }
    let at = bangkok(ORIGINAL_DAY, 10, 0);
    let money = |s: &str| Money::parse(s).expect("money");
    let id = Uuid::new_v4();
    let t = ExchangeTransaction {
        id: TransactionId(id),
        organization_id: Uuid::parse_str(ORG).expect("org"),
        branch_id: BranchId::new(branch),
        mode: Mode::Live,
        status: TransactionStatus::Draft,
        document_ref: None,
        quote_id: None,
        booking_id: None,
        shift_id: Some(shift.clone()),
        submission_id: unique("sub"),
        payload_hash: "hash".into(),
        customer_id: "C-REV".into(),
        customer_name: "C-REV".into(),
        nationality: "TH".into(),
        staff_id: "staff".into(),
        booking_no: String::new(),
        notes: String::new(),
        value_date: NaiveDate::parse_from_str(ORIGINAL_DAY, "%Y-%m-%d").expect("a date"),
        recorded_at: at,
        recorded_by: "test".into(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        passport_file_key: format!("PASSPORT-{branch}"),
        kiosk_request_id: None,
        staff_draft_id: None,
        count_confirmed: false,
        lines: vec![TransactionLine {
            id: Uuid::new_v4(),
            quote_line_id: None,
            shop_side: side,
            currency: CurrencyCode::parse("USD").expect("USD"),
            quantity: ForeignAmount::from_decimal(Decimal::from_str(thb).expect("a number")),
            unit_size: UnitSize::parse("1").expect("unit"),
            rate_snapshot: Rate::parse("1").expect("rate"),
            reference_rate: None,
            thb_amount: money(thb),
            fee_thb: money("0.00"),
            settle_thb: money(thb),
            cost_basis_thb: None,
            realized_margin_thb: None,
            line_no: 1,
        }],
        payment: TransactionPayment {
            id: Uuid::new_v4(),
            payment_no: 1,
            method: PaymentMethod::Cash,
            reference: String::new(),
            account: String::new(),
            slip_file_key: None,
            amount_thb: money(thb),
            bank_fee_thb: money("0.00"),
            paid_at: at,
        },
        receipts: vec![],
    };
    let store = PgTransactionStore::new(pool.clone());
    let mut tx = pool.begin().await.expect("begin");
    store
        .insert_draft_in(&mut tx, &t)
        .await
        .expect("insert the draft");
    let to_confirmed = StatusMove {
        from: TransactionStatus::Draft,
        to: TransactionStatus::Confirmed,
        reason: None,
        approval_request_id: None,
    };
    store
        .set_status_in(&mut tx, t.id, to_confirmed, at)
        .await
        .expect("confirm");
    store
        .complete_in(&mut tx, t.id, &format!("RECEIPT-{branch}"), at)
        .await
        .expect("complete");
    tx.commit().await.expect("commit");
    let receipt: Uuid = sqlx::query_scalar("SELECT id FROM receipts WHERE transaction_id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("its receipt");
    (id, receipt)
}

/// A reversal at `branch` of `original` in `status`, for the original's whole THB (`amount`), requested and
/// created at `at` (so a row that is not POSTED still carries a date in the period: a reader that dates pending rows
/// by `coalesce(posted_at, requested_at)` must not count them). A POSTED one was posted at `at` too.
/// Seeded directly: RV-3 #684 (posting) is not built.
async fn reversal(
    room: &Room,
    branch: &str,
    original: (Uuid, Uuid),
    status: &str,
    amount: &str,
    at: DateTime<Utc>,
) {
    let request: Uuid = sqlx::query_scalar(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
           proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
         VALUES (gen_random_uuid(), 'REVERSAL', $1, $2, 'h', 'v1', '{}'::jsonb, 'test', $4, 'APPROVED', $2, $3::uuid) \
         RETURNING id",
    )
    .bind(branch)
    .bind(unique("reversal"))
    .bind(ORG)
    .bind(at)
    .fetch_one(&room.pool)
    .await
    .expect("its approval request");
    let posted = status == "POSTED";
    let posting_id = if posted {
        let pid = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, source_key, status, posted_at, posted_by) \
             VALUES ($1, $2::uuid, $3, 'REVERSAL', $4, 'reversal', 'POSTED', $5, 'finance')",
        )
        .bind(pid)
        .bind(ORG)
        .bind(branch)
        .bind(original.0.to_string())
        .bind(at)
        .execute(&room.pool)
        .await
        .expect("ledger posting");
        Some(pid)
    } else {
        None
    };
    sqlx::query(
        "INSERT INTO reversals (organization_id, branch_id, original_transaction_id, status, reason, amount_thb, \
           business_date, approval_request_id, posting_id, receipt_id, requested_by, requested_at, posted_by, posted_at, created_at, updated_at) \
         VALUES ($1::uuid, $2, $3, $4, 'test', $5::numeric, $6::date, $7, $8, $9, 'test', $12, $10, $11, $12, $12)",
    )
    .bind(ORG)
    .bind(branch)
    .bind(original.0)
    .bind(status)
    .bind(amount)
    .bind(ORIGINAL_DAY)
    .bind(request)
    .bind(posting_id)
    .bind(posted.then_some(original.1))
    .bind(posted.then_some("finance"))
    .bind(posted.then_some(at))
    .bind(at)
    .execute(&room.pool)
    .await
    .expect("a reversal");
}

/// A BUY of 1000 and a SELL of 400, both reversed and POSTED on 10-05: A01 for 10-05 shows `buyReversedThb`
/// "1000.00" and `sellReversedThb` "400.00". Over 10-01…10-05 the volumes stay gross (1000 and 400, not 0),
/// and the reversed figures are the same two numbers beside them.
#[actix_web::test]
async fn posted_reversals_are_shown_per_side_beside_gross_volumes() {
    let room = Room::start().await;
    let buy = completed_exchange(&room, B1, Side::Buy, "1000.00").await;
    let sell = completed_exchange(&room, B1, Side::Sell, "400.00").await;
    reversal(
        &room,
        B1,
        buy,
        "POSTED",
        "1000.00",
        bangkok("2026-10-05", 10, 0),
    )
    .await;
    reversal(
        &room,
        B1,
        sell,
        "POSTED",
        "400.00",
        bangkok("2026-10-05", 15, 0),
    )
    .await;

    let d = a01(&room, REVERSAL_DAY).await;
    assert_eq!(text(&d, "buyReversedThb"), "1000.00", "{d}");
    assert_eq!(text(&d, "sellReversedThb"), "400.00", "{d}");

    let d = a01(&room, "from=2026-10-01&to=2026-10-05").await;
    assert_eq!(kpi(&d, "buy_volume_thb"), "1000.00", "gross, not net: {d}");
    assert_eq!(kpi(&d, "sell_volume_thb"), "400.00", "gross, not net: {d}");
    assert_eq!(text(&d, "buyReversedThb"), "1000.00", "{d}");
    assert_eq!(text(&d, "sellReversedThb"), "400.00", "{d}");
    room.finish().await;
}

/// Only POSTED reversals posted inside the period count, by the posting time in the branch's timezone (Bangkok):
/// - 300, POSTED at 23:30 on 10-05: counts;
/// - 125, POSTED at 00:30 on 10-06, and 60, POSTED at 23:59 on 10-04: outside 10-05;
/// - 70 APPROVED, 50 REQUESTED and 40 REJECTED: never posted.
#[actix_web::test]
async fn only_reversals_posted_inside_the_period_count() {
    let room = Room::start().await;
    for (status, amount, posted_at) in [
        ("POSTED", "300.00", bangkok("2026-10-05", 23, 30)),
        ("POSTED", "125.00", bangkok("2026-10-06", 0, 30)),
        ("POSTED", "60.00", bangkok("2026-10-04", 23, 59)),
        ("APPROVED", "70.00", bangkok("2026-10-05", 12, 0)),
        ("REQUESTED", "50.00", bangkok("2026-10-05", 12, 0)),
        ("REJECTED", "40.00", bangkok("2026-10-05", 12, 0)),
    ] {
        let original = completed_exchange(&room, B1, Side::Buy, amount).await;
        reversal(&room, B1, original, status, amount, posted_at).await;
    }

    let d = a01(&room, REVERSAL_DAY).await;
    assert_eq!(text(&d, "buyReversedThb"), "300.00", "{d}");
    assert_eq!(text(&d, "sellReversedThb"), "0.00", "{d}");
    room.finish().await;
}

/// With no reversal at all, both figures are present and read "0.00", not absent or null.
#[actix_web::test]
async fn no_reversal_reads_zero_on_both_sides() {
    let room = Room::start().await;
    completed_exchange(&room, B1, Side::Buy, "500.00").await;

    let d = a01(&room, REVERSAL_DAY).await;
    assert_eq!(text(&d, "buyReversedThb"), "0.00", "{d}");
    assert_eq!(text(&d, "sellReversedThb"), "0.00", "{d}");
    room.finish().await;
}

/// The figures follow A01's scope: a reversal at B2 is not in `branches=XC-B01`, and both are in the
/// organisation-wide answer.
#[actix_web::test]
async fn the_reversed_figures_follow_the_branch_scope() {
    let room = Room::start().await;
    let at = bangkok("2026-10-05", 11, 0);
    let b1 = completed_exchange(&room, B1, Side::Sell, "200.00").await;
    reversal(&room, B1, b1, "POSTED", "200.00", at).await;
    let b2 = completed_exchange(&room, B2, Side::Sell, "50.00").await;
    reversal(&room, B2, b2, "POSTED", "50.00", at).await;

    let d = a01(&room, &format!("{REVERSAL_DAY}&branches={B1}")).await;
    assert_eq!(text(&d, "sellReversedThb"), "200.00", "{d}");
    assert_eq!(text(&d, "buyReversedThb"), "0.00", "{d}");

    let d = a01(&room, REVERSAL_DAY).await;
    assert_eq!(
        text(&d, "sellReversedThb"),
        "250.00",
        "organisation-wide: {d}"
    );
    room.finish().await;
}
