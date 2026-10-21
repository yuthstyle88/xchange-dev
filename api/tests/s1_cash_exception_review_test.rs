//! S1 of the FE survey section B (#3413; design `docs/design/blueprint/section-b-gaps.md` §S1, PR 3387;
//! `executive-finance.md` §5.2 "ตั้ง exception"; `cash-shift-transfer.md:230-233`).
//!
//! 🔴 Written by the ticket writer claude/studio-claude-4 before the implementation. Do not edit,
//! delete or weaken; add tests in a file of your own. The harness above the tests is copied from
//! `finance_reconciliation_test.rs` (FN-1).
//!
//! FINANCE reviews an OPEN cash exception (`POST /api/v3/commands/cash_exception_review`) or raises
//! one by hand (`POST /api/v3/commands/cash_exception_raise`), each with one audit row.

#![allow(dead_code)]

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
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
use xchange::presentation::pages::{Callers, SessionReader};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
/// The organisation the migrations seed.
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const B1: &str = "XC-B01";
const B2: &str = "XC-B02";
const DAY: &str = "2026-10-05";
const CERTIFY: &str = "/api/v3/commands/reconciliation_certify";
const AUDIT_ACTION: &str = "RECONCILIATION_CERTIFY";

/// FINANCE of B1: the one who certifies.
const CERTIFIER: &str = "FINANCE@XC-B01#1";
/// FINANCE of B1 too, who verified a count of the day.
const VERIFIER: &str = "FINANCE@XC-B01#2";
/// FINANCE of B2 only.
const OTHER_BRANCH: &str = "FINANCE@XC-B02#3";
/// The teller who counted.
const COUNTER: &str = "TELLER@XC-B01#4";
const CALLERS: [&str; 5] = [
    CERTIFIER,
    VERIFIER,
    OTHER_BRANCH,
    COUNTER,
    "TELLER@XC-B02#5",
];

/// The four parts FN-2 fills, and the ticket each waits for until then (§7, `UnavailableParts`).
const WAITING: [(&str, &str); 4] = [
    ("counts", "CA-3"),
    ("transfers", "CA-5"),
    ("integrity", "LG-7"),
    ("reversalsPending", "RV-1"),
];

fn unique(prefix: &str) -> String {
    xc_testdb::unique_schema_name(prefix)
}

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("a date")
}

/// `hh:mm` local time on `day` in Asia/Bangkok (UTC+7), as UTC.
fn bangkok(day: &str, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date(day).and_hms_opt(hour, minute, 0).expect("a time"))
        - chrono::Duration::hours(7)
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap_or_else(|e| panic!("{s:?} is a decimal: {e}"))
}

/// A fixed uuid per caller spec, so `certified_by`, `verified_by` and the audit's actor can be checked.
fn user_id(spec: &str) -> String {
    let (_, n) = spec.rsplit_once('#').expect("a spec ends in #n");
    format!(
        "00000000-0000-4000-8000-{:012}",
        n.parse::<u64>().expect("n")
    )
}

// ------------------------------------------------------------ database

struct Room {
    pool: PgPool,
    schema: String,
}

/// One exchange to seed through TX-2's store.
struct Ex<'a> {
    branch: &'a str,
    side: &'a str,
    thb: &'a str,
    fee: &'a str,
    bank_fee: &'a str,
    method: &'a str,
    value_date: &'a str,
    live: bool,
    status: &'a str,
}

