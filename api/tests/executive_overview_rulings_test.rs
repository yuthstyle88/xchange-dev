//! Ticket #1071 (EX-3): the implementer's tests for the lane controller's rulings on #1071, beside the up-front
//! tests (which stay unedited):
//! - gap 4 (issuecomment-6108369072): an exception row the custody reader cannot decode (the new cash flow's
//!   `{countId, currency, difference}`) does not fail A01, and is one `countDifference` alert whose detail is the
//!   row's id only;
//! - gap 7 (same comment): `branches` is a comma list, and an empty item is `400 validationFailed`; an id that is
//!   in no organisation is `403 outOfScope`, like another organisation's;
//! - the `views/` mount (issuecomment-6108432223): `/api/v3/views/executive_overview?branch=…` answers an id outside
//!   the caller's organisation with `400 noSuchBranch`, and one of its own branches with A01 for that branch only;
//! - `ExchangeFacts::branch_day_totals` reports POSTED reversals apart, by their posting business date (§7, design §3),
//!   now that RV-1 #682 has merged.
//!
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
use serde_json::{Value, json};
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
use xchange::application::executive::ports::ExchangeFacts;
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::executive::wide_transaction_facts::WideTransactionFacts;
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const HOST: &str = "xc.test";
/// The organisation the migrations seed.
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const ORG2: &str = "00000000-0000-0000-0000-0000000000f2";
const B1: &str = "XC-B01";
const B2: &str = "XC-B02";
const B9: &str = "XC-B90";
const EXECUTIVE: &str = "00000000-0000-0000-0000-0000000000e1";
const PERIOD: &str = "from=2026-10-05&to=2026-10-05";

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
        let schema = unique("ex3_rulings");
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
            "INSERT INTO organizations (id, code, timezone, created_at) \
             VALUES ($1::uuid, 'ORG2', 'Asia/Bangkok', now())",
        )
        .bind(ORG2)
        .execute(&pool)
        .await
        .expect("a second organisation");
        for (branch, org) in [(B1, ORG), (B2, ORG), (B9, ORG2)] {
            sqlx::query(
                "INSERT INTO branches (id, name, organization_id, timezone) \
                 VALUES ($1, $1, $2::uuid, 'Asia/Bangkok')",
            )
            .bind(branch)
            .bind(org)
            .execute(&pool)
            .await
            .expect("a branch");
            sqlx::query(
                "INSERT INTO branch_versions (branch_id, version, organization_id, changed_at) \
                 VALUES ($1, 1, $2::uuid, now())",
            )
            .bind(branch)
            .bind(org)
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

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn get(room: &Room, uri: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(Executive);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn refused(a: &Answer, status: StatusCode, code: &str, what: &str) {
    assert_eq!(
        (a.status, a.body["error"]["code"].as_str().unwrap_or("")),
        (status, code),
        "{what}: {}",
        a.text
    );
    assert!(
        a.body.get("data").is_none_or(Value::is_null),
        "{what}: {}",
        a.text
    );
}

fn row_ids(a: &Answer) -> Vec<String> {
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    a.body["data"]["branches"]
        .as_array()
        .unwrap_or_else(|| panic!("data.branches: {}", a.text))
        .iter()
        .map(|r| r["branchId"].as_str().unwrap_or("").to_owned())
        .collect()
}

/// Gap 4: an exception the new cash flow wrote (`xc-cash` `count_service.rs`, `{countId, currency, difference}`)
/// cannot be decoded by the custody reader. A01 still answers 200, and that branch has one `countDifference` alert
/// whose detail is the row's id, nothing read from `data`.
#[actix_web::test]
async fn an_exception_the_custody_reader_cannot_decode_is_one_alert_with_its_id() {
    let room = Room::start().await;
    let id = unique("EXNEW");
    sqlx::query(
        "INSERT INTO cash_exceptions (id, branch_id, position, data, organization_id) \
         VALUES ($1, $2, 1, $3, $4::uuid)",
    )
    .bind(&id)
    .bind(B1)
    .bind(json!({ "countId": unique("count"), "currency": "USD", "difference": "-25.00" }))
    .bind(ORG)
    .execute(&room.pool)
    .await
    .expect("a new-shape exception");

    let a = get(&room, &format!("/api/executive/overview?{PERIOD}")).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "A01 is not failed by one row: {}",
        a.text
    );
    let alerts: Vec<&Value> = a.body["data"]["alerts"]
        .as_array()
        .unwrap_or_else(|| panic!("data.alerts: {}", a.text))
        .iter()
        .filter(|x| x["kind"] == "countDifference")
        .collect();
    assert_eq!(alerts.len(), 1, "{}", a.text);
    assert_eq!(alerts[0]["branchId"], B1, "{}", a.text);
    assert_eq!(
        alerts[0]["detail"],
        id.as_str(),
        "the row's id only: {}",
        a.text
    );
    room.finish().await;
}

/// Gap 7: an empty item anywhere in `branches` is `400 validationFailed`. An id that names no branch at all is
/// `403 outOfScope`, the same answer as another organisation's branch, so the list cannot probe which ids exist.
/// Two of the caller's own branches are both answered.
#[actix_web::test]
async fn branches_is_a_comma_list_and_an_empty_item_is_refused() {
    let room = Room::start().await;
    for list in [
        format!("{B1},,{B2}"),
        format!(",{B1}"),
        format!("{B1},"),
        String::new(),
    ] {
        let a = get(
            &room,
            &format!("/api/executive/overview?{PERIOD}&branches={list}"),
        )
        .await;
        refused(
            &a,
            StatusCode::BAD_REQUEST,
            "validationFailed",
            &format!("branches={list}"),
        );
    }
    for list in ["XC-NOPE".to_owned(), format!("{B1},{B9}")] {
        let a = get(
            &room,
            &format!("/api/executive/overview?{PERIOD}&branches={list}"),
        )
        .await;
        refused(
            &a,
            StatusCode::FORBIDDEN,
            "outOfScope",
            &format!("branches={list}"),
        );
    }
    let a = get(
        &room,
        &format!("/api/executive/overview?{PERIOD}&branches={B2},{B1}"),
    )
    .await;
    assert_eq!(row_ids(&a), vec![B1, B2]);
    room.finish().await;
}

/// The `views/` mount: one `branch`. An id that is not a branch of the caller's organisation is `400 noSuchBranch`,
/// whether it exists elsewhere (B9) or nowhere (XC-GONE). One of its own branches is A01 for that branch only.
#[actix_web::test]
async fn the_views_path_answers_one_branch_of_the_callers_organisation() {
    let room = Room::start().await;
    for branch in ["XC-GONE", B9] {
        let a = get(
            &room,
            &format!("/api/v3/views/executive_overview?branch={branch}"),
        )
        .await;
        refused(&a, StatusCode::BAD_REQUEST, "noSuchBranch", branch);
        assert_eq!(a.body["error"]["field"], "branch", "{}", a.text);
    }
    let a = get(
        &room,
        &format!("/api/v3/views/executive_overview?branch={B1}&{PERIOD}"),
    )
    .await;
    assert_eq!(row_ids(&a), vec![B1]);
    assert_eq!(a.body["versions"], json!({ B1: 1 }), "{}", a.text);
    room.finish().await;
}

/// `hh:mm` local time on `day` in Asia/Bangkok (UTC+7), as UTC.
fn bangkok(day: &str, hour: u32, minute: u32) -> DateTime<Utc> {
    let d = NaiveDate::parse_from_str(day, "%Y-%m-%d").expect("a date");
    Utc.from_utc_datetime(&d.and_hms_opt(hour, minute, 0).expect("a time"))
        - chrono::Duration::hours(7)
}

/// One COMPLETED LIVE cash BUY of 1000 THB at B1 on 2026-10-01, through TX-2's store, which issues its receipt.
/// The shift, customer and files are seeded by the first call. Returns (the transaction's id, its receipt's id).
async fn completed_exchange(room: &Room) -> (Uuid, Uuid) {
    let pool = &room.pool;
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
         VALUES ('SH-B1', $1, 'OPEN', now(), 'test', 'open-b1', $2::uuid, $3::uuid) ON CONFLICT DO NOTHING",
    )
    .bind(B1)
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
        .bind(format!("{kind}-REV"))
        .bind(ORG)
        .bind(B1)
        .execute(pool)
        .await
        .expect("a file");
    }
    let at = bangkok("2026-10-01", 10, 0);
    let money = |s: &str| Money::parse(s).expect("money");
    let id = Uuid::new_v4();
    let t = ExchangeTransaction {
        id: TransactionId(id),
        organization_id: Uuid::parse_str(ORG).expect("org"),
        branch_id: BranchId::new(B1),
        mode: Mode::Live,
        status: TransactionStatus::Draft,
        document_ref: None,
        quote_id: None,
        booking_id: None,
        shift_id: Some("SH-B1".into()),
        submission_id: unique("sub"),
        payload_hash: "hash".into(),
        customer_id: "C-REV".into(),
        customer_name: "C-REV".into(),
        nationality: "TH".into(),
        staff_id: "staff".into(),
        booking_no: String::new(),
        notes: String::new(),
        value_date: NaiveDate::from_ymd_opt(2026, 10, 1).expect("a date"),
        recorded_at: at,
        recorded_by: "test".into(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        passport_file_key: "PASSPORT-REV".into(),
        kiosk_request_id: None,
        staff_draft_id: None,
        count_confirmed: false,
        lines: vec![TransactionLine {
            id: Uuid::new_v4(),
            quote_line_id: None,
            shop_side: Side::Buy,
            currency: CurrencyCode::parse("USD").expect("USD"),
            quantity: ForeignAmount::from_decimal(Decimal::from(1000)),
            unit_size: UnitSize::parse("1").expect("unit"),
            rate_snapshot: Rate::parse("1").expect("rate"),
            reference_rate: None,
            thb_amount: money("1000.00"),
            fee_thb: money("0.00"),
            settle_thb: money("1000.00"),
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
            amount_thb: money("1000.00"),
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
        .complete_in(&mut tx, t.id, "RECEIPT-REV", at)
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

/// A reversal of `amount` at B1 in `status`; a POSTED one was posted at `posted_at`.
async fn reversal(
    room: &Room,
    original: (Uuid, Uuid),
    status: &str,
    amount: &str,
    posted_at: DateTime<Utc>,
) {
    let request: Uuid = sqlx::query_scalar(
        "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
           proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
         VALUES (gen_random_uuid(), 'REVERSAL', $1, $2, 'h', 'v1', '{}'::jsonb, 'test', now(), 'APPROVED', $2, $3::uuid) \
         RETURNING id",
    )
    .bind(B1)
    .bind(unique("reversal"))
    .bind(ORG)
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
        .bind(B1)
        .bind(original.0.to_string())
        .bind(posted_at)
        .execute(&room.pool)
        .await
        .expect("ledger posting");
        Some(pid)
    } else {
        None
    };
    sqlx::query(
        "INSERT INTO reversals (organization_id, branch_id, original_transaction_id, status, reason, amount_thb, \
           business_date, approval_request_id, posting_id, receipt_id, requested_by, requested_at, posted_by, posted_at) \
         VALUES ($1::uuid, $2, $3, $4, 'test', $5::numeric, '2026-10-01', $6, $7, $8, 'test', now(), $9, $10)",
    )
    .bind(ORG)
    .bind(B1)
    .bind(original.0)
    .bind(status)
    .bind(amount)
    .bind(request)
    .bind(posting_id)
    .bind(posted.then_some(original.1))
    .bind(posted.then_some("finance"))
    .bind(posted.then_some(posted_at))
    .execute(&room.pool)
    .await
    .expect("a reversal");
}

/// `branch_day_totals(B1, d)`: POSTED reversals posted on business date `d` in the branch's timezone, apart from the
/// completed totals. A reversal posted at 00:30 the next local day, and one only APPROVED, are not in `d`.
#[actix_web::test]
async fn branch_day_totals_report_posted_reversals_by_their_posting_date() {
    let room = Room::start().await;
    // one live reversal per transaction (RV-1): each reverses its own original
    for (status, amount, posted_at) in [
        ("POSTED", "250.00", bangkok("2026-10-05", 23, 30)),
        ("POSTED", "125.00", bangkok("2026-10-06", 0, 30)),
        ("APPROVED", "70.00", bangkok("2026-10-05", 12, 0)),
    ] {
        let original = completed_exchange(&room).await;
        reversal(&room, original, status, amount, posted_at).await;
    }

    let facts = WideTransactionFacts::new(room.pool.clone());
    let day = |d: u32| NaiveDate::from_ymd_opt(2026, 10, d).expect("a date");
    let b1 = BranchId::new(B1);
    let got = facts.branch_day_totals(&b1, day(5)).await.expect("10-05");
    assert_eq!(got.reversals_thb, Decimal::from(250), "{got:?}");
    assert!(
        got.by_method.is_empty(),
        "no exchange completed on 10-05: {got:?}"
    );
    let got = facts.branch_day_totals(&b1, day(6)).await.expect("10-06");
    assert_eq!(got.reversals_thb, Decimal::from(125), "{got:?}");
    let got = facts.branch_day_totals(&b1, day(1)).await.expect("10-01");
    assert_eq!(
        got.reversals_thb,
        Decimal::ZERO,
        "the original's day posts nothing: {got:?}"
    );
    room.finish().await;
}
