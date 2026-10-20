//! Up-front tests for the TX-9 child "my_transactions reads the lines"
//! (`api/src/presentation/http/v3/views/my_transactions.rs`, customer history C04,
//! `docs/design/blueprint/customer-app.md` §4.4). Written before the implementation by the
//! ticket author for the BE lane controller claude/7ca80c. **Do not edit, delete or weaken this
//! file**; add tests in a file of your own.
//!
//! Controller's ruling (2026-10-15), option (b):
//! - one history item per transaction;
//! - the descriptive fields (`type`, `currency`, `amount`, `rate`) come from the line with
//!   `line_no = 1`;
//! - the money fields (`principal`, `fee`, `settlement`) are summed over all the lines;
//! - the settlement is the lines' `settle_thb`, so a BUY shows principal − fee (today
//!   `my_transactions.rs:79` adds the fee for both sides);
//! - the view reads none of the wide `transactions` columns `type`, `code`, `amount`, `rate`,
//!   `principal`, `fee`.
//!
//! Every exchange is saved through `PgTransactionStore` (DRAFT → CONFIRMED → COMPLETED), which
//! writes line 1 and payment 1. The store refuses a second line, so a further line is added with a
//! plain SQL insert into `transaction_lines`.
//!
//! | Test | On `origin/main` `1eba5943` |
//! |---|---|
//! | `a_one_line_sell_shows_its_line` | green: a pin on today's SELL item |
//! | `a_one_line_buy_settles_principal_minus_fee` | red: settlement `3400.00` |
//! | `a_two_line_exchange_is_one_item_with_summed_money_and_line_ones_figures` | red: USD, `4000.00` |
//! | `a_reversed_exchange_is_reversed_and_a_cancelled_one_is_absent` | green: a pin |
//! | `the_history_answers_with_the_wide_figure_columns_dropped` | red: 500, column does not exist |
//! | `no_query_in_the_view_names_a_wide_transactions_column` | red: `t.type`, `t.code`, … |
//!
//! Needs `TEST_DATABASE_URL` naming a database whose name ends in `_test`.

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
use rust_decimal::{Decimal, RoundingStrategy};
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
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const VIEW_PATH: &str = "/api/v3/views/my_transactions";
const VIEW_SOURCE: &str = "src/presentation/http/v3/views/my_transactions.rs";

const MAIN: &str = "MAIN";
const ORG: &str = "00000000-0000-0000-0000-000000000001";
const CX: &str = "CX-TX9-1";

/// The wide `transactions` columns migration 3 drops that this view read on `main`.
const WIDE: [&str; 6] = ["type", "code", "amount", "rate", "principal", "fee"];

static NEXT: AtomicU64 = AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970")
        .as_nanos();
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{nanos}_{n}", std::process::id())
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap_or_else(|e| panic!("a decimal {s:?}: {e}"))
}

fn money(d: Decimal) -> Money {
    let mut cents = d.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    cents.rescale(2);
    Money::parse(&cents.to_string()).unwrap_or_else(|e| panic!("money {cents}: {e:?}"))
}

fn at(minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 12, 8, minute, 0).unwrap()
}

// ------------------------------------------------------------ database

struct Room {
    pool: PgPool,
    schema: String,
}

/// One line of a seeded exchange (unit size 1): `thb = quantity × rate`, the settlement is
/// `thb − fee` for a BUY and `thb + fee` for a SELL, as the desk writes them.
struct L {
    side: Side,
    currency: &'static str,
    quantity: &'static str,
    rate: &'static str,
    fee: &'static str,
    line_no: i32,
}

impl L {
    fn thb(&self) -> Money {
        money(dec(self.quantity) * dec(self.rate))
    }

    fn settle(&self) -> Money {
        let thb = dec(&self.thb().to_string());
        money(match self.side {
            Side::Buy => thb - dec(self.fee),
            Side::Sell => thb + dec(self.fee),
        })
    }