impl Default for Ex<'_> {
    fn default() -> Self {
        Ex {
            branch: B1,
            side: "BUY",
            thb: "100.00",
            fee: "0.00",
            bank_fee: "0.00",
            method: "Cash",
            value_date: DAY,
            live: true,
            status: "COMPLETED",
        }
    }
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
        let schema = unique("fn1");
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
            .bind(user_id(if branch == B1 { COUNTER } else { "TELLER@XC-B02#5" }))
            .execute(&room.pool)
            .await
            .expect("an open shift");
        }
        for kind in ["PASSPORT", "RECEIPT"] {
            sqlx::query(
                "INSERT INTO files (key, organization_id, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ($1, $2::uuid, $3, 'owner', 'application/pdf', 'sha', now())",
            )
            .bind(format!("{kind}-FN1"))
            .bind(ORG)
            .bind(B1)
            .execute(&room.pool)
            .await
            .expect("a file");
        }
        room
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(AssertSqlSafe(sql.to_owned()))
            .fetch_one(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
    }

    async fn certifications(&self) -> i64 {
        self.count("SELECT count(*) FROM reconciliation_certifications")
            .await
    }

    async fn audits(&self) -> i64 {
        self.count(&format!(
            "SELECT count(*) FROM audit_events WHERE action = '{AUDIT_ACTION}'"
        ))
        .await
    }

    /// Seeds one exchange through TX-2's store (draft, then its status moves; a COMPLETED one through
    /// `complete_in`). Returns its uuid.
    async fn exchange(&self, x: Ex<'_>) -> Uuid {
        let customer = unique("C");
        sqlx::query(
            "INSERT INTO customers (id, mode, created_at, organization_id, customer_no, display_name) \
             VALUES ($1, 'LIVE', now(), $2::uuid, $1, $1)",
        )
        .bind(&customer)
        .bind(ORG)
        .execute(&self.pool)
        .await
        .expect("a customer");
        let money = |s: &str| Money::parse(s).expect("money");
        let thb = d(x.thb);
        let fee = d(x.fee);
        let settle = if x.side == "BUY" {
            thb - fee
        } else {
            thb + fee
        };
        let at = bangkok(x.value_date, 10, 0);
        let id = Uuid::new_v4();
        let t = ExchangeTransaction {
            id: TransactionId(id),
            organization_id: Uuid::parse_str(ORG).expect("org"),
            branch_id: BranchId::new(x.branch),
            mode: if x.live { Mode::Live } else { Mode::Demo },
            status: TransactionStatus::Draft,
            document_ref: None,
            quote_id: None,
            booking_id: None,
            shift_id: x.live.then(|| format!("SH-{}", x.branch)),
            submission_id: unique("sub"),
            payload_hash: "hash".into(),
            customer_id: customer.clone(),
            customer_name: customer,
            nationality: "TH".into(),
            staff_id: "staff".into(),
            booking_no: String::new(),
            notes: String::new(),
            value_date: date(x.value_date),
            recorded_at: at,
            recorded_by: "test".into(),
            completed_at: None,
            cancelled_at: None,
            cancel_reason: None,
            approval_request_id: None,
            passport_file_key: "PASSPORT-FN1".into(),
            kiosk_request_id: None,
            staff_draft_id: None,
            count_confirmed: false,
            lines: vec![TransactionLine {
                id: Uuid::new_v4(),
                quote_line_id: None,
                shop_side: if x.side == "BUY" {
                    Side::Buy
                } else {
                    Side::Sell
                },
                currency: CurrencyCode::parse("USD").expect("USD"),
                quantity: ForeignAmount::from_decimal(thb),
                unit_size: UnitSize::parse("1").expect("unit"),
                rate_snapshot: Rate::parse("1").expect("rate"),
                reference_rate: None,
                thb_amount: money(x.thb),
                fee_thb: money(x.fee),
                settle_thb: money(&settle.to_string()),
                cost_basis_thb: None,
                realized_margin_thb: None,
                line_no: 1,
            }],
            payment: TransactionPayment {
                id: Uuid::new_v4(),
                payment_no: 1,
                method: match x.method {
                    "Cash" => PaymentMethod::Cash,
                    "PromptPay" => PaymentMethod::PromptPay,
                    other => panic!("a payment method this file seeds: {other}"),
                },
                reference: String::new(),
                account: String::new(),
                slip_file_key: None,
                amount_thb: money(&settle.to_string()),
                bank_fee_thb: money(x.bank_fee),
                paid_at: at,
            },
            receipts: vec![],
        };
        let store = PgTransactionStore::new(self.pool.clone());
        let mut tx = self.pool.begin().await.expect("begin");
        store
            .insert_draft_in(&mut tx, &t)
            .await
            .expect("insert the draft");
        if x.status == "COMPLETED" {
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
                .complete_in(&mut tx, t.id, "RECEIPT-FN1", at)
                .await
                .expect("complete");
        } else {
            assert_eq!(x.status, "DRAFT", "a status this file seeds");
        }
        tx.commit().await.expect("commit");
        id
    }

    /// A second payment (`payment_no = 2`) of `transaction`, with its own bank fee.
    async fn second_payment(&self, transaction: Uuid, method: &str, amount: &str, bank_fee: &str) {
        let at = bangkok(DAY, 10, 0);
        sqlx::query(
            "INSERT INTO transaction_payments (id, organization_id, transaction_id, payment_no, method, amount_thb, \
               bank_fee_thb, paid_at, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, 2, $4, $5::numeric, $6::numeric, $7, $7, $7)",
        )
        .bind(Uuid::new_v4())
        .bind(ORG)
        .bind(transaction)
        .bind(method)
        .bind(amount)
        .bind(bank_fee)
        .bind(at)
        .execute(&self.pool)
        .await
        .expect("a second payment");
    }

    /// A POSTED reversal of `original` (a COMPLETED exchange at B1), posted at `posted_at`.
    async fn posted_reversal(&self, original: Uuid, amount: &str, posted_at: DateTime<Utc>) {
        let receipt: Uuid = sqlx::query_scalar("SELECT id FROM receipts WHERE transaction_id = $1")
            .bind(original)
            .fetch_one(&self.pool)
            .await
            .expect("its receipt");
        let request: Uuid = sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
               proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
             VALUES (gen_random_uuid(), 'TRANSACTION_REVERSAL', $1, $2, 'h', 'v1', '{}'::jsonb, 'test', now(), \
               'APPROVED', $2, $3::uuid) \
             RETURNING id",
        )
        .bind(B1)
        .bind(unique("reversal"))
        .bind(ORG)
        .fetch_one(&self.pool)
        .await
        .expect("its approval request");
        let posting_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, source_key, status, posted_at, posted_by) \
             VALUES ($1, $2::uuid, $3, 'REVERSAL', $4, 'reversal', 'POSTED', $5, 'finance')",
        )
        .bind(posting_id)
        .bind(ORG)
        .bind(B1)
        .bind(original.to_string())
        .bind(posted_at)
        .execute(&self.pool)
        .await
        .expect("ledger posting");
        sqlx::query(
            "INSERT INTO reversals (organization_id, branch_id, original_transaction_id, status, reason, amount_thb, \
               business_date, approval_request_id, posting_id, receipt_id, requested_by, requested_at, posted_by, posted_at) \
             VALUES ($1::uuid, $2, $3, 'POSTED', 'test', $4::numeric, $5, $6, $7, $8, 'test', now(), 'finance', $9)",
        )
        .bind(ORG)
        .bind(B1)
        .bind(original)
        .bind(amount)
        .bind(date(DAY))
        .bind(request)
        .bind(posting_id)
        .bind(receipt)
        .bind(posted_at)
        .execute(&self.pool)
        .await
        .expect("a posted reversal");
    }

    /// A CLOSE count at B1 counted by COUNTER at `counted_at`, verified by `verifier` at `verified_at`.
    async fn verified_count(
        &self,
        verifier: &str,
        counted_at: DateTime<Utc>,
        verified_at: DateTime<Utc>,
    ) {
        sqlx::query(
            "INSERT INTO cash_counts (organization_id, branch_id, kind, counted_by, counted_at, verified_by, \
               verified_at, idempotency_key) \
             VALUES ($1::uuid, $2, 'CLOSE', $3::uuid, $4, $5::uuid, $6, $7)",
        )
        .bind(ORG)
        .bind(B1)
        .bind(user_id(COUNTER))
        .bind(counted_at)
        .bind(user_id(verifier))
        .bind(verified_at)
        .bind(unique("count"))
        .execute(&self.pool)
        .await
        .expect("a verified count");
    }

    /// One `cash_exceptions` row of B1 at the next position. Returns its id.
    async fn exception(&self, data: Value) -> String {
        let id = unique("EX");
        sqlx::query(
            "INSERT INTO cash_exceptions (id, branch_id, position, data, organization_id) \
             VALUES ($1, $2, (SELECT coalesce(max(position), 0) + 1 FROM cash_exceptions WHERE branch_id = $2), \
               $3, $4::uuid)",
        )
        .bind(&id)
        .bind(B1)
        .bind(data)
        .bind(ORG)
        .execute(&self.pool)
        .await
        .expect("an exception");
        id
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

// ------------------------------------------------------------ callers

/// `x-test-as: <ROLE>@<branch>#<n>`: one Blueprint role row at that branch, with user id `user_id(spec)`.
struct HeaderUsers;

fn user_of(spec: &str) -> Option<User> {
    let (role, rest) = spec.split_once('@')?;
    let (branch, _) = rest.split_once('#').expect("a spec ends in #n");
    let role = match role {
        "FINANCE" => Role::Finance,
        "TELLER" => Role::Teller,
        other => panic!("unknown role {other}"),
    };
    Some(User::new(
        &user_id(spec),
        ORG,
        vec![ScopedRole::new(
            role,
            Some(BranchId::new(branch)),
            DateTime::<Utc>::UNIX_EPOCH,
            None,
        )],
        None,
    ))
}

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let user = user_of(&spec);
            Callers {
                user: user.clone(),
                staff_user: user,
            }
        })
    }
}

