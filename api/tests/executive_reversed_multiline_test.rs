//! Ticket #2374 — Executive overview (`A01`) counts each POSTED reversal once per side of its
//! original exchange, even when the original exchange has multiple `transaction_lines`.

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
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::{StatusMove, TransactionStore};
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const HOST: &str = "xc.test";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const B1: &str = "XC-B01";
const EXECUTIVE: &str = "00000000-0000-0000-0000-0000000000e1";
const ORIGINAL_DAY: &str = "2026-10-01";
const REVERSAL_DAY: &str = "from=2026-10-05&to=2026-10-05";

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
        let url = std::env::var("TEST_DATABASE_URL").expect(
            "TEST_DATABASE_URL is not set; these tests need a real PostgreSQL and do not skip",
        );
        let options = PgConnectOptions::from_str(&url).expect("a PostgreSQL URL");
        let database = options.get_database().unwrap_or("").to_owned();
        assert!(
            database.ends_with("_test"),
            "'{database}' does not end in `_test`"
        );
        let schema = unique("ex_rev_multiline");
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
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id, timezone) \
             VALUES ($1, $1, $2::uuid, 'Asia/Bangkok')",
        )
        .bind(B1)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("a branch");
        sqlx::query(
            "INSERT INTO branch_versions (branch_id, version, organization_id, changed_at) \
             VALUES ($1, 1, $2::uuid, now())",
        )
        .bind(B1)
        .bind(ORG)
        .execute(&pool)
        .await
        .expect("a branch version");
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
                user: Some(user.clone()),
                staff_user: Some(user),
            }
        })
    }
}

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

fn text<'a>(data: &'a Value, key: &str) -> &'a str {
    data[key]
        .as_str()
        .unwrap_or_else(|| panic!("data.{key} is not a string: {data}"))
}

fn bangkok(day: &str, hour: u32, minute: u32) -> DateTime<Utc> {
    let d = NaiveDate::parse_from_str(day, "%Y-%m-%d").expect("a date");
    Utc.from_utc_datetime(&d.and_hms_opt(hour, minute, 0).expect("a time"))
        - chrono::Duration::hours(7)
}

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

async fn add_line(room: &Room, tx_id: Uuid, line_no: i32, currency: &str, thb: &str) {
    sqlx::query(
        "INSERT INTO transaction_lines (id, organization_id, transaction_id, quote_line_id, shop_side, \
           currency, quantity, unit_size, rate_snapshot, reference_rate, thb_amount, fee_thb, settle_thb, \
           cost_basis_thb, realized_margin_thb, line_no, created_at, updated_at) \
         SELECT gen_random_uuid(), organization_id, transaction_id, quote_line_id, shop_side, \
           $3, $4::numeric, unit_size, rate_snapshot, reference_rate, $4::numeric, fee_thb, $4::numeric, \
           cost_basis_thb, realized_margin_thb, $2, created_at, updated_at \
         FROM transaction_lines WHERE transaction_id = $1 AND line_no = 1",
    )
    .bind(tx_id)
    .bind(line_no)
    .bind(currency)
    .bind(thb)
    .execute(&room.pool)
    .await
    .expect("copy transaction line");
}

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

// Row M1: one POSTED reversal (2000.00) of a 2-line BUY exchange (1000 + 1000) -> buyReversedThb = 2000.00
#[actix_web::test]
async fn m1_reversal_of_two_line_buy_exchange_is_counted_once() {
    let room = Room::start().await;
    let orig = completed_exchange(&room, B1, Side::Buy, "1000.00").await;
    add_line(&room, orig.0, 2, "EUR", "1000.00").await;
    reversal(
        &room,
        B1,
        orig,
        "POSTED",
        "2000.00",
        bangkok("2026-10-05", 10, 0),
    )
    .await;

    let d = a01(&room, REVERSAL_DAY).await;
    assert_eq!(text(&d, "buyReversedThb"), "2000.00", "{d}");
    assert_eq!(text(&d, "sellReversedThb"), "0.00", "{d}");
    room.finish().await;
}

// Row M2: the same with 3 BUY lines (3 x 500, reversal 1500.00) -> 1500.00
#[actix_web::test]
async fn m2_reversal_of_three_line_buy_exchange_is_counted_once() {
    let room = Room::start().await;
    let orig = completed_exchange(&room, B1, Side::Buy, "500.00").await;
    add_line(&room, orig.0, 2, "EUR", "500.00").await;
    add_line(&room, orig.0, 3, "GBP", "500.00").await;
    reversal(
        &room,
        B1,
        orig,
        "POSTED",
        "1500.00",
        bangkok("2026-10-05", 11, 0),
    )
    .await;

    let d = a01(&room, REVERSAL_DAY).await;
    assert_eq!(text(&d, "buyReversedThb"), "1500.00", "{d}");
    assert_eq!(text(&d, "sellReversedThb"), "0.00", "{d}");
    room.finish().await;
}

// Row M3: a 2-line BUY exchange whose lines are both USD (1000 + 1000), reversal 2000.00,
// A01 filtered currency=USD -> 2000.00
#[actix_web::test]
async fn m3_reversal_of_two_usd_lines_filtered_by_usd_is_counted_once() {
    let room = Room::start().await;
    let orig = completed_exchange(&room, B1, Side::Buy, "1000.00").await;
    add_line(&room, orig.0, 2, "USD", "1000.00").await;
    reversal(
        &room,
        B1,
        orig,
        "POSTED",
        "2000.00",
        bangkok("2026-10-05", 12, 0),
    )
    .await;

    let d = a01(&room, &format!("{REVERSAL_DAY}&currency=USD")).await;
    assert_eq!(text(&d, "buyReversedThb"), "2000.00", "{d}");
    assert_eq!(text(&d, "sellReversedThb"), "0.00", "{d}");
    room.finish().await;
}

// Row M5: two different reversals of one-line exchanges on the same side -> their sum, each once (kept)
#[actix_web::test]
async fn m5_two_reversals_on_the_same_side_sum_each_once() {
    let room = Room::start().await;
    let orig1 = completed_exchange(&room, B1, Side::Buy, "1000.00").await;
    let orig2 = completed_exchange(&room, B1, Side::Buy, "650.00").await;
    reversal(
        &room,
        B1,
        orig1,
        "POSTED",
        "1000.00",
        bangkok("2026-10-05", 13, 0),
    )
    .await;
    reversal(
        &room,
        B1,
        orig2,
        "POSTED",
        "650.00",
        bangkok("2026-10-05", 14, 0),
    )
    .await;

    let d = a01(&room, REVERSAL_DAY).await;
    assert_eq!(text(&d, "buyReversedThb"), "1650.00", "{d}");
    assert_eq!(text(&d, "sellReversedThb"), "0.00", "{d}");
    room.finish().await;
}
