//! Triage T3 (2026-11-01 real-staff e2e): an approver cannot see what they approve. Both approval screens are built
//! from rows that hold only ids: the finance sheet's `ReversalPendingRow` (`dto/v3/finance_reconciliation.rs:145-150`,
//! filled from `approval_requests` alone in `application/finance/sheet_parts.rs:394-427`, its `transactionId` holding
//! the reversal id) and the `approval_requests` view (`http/v3/views/approval_requests.rs:61-80`), whose
//! `requestedBy` is a user id and whose subject is the raw payload.
//!
//! Written before the implementation by the ticket author (claude/b091f5e0). Do not edit, delete or weaken; add tests
//! in a file of your own.
//!
//! Pinned here (design `docs/design/blueprint/staff-e2e-1101-reads.md` §2):
//! - every `approval_requests` item carries `requestedByName` (`users.name` of `requested_by`, blank ⇒ `null`) and
//!   `subject`, tagged by `kind`;
//! - a `TRANSACTION_REVERSAL` item's subject is `kind: "reversal"`: the receipt number, amount, reason, customer,
//!   requesting teller, the original's shift and its lines in `line_no` order;
//! - a `COUNT_DIFFERENCE` item's subject is `kind: "countDifference"`: the counter (not the manager who requested),
//!   the count's own kind as `countKind` (the design's `kind` field would collide with the serde tag `kind`), the
//!   shift, the payload's net difference and the count's totals by currency;
//! - `subject` is `null` (key present) for another type and when the reversal no longer exists;
//! - the finance sheet's pending reversal is `{ requestId, createdAt, subject }`, `subject` the same object as the
//!   view's; `transactionId` is gone;
//! - the figures hash takes only `requestId`, `reversalId`, `amountThb` and `createdAt`: renaming the teller or the
//!   customer does not stale a certification, changing the amount does.
//!
//! Callers are Blueprint `access::Actor`s from the `x-test-as` header, as in `finance_reconciliation_test.rs`, whose
//! harness this copies. Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`; nothing skips. Each
//! test works in a schema of its own.

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
use sqlx::{AssertSqlSafe, PgPool, Row};
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
const DAY: &str = "2026-10-05";
const SHIFT: &str = "SH-XC-B01";
const CERTIFY: &str = "/api/v3/commands/reconciliation_certify";

/// FINANCE of B1: reads both screens and certifies.
const FINANCE: &str = "FINANCE@XC-B01#1";
/// The teller who made the sale, asked for its reversal and counted the drawer.
const TELLER: &str = "TELLER@XC-B01#4";
/// The manager who ran verify and so requested the count difference; a user row only, never a caller.
const MANAGER: &str = "MANAGER@XC-B01#6";
/// A user whose name is blank.
const BLANK: &str = "TELLER@XC-B01#7";