// ------------------------------------------------------------ requests

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn call(
    room: &Room,
    who: &str,
    method: Method,
    uri: &str,
    body: Option<Value>,
    key: &str,
) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let mut req = atest::TestRequest::default()
        .method(method)
        .uri(uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who));
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", key.to_owned()))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

fn view_uri(branch: &str, day: &str) -> String {
    format!("/api/v3/views/finance_reconciliation?branch={branch}&date={day}")
}

async fn read(room: &Room, who: &str, branch: &str, day: &str) -> Answer {
    call(room, who, Method::GET, &view_uri(branch, day), None, "").await
}

/// The sheet (`data` of a 200) of `branch` on `day`, as `who`.
async fn sheet(room: &Room, who: &str, branch: &str, day: &str) -> Value {
    let a = read(room, who, branch, day).await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "read {branch} {day} as {who}: {}",
        a.text
    );
    let data = a.body["data"].clone();
    assert!(data.is_object(), "the view's data is the sheet: {}", a.text);
    data
}

fn hash_of(sheet: &Value) -> String {
    let h = sheet["figuresHash"]
        .as_str()
        .unwrap_or_else(|| panic!("figuresHash is a string: {sheet}"));
    assert!(!h.is_empty(), "figuresHash is not empty: {sheet}");
    h.to_owned()
}

