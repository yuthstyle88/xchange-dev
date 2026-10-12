//! FN-1 (#1068): F01, FINANCE's reconcile-and-certify sheet — `GET /api/v3/views/finance_reconciliation?branch=&date=`
//! and `POST /api/v3/commands/reconciliation_certify` (design `docs/design/blueprint/executive-finance.md` §5, §8.2;
//! owner decision E-8). Written before the implementation by the up-front test writer (claude/07d3765f) on the lane
//! controller's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! Shapes these tests assume (ticket §7–§8, [required] JSON, camelCase):
//! - The view answers the v3 envelope; its `data` is `FinanceSheet { branch, date, transactions, counts, transfers,
//!   exceptions, integrity, reversalsPending, figuresHash, certifications }`. Each part is
//!   `Part { available, reason, data }`.
//! - `transactions.data` is `{ byMethod: [{ method, buyThb, sellThb, fees, bankFees, count }], reversedThb }`, money
//!   as decimal strings, one row per payment method ordered by method (EX-3's `branch_day_totals`).
//! - `certifications` is `[CertificationRow { id, certifiedBy, certifiedAt, note, figuresHash, stale }]`, latest
//!   first. The certify command answers one `CertificationRow`.
//! - Refusals are `{ error: { code, message } }`: `samePerson` (409, `CashError::SameCounter`'s sentence),
//!   `staleVersion` (409), `idempotencyConflict` (409, the seam's), `outOfScope` (403), `invalidRequest` (400).
//! - One `audit_events` row per certification: `action = 'RECONCILIATION_CERTIFY'`,
//!   `object_ref = 'reconciliation:<branch>:<date>'`, `after_redacted = {figuresHash, note}`.
//!
//! The lane controller's rulings on #1068 (issuecomment-6109917312), pinned here:
//! - Q1: `count` comes through EX-3's port; each COMPLETED transaction counts once, under its `payment_no = 1`
//!   method. A CASH-first transaction with a second PromptPay payment is CASH 1, PromptPay 0; the second payment's
//!   bank fee still goes to PromptPay.
//! - Q2: the exceptions part lists open rows only (`reviewed: false` on each), is not day-scoped, and is **not**
//!   in the certification hash. A row the custody reader cannot decode is open, with `summary` = its id.
//! - Q3: a count belongs to the business date of its `counted_at` in the branch's timezone (E-8's "that day").
//!
//! The hash algorithm is [proposed]: it is pinned as stable and change-sensitive, never by its hex value.
//!
//! Callers are Blueprint `access::Actor`s through `SessionReader::callers`, as in `branch_alert_settings_test.rs`.
//! Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips. Each test works in a schema of its own.

mod common;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Permission, Role, ScopedRole};
use xc_auth::domain::policy::entities::policy::{Need, Policy};
use xc_auth::domain::policy::value_objects::action::Action;
use xc_auth::domain::policy::value_objects::actor::Actor;
use xc_auth::domain::policy::value_objects::entry_path::EntryPath;
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
use xc_cash::domain::error::CashError;
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
const CALLERS: [&str; 4] = [CERTIFIER, VERIFIER, OTHER_BRANCH, COUNTER];

/// The four parts FN-2 fills, and the ticket each waits for until then (§7, `UnavailableParts`).
const WAITING: [(&str, &str); 4] = [
    ("counts", "CA-3"),
    ("transfers", "CA-5"),
    ("integrity", "LG-7"),
    ("reversalsPending", "RV-1"),
];

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
            .bind(user_id(COUNTER))
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
        .bind(Uuid::new_v4())
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

/// The legacy actor the seam still carries beside the user; its role is not what decides here.
fn legacy_of(spec: &str) -> Sessions {
    match spec.split_once('@') {
        None => Sessions {
            owner: None,
            staff: None,
        },
        Some((role, rest)) => {
            let (branch, _) = rest.split_once('#').expect("a spec ends in #n");
            let old = if role == "TELLER" {
                OldRole::Front
            } else {
                OldRole::Finance
            };
            Sessions {
                owner: None,
                staff: Actor::staff(user_id(spec), old, BranchId::new(branch)),
            }
        }
    }
}