    fn line(&self) -> TransactionLine {
        TransactionLine {
            id: Uuid::new_v4(),
            quote_line_id: None,
            shop_side: self.side,
            currency: CurrencyCode::parse(self.currency).expect("a currency"),
            quantity: ForeignAmount::from_decimal(dec(self.quantity)),
            unit_size: UnitSize::parse("1").expect("unit 1"),
            rate_snapshot: Rate::parse(self.rate).expect("a rate"),
            reference_rate: None,
            thb_amount: self.thb(),
            fee_thb: money(dec(self.fee)),
            settle_thb: self.settle(),
            cost_basis_thb: None,
            realized_margin_thb: None,
            line_no: self.line_no,
        }
    }

    fn side(&self) -> &'static str {
        match self.side {
            Side::Buy => "BUY",
            Side::Sell => "SELL",
        }
    }
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
        let schema = unique("tx9_my_tx");
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

        let room = Self { pool, schema };
        room.exec(
            "INSERT INTO branches (id, name) VALUES ('MAIN', 'MAIN') ON CONFLICT (id) DO NOTHING",
        )
        .await;
        room.exec(&format!(
            "INSERT INTO customers (id, organization_id, customer_no, display_name, mode, created_at) \
             VALUES ('{CX}', '{ORG}', '{CX}', 'Somchai', 'LIVE', now())"
        ))
        .await;
        room.exec(
            "WITH u AS (INSERT INTO users (organization_id, name, status, created_by) \
                 VALUES ('00000000-0000-0000-0000-000000000001'::uuid, 'Teller MAIN', 'ACTIVE', 'test') \
                 RETURNING id) \
             INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
             SELECT 'SH-MAIN', 'MAIN', 'OPEN', now(), 'test', 'req-open-main', u.id FROM u",
        )
        .await;
        room.exec(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) VALUES \
             ('PASSPORT-TX9', 'MAIN', 'test', 'image/jpeg', 'sha-p', now()), \
             ('RECEIPT-TX9', 'MAIN', 'test', 'text/html', 'sha-r', now())",
        )
        .await;
        room
    }

    async fn exec(&self, sql: &str) {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// Saves one exchange of customer `CX` at MAIN through `PgTransactionStore`, with the first of
    /// `lines` written by the store, then moves it to `status` (COMPLETED or CANCELLED). Every
    /// further line is added with an SQL insert. Returns the transaction id.
    async fn seed(&self, lines: &[L], status: TransactionStatus, minute: u32) -> Uuid {
        let t = at(minute);
        let id = TransactionId(Uuid::new_v4());
        let settle: Decimal = lines.iter().map(|l| dec(&l.settle().to_string())).sum();
        let entity = ExchangeTransaction {
            id,
            organization_id: Uuid::parse_str(ORG).expect("org"),
            branch_id: BranchId::new(MAIN),
            mode: Mode::Live,
            status: TransactionStatus::Draft,
            document_ref: None,
            quote_id: None,
            booking_id: None,
            shift_id: Some("SH-MAIN".to_owned()),
            submission_id: format!("sub-tx9-{}", id.0),
            payload_hash: "hash".to_owned(),
            customer_id: CX.to_owned(),
            customer_name: "Somchai".to_owned(),
            nationality: "THA".to_owned(),
            staff_id: "teller-1".to_owned(),
            booking_no: "WALK-IN".to_owned(),
            notes: String::new(),
            value_date: NaiveDate::from_ymd_opt(2026, 10, 12).expect("a day"),
            recorded_at: t,
            recorded_by: "teller-1".to_owned(),
            completed_at: None,
            cancelled_at: None,
            cancel_reason: None,
            approval_request_id: None,
            passport_file_key: "PASSPORT-TX9".to_owned(),
            kiosk_request_id: None,
            staff_draft_id: None,
            count_confirmed: false,
            lines: vec![lines[0].line()],
            payment: TransactionPayment {
                id: Uuid::new_v4(),
                payment_no: 1,
                method: PaymentMethod::Cash,
                reference: String::new(),
                account: "drawer".to_owned(),
                slip_file_key: None,
                amount_thb: money(settle),
                bank_fee_thb: money(Decimal::ZERO),
                paid_at: t,
            },
            receipts: vec![],
        };

        let store = PgTransactionStore::new(self.pool.clone());
        let mut db = self.pool.begin().await.expect("begin");
        store
            .insert_draft_in(&mut db, &entity)
            .await
            .expect("insert the draft");
        match status {
            TransactionStatus::Completed => {
                store
                    .set_status_in(
                        &mut db,
                        id,
                        StatusMove {
                            from: TransactionStatus::Draft,
                            to: TransactionStatus::Confirmed,
                            reason: None,
                            approval_request_id: None,
                        },
                        t,
                    )
                    .await
                    .expect("confirm");
                store
                    .complete_in(&mut db, id, "RECEIPT-TX9", t)
                    .await
                    .expect("complete");
            }
            TransactionStatus::Cancelled => {
                store
                    .set_status_in(
                        &mut db,
                        id,
                        StatusMove {
                            from: TransactionStatus::Draft,
                            to: TransactionStatus::Cancelled,
                            reason: Some("customer cancelled".to_owned()),
                            approval_request_id: None,
                        },
                        t,
                    )
                    .await
                    .expect("cancel");
            }
            other => panic!("not used here: {other:?}"),
        }
        db.commit().await.expect("commit");

        for l in &lines[1..] {
            let line = l.line();
            sqlx::query(
                "INSERT INTO transaction_lines (id, organization_id, transaction_id, shop_side, \
                    currency, quantity, unit_size, rate_snapshot, thb_amount, fee_thb, settle_thb, \
                    line_no, created_at, updated_at) \
                 VALUES ($1, $2::uuid, $3, $4, $5, $6::numeric, 1, $7::numeric, $8::numeric, \
                    $9::numeric, $10::numeric, $11, $12, $12)",
            )
            .bind(line.id)
            .bind(ORG)
            .bind(id.0)
            .bind(l.side())
            .bind(l.currency)
            .bind(l.quantity)
            .bind(l.rate)
            .bind(line.thb_amount.to_string())
            .bind(line.fee_thb.to_string())
            .bind(line.settle_thb.to_string())
            .bind(line.line_no)
            .bind(t)
            .execute(&self.pool)
            .await
            .unwrap_or_else(|e| panic!("insert line {}: {e}", line.line_no));
        }
        id.0
    }

    /// Posts a reversal of the completed exchange `tx_id`, as `v3_my_transactions_test.rs` does.
    async fn reverse(&self, tx_id: Uuid) {
        let (rev_id, req_id, post_id, rec_id) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let when = "2026-10-12T09:00:00Z";
        self.exec(&format!(
            "INSERT INTO approval_requests (id, organization_id, type, branch_id, object_id, \
                object_version_hash, policy_version, proposed_payload, requested_by, requested_at, \
                status, idempotency_key) \
             VALUES ('{req_id}', '{ORG}', 'TRANSACTION_REVERSAL', 'MAIN', '{rev_id}', 'h', '1', \
                '{{}}', 'teller-1', '{when}', 'APPROVED', 'k-{req_id}')"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
             VALUES ('RECEIPT-REV-{rev_id}', 'MAIN', 'test', 'text/html', 'sha-rev', now())"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO receipts (id, organization_id, transaction_id, number, revision, issued_at, \
                document_ref, kind, created_at, updated_at) \
             VALUES ('{rec_id}', '{ORG}', '{tx_id}', 'REV-{rev_id}', 2, '{when}', \
                'RECEIPT-REV-{rev_id}', 'REVERSAL', '{when}', '{when}')"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO ledger_postings (id, organization_id, branch_id, source_type, source_id, \
                source_key, status, posted_at, posted_by) \
             VALUES ('{post_id}', '{ORG}', 'MAIN', 'REVERSAL', '{rev_id}', 'reversal', 'POSTED', \
                '{when}', 'teller-1')"
        ))
        .await;
        self.exec(&format!(
            "INSERT INTO reversals (id, organization_id, branch_id, original_transaction_id, status, \
                reason, amount_thb, business_date, approval_request_id, posting_id, receipt_id, \
                requested_by, requested_at, posted_by, posted_at, created_at, updated_at) \
             VALUES ('{rev_id}', '{ORG}', 'MAIN', '{tx_id}', 'POSTED', 'Customer request', \
                1, '2026-10-12', '{req_id}', '{post_id}', '{rec_id}', 'teller-1', \
                '{when}', 'teller-1', '{when}', '{when}', '{when}')"
        ))
        .await;
    }

    async fn document_ref(&self, tx_id: Uuid) -> String {
        sqlx::query_scalar("SELECT document_ref FROM transactions WHERE id = $1")
            .bind(tx_id)
            .fetch_one(&self.pool)
            .await
            .expect("read the document ref")
    }

    async fn customer(&self) -> String {
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG)
        .bind(unique("u"))
        .fetch_one(&self.pool)
        .await
        .expect("insert a user");
        format!("{id}|CUSTOMER|{CX}")
    }

    /// The customer's history items, asserting a 200.
    async fn history(&self) -> Vec<Value> {
        let who = self.customer().await;
        let (status, body) = call(&self.pool, &who).await;
        assert_eq!(status, StatusCode::OK, "GET {VIEW_PATH}: {body}");
        body.pointer("/data/transactions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| panic!("data.transactions in {body}"))
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

// ------------------------------------------------------------ who is signed in

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, customer) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        ORG,
        vec![ScopedRole::new(role, None::<BranchId>, from, None)],
        (!customer.is_empty()).then_some(customer),
    ))
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn sessions<'a>(
        &'a self,
        _req: &'a HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Sessions> + 'a>> {
        Box::pin(async {
            Sessions {
                owner: None,
                staff: None,
            }
        })
    }

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