fn certify_body(branch: &str, day: &str, hash: &str, note: &str) -> Value {
    json!({ "branch": branch, "date": day, "figuresHash": hash, "note": note })
}

async fn certify(room: &Room, who: &str, body: Value, key: &str) -> Answer {
    call(room, who, Method::POST, CERTIFY, Some(body), key).await
}

/// Reads B1's sheet on DAY as CERTIFIER and certifies it with its current hash. Returns the hash.
async fn certify_current(room: &Room, note: &str) -> (String, Answer) {
    let hash = hash_of(&sheet(room, CERTIFIER, B1, DAY).await);
    let a = certify(
        room,
        CERTIFIER,
        certify_body(B1, DAY, &hash, note),
        &unique("key"),
    )
    .await;
    (hash, a)
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

fn assert_refused(a: &Answer, status: StatusCode, want: &str, what: &str) {
    assert_eq!(
        (a.status, code(a).as_str()),
        (status, want),
        "{what}: {}",
        a.text
    );
}

fn certifications_of(sheet: &Value) -> Vec<Value> {
    sheet["certifications"]
        .as_array()
        .unwrap_or_else(|| panic!("certifications is an array: {sheet}"))
        .clone()
}

const REVIEW: &str = "/api/v3/commands/cash_exception_review";
const RAISE: &str = "/api/v3/commands/cash_exception_raise";
const AT: i64 = 1_791_263_520_000;

/// A count-sourced OPEN exception of B1 in the custody shape (CA-7d).
async fn open_exception(room: &Room) -> String {
    room.exception(
        json!({ "requestId": unique("count"), "status": "OPEN", "note": "COUNT c-1", "at": AT }),
    )
    .await
}

async fn data_of(room: &Room, id: &str) -> Value {
    sqlx::query_scalar("SELECT data FROM cash_exceptions WHERE id = $1")
        .bind(id)
        .fetch_one(&room.pool)
        .await
        .expect("the exception row")
}

async fn audit_count(room: &Room, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_events WHERE action = $1")
        .bind(action)
        .fetch_one(&room.pool)
        .await
        .expect("count audit rows")
}

async fn review(room: &Room, who: &str, id: &str, note: &str, key: &str) -> Answer {
    call(
        room,
        who,
        Method::POST,
        REVIEW,
        Some(json!({ "branch": B1, "exceptionId": id, "note": note })),
        key,
    )
    .await
}

/// B1: reviewing an OPEN exception marks it REVIEWED by the actor, keeps its source note, stores the
/// reviewer's note apart, and writes one audit row.
#[actix_web::test]
async fn s1_reviewing_an_open_exception_marks_it_reviewed_with_one_audit_row() {
    let room = Room::start().await;
    let id = open_exception(&room).await;

    let a = review(
        &room,
        CERTIFIER,
        &id,
        "counted again, the drawer was right",
        &unique("k"),
    )
    .await;

    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["status"], "REVIEWED", "{}", a.text);
    let data = data_of(&room, &id).await;
    assert_eq!(data["status"], "REVIEWED", "{data}");
    assert_eq!(data["reviewedBy"], user_id(CERTIFIER), "{data}");
    assert!(
        data["reviewedAt"].is_number() || data["reviewedAt"].is_string(),
        "{data}"
    );
    assert_eq!(
        data["reviewNote"], "counted again, the drawer was right",
        "{data}"
    );
    assert_eq!(data["note"], "COUNT c-1", "the source note is kept: {data}");

    let audit: (String, String) = sqlx::query_as(
        "SELECT actor_id, object_ref FROM audit_events WHERE action = 'CASH_EXCEPTION_REVIEW'",
    )
    .fetch_one(&room.pool)
    .await
    .expect("exactly one review audit row");
    assert_eq!(audit, (user_id(CERTIFIER), format!("cash_exception:{id}")));
    room.finish().await;
}