fn spec_of(req: &HttpRequest) -> String {
    req.headers()
        .get(AS)
        .map(|v| v.to_str().expect("ASCII").to_owned())
        .unwrap_or_default()
}

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move { legacy_of(&spec) })
    }

    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = spec_of(req);
        Box::pin(async move {
            let s = legacy_of(&spec);
            let user = user_of(&spec);
            Callers {
                owner: s.owner,
                staff: s.staff,
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

// ------------------------------------------------------------ §9 row 1: the seeded day

/// §9 "a seeded day": the transactions part is B1's COMPLETED LIVE totals of DAY per payment method (EX-3's
/// `branch_day_totals`), with `count` per the Q1 ruling and posted reversals apart. DEMO, unfinished, another
/// day's and another branch's rows are left out. The hash is a string and is stable over two reads.
#[actix_web::test]
async fn a_seeded_day_shows_its_transaction_figures_and_a_stable_hash() {
    let room = Room::start().await;
    let first = room
        .exchange(Ex {
            side: "BUY",
            thb: "1000.00",
            fee: "10.00",
            ..Ex::default()
        })
        .await;
    room.exchange(Ex {
        side: "SELL",
        thb: "2000.00",
        fee: "20.00",
        ..Ex::default()
    })
    .await;
    room.exchange(Ex {
        side: "SELL",
        thb: "500.00",
        fee: "5.00",
        bank_fee: "3.00",
        method: "PromptPay",
        ..Ex::default()
    })
    .await;
    // Q1: CASH first, PromptPay second ⇒ counted once, under CASH; the second payment's bank fee is PromptPay's
    let split = room
        .exchange(Ex {
            side: "BUY",
            thb: "300.00",
            ..Ex::default()
        })
        .await;
    room.second_payment(split, "PromptPay", "0.00", "2.00")
        .await;
    for left_out in [
        Ex {
            thb: "7000.00",
            live: false,
            ..Ex::default()
        },
        Ex {
            thb: "7000.00",
            status: "DRAFT",
            ..Ex::default()
        },
        Ex {
            thb: "7000.00",
            value_date: "2026-10-06",
            ..Ex::default()
        },
        Ex {
            thb: "7000.00",
            branch: B2,
            ..Ex::default()
        },
    ] {
        room.exchange(left_out).await;
    }
    room.posted_reversal(first, "250.00", bangkok(DAY, 15, 0))
        .await;

    let s = sheet(&room, CERTIFIER, B1, DAY).await;
    assert_eq!(s["branch"], B1, "{s}");
    assert_eq!(s["date"], DAY, "{s}");
    let part = &s["transactions"];
    assert_eq!(part["available"], true, "transactions: {s}");
    let rows = part["data"]["byMethod"]
        .as_array()
        .unwrap_or_else(|| panic!("transactions.data.byMethod: {s}"));
    let got: Vec<(String, Decimal, Decimal, Decimal, Decimal, i64)> = rows
        .iter()
        .map(|r| {
            let money = |k: &str| {
                d(r[k]
                    .as_str()
                    .unwrap_or_else(|| panic!("{k} is a decimal string: {r}")))
            };
            (
                r["method"].as_str().unwrap_or_default().to_owned(),
                money("buyThb"),
                money("sellThb"),
                money("fees"),
                money("bankFees"),
                r["count"]
                    .as_i64()
                    .unwrap_or_else(|| panic!("count is a number: {r}")),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (
                "Cash".to_owned(),
                d("1300.00"),
                d("2000.00"),
                d("30.00"),
                d("0.00"),
                3
            ),
            (
                "PromptPay".to_owned(),
                d("0.00"),
                d("500.00"),
                d("5.00"),
                d("5.00"),
                1
            ),
        ],
        "{s}"
    );
    let reversed = part["data"]["reversedThb"]
        .as_str()
        .unwrap_or_else(|| panic!("reversedThb is a decimal string: {s}"));
    assert_eq!(d(reversed), d("250.00"), "{s}");

    // the exceptions part is today's own reader: available, and empty when the branch has none
    assert_eq!(s["exceptions"]["available"], true, "{s}");
    assert_eq!(s["exceptions"]["data"], json!([]), "{s}");
    assert_eq!(s["certifications"], json!([]), "{s}");

    let again = sheet(&room, CERTIFIER, B1, DAY).await;
    assert_eq!(
        hash_of(&s),
        hash_of(&again),
        "the same figures give the same hash"
    );
    room.finish().await;
}

/// §8: a bad date is `400 invalidRequest`.
#[actix_web::test]
async fn a_bad_date_is_an_invalid_request() {
    let room = Room::start().await;
    for day in ["2026-13-01", "yesterday", ""] {
        let a = read(&room, CERTIFIER, B1, day).await;
        assert_refused(
            &a,
            StatusCode::BAD_REQUEST,
            "invalidRequest",
            &format!("date={day}"),
        );
    }
    room.finish().await;
}

// ------------------------------------------------------------ §9 row 2: E-8

/// §9 "the verifier of one of that day's counts certifies": `409 samePerson` with `CashError::SameCounter`'s
/// sentence, no row, no audit. Another FINANCE of the branch certifies the same figures.
#[actix_web::test]
async fn the_verifier_of_a_count_of_the_day_cannot_certify_it() {
    let room = Room::start().await;
    room.exchange(Ex::default()).await;
    room.verified_count(VERIFIER, bangkok(DAY, 9, 0), bangkok(DAY, 9, 30))
        .await;
    let hash = hash_of(&sheet(&room, VERIFIER, B1, DAY).await);

    let a = certify(
        &room,
        VERIFIER,
        certify_body(B1, DAY, &hash, "mine"),
        &unique("key"),
    )
    .await;
    assert_refused(&a, StatusCode::CONFLICT, "samePerson", "the verifier");
    assert_eq!(
        a.body["error"]["message"],
        CashError::SameCounter.to_string(),
        "{}",
        a.text
    );
    assert_eq!(room.certifications().await, 0, "nothing written");
    assert_eq!(room.audits().await, 0, "nothing audited");

    let ok = certify(
        &room,
        CERTIFIER,
        certify_body(B1, DAY, &hash, "theirs"),
        &unique("key"),
    )
    .await;
    assert_eq!(
        ok.status,
        StatusCode::OK,
        "another FINANCE certifies: {}",
        ok.text
    );
    assert_eq!(room.certifications().await, 1);
    room.finish().await;
}

/// Q3: a count belongs to its `counted_at` business date in the branch's timezone. A count made at 23:50 local on
/// DAY and verified at 00:10 the next day blocks its verifier on DAY, not on the next day. A count of the day
/// before does not block DAY.
#[actix_web::test]
async fn a_count_belongs_to_the_business_date_it_was_counted() {
    let room = Room::start().await;
    room.verified_count(VERIFIER, bangkok(DAY, 23, 50), bangkok("2026-10-06", 0, 10))
        .await;
    room.verified_count(
        VERIFIER,
        bangkok("2026-10-04", 10, 0),
        bangkok("2026-10-04", 10, 30),
    )
    .await;

    let hash = hash_of(&sheet(&room, VERIFIER, B1, DAY).await);
    let a = certify(
        &room,
        VERIFIER,
        certify_body(B1, DAY, &hash, ""),
        &unique("key"),
    )
    .await;
    assert_refused(&a, StatusCode::CONFLICT, "samePerson", "counted on DAY");

    let next = "2026-10-06";
    let hash = hash_of(&sheet(&room, VERIFIER, B1, next).await);
    let a = certify(
        &room,
        VERIFIER,
        certify_body(B1, next, &hash, ""),
        &unique("key"),
    )
    .await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "verified on the next day, counted on DAY: {}",
        a.text
    );
    assert_eq!(room.certifications().await, 1);
    room.finish().await;
}

// ------------------------------------------------------------ §9 row 3: certify

/// §9 "certify": one certification row and one `audit_events` row. The answer is the `CertificationRow`, and the
/// sheet lists it, not stale.
#[actix_web::test]
async fn certify_writes_one_row_and_one_audit_event() {
    let room = Room::start().await;
    room.exchange(Ex::default()).await;
    let (hash, a) = certify_current(&room, "checked").await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    let row = &a.body;
    assert!(row["id"].is_string(), "{}", a.text);
    assert_eq!(row["certifiedBy"], user_id(CERTIFIER), "{}", a.text);
    assert!(row["certifiedAt"].is_string(), "{}", a.text);
    assert_eq!(row["note"], "checked", "{}", a.text);
    assert_eq!(row["figuresHash"], hash.as_str(), "{}", a.text);
    assert_eq!(row["stale"], false, "{}", a.text);

    let stored: (String, String, NaiveDate, String, String, Option<String>) = sqlx::query_as(
        "SELECT id::text, branch_id, business_date, figures_hash, certified_by::text, note \
           FROM reconciliation_certifications",
    )
    .fetch_one(&room.pool)
    .await
    .expect("exactly one certification row");
    assert_eq!(
        stored,
        (
            row["id"].as_str().unwrap_or_default().to_owned(),
            B1.to_owned(),
            date(DAY),
            hash.clone(),
            user_id(CERTIFIER),
            Some("checked".to_owned()),
        )
    );

    let audit: (String, String, Option<String>, Value) = sqlx::query_as(
        "SELECT actor_id, object_ref, branch_id, after_redacted FROM audit_events WHERE action = $1",
    )
    .bind(AUDIT_ACTION)
    .fetch_one(&room.pool)
    .await
    .expect("exactly one audit row");
    assert_eq!(audit.0, user_id(CERTIFIER));
    assert_eq!(audit.1, format!("reconciliation:{B1}:{DAY}"));
    assert_eq!(audit.3, json!({ "figuresHash": hash, "note": "checked" }));
    assert_eq!(room.audits().await, 1);

    let listed = certifications_of(&sheet(&room, CERTIFIER, B1, DAY).await);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["id"], row["id"]);
    assert_eq!(listed[0]["stale"], false);
    assert!(
        certifications_of(&sheet(&room, CERTIFIER, B1, "2026-10-06").await).is_empty(),
        "a certification belongs to its own day"
    );
    room.finish().await;
}

// ------------------------------------------------------------ §9 rows 4–5: idempotency

/// §9 "the same key twice": one row, one audit, the same answer.
#[actix_web::test]
async fn the_same_key_twice_writes_one_row() {
    let room = Room::start().await;
    room.exchange(Ex::default()).await;
    let hash = hash_of(&sheet(&room, CERTIFIER, B1, DAY).await);
    let key = unique("key");
    let body = certify_body(B1, DAY, &hash, "once");
    let first = certify(&room, CERTIFIER, body.clone(), &key).await;
    let second = certify(&room, CERTIFIER, body, &key).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    assert_eq!(second.status, StatusCode::OK, "{}", second.text);
    assert_eq!(first.body["id"], second.body["id"], "the stored answer");
    assert_eq!(room.certifications().await, 1);
    assert_eq!(room.audits().await, 1);
    room.finish().await;
}

/// §9 "the same key, another body": `idempotencyConflict`, nothing more written.
#[actix_web::test]
async fn the_same_key_with_another_body_is_a_conflict() {
    let room = Room::start().await;
    room.exchange(Ex::default()).await;
    let hash = hash_of(&sheet(&room, CERTIFIER, B1, DAY).await);
    let key = unique("key");
    let first = certify(&room, CERTIFIER, certify_body(B1, DAY, &hash, "one"), &key).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let other = certify(&room, CERTIFIER, certify_body(B1, DAY, &hash, "two"), &key).await;
    assert_refused(
        &other,
        StatusCode::CONFLICT,
        "idempotencyConflict",
        "another note",
    );
    assert_eq!(room.certifications().await, 1);
    assert_eq!(room.audits().await, 1);
    room.finish().await;
}

// ------------------------------------------------------------ §9 row 6: an old hash

/// §9 "an old `figuresHash`": `409 staleVersion`, nothing written. The hash read before a figure changed is old.
#[actix_web::test]
async fn an_old_hash_is_a_stale_version() {
    let room = Room::start().await;
    room.exchange(Ex::default()).await;
    let old = hash_of(&sheet(&room, CERTIFIER, B1, DAY).await);
    room.exchange(Ex {
        thb: "50.00",
        ..Ex::default()
    })
    .await;
    let a = certify(
        &room,
        CERTIFIER,
        certify_body(B1, DAY, &old, ""),
        &unique("key"),
    )
    .await;
    assert_refused(&a, StatusCode::CONFLICT, "staleVersion", "the old hash");
    let a = certify(
        &room,
        CERTIFIER,
        certify_body(B1, DAY, "not-a-hash", ""),
        &unique("key"),
    )
    .await;
    assert_refused(&a, StatusCode::CONFLICT, "staleVersion", "a made-up hash");
    assert_eq!(room.certifications().await, 0);
    assert_eq!(room.audits().await, 0);
    room.finish().await;
}

// ------------------------------------------------------------ §9 rows 7–8: stale, then re-certify

/// §9 "a figure changes after certifying": the sheet's hash moves and the certification reads `stale: true`;
/// nothing is deleted. §9 "re-certify after the change": a second row; the first stays, stale, listed after it.
#[actix_web::test]
async fn a_later_change_makes_the_certification_stale_and_a_recertify_adds_a_row() {
    let room = Room::start().await;
    room.exchange(Ex::default()).await;
    let (first_hash, first) = certify_current(&room, "first").await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);

    room.exchange(Ex {
        side: "SELL",
        thb: "75.00",
        ..Ex::default()
    })
    .await;
    let s = sheet(&room, CERTIFIER, B1, DAY).await;
    assert_ne!(hash_of(&s), first_hash, "a new figure moves the hash");
    let listed = certifications_of(&s);
    assert_eq!(listed.len(), 1, "nothing deleted: {s}");
    assert_eq!(listed[0]["id"], first.body["id"]);
    assert_eq!(listed[0]["stale"], true, "{s}");
    assert_eq!(room.certifications().await, 1);

    let (second_hash, second) = certify_current(&room, "second").await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.text);
    assert_eq!(room.certifications().await, 2);
    assert_eq!(room.audits().await, 2);
    let listed = certifications_of(&sheet(&room, CERTIFIER, B1, DAY).await);
    let got: Vec<(Value, Value, Value)> = listed
        .iter()
        .map(|c| {
            (
                c["id"].clone(),
                c["figuresHash"].clone(),
                c["stale"].clone(),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (second.body["id"].clone(), json!(second_hash), json!(false)),
            (first.body["id"].clone(), json!(first_hash), json!(true)),
        ],
        "latest first; the first stays, stale"
    );
    room.finish().await;
}

// ------------------------------------------------------------ §9 row 9: scope

/// §9 "a FINANCE of another branch": `403 outOfScope` on the view and on certify; nothing written. Its own branch
/// answers.
#[actix_web::test]
async fn a_finance_of_another_branch_is_out_of_scope() {
    let room = Room::start().await;
    room.exchange(Ex::default()).await;
    let hash = hash_of(&sheet(&room, CERTIFIER, B1, DAY).await);

    let a = read(&room, OTHER_BRANCH, B1, DAY).await;
    assert_refused(&a, StatusCode::FORBIDDEN, "outOfScope", "read B1 as B2's");
    let a = certify(
        &room,
        OTHER_BRANCH,
        certify_body(B1, DAY, &hash, ""),
        &unique("key"),
    )
    .await;
    assert_refused(
        &a,
        StatusCode::FORBIDDEN,
        "outOfScope",
        "certify B1 as B2's",
    );
    assert_eq!(room.certifications().await, 0);
    assert_eq!(room.audits().await, 0);

    let own = sheet(&room, OTHER_BRANCH, B2, DAY).await;
    assert_eq!(own["branch"], B2, "{own}");
    room.finish().await;
}

// ------------------------------------------------------------ §9 row 10: append-only

/// §9 "`UPDATE` or `DELETE` on `reconciliation_certifications`": refused by EX-2's trigger, on a row the command
/// wrote.
#[actix_web::test]
async fn a_certification_cannot_be_updated_or_deleted() {
    let room = Room::start().await;
    room.exchange(Ex::default()).await;
    let (_, a) = certify_current(&room, "kept").await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    for sql in [
        "UPDATE reconciliation_certifications SET note = 'changed'",
        "DELETE FROM reconciliation_certifications",
    ] {
        let err = sqlx::query(sql).execute(&room.pool).await.expect_err(sql);
        assert!(err.to_string().contains("append-only"), "{sql}: {err}");
    }
    let note: Option<String> = sqlx::query_scalar("SELECT note FROM reconciliation_certifications")
        .fetch_one(&room.pool)
        .await
        .expect("the row stays");
    assert_eq!(note.as_deref(), Some("kept"));
    room.finish().await;
}

// ------------------------------------------------------------ §9 row 11: UnavailableParts

/// §9 "`UnavailableParts`": until FN-2, counts, transfers, integrity and reversalsPending read `available: false`,
/// each naming the ticket it waits for, with no data.
#[actix_web::test]
async fn the_four_fn2_parts_are_unavailable_with_their_tickets() {
    let room = Room::start().await;
    let s = sheet(&room, CERTIFIER, B1, DAY).await;
    for (part, ticket) in WAITING {
        let p = &s[part];
        assert_eq!(p["available"], false, "{part}: {s}");
        let reason = p["reason"]
            .as_str()
            .unwrap_or_else(|| panic!("{part}.reason is a string: {s}"));
        assert!(reason.contains(ticket), "{part} waits for {ticket}: {s}");
        assert!(p["data"].is_null(), "{part}.data: {s}");
    }
    room.finish().await;
}

// ------------------------------------------------------------ Q2: the exceptions part

/// Q2: the exceptions part lists the branch's open rows only, whatever their day: a custody-shape OPEN row and a
/// new-shape row (open, `summary` = its id), not a REVIEWED one. Every listed row reads `reviewed: false`.
#[actix_web::test]
async fn the_exceptions_part_lists_open_rows_only() {
    let room = Room::start().await;
    let at = 1_791_263_520_000_i64;
    let open = room
        .exception(json!({ "requestId": unique("r"), "status": "OPEN", "note": "short", "at": at }))
        .await;
    room.exception(json!({
        "requestId": unique("r"), "status": "REVIEWED", "note": "done", "at": at,
        "reviewedBy": "fin", "reviewedAt": at
    }))
    .await;
    let new_shape = room
        .exception(json!({ "countId": unique("count"), "currency": "USD", "difference": "-25.00" }))
        .await;

    for day in [DAY, "2026-10-06"] {
        let s = sheet(&room, CERTIFIER, B1, day).await;
        assert_eq!(s["exceptions"]["available"], true, "{s}");
        let rows = s["exceptions"]["data"]
            .as_array()
            .unwrap_or_else(|| panic!("exceptions.data: {s}"));
        let mut ids: Vec<&str> = rows
            .iter()
            .map(|r| r["id"].as_str().unwrap_or(""))
            .collect();
        ids.sort_unstable();
        let mut want = vec![open.as_str(), new_shape.as_str()];
        want.sort_unstable();
        assert_eq!(ids, want, "{day}: {s}");
        for r in rows {
            assert_eq!(r["reviewed"], false, "{r}");
            assert!(r["position"].is_number(), "{r}");
        }
        let fresh = rows
            .iter()
            .find(|r| r["id"] == new_shape.as_str())
            .expect("the new-shape row");
        assert_eq!(fresh["summary"], new_shape.as_str(), "the id only: {fresh}");
    }
    room.finish().await;
}

/// Q2: the exceptions part is not in the certification hash. A new exception leaves the hash and a certification
/// as they were.
#[actix_web::test]
async fn an_exception_does_not_move_the_hash_or_stale_a_certification() {
    let room = Room::start().await;
    room.exchange(Ex::default()).await;
    let (hash, a) = certify_current(&room, "").await;
    assert_eq!(a.status, StatusCode::OK, "{}", a.text);
    room.exception(json!({ "countId": unique("count"), "currency": "USD", "difference": "1.00" }))
        .await;
    let s = sheet(&room, CERTIFIER, B1, DAY).await;
    assert_eq!(
        s["exceptions"]["data"].as_array().map(Vec::len),
        Some(1),
        "{s}"
    );
    assert_eq!(hash_of(&s), hash, "{s}");
    assert_eq!(certifications_of(&s)[0]["stale"], false, "{s}");
    room.finish().await;
}

// ------------------------------------------------------------ §3.6: the policy rows

/// §3.6: both routes need `FinanceReconcile`.
#[test]
fn both_routes_need_finance_reconcile() {
    for action in ["financeReconciliation", "reconciliationCertify"] {
        assert_eq!(
            Policy::need(EntryPath::Rpc, &Action::new(action)),
            Some(Need::Permission(Permission::FinanceReconcile)),
            "{action}"
        );
    }
}

/// Q3, the timezone half (dell-claude-3's review, issuecomment-6110350440): the day is read in the branch's
/// timezone, not in UTC. A count made at 00:30 local on DAY is 17:30 UTC on the day before. It blocks its
/// verifier on DAY, and not on the day before; a reader of `counted_at`'s UTC date gets both the other way round.
#[actix_web::test]
async fn a_count_after_local_midnight_belongs_to_the_local_day_not_the_utc_one() {
    let room = Room::start().await;
    let counted = bangkok(DAY, 0, 30);
    assert_eq!(
        counted.date_naive(),
        date("2026-10-04"),
        "the seed is on the day before in UTC"
    );
    room.verified_count(VERIFIER, counted, bangkok(DAY, 0, 45))
        .await;

    let hash = hash_of(&sheet(&room, VERIFIER, B1, DAY).await);
    let a = certify(
        &room,
        VERIFIER,
        certify_body(B1, DAY, &hash, ""),
        &unique("key"),
    )
    .await;
    assert_refused(
        &a,
        StatusCode::CONFLICT,
        "samePerson",
        "counted at 00:30 local on DAY",
    );

    let before = "2026-10-04";
    let hash = hash_of(&sheet(&room, VERIFIER, B1, before).await);
    let a = certify(
        &room,
        VERIFIER,
        certify_body(B1, before, &hash, ""),
        &unique("key"),
    )
    .await;
    assert_eq!(
        a.status,
        StatusCode::OK,
        "the UTC date of the count is not its business date: {}",
        a.text
    );
    assert_eq!(room.certifications().await, 1);
    room.finish().await;
}