async fn call(pool: &PgPool, who: &str) -> (StatusCode, Value) {
    let state = AppState::new(Arc::new(common::settings(&[])), pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri(VIEW_PATH)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who.to_owned()))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, body)
}

fn item(items: &[Value], id: Uuid) -> &Value {
    let id = id.to_string();
    let found: Vec<&Value> = items.iter().filter(|i| i["id"] == id.as_str()).collect();
    assert_eq!(
        found.len(),
        1,
        "exactly one history item for transaction {id}: {items:?}"
    );
    found[0]
}

/// The figures of one item: `{type, currency, amount, rate, principal, fee, settlement, status}`.
fn figures(item: &Value) -> Value {
    json!({
        "type": item["type"],
        "currency": item["currency"],
        "amount": item["amount"],
        "rate": item["rate"],
        "principal": item["principal"],
        "fee": item["fee"],
        "settlement": item["settlement"],
        "status": item["status"],
    })
}

// ------------------------------------------------------------ tests

/// Pin (green on main): a one-line SELL (USD 100 at 40, fee 20.00) shows its line, with today's
/// formatting: the amount and money to two places, the rate as the stored `numeric(20,8)`,
/// settlement = principal + fee, branch and receipt ref.
#[actix_web::test]
async fn a_one_line_sell_shows_its_line() {
    let room = Room::start().await;
    let id = room
        .seed(
            &[L {
                side: Side::Sell,
                currency: "USD",
                quantity: "100",
                rate: "40",
                fee: "20",
                line_no: 1,
            }],
            TransactionStatus::Completed,
            1,
        )
        .await;
    let receipt = room.document_ref(id).await;

    let items = room.history().await;
    assert_eq!(items.len(), 1, "{items:?}");
    let it = item(&items, id);
    assert_eq!(
        figures(it),
        json!({
            "type": "SELL",
            "currency": "USD",
            "amount": "100.00",
            "rate": "40.00000000",
            "principal": "4000.00",
            "fee": "20.00",
            "settlement": "4020.00",
            "status": "COMPLETED",
        })
    );
    assert_eq!(it["branch"], json!(MAIN), "{it}");
    assert_eq!(it["receiptRef"], json!(receipt), "{it}");
    assert_eq!(it["date"], json!("2026-10-12"), "{it}");

    room.finish().await;
}