/// B2 + B3: reviewing again is 409 alreadyReviewed; an unknown id is 404 notFound; nothing changes.
#[actix_web::test]
async fn s1_a_second_review_is_already_reviewed_and_an_unknown_id_is_not_found() {
    let room = Room::start().await;
    let id = open_exception(&room).await;
    let first = review(&room, CERTIFIER, &id, "first", &unique("k")).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let after_first = data_of(&room, &id).await;

    let again = review(&room, CERTIFIER, &id, "second", &unique("k")).await;
    assert_refused(
        &again,
        StatusCode::CONFLICT,
        "alreadyReviewed",
        "a second review",
    );
    assert_eq!(
        data_of(&room, &id).await,
        after_first,
        "the row is unchanged"
    );

    let unknown = review(&room, CERTIFIER, "EX-nope", "x", &unique("k")).await;
    assert_refused(&unknown, StatusCode::NOT_FOUND, "notFound", "an unknown id");
    assert_eq!(audit_count(&room, "CASH_EXCEPTION_REVIEW").await, 1);
    room.finish().await;
}

/// B4: a reviewed exception leaves the F01 sheet's open list.
#[actix_web::test]
async fn s1_a_reviewed_exception_leaves_the_sheets_open_list() {
    let room = Room::start().await;
    let id = open_exception(&room).await;
    let a = review(&room, CERTIFIER, &id, "ok", &unique("k")).await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);

    let s = sheet(&room, CERTIFIER, B1, DAY).await;
    let ids: Vec<&str> = s["exceptions"]["data"]
        .as_array()
        .unwrap_or_else(|| panic!("exceptions.data: {s}"))
        .iter()
        .map(|r| r["id"].as_str().unwrap_or(""))
        .collect();
    assert!(!ids.contains(&id.as_str()), "{s}");
    room.finish().await;
}