const USERS: [(&str, &str); 4] = [
    (FINANCE, "Finance Fah"),
    (TELLER, "Teller Malee"),
    (MANAGER, "Manager Somsak"),
    (BLANK, "   "),
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

/// A JSON money or quantity, as a decimal string or a number.
fn dec(v: &Value) -> Decimal {
    match v {
        Value::String(s) => d(s),
        Value::Number(n) => d(&n.to_string()),
        other => panic!("a decimal, not {other}"),
    }
}

/// A JSON RFC 3339 time.
fn instant(v: &Value) -> DateTime<Utc> {
    let s = v
        .as_str()
        .unwrap_or_else(|| panic!("an RFC 3339 string, not {v}"));
    DateTime::parse_from_rfc3339(s)
        .unwrap_or_else(|e| panic!("{s:?} is RFC 3339: {e}"))
        .with_timezone(&Utc)
}

/// A fixed uuid per caller spec.
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

/// One seeded sale and its pending reversal.
struct Pending {
    transaction: Uuid,
    reversal: Uuid,
    request: Uuid,
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
        let schema = unique("t3");
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
        for (spec, name) in USERS {
            room.exec(
                "INSERT INTO users (id, organization_id, name, status, created_by) \
                 VALUES ($1::uuid, $2::uuid, $3, 'ACTIVE', 'test')",
                &[&user_id(spec), ORG, name],
            )
            .await;
        }
        room.exec(
            "INSERT INTO branches (id, name, organization_id, timezone) \
             VALUES ($1, $1, $2::uuid, 'Asia/Bangkok')",
            &[B1, ORG],
        )
        .await;
        room.exec(
            "INSERT INTO branch_versions (branch_id, version, organization_id, changed_at) \
             VALUES ($1, 1, $2::uuid, now()) ON CONFLICT (branch_id) DO NOTHING",
            &[B1, ORG],
        )
        .await;
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, organization_id, teller_id) \
             VALUES ($1, $2, 'OPEN', $3, 'test', 'open-' || $2, $4::uuid, $5::uuid)",
        )
        .bind(SHIFT)
        .bind(B1)
        .bind(bangkok(DAY, 8, 0))
        .bind(ORG)
        .bind(user_id(TELLER))
        .execute(&room.pool)
        .await
        .expect("an open shift");
        for kind in ["PASSPORT", "RECEIPT"] {
            room.exec(
                "INSERT INTO files (key, organization_id, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ($1, $2::uuid, $3, 'owner', 'application/pdf', 'sha', now())",
                &[&format!("{kind}-T3"), ORG, B1],
            )
            .await;
        }
        room
    }

    async fn exec(&self, sql: &str, binds: &[&str]) {
        let mut q = sqlx::query(AssertSqlSafe(sql.to_owned()));
        for b in binds {
            q = q.bind(*b);
        }
        q.execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// A COMPLETED LIVE sale at B1 in SHIFT for a customer named `customer` (number `customer_no`), with two lines:
    /// line 1 BUY 100 USD and line 2 SELL 50 EUR (line 2 stored first). Returns its uuid.
    async fn sale(&self, customer: &str, customer_no: &str) -> Uuid {
        let customer_id = unique("C");
        self.exec(
            "INSERT INTO customers (id, mode, created_at, organization_id, customer_no, display_name) \
             VALUES ($1, 'LIVE', now(), $2::uuid, $3, $4)",
            &[&customer_id, ORG, customer_no, customer],
        )
        .await;
        let money = |s: &str| Money::parse(s).expect("money");
        let at = bangkok(DAY, 10, 0);
        let line = |no: i32, side: Side, ccy: &str, qty: &str, thb: &str| TransactionLine {
            id: Uuid::new_v4(),
            quote_line_id: None,
            shop_side: side,
            currency: CurrencyCode::parse(ccy).expect("a currency"),
            quantity: ForeignAmount::from_decimal(d(qty)),
            unit_size: UnitSize::parse("1").expect("unit"),
            rate_snapshot: Rate::parse("1").expect("rate"),
            reference_rate: None,
            thb_amount: money(thb),
            fee_thb: money("0.00"),
            settle_thb: money(thb),
            cost_basis_thb: None,
            realized_margin_thb: None,
            line_no: no,
        };
        let id = Uuid::new_v4();
        let t = ExchangeTransaction {
            queue_ticket_id: None,
            id: TransactionId(id),
            organization_id: Uuid::parse_str(ORG).expect("org"),
            branch_id: BranchId::new(B1),
            mode: Mode::Live,
            status: TransactionStatus::Draft,
            document_ref: None,
            quote_id: None,
            booking_id: None,
            shift_id: Some(SHIFT.to_owned()),
            submission_id: unique("sub"),
            payload_hash: "hash".into(),
            customer_id,
            customer_name: customer.to_owned(),
            nationality: "TH".into(),
            staff_id: user_id(TELLER),
            booking_no: String::new(),
            notes: String::new(),
            value_date: date(DAY),
            recorded_at: at,
            recorded_by: user_id(TELLER),
            completed_at: None,
            cancelled_at: None,
            cancel_reason: None,
            approval_request_id: None,
            passport_file_key: "PASSPORT-T3".into(),
            kiosk_request_id: None,
            staff_draft_id: None,
            count_confirmed: false,
            lines: vec![line(2, Side::Sell, "EUR", "50", "1900.00")],
            payment: TransactionPayment {
                id: Uuid::new_v4(),
                payment_no: 1,
                method: PaymentMethod::Cash,
                reference: String::new(),
                account: String::new(),
                slip_file_key: None,
                amount_thb: money("1600.00"),
                bank_fee_thb: money("0.00"),
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
        // A draft holds one line; line 1 is added after it, so the rows are not stored in line_no order.
        sqlx::query(
            "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, currency, quantity, \
               unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, line_no, created_at, updated_at) \
             VALUES ($1, $2::uuid, $3, 'BUY', 'USD', 100, 1, 35, 3500.00, 0, 3500.00, 1, $4, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(ORG)
        .bind(id)
        .bind(at)
        .execute(&mut *tx)
        .await
        .expect("line 1");
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
            .complete_in(&mut tx, t.id, "RECEIPT-T3", at)
            .await
            .expect("complete");
        tx.commit().await.expect("commit");
        id
    }

    /// A REQUESTED reversal of `original` by `requester`, and its PENDING `TRANSACTION_REVERSAL` request (the
    /// request's `object_id` is the reversal id, as `reversal_request.rs:144-146` writes it).
    async fn pending_reversal(
        &self,
        original: Uuid,
        amount: &str,
        reason: &str,
        requester: &str,
        at: DateTime<Utc>,
    ) -> (Uuid, Uuid) {
        let reversal: Uuid = sqlx::query_scalar(
            "INSERT INTO reversals (organization_id, branch_id, original_transaction_id, status, reason, amount_thb, \
               business_date, requested_by, requested_at) \
             VALUES ($1::uuid, $2, $3, 'REQUESTED', $4, $5::numeric, $6, $7, $8) RETURNING id",
        )
        .bind(ORG)
        .bind(B1)
        .bind(original)
        .bind(reason)
        .bind(amount)
        .bind(date(DAY))
        .bind(user_id(requester))
        .bind(at)
        .fetch_one(&self.pool)
        .await
        .expect("a requested reversal");
        let request = self
            .request(
                "TRANSACTION_REVERSAL",
                &reversal.to_string(),
                json!({ "reason": reason }),
                &user_id(requester),
                at,
            )
            .await;
        sqlx::query("UPDATE reversals SET approval_request_id = $1 WHERE id = $2")
            .bind(request)
            .bind(reversal)
            .execute(&self.pool)
            .await
            .expect("link the request");
        (reversal, request)
    }

    /// A sale for "Somchai Jaidee" (C-0001) and its pending reversal of 3500.00, asked for by TELLER.
    async fn seeded_pending(&self) -> Pending {
        let transaction = self.sale("Somchai Jaidee", "C-0001").await;
        let (reversal, request) = self
            .pending_reversal(
                transaction,
                "3500.00",
                "wrong currency keyed",
                TELLER,
                bangkok(DAY, 11, 0),
            )
            .await;
        Pending {
            transaction,
            reversal,
            request,
        }
    }

    /// A PENDING request of `type_name` at B1.
    async fn request(
        &self,
        type_name: &str,
        object_id: &str,
        payload: Value,
        requested_by: &str,
        at: DateTime<Utc>,
    ) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
               proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
             VALUES (gen_random_uuid(), $1, $2, $3, 'h', '1', $4, $5, $6, 'PENDING', $7, $8::uuid) RETURNING id",
        )
        .bind(type_name)
        .bind(B1)
        .bind(object_id)
        .bind(payload)
        .bind(requested_by)
        .bind(at)
        .bind(unique("key"))
        .bind(ORG)
        .fetch_one(&self.pool)
        .await
        .expect("an approval request")
    }

    /// A CLOSE count of SHIFT counted by TELLER, with totals for USD and EUR, and the PENDING `COUNT_DIFFERENCE`
    /// request MANAGER's verify made (`count_service.rs:1295-1305`), payload as `count_difference.rs:236-239`.
    async fn count_difference(&self) -> (Uuid, Uuid) {
        let count: Uuid = sqlx::query_scalar(
            "INSERT INTO cash_counts (organization_id, branch_id, shift_id, kind, counted_by, counted_at, idempotency_key) \
             VALUES ($1::uuid, $2, $3, 'CLOSE', $4::uuid, $5, $6) RETURNING id",
        )
        .bind(ORG)
        .bind(B1)
        .bind(SHIFT)
        .bind(user_id(TELLER))
        .bind(bangkok(DAY, 18, 0))
        .bind(unique("count"))
        .fetch_one(&self.pool)
        .await
        .expect("a count");
        for (ccy, counted, expected, difference) in [
            ("USD", "990.0000", "1000.0000", "-10.0000"),
            ("EUR", "205.0000", "200.0000", "5.0000"),
        ] {
            sqlx::query(
                "INSERT INTO cash_count_totals (count_id, organization_id, currency, counted, expected, difference) \
                 VALUES ($1, $2::uuid, $3, $4::numeric, $5::numeric, $6::numeric)",
            )
            .bind(count)
            .bind(ORG)
            .bind(ccy)
            .bind(counted)
            .bind(expected)
            .bind(difference)
            .execute(&self.pool)
            .await
            .expect("a count total");
        }
        let request = self
            .request(
                "COUNT_DIFFERENCE",
                &count.to_string(),
                json!({ "count_id": count.to_string(), "net_difference_thb": "-152.50" }),
                &user_id(MANAGER),
                bangkok(DAY, 18, 30),
            )
            .await;
        (count, request)
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

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
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

async fn call(room: &Room, method: Method, uri: &str, body: Option<Value>) -> Answer {
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
        .insert_header((AS, FINANCE));
    if let Some(body) = &body {
        req = req
            .insert_header(("x-xchange-request", "1"))
            .insert_header(("origin", SAME_ORIGIN))
            .insert_header(("content-type", "application/json"))
            .insert_header(("idempotency-key", unique("key")))
            .set_payload(body.to_string());
    }
    let resp = atest::call_service(&app, req.to_request()).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// The `approval_requests` view of B1 as FINANCE: its items.
async fn approval_items(room: &Room) -> Vec<Value> {
    let a = call(
        room,
        Method::GET,
        &format!("/api/v3/views/approval_requests?branch={B1}"),
        None,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "approval_requests: {}", a.text);
    a.body
        .pointer("/data/requests")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("no requests list in {}", a.text))
        .clone()
}

fn item_of(items: &[Value], request: Uuid) -> Value {
    items
        .iter()
        .find(|r| r["id"] == request.to_string())
        .unwrap_or_else(|| panic!("request {request} is listed: {items:?}"))
        .clone()
}

/// The `subject` of `item`; panics if the key is missing.
fn subject_of(item: &Value) -> Value {
    assert!(
        item.as_object().expect("an object").contains_key("subject"),
        "every item carries `subject` (design §2): {item}"
    );
    item["subject"].clone()
}

/// The finance sheet of B1 on DAY as FINANCE.
async fn sheet(room: &Room) -> Value {
    let a = call(
        room,
        Method::GET,
        &format!("/api/v3/views/finance_reconciliation?branch={B1}&date={DAY}"),
        None,
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "finance sheet: {}", a.text);
    a.body["data"].clone()
}

fn pending_rows(sheet: &Value) -> Vec<Value> {
    let part = &sheet["reversalsPending"];
    assert_eq!(part["available"], true, "reversalsPending: {part}");
    part["data"]
        .as_array()
        .unwrap_or_else(|| panic!("reversalsPending.data is an array: {part}"))
        .clone()
}

fn hash_of(sheet: &Value) -> String {
    sheet["figuresHash"]
        .as_str()
        .unwrap_or_else(|| panic!("figuresHash is a string: {sheet}"))
        .to_owned()
}

// ------------------------------------------------------------ the approval_requests view

/// Design §2: the reversal subject is read from the reversal, its original transaction, the customer, the requester
/// and the original's shift.
#[actix_web::test]
async fn t3_a_reversal_request_shows_receipt_amount_customer_teller_shift_and_lines() {
    let room = Room::start().await;
    let p = room.seeded_pending().await;

    let receipt: Option<String> =
        sqlx::query_scalar("SELECT document_ref FROM transactions WHERE id = $1")
            .bind(p.transaction)
            .fetch_one(&room.pool)
            .await
            .expect("the original's document_ref");
    let receipt = receipt.expect("a COMPLETED sale has a document_ref");
    let opened: DateTime<Utc> = sqlx::query_scalar("SELECT opened_at FROM shifts WHERE id = $1")
        .bind(SHIFT)
        .fetch_one(&room.pool)
        .await
        .expect("the shift's opened_at");
    let lines = sqlx::query(
        "SELECT shop_side::text AS side, trim(currency::text) AS currency, quantity::text AS quantity \
         FROM transaction_lines WHERE transaction_id = $1 ORDER BY line_no",
    )
    .bind(p.transaction)
    .fetch_all(&room.pool)
    .await
    .expect("the original's lines");
    assert_eq!(lines.len(), 2, "the seeded sale has two lines");

    let item = item_of(&approval_items(&room).await, p.request);
    assert_eq!(
        item["requestedByName"], "Teller Malee",
        "the item names its requester: {item}"
    );
    let s = subject_of(&item);
    assert_eq!(s["kind"], "reversal", "{item}");
    assert_eq!(s["reversalId"], p.reversal.to_string(), "{s}");
    assert_eq!(s["reversalId"], item["objectId"], "{s}");
    assert_eq!(s["originalTransactionId"], p.transaction.to_string(), "{s}");
    assert_eq!(s["receiptNumber"], receipt.as_str(), "{s}");
    assert_eq!(dec(&s["amountThb"]), d("3500.00"), "{s}");
    assert_eq!(s["reason"], "wrong currency keyed", "{s}");
    assert_eq!(s["customerName"], "Somchai Jaidee", "{s}");
    assert_eq!(s["customerNo"], "C-0001", "{s}");
    assert_eq!(s["requestedByName"], "Teller Malee", "{s}");
    assert_eq!(s["shiftId"], SHIFT, "the shift the sale was made in: {s}");
    assert_eq!(instant(&s["shiftOpenedAt"]), opened, "{s}");

    let shown = s["lines"]
        .as_array()
        .unwrap_or_else(|| panic!("lines is an array: {s}"));
    assert_eq!(shown.len(), 2, "{s}");
    for (i, (row, l)) in lines.iter().zip(shown).enumerate() {
        let side: String = row.get("side");
        let currency: String = row.get("currency");
        let quantity: String = row.get("quantity");
        assert_eq!(l["side"], side.as_str(), "line {i} side: {s}");
        assert_eq!(l["currency"], currency.as_str(), "line {i} currency: {s}");
        assert_eq!(dec(&l["quantity"]), d(&quantity), "line {i} quantity: {s}");
    }
    assert_eq!(shown[0]["currency"], "USD", "line_no order: {s}");
    assert_eq!(shown[1]["currency"], "EUR", "line_no order: {s}");
    room.finish().await;
}

/// Design §2: a count difference names the counter (the requester is the manager who verified), the shift, the
/// payload's net difference and the totals by currency.
#[actix_web::test]
async fn t3_a_count_difference_request_shows_the_counter_shift_and_totals() {
    let room = Room::start().await;
    let (count, request) = room.count_difference().await;
    let opened: DateTime<Utc> = sqlx::query_scalar("SELECT opened_at FROM shifts WHERE id = $1")
        .bind(SHIFT)
        .fetch_one(&room.pool)
        .await
        .expect("the shift's opened_at");

    let item = item_of(&approval_items(&room).await, request);
    assert_eq!(
        item["requestedByName"], "Manager Somsak",
        "the requester of a count difference is the manager who ran verify: {item}"
    );
    let s = subject_of(&item);
    assert_eq!(s["kind"], "countDifference", "{item}");
    assert_eq!(s["countId"], count.to_string(), "{s}");
    assert_eq!(
        s["countKind"], "CLOSE",
        "the count's own kind; `kind` is the subject's tag (design §2 names both `kind`): {s}"
    );
    assert_eq!(
        s["countedBy"],
        user_id(TELLER),
        "the counter, not the manager: {s}"
    );
    assert_eq!(s["countedByName"], "Teller Malee", "{s}");
    assert_eq!(s["shiftId"], SHIFT, "{s}");
    assert_eq!(instant(&s["shiftOpenedAt"]), opened, "{s}");
    assert_eq!(
        dec(&s["netDifferenceThb"]),
        d("-152.50"),
        "the payload's net_difference_thb: {s}"
    );
    let lines = s["lines"]
        .as_array()
        .unwrap_or_else(|| panic!("lines is an array: {s}"));
    let shown: Vec<(String, Decimal, Decimal, Decimal)> = lines
        .iter()
        .map(|l| {
            (
                l["currency"].as_str().unwrap_or_default().trim().to_owned(),
                dec(&l["counted"]),
                dec(&l["expected"]),
                dec(&l["difference"]),
            )
        })
        .collect();
    assert_eq!(
        shown,
        vec![
            ("EUR".to_owned(), d("205"), d("200"), d("5")),
            ("USD".to_owned(), d("990"), d("1000"), d("-10")),
        ],
        "cash_count_totals of the count, by currency: {s}"
    );
    room.finish().await;
}

/// Design §2 [required]: `subject` is `null` for every other request type and when its object no longer exists.
#[actix_web::test]
async fn t3_another_type_and_a_missing_reversal_answer_a_null_subject() {
    let room = Room::start().await;
    let other = room
        .request(
            "LEDGER_ADJUSTMENT",
            &Uuid::new_v4().to_string(),
            json!({ "branch": B1 }),
            &user_id(MANAGER),
            bangkok(DAY, 9, 0),
        )
        .await;
    let gone = room
        .request(
            "TRANSACTION_REVERSAL",
            &Uuid::new_v4().to_string(),
            json!({}),
            &user_id(TELLER),
            bangkok(DAY, 9, 30),
        )
        .await;

    let items = approval_items(&room).await;
    let o = item_of(&items, other);
    assert_eq!(subject_of(&o), Value::Null, "another type: {o}");
    assert_eq!(o["requestedByName"], "Manager Somsak", "every type: {o}");
    let g = item_of(&items, gone);
    assert_eq!(
        subject_of(&g),
        Value::Null,
        "the reversal no longer exists: {g}"
    );
    room.finish().await;
}

/// `users.name` blank (spaces or empty) ⇒ `requestedByName: null`, on the item and in the subject.
#[actix_web::test]
async fn t3_a_blank_requester_name_is_null() {
    let room = Room::start().await;
    let transaction = room.sale("Somchai Jaidee", "C-0001").await;
    let (_, request) = room
        .pending_reversal(transaction, "100.00", "typo", BLANK, bangkok(DAY, 11, 0))
        .await;
    room.exec(
        "UPDATE users SET name = '' WHERE id = $1::uuid",
        &[&user_id(MANAGER)],
    )
    .await;
    let (_, count_request) = room.count_difference().await;

    let items = approval_items(&room).await;
    let item = item_of(&items, request);
    assert!(
        item.as_object()
            .expect("an object")
            .contains_key("requestedByName"),
        "every item carries requestedByName: {item}"
    );
    assert_eq!(item["requestedByName"], Value::Null, "spaces only: {item}");
    let s = subject_of(&item);
    assert_eq!(s["kind"], "reversal", "{item}");
    assert_eq!(s["requestedByName"], Value::Null, "spaces only: {s}");
    let c = item_of(&items, count_request);
    assert_eq!(c["requestedByName"], Value::Null, "empty name: {c}");
    room.finish().await;
}

/// F17a: a request with `requested_by = 'SYSTEM:...'` shows the actor id itself as `requestedByName`.
#[actix_web::test]
async fn t3_f17a_system_requester_actor_names_itself() {
    let room = Room::start().await;
    let request = room
        .request(
            "LEDGER_ADJUSTMENT",
            &Uuid::new_v4().to_string(),
            json!({ "branch": B1 }),
            "SYSTEM:rate-feed:BOT",
            bangkok(DAY, 9, 0),
        )
        .await;

    let items = approval_items(&room).await;
    let item = item_of(&items, request);
    assert_eq!(
        item["requestedByName"], "SYSTEM:rate-feed:BOT",
        "system actor id is used when no user matches: {item}"
    );
    room.finish().await;
}

/// F17d: a request decided by a `SYSTEM:*` approver shows the actor id itself as `approverName`.
#[actix_web::test]
async fn t3_f17d_system_approver_actor_names_itself() {
    let room = Room::start().await;
    let request = room
        .request(
            "LEDGER_ADJUSTMENT",
            &Uuid::new_v4().to_string(),
            json!({ "branch": B1 }),
            &user_id(MANAGER),
            bangkok(DAY, 9, 0),
        )
        .await;
    room.exec(
        "UPDATE approval_requests SET status = 'APPROVED' WHERE id = $1::uuid",
        &[&request.to_string()],
    )
    .await;
    room.exec(
        "INSERT INTO approval_decisions (id, request_id, approver_id, approver_role, outcome, reason, at) \
         VALUES (gen_random_uuid(), $1::uuid, 'SYSTEM:pricing-follow', 'SYSTEM', 'APPROVED', 'auto', now())",
        &[&request.to_string()],
    )
    .await;

    let items = approval_items(&room).await;
    let item = item_of(&items, request);
    let decision = &item["decision"];
    assert_eq!(
        decision["approverName"], "SYSTEM:pricing-follow",
        "system approver actor id is used when no user matches: {item}"
    );
    room.finish().await;
}

// ------------------------------------------------------------ the finance sheet

/// Design §2: `ReversalPendingRow` is `{ requestId, createdAt, subject }`, `subject` the view's `ReversalSubject`.
#[actix_web::test]
async fn t3_the_sheet_pending_reversal_carries_the_same_subject() {
    let room = Room::start().await;
    let p = room.seeded_pending().await;

    let rows = pending_rows(&sheet(&room).await);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    let mut keys: Vec<&str> = row
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["createdAt", "requestId", "subject"],
        "the misnamed transactionId and the bare requestedBy are gone: {row}"
    );
    assert_eq!(row["requestId"], p.request.to_string(), "{row}");
    assert_eq!(instant(&row["createdAt"]), bangkok(DAY, 11, 0), "{row}");

    // The view's subject is the tagged `ApprovalSubject` (`kind: "reversal"`); the sheet's is the bare
    // `ReversalSubject`. Everything but the tag is the same object.
    let untagged = |v: &Value| {
        let mut m = v
            .as_object()
            .unwrap_or_else(|| panic!("a subject object, not {v}"))
            .clone();
        m.remove("kind");
        Value::Object(m)
    };
    let item = item_of(&approval_items(&room).await, p.request);
    let view = subject_of(&item);
    assert_eq!(view["kind"], "reversal", "{item}");
    assert_eq!(
        untagged(&row["subject"]),
        untagged(&view),
        "one ReversalSubject for both screens"
    );
    room.finish().await;
}

/// Design §2 [required]: the figures hash takes, per pending row, only requestId, reversalId, amountThb and
/// createdAt. Renaming the teller or the customer leaves the certification current; changing the amount stales it.
#[actix_web::test]
async fn t3_names_do_not_move_the_figures_hash_but_the_amount_does() {
    let room = Room::start().await;
    let p = room.seeded_pending().await;

    let before = sheet(&room).await;
    let hash = hash_of(&before);
    let a = call(
        &room,
        Method::POST,
        CERTIFY,
        Some(json!({ "branch": B1, "date": DAY, "figuresHash": hash, "note": "t3" })),
    )
    .await;
    assert_eq!(a.status, StatusCode::OK, "certify: {}", a.text);

    room.exec(
        "UPDATE users SET name = 'Renamed' WHERE id = $1::uuid",
        &[&user_id(TELLER)],
    )
    .await;
    room.exec(
        "UPDATE customers SET display_name = 'Renamed' \
         WHERE id = (SELECT customer_id FROM transactions WHERE id = $1::uuid)",
        &[&p.transaction.to_string()],
    )
    .await;
    let renamed = sheet(&room).await;
    let row = &pending_rows(&renamed)[0];
    assert_eq!(
        row["subject"]["requestedByName"], "Renamed",
        "the sheet shows the new name: {row}"
    );
    assert_eq!(
        row["subject"]["customerName"], "Renamed",
        "the sheet shows the new customer name: {row}"
    );
    assert_eq!(hash_of(&renamed), hash, "names are not figures");
    assert_eq!(
        renamed["certifications"][0]["stale"], false,
        "a rename does not stale a certification: {renamed}"
    );

    room.exec(
        "UPDATE reversals SET amount_thb = 3400.00 WHERE id = $1::uuid",
        &[&p.reversal.to_string()],
    )
    .await;
    let changed = sheet(&room).await;
    assert_ne!(hash_of(&changed), hash, "the amount is a figure");
    assert_eq!(
        changed["certifications"][0]["stale"], true,
        "a changed amount stales the certification: {changed}"
    );
    room.finish().await;
}