/// A one-line BUY (USD 100 at 33.50, principal 3,350.00, fee 50.00): the customer receives
/// principal − fee, so the settlement is 3,300.00, the line's `settle_thb`.
/// Red on main: `my_transactions.rs:79` computes principal + fee = 3,400.00 for both sides.
#[actix_web::test]
async fn a_one_line_buy_settles_principal_minus_fee() {
    let room = Room::start().await;
    let id = room
        .seed(
            &[L {
                side: Side::Buy,
                currency: "USD",
                quantity: "100",
                rate: "33.50",
                fee: "50",
                line_no: 1,
            }],
            TransactionStatus::Completed,
            1,
        )
        .await;

    let items = room.history().await;
    assert_eq!(
        figures(item(&items, id)),
        json!({
            "type": "BUY",
            "currency": "USD",
            "amount": "100.00",
            "rate": "33.50000000",
            "principal": "3350.00",
            "fee": "50.00",
            "settlement": "3300.00",
            "status": "COMPLETED",
        }),
        "a BUY settles principal − fee"
    );

    room.finish().await;
}

/// Ruling (b): a transaction with two lines is **one** item. `type`, `currency`, `amount` and
/// `rate` come from `line_no = 1` (EUR 50 at 38), and `principal`, `fee` and `settlement` are
/// the sums over both lines: 1,900.00 + 4,000.00, 10.00 + 20.00, 1,910.00 + 4,020.00.
/// The store writes the USD line as `line_no = 2` and the EUR line is inserted afterwards as
/// `line_no = 1`, so line 1 is chosen by its number, not by insertion order.
/// Red on main: the wide row carries the USD line only (currency USD, principal 4,000.00).
#[actix_web::test]
async fn a_two_line_exchange_is_one_item_with_summed_money_and_line_ones_figures() {
    let room = Room::start().await;
    let id = room
        .seed(
            &[
                L {
                    side: Side::Sell,
                    currency: "USD",
                    quantity: "100",
                    rate: "40",
                    fee: "20",
                    line_no: 2,
                },
                L {
                    side: Side::Sell,
                    currency: "EUR",
                    quantity: "50",
                    rate: "38",
                    fee: "10",
                    line_no: 1,
                },
            ],
            TransactionStatus::Completed,
            1,
        )
        .await;

    let items = room.history().await;
    assert_eq!(items.len(), 1, "one item per transaction: {items:?}");
    assert_eq!(
        figures(item(&items, id)),
        json!({
            "type": "SELL",
            "currency": "EUR",
            "amount": "50.00",
            "rate": "38.00000000",
            "principal": "5900.00",
            "fee": "30.00",
            "settlement": "5930.00",
            "status": "COMPLETED",
        }),
        "line 1 describes the item, the money is summed over the lines"
    );

    room.finish().await;
}