/// A blank review note is refused; nothing changes.
#[actix_web::test]
async fn s1_a_blank_review_note_is_refused() {
    let room = Room::start().await;
    let id = open_exception(&room).await;
    let a = review(&room, CERTIFIER, &id, "   ", &unique("k")).await;
    assert_refused(
        &a,
        StatusCode::BAD_REQUEST,
        "validationFailed",
        "a blank note",
    );
    assert_eq!(data_of(&room, &id).await["status"], "OPEN");
    room.finish().await;
}

/// B5: raising by hand appends one OPEN row in the custody shape, with a MANUAL request id.
#[actix_web::test]
async fn s1_raising_an_exception_appends_one_open_manual_row() {
    let room = Room::start().await;
    let before = open_exception(&room).await;

    let a = call(
        &room,
        CERTIFIER,
        Method::POST,
        RAISE,
        Some(json!({
            "branch": B1, "reason": "drawer short after close", "place": "DRAWER",
            "differences": [{ "currency": "THB", "amount": "-100.00" }]
        })),
        &unique("k"),
    )
    .await;

    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    assert_eq!(a.body["status"], "OPEN", "{}", a.text);
    let new_id = a.body["exceptionId"]
        .as_str()
        .expect("exceptionId")
        .to_owned();
    assert_ne!(new_id, before);
    let data = data_of(&room, &new_id).await;
    assert!(
        data["requestId"]
            .as_str()
            .unwrap_or("")
            .starts_with("MANUAL-"),
        "{data}"
    );
    assert_eq!(data["status"], "OPEN", "{data}");
    assert_eq!(data["note"], "MANUAL drawer short after close", "{data}");
    assert!(
        !data["cashVariance"].is_null(),
        "a drawer difference: {data}"
    );
    assert!(data["safeVariance"].is_null(), "{data}");
    assert_eq!(audit_count(&room, "CASH_EXCEPTION_RAISE").await, 1);
    room.finish().await;
}

/// B6 + validation: the same key twice gives one row; empty or zero differences are refused.
#[actix_web::test]
async fn s1_the_same_raise_key_twice_writes_one_row_and_bad_differences_are_refused() {
    let room = Room::start().await;
    let body = json!({
        "branch": B1, "reason": "safe over", "place": "SAFE",
        "differences": [{ "currency": "USD", "amount": "20.00" }]
    });
    let key = unique("k");
    let one = call(
        &room,
        CERTIFIER,
        Method::POST,
        RAISE,
        Some(body.clone()),
        &key,
    )
    .await;
    let two = call(&room, CERTIFIER, Method::POST, RAISE, Some(body), &key).await;
    assert_eq!(one.status, StatusCode::OK, "{}", one.text);
    assert_eq!(two.body, one.body, "the same answer");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM cash_exceptions WHERE branch_id = $1")
        .bind(B1)
        .fetch_one(&room.pool)
        .await
        .expect("count");
    assert_eq!(rows, 1, "one row for one key");

    for differences in [json!([]), json!([{ "currency": "USD", "amount": "0" }])] {
        let a = call(
            &room,
            CERTIFIER,
            Method::POST,
            RAISE,
            Some(
                json!({ "branch": B1, "reason": "x", "place": "SAFE", "differences": differences }),
            ),
            &unique("k"),
        )
        .await;
        assert_refused(
            &a,
            StatusCode::BAD_REQUEST,
            "validationFailed",
            "bad differences",
        );
    }
    room.finish().await;
}

/// B7: a TELLER can do neither.
#[actix_web::test]
async fn s1_a_teller_cannot_review_or_raise() {
    let room = Room::start().await;
    let id = open_exception(&room).await;
    let r = review(&room, COUNTER, &id, "x", &unique("k")).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text);
    let r = call(
        &room,
        COUNTER,
        Method::POST,
        RAISE,
        Some(json!({ "branch": B1, "reason": "x", "place": "DRAWER",
                     "differences": [{ "currency": "THB", "amount": "-1" }] })),
        &unique("k"),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text);
    assert_eq!(data_of(&room, &id).await["status"], "OPEN");
    room.finish().await;
}