/// Pin (green on main): a completed exchange with a POSTED reversal is still listed, once, as
/// `REVERSED`, with its own figures; a CANCELLED exchange of the same customer is not listed.
#[actix_web::test]
async fn a_reversed_exchange_is_reversed_and_a_cancelled_one_is_absent() {
    let room = Room::start().await;
    let sell = || L {
        side: Side::Sell,
        currency: "USD",
        quantity: "100",
        rate: "40",
        fee: "20",
        line_no: 1,
    };
    let reversed = room.seed(&[sell()], TransactionStatus::Completed, 1).await;
    room.reverse(reversed).await;
    let cancelled = room.seed(&[sell()], TransactionStatus::Cancelled, 2).await;

    let items = room.history().await;
    assert_eq!(items.len(), 1, "only the completed exchange: {items:?}");
    assert!(
        items
            .iter()
            .all(|i| i["id"] != cancelled.to_string().as_str()),
        "a CANCELLED exchange is not history: {items:?}"
    );
    assert_eq!(
        figures(item(&items, reversed)),
        json!({
            "type": "SELL",
            "currency": "USD",
            "amount": "100.00",
            "rate": "40.00000000",
            "principal": "4000.00",
            "fee": "20.00",
            "settlement": "4020.00",
            "status": "REVERSED",
        })
    );

    room.finish().await;
}

/// The view reads no wide figure column: with `type`, `code`, `amount`, `rate`, `principal` and
/// `fee` dropped from `transactions` (as TX-9's migration 3 will), the history still answers 200
/// with the line's figures.
/// Red on main: the SELECT names `t.type`, … and Postgres answers `column does not exist` (500).
#[actix_web::test]
async fn the_history_answers_with_the_wide_figure_columns_dropped() {
    let room = Room::start().await;
    let id = room
        .seed(
            &[L {
                side: Side::Buy,
                currency: "USD",
                quantity: "100",
                rate: "33.50",
                fee: "50",
                line_no: 1,
            }],
            TransactionStatus::Completed,
            1,
        )
        .await;
    let drops: Vec<String> = WIDE
        .iter()
        .map(|c| format!("DROP COLUMN IF EXISTS {c}"))
        .collect();
    room.exec(&format!("ALTER TABLE transactions {}", drops.join(", ")))
        .await;

    let items = room.history().await;
    assert_eq!(
        figures(item(&items, id)),
        json!({
            "type": "BUY",
            "currency": "USD",
            "amount": "100.00",
            "rate": "33.50000000",
            "principal": "3350.00",
            "fee": "50.00",
            "settlement": "3300.00",
            "status": "COMPLETED",
        })
    );

    room.finish().await;
}

/// Static check: no SQL in `my_transactions.rs` names a wide `transactions` column `type`,
/// `code`, `amount`, `rate`, `principal` or `fee`, qualified by an alias of `transactions` or
/// bare, nor selects `*` from it. A name after `AS` is an output alias and is allowed
/// (`l.shop_side AS type`), as are the child tables' own columns (`l.thb_amount`, `l.fee_thb`,
/// `l.rate_snapshot`, …), which are different identifiers.
/// Red on main: `t.type`, `t.code`, `t.amount`, `t.rate`, `t.principal`, `t.fee`.
#[test]
fn no_query_in_the_view_names_a_wide_transactions_column() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(VIEW_SOURCE);
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let sql: Vec<String> = string_literals(&source)
        .into_iter()
        .filter(|s| {
            let up = s.to_uppercase();
            ["SELECT", "FROM ", "JOIN ", "WHERE "]
                .iter()
                .any(|k| up.contains(k))
        })
        .collect();
    assert!(
        sql.iter().any(|s| s.to_uppercase().contains("SELECT")),
        "found no SQL in {VIEW_SOURCE}; this check must see the view's query"
    );
    let mut found = Vec::new();
    for text in &sql {
        found.extend(wide_reads(text));
    }
    assert!(
        found.is_empty(),
        "{VIEW_SOURCE} reads wide `transactions` columns: {found:?}"
    );
}

/// Every string literal of a Rust source (`"…"`, `r"…"`, `r#"…"#`), comments skipped.
fn string_literals(src: &str) -> Vec<String> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else if b[i] == b'\'' && b.get(i + 1) == Some(&b'"') && b.get(i + 2) == Some(&b'\'') {
            i += 3;
        } else if b[i] == b'r' && (b.get(i + 1) == Some(&b'"') || b.get(i + 1) == Some(&b'#')) {
            let mut j = i + 1;
            let mut hashes = 0;
            while b.get(j) == Some(&b'#') {
                hashes += 1;
                j += 1;
            }
            if b.get(j) != Some(&b'"') {
                i += 1;
                continue;
            }
            let close: String = std::iter::once('"')
                .chain(std::iter::repeat_n('#', hashes))
                .collect();
            let start = j + 1;
            let end = src[start..]
                .find(&close)
                .map(|k| start + k)
                .expect("a closed raw string");
            out.push(src[start..end].to_owned());
            i = end + close.len();
        } else if b[i] == b'"' {
            let mut s = String::new();
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                if b[j] == b'\\' {
                    j += 1;
                    // A line continuation (`\` + newline + indentation) joins as nothing.
                    if b.get(j) == Some(&b'\n') {
                        while j + 1 < b.len() && (b[j + 1] == b' ' || b[j + 1] == b'\t') {
                            j += 1;
                        }
                    } else if let Some(&c) = b.get(j) {
                        s.push(c as char);
                    }
                } else {
                    s.push(b[j] as char);
                }
                j += 1;
            }
            out.push(s);
            i = j + 1;
        } else if is_ident(b[i]) {
            // Skip identifiers whole, so the `r` of `for` or `br` never opens a raw string.
            while i < b.len() && is_ident(b[i]) {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    out
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The wide-column reads in one SQL text: `<alias>.<wide>` for an alias of `transactions`, a
/// bare `<wide>` that is not an output alias (`AS <wide>`), and `<alias>.*` / `SELECT *`.
fn wide_reads(sql: &str) -> Vec<String> {
    // Drop `-- comments` and the contents of '…' SQL literals.
    let mut clean = String::new();
    for line in sql.lines() {
        let code = line.split("--").next().unwrap_or("");
        let mut quoted = false;
        for c in code.chars() {
            if c == '\'' {
                quoted = !quoted;
                clean.push(' ');
            } else if !quoted {
                clean.push(c);
            }
        }
        clean.push('\n');
    }
    // Tokens: identifiers, and single punctuation characters.
    let mut tokens: Vec<String> = Vec::new();
    let mut word = String::new();
    for c in clean.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            if !word.is_empty() {
                tokens.push(std::mem::take(&mut word));
            }
            if !c.is_whitespace() {
                tokens.push(c.to_string());
            }
        }
    }
    if !word.is_empty() {
        tokens.push(word);
    }
    let lower: Vec<String> = tokens.iter().map(|t| t.to_lowercase()).collect();

    const NOT_ALIAS: [&str; 14] = [
        "on", "where", "join", "left", "right", "inner", "full", "cross", "group", "order",
        "limit", "union", "lateral", ",",
    ];
    let mut aliases = vec!["transactions".to_owned()];
    for (k, t) in lower.iter().enumerate() {
        if t == "transactions" && (k == 0 || lower[k - 1] != ".") {
            let mut n = k + 1;
            if lower.get(n).map(String::as_str) == Some("as") {
                n += 1;
            }
            if let Some(a) = lower.get(n)
                && a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !NOT_ALIAS.contains(&a.as_str())
            {
                aliases.push(a.clone());
            }
        }
    }

    let mut found = Vec::new();
    for (k, t) in lower.iter().enumerate() {
        let prev = if k > 0 { lower[k - 1].as_str() } else { "" };
        if t == "*" {
            let qualifier = if prev == "." && k >= 2 {
                Some(lower[k - 2].as_str())
            } else {
                None
            };
            if qualifier.is_some_and(|q| aliases.iter().any(|a| a == q)) || prev == "select" {
                found.push(format!(
                    "{}*",
                    qualifier.map(|q| format!("{q}.")).unwrap_or_default()
                ));
            }
            continue;
        }
        if !WIDE.contains(&t.as_str()) {
            continue;
        }
        if prev == "." {
            let q = if k >= 2 { lower[k - 2].as_str() } else { "" };
            if aliases.iter().any(|a| a == q) {
                found.push(format!("{q}.{t}"));
            }
        } else if prev != "as" {
            found.push(t.clone());
        }
    }
    found
}
