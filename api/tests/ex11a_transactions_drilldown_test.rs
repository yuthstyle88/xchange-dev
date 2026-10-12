//! Ticket #1103 (EX-11a): the A01 drill-down to the transaction list keeps the currency and channel the executive
//! chose — `GET /api/v3/views/admin_transactions` gains `currency` and `channel` (design
//! `docs/design/blueprint/executive-finance.md` §4.4; the lane controller's G1–G3 ruling on #1071,
//! issuecomment-6098404213, and its EX-11 rulings on #1104).
//!
//! Written before the implementation by the up-front test-writer claude/07d3765f, on the BE lane controller
//! claude/ad05bf8d's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! **What is pinned** (ticket §7–§9):
//! - `currency`: three upper-case ASCII letters, matched against the exchange's **line** currency (TX-8a reads
//!   `transaction_lines`). Anything else is `400 invalidRequest`, field `currency`. A well-formed code never traded
//!   is an empty page, not an error.
//! - `channel`: `WalkIn` | `Booking` | `Kiosk` | `Referred`, classified by #1071's `Channel::condition`. A referred
//!   booking is under both `Booking` and `Referred`. Anything else is `400 invalidRequest`, field `channel`.
//! - The new keys combine with the period and `type` by AND. A request with neither answers as before: every
//!   seeded row, each with exactly `TransactionRow`'s fields.
//! - `DRILLDOWN_KEYS` in `dto/v3/admin_transactions.rs` holds `currency` and `channel` (G2), and A01's drill-down to
//!   this view now passes them in `filter`, with nothing `dropped`.
//!
//! `Room::exchange` (EX-3's harness, with a `currency`) overwrites the wide `transactions` row with other figures and
//! `code = 'EUR'`, so a filter on the wide columns goes red. Readers are the organisation-wide EXECUTIVE (A01's own
//! reader, who holds `CompatRead`). Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips.

mod common;

use std::collections::BTreeMap;
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
use xc_auth::domain::policy::value_objects::role::Role as OldRole;
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
use xchange::application::executive::drilldown::accepted_keys;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader, Sessions};

const MUST_FINISH: Duration = Duration::from_secs(30);
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
/// The organisation the migrations seed, with B1 and B2.
const ORG: &str = "00000000-0000-0000-0000-000000000001";
/// Another organisation, with B9.
const ORG2: &str = "00000000-0000-0000-0000-0000000000f2";
const B1: &str = "XC-B01";
const B2: &str = "XC-B02";
const B9: &str = "XC-B90";
const OVERVIEW: &str = "/api/executive/overview";

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
    let d = date(day);
    Utc.from_utc_datetime(&d.and_hms_opt(hour, minute, 0).expect("a time"))
        - chrono::Duration::hours(7)
}

// ------------------------------------------------------------ the database

struct Room {
    pool: PgPool,
    schema: String,
    /// A teller user per organisation, for the shifts the LIVE rows need.
    teller: BTreeMap<&'static str, String>,
}

/// One seeded exchange. The wide row, its line and its payment agree.
struct Ex<'a> {
    branch: &'a str,
    side: &'a str,
    currency: &'a str,
    thb: &'a str,
    fee: &'a str,
    bank_fee: &'a str,
    method: &'a str,
    customer: &'a str,
    value_date: &'a str,
    recorded_at: DateTime<Utc>,
    live: bool,
    status: &'a str,
    booking_no: &'a str,
}

impl Default for Ex<'_> {
    fn default() -> Self {
        Ex {
            branch: B1,
            side: "BUY",
            currency: "USD",
            thb: "100.00",
            fee: "0.00",
            bank_fee: "0.00",
            method: "Cash",
            customer: "",
            value_date: "2026-10-05",
            recorded_at: bangkok("2026-10-05", 10, 0),
            live: true,
            status: "COMPLETED",
            booking_no: "",
        }
    }
}

fn org_of(branch: &str) -> &'static str {
    if branch == B9 { ORG2 } else { ORG }
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
        let schema = unique("ex3_overview");
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
        let mut teller = BTreeMap::new();
        for org in [ORG, ORG2] {
            let id: String = sqlx::query_scalar(
                "INSERT INTO users (organization_id, name, status, created_by) \
                 VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
            )
            .bind(org)
            .bind(unique("teller"))
            .fetch_one(&pool)
            .await
            .expect("a teller user");
            teller.insert(if org == ORG { ORG } else { ORG2 }, id);
        }
        let room = Room {
            pool,
            schema,
            teller,
        };
        for branch in [B1, B2, B9] {
            room.branch(branch).await;
        }
        // the passport and receipt files TX-2's store links each exchange to, one pair per organisation
        for (org, branch) in [(ORG, B1), (ORG2, B9)] {
            for kind in ["PASSPORT", "RECEIPT"] {
                sqlx::query(
                    "INSERT INTO files (key, organization_id, branch_id, owner_id, mime, sha256, created_at) \
                     VALUES ($1, $2::uuid, $3, 'owner', 'application/pdf', 'sha', now())",
                )
                .bind(format!("{kind}-{org}"))
                .bind(org)
                .bind(branch)
                .execute(&room.pool)
                .await
                .expect("a file");
            }
        }
        room
    }

    /// A branch in Asia/Bangkok with an OPEN shift `SH-<branch>` and a `branch_versions` row at version 1.
    async fn branch(&self, branch: &str) {
        let org = org_of(branch);
        sqlx::query(
            "INSERT INTO branches (id, name, organization_id, timezone) \
             VALUES ($1, $1, $2::uuid, 'Asia/Bangkok') ON CONFLICT (id) DO NOTHING",
        )
        .bind(branch)
        .bind(org)
        .execute(&self.pool)
        .await
        .expect("a branch");
        sqlx::query(
            "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, organization_id, teller_id) \
             VALUES ('SH-' || $1, $1, 'OPEN', now(), 'test', 'open-' || $1, $2::uuid, $3::uuid)",
        )
        .bind(branch)
        .bind(org)
        .bind(&self.teller[org])
        .execute(&self.pool)
        .await
        .expect("an open shift");
        sqlx::query(
            "INSERT INTO branch_versions (branch_id, version, organization_id, changed_at) \
             VALUES ($1, 1, $2::uuid, now()) \
             ON CONFLICT (branch_id) DO UPDATE SET version = 1",
        )
        .bind(branch)
        .bind(org)
        .execute(&self.pool)
        .await
        .expect("a branch version");
    }

    async fn customer(&self, org: &str, id: &str) {
        sqlx::query(
            "INSERT INTO customers (id, mode, created_at, organization_id, customer_no, display_name) \
             VALUES ($1, 'LIVE', now(), $2::uuid, $1, $1) ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(org)
        .execute(&self.pool)
        .await
        .expect("a customer");
    }

    /// Seeds one exchange through TX-2's store (`PgTransactionStore`: draft, then its status moves; a COMPLETED one
    /// through `complete_in`), then overwrites every figure of its wide `transactions` row with other values
    /// (lane controller's ruling on #1071, issuecomment-6107626913): the line and the payment are the source, so
    /// a reader of the wide columns goes red. Returns (its uuid, its document number when COMPLETED).
    async fn exchange(&self, x: Ex<'_>) -> (String, Option<String>) {
        let org = org_of(x.branch);
        let customer = if x.customer.is_empty() {
            unique("C")
        } else {
            x.customer.to_owned()
        };
        self.customer(org, &customer).await;
        let money = |s: &str| Money::parse(s).expect("money");
        let thb = Decimal::from_str(x.thb).expect("thb");
        let fee = Decimal::from_str(x.fee).expect("fee");
        // the shop pays out on a BUY and takes in on a SELL
        let settle = if x.side == "BUY" {
            thb - fee
        } else {
            thb + fee
        };
        let id = Uuid::new_v4();
        let t = ExchangeTransaction {
            id: TransactionId(id),
            organization_id: Uuid::parse_str(org).expect("org"),
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
            customer_name: customer.clone(),
            nationality: "TH".into(),
            staff_id: "staff".into(),
            booking_no: x.booking_no.into(),
            notes: String::new(),
            value_date: date(x.value_date),
            recorded_at: x.recorded_at,
            recorded_by: "test".into(),
            completed_at: None,
            cancelled_at: None,
            cancel_reason: None,
            approval_request_id: None,
            passport_file_key: format!("PASSPORT-{org}"),
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
                currency: CurrencyCode::parse(x.currency).expect("a currency"),
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
                paid_at: x.recorded_at,
            },
            receipts: vec![],
        };
        let store = PgTransactionStore::new(self.pool.clone());
        let mut tx = self.pool.begin().await.expect("begin");
        store
            .insert_draft_in(&mut tx, &t)
            .await
            .expect("insert the draft");
        let mv = |to, reason: Option<&str>| StatusMove {
            from: TransactionStatus::Draft,
            to,
            reason: reason.map(str::to_owned),
            approval_request_id: None,
        };
        let mut doc = None;
        match x.status {
            "DRAFT" => {}
            "CONFIRMED" | "COMPLETED" => {
                store
                    .set_status_in(
                        &mut tx,
                        t.id,
                        mv(TransactionStatus::Confirmed, None),
                        x.recorded_at,
                    )
                    .await
                    .expect("confirm");
                if x.status == "COMPLETED" {
                    let (number, _) = store
                        .complete_in(&mut tx, t.id, &format!("RECEIPT-{org}"), x.recorded_at)
                        .await
                        .expect("complete");
                    doc = Some(number);
                }
            }
            "CANCELLED" => {
                store
                    .set_status_in(
                        &mut tx,
                        t.id,
                        mv(TransactionStatus::Cancelled, Some("test")),
                        x.recorded_at,
                    )
                    .await
                    .expect("cancel");
            }
            other => panic!("a status this file seeds: {other}"),
        }
        tx.commit().await.expect("commit");
        // every figure of the wide row now differs from the line and the payment
        sqlx::query(
            "UPDATE transactions SET type = CASE type WHEN 'BUY' THEN 'SELL' ELSE 'BUY' END, code = 'EUR', \
               amount = amount * 7 + 3, rate = rate * 7 + 3, principal = principal * 7 + 3, fee = fee * 7 + 3, \
               bank_fee = bank_fee * 7 + 3, received = received * 7 + 3, paid = paid * 7 + 3, \
               method = 'MUTATED', payment_ref = 'MUTATED', account = 'MUTATED' \
             WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .expect("overwrite the wide figures");
        (id.to_string(), doc)
    }

    async fn finish(self) {
        let Room { pool, schema, .. } = self;
        sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .expect("drop the test schema");
        pool.close().await;
    }
}

// ------------------------------------------------------------ who calls

/// `<user id>|<ROLE>|<branch or *>`, read from the header the way `v3_cash_limits_test.rs` hands the seam its
/// callers.
fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        ORG,
        vec![ScopedRole::new(role, branch, from, None)],
        None,
    ))
}

fn legacy_of(spec: &str) -> Sessions {
    let mut parts = spec.splitn(3, '|');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(id), Some(_), Some("*")) => Sessions {
            owner: Some(Actor::owner(id)),
            staff: None,
        },
        (Some(id), Some(_), Some(branch)) => Sessions {
            owner: None,
            staff: Actor::staff(id, OldRole::Finance, BranchId::new(branch)),
        },
        _ => Sessions {
            owner: None,
            staff: None,
        },
    }
}

struct HeaderUsers;

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

/// The organisation-wide EXECUTIVE of ORG.
const EXECUTIVE: &str = "00000000-0000-0000-0000-0000000000e1|EXECUTIVE|*";
struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

/// `GET /api/executive/overview?<query>` as `who`.
async fn overview(room: &Room, who: &str, query: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let uri = if query.is_empty() {
        OVERVIEW.to_owned()
    } else {
        format!("{OVERVIEW}?{query}")
    };
    let req = atest::TestRequest::get()
        .uri(&uri)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who.to_owned()))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// The `data` of a 200 answer.
fn data(a: &Answer, what: &str) -> Value {
    assert_eq!(a.status, StatusCode::OK, "{what}: {}", a.text);
    a.body["data"].clone()
}

fn code(a: &Answer) -> String {
    a.body["error"]["code"].as_str().unwrap_or("").to_owned()
}

fn kpi<'a>(data: &'a Value, key: &str) -> &'a Value {
    data["kpis"]
        .as_array()
        .unwrap_or_else(|| panic!("kpis: {data}"))
        .iter()
        .find(|m| m["key"] == key)
        .unwrap_or_else(|| panic!("the {key} KPI: {data}"))
}

const PERIOD: &str = "from=2026-10-05&to=2026-10-05";

/// The rows of §9 row 4, each (its uuid, its number). Their THB amounts are powers of two (100 · 200 · 400 · 800 ·
/// 1600), so a volume names the rows it summed.
struct ChannelRows {
    walk_in: (String, Option<String>),
    booked: (String, Option<String>),
    kiosk: (String, Option<String>),
    referred: (String, Option<String>),
    referred_booking: (String, Option<String>),
}

/// A USED booking `id` at B1 whose `transaction_ref` is the number `doc` (the row redeemed it).
async fn booking_for(room: &Room, id: &str, doc: &str) {
    sqlx::query(
        "INSERT INTO v2_bookings (id, branch_id, request_id, request_hash, name, phone, type, code, amount, rate, \
           status, created_at, created_by, submission_id, transaction_ref, organization_id) \
         VALUES ($1, $2, $1, 'h', 'n', '0800000000', 'BUY', 'USD', 1, 1, 'USED', now(), 'test', $1, $3, $4::uuid)",
    )
    .bind(id)
    .bind(B1)
    .bind(doc)
    .bind(ORG)
    .execute(&room.pool)
    .await
    .expect("a used booking");
}

async fn seed_channels(room: &Room) -> ChannelRows {
    let walk_in = room
        .exchange(Ex {
            thb: "100.00",
            ..Ex::default()
        })
        .await;
    // a booked row: its booking_no names a booking whose transaction_ref is its number
    let booking_no = unique("BK");
    let booked = room
        .exchange(Ex {
            thb: "200.00",
            booking_no: &booking_no,
            ..Ex::default()
        })
        .await;
    booking_for(room, &booking_no, booked.1.as_deref().expect("a number")).await;
    let kiosk = room
        .exchange(Ex {
            thb: "400.00",
            ..Ex::default()
        })
        .await;
    sqlx::query(
        "INSERT INTO kiosk_requests (branch_id, id, status, created_at, staff, side, currency, rate, foreign_amount, \
           thb, input_amount, payment_method, submission_id, document_ref, organization_id) \
         VALUES ($1, $2, 'COMPLETED', now(), 'staff', 'BUY', 'USD', 1, 400, 400, 400, 'Cash', $2, $3, $4::uuid)",
    )
    .bind(B1)
    .bind(unique("K"))
    .bind(kiosk.1.as_deref().expect("a number"))
    .bind(ORG)
    .execute(&room.pool)
    .await
    .expect("the kiosk request it came from");
    let referred = room
        .exchange(Ex {
            thb: "800.00",
            customer: "C-REF",
            ..Ex::default()
        })
        .await;
    refer(room, &referred.0, "C-REF").await;
    let ref_booking_no = unique("BK");
    let referred_booking = room
        .exchange(Ex {
            thb: "1600.00",
            customer: "C-REFBK",
            booking_no: &ref_booking_no,
            ..Ex::default()
        })
        .await;
    booking_for(
        room,
        &ref_booking_no,
        referred_booking.1.as_deref().expect("a number"),
    )
    .await;
    refer(room, &referred_booking.0, "C-REFBK").await;
    // a SUPERSEDED attribution does not make the walk-in row referred
    sqlx::query(
        "INSERT INTO referral_attributions (id, organization_id, branch_id, code, customer_id, transaction_id, \
           source, captured_at, captured_by, status) \
         SELECT gen_random_uuid(), $1::uuid, $2, 'OLDCODE1', t.customer_id, t.id, 'DESK', now(), 'test', 'SUPERSEDED' \
         FROM transactions t WHERE t.id = $3::uuid",
    )
    .bind(ORG)
    .bind(B1)
    .bind(&walk_in.0)
    .execute(&room.pool)
    .await
    .expect("a superseded attribution");
    ChannelRows {
        walk_in,
        booked,
        kiosk,
        referred,
        referred_booking,
    }
}

async fn refer(room: &Room, transaction: &str, customer: &str) {
    sqlx::query(
        "INSERT INTO referral_attributions (id, organization_id, branch_id, code, customer_id, transaction_id, \
           source, captured_at, captured_by, status) \
         VALUES (gen_random_uuid(), $1::uuid, $2, 'REFCODE1', $3, $4::uuid, 'DESK', now(), 'test', 'ACTIVE')",
    )
    .bind(ORG)
    .bind(B1)
    .bind(customer)
    .bind(transaction)
    .execute(&room.pool)
    .await
    .expect("an ACTIVE referral attribution");
}

// ------------------------------------------------------------ the view

/// `GET <path>` as `who` (any v3 view path with its query).
async fn get(room: &Room, who: &str, path: &str) -> Answer {
    let state = AppState::new(Arc::new(common::settings(&[])), room.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri(path)
        .insert_header((header::HOST, HOST))
        .insert_header((AS, who.to_owned()))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    let status = resp.status();
    let bytes = atest::read_body(resp).await;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Answer { status, text, body }
}

/// A refusal `400 invalidRequest` naming `field`, with no data.
fn assert_invalid(a: &Answer, field: &str, what: &str) {
    assert_eq!(a.status, StatusCode::BAD_REQUEST, "{what}: {}", a.text);
    assert_eq!(code(a), "invalidRequest", "{what}: {}", a.text);
    assert_eq!(a.body["error"]["field"], field, "{what}: {}", a.text);
    assert!(
        a.body.get("data").is_none_or(Value::is_null),
        "{what}: no data: {}",
        a.text
    );
}

/// The source text of `rel` under `api/`, whitespace removed.
fn flat_source(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} exists: {e}", path.display()))
        .split_whitespace()
        .collect()
}

const VIEW: &str = "/api/v3/views/admin_transactions";

/// The page of `admin_transactions` at B1 over PERIOD plus `extra`, as A01's reader.
async fn page(room: &Room, extra: &str) -> Answer {
    get(
        room,
        EXECUTIVE,
        &format!("{VIEW}?branch={B1}&{PERIOD}{extra}"),
    )
    .await
}

/// The customers of the rows a 200 page lists, as a set (each seeded exchange has its own customer).
async fn listed(room: &Room, extra: &str) -> std::collections::BTreeSet<String> {
    let a = page(room, extra).await;
    assert_eq!(a.status, StatusCode::OK, "{extra}: {}", a.text);
    a.body["data"]
        .as_array()
        .unwrap_or_else(|| panic!("{extra}: a page is an array: {}", a.text))
        .iter()
        .map(|r| r["customerId"].as_str().unwrap_or("").to_owned())
        .collect()
}

/// The customer of a seeded exchange.
async fn customer_of(room: &Room, tx: &(String, Option<String>)) -> String {
    sqlx::query_scalar("SELECT customer_id FROM transactions WHERE id = $1::uuid")
        .bind(&tx.0)
        .fetch_one(&room.pool)
        .await
        .expect("the exchange's customer")
}

fn set(items: &[&String]) -> std::collections::BTreeSet<String> {
    items.iter().map(|s| (*s).clone()).collect()
}

/// §9: `currency=USD` lists only the USD exchanges; a well-formed code the branch never traded is an empty page.
#[actix_web::test]
async fn currency_lists_only_that_currencys_exchanges() {
    let room = Room::start().await;
    let usd_buy = room.exchange(Ex::default()).await;
    let usd_sell = room
        .exchange(Ex {
            side: "SELL",
            thb: "300.00",
            ..Ex::default()
        })
        .await;
    let jpy = room
        .exchange(Ex {
            currency: "JPY",
            thb: "700.00",
            ..Ex::default()
        })
        .await;
    let (usd_buy, usd_sell, jpy) = (
        customer_of(&room, &usd_buy).await,
        customer_of(&room, &usd_sell).await,
        customer_of(&room, &jpy).await,
    );
    assert_eq!(
        listed(&room, "&currency=USD").await,
        set(&[&usd_buy, &usd_sell])
    );
    assert_eq!(listed(&room, "&currency=JPY").await, set(&[&jpy]));
    assert!(
        listed(&room, "&currency=GBP").await.is_empty(),
        "a code never traded is an empty page"
    );
    room.finish().await;
}

/// §9: `channel` = each of the four over walk-in, booked, kiosk and referred exchanges plus one referred booking;
/// the referred booking is under both `Booking` and `Referred`.
#[actix_web::test]
async fn each_channel_lists_its_exchanges_and_a_referred_booking_is_under_both() {
    let room = Room::start().await;
    let c = seed_channels(&room).await;
    let walk_in = customer_of(&room, &c.walk_in).await;
    let booked = customer_of(&room, &c.booked).await;
    let kiosk = customer_of(&room, &c.kiosk).await;
    let referred = customer_of(&room, &c.referred).await;
    let referred_booking = customer_of(&room, &c.referred_booking).await;
    for (channel, want) in [
        ("WalkIn", set(&[&walk_in])),
        ("Booking", set(&[&booked, &referred_booking])),
        ("Kiosk", set(&[&kiosk])),
        ("Referred", set(&[&referred, &referred_booking])),
    ] {
        assert_eq!(
            listed(&room, &format!("&channel={channel}")).await,
            want,
            "channel={channel}"
        );
    }
    room.finish().await;
}

/// §9: `currency` and `channel` together, then with `type`: AND.
#[actix_web::test]
async fn currency_channel_and_type_combine_by_and() {
    let room = Room::start().await;
    let mut booked = Vec::new();
    for (side, currency) in [("BUY", "USD"), ("SELL", "USD"), ("BUY", "JPY")] {
        let no = unique("BK");
        let t = room
            .exchange(Ex {
                side,
                currency,
                booking_no: &no,
                ..Ex::default()
            })
            .await;
        booking_for(&room, &no, t.1.as_deref().expect("a number")).await;
        booked.push(customer_of(&room, &t).await);
    }
    let walk_in_usd = room.exchange(Ex::default()).await;
    let walk_in_usd = customer_of(&room, &walk_in_usd).await;
    assert_eq!(
        listed(&room, "&currency=USD&channel=Booking").await,
        set(&[&booked[0], &booked[1]]),
        "USD and booked"
    );
    assert_eq!(
        listed(&room, "&currency=USD&channel=Booking&type=BUY").await,
        set(&[&booked[0]]),
        "USD, booked and BUY"
    );
    assert_eq!(
        listed(&room, "&currency=USD&channel=WalkIn").await,
        set(&[&walk_in_usd]),
        "USD and walk-in"
    );
    room.finish().await;
}

/// §9: a bad `currency` (`usd`, `US`, `USDT`) or `channel` (`walkin`, `Online`) is `400 invalidRequest` naming it.
#[actix_web::test]
async fn a_bad_currency_or_channel_is_refused_naming_the_field() {
    let room = Room::start().await;
    room.exchange(Ex::default()).await;
    for bad in ["usd", "US", "USDT"] {
        assert_invalid(
            &page(&room, &format!("&currency={bad}")).await,
            "currency",
            &format!("currency={bad}"),
        );
    }
    for bad in ["walkin", "Online"] {
        assert_invalid(
            &page(&room, &format!("&channel={bad}")).await,
            "channel",
            &format!("channel={bad}"),
        );
    }
    room.finish().await;
}

/// §9: with neither new key the page answers as before: every seeded exchange of every currency and channel, each row
/// with exactly `TransactionRow`'s fields (no new field in the row).
#[actix_web::test]
async fn no_new_key_lists_every_exchange_with_the_same_row_fields() {
    let room = Room::start().await;
    let c = seed_channels(&room).await;
    let jpy = room
        .exchange(Ex {
            currency: "JPY",
            ..Ex::default()
        })
        .await;
    let mut want = std::collections::BTreeSet::new();
    for t in [
        &c.walk_in,
        &c.booked,
        &c.kiosk,
        &c.referred,
        &c.referred_booking,
        &jpy,
    ] {
        want.insert(customer_of(&room, t).await);
    }
    assert_eq!(listed(&room, "").await, want);
    let a = page(&room, "").await;
    let fields: std::collections::BTreeSet<&str> = [
        "id",
        "date",
        "created",
        "type",
        "currency",
        "amount",
        "rate",
        "unitSize",
        "principal",
        "fee",
        "bankFee",
        "settlement",
        "method",
        "customer",
        "customerId",
        "staff",
        "status",
        "notes",
        "cancelReason",
        "shiftId",
        "referenceRate",
        "referenceValue",
        "referenceSpread",
        "referencePercent",
        "paymentRef",
        "slipUrl",
    ]
    .into_iter()
    .collect();
    for row in a.body["data"].as_array().expect("a page") {
        let keys: std::collections::BTreeSet<&str> = row
            .as_object()
            .expect("a row object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, fields, "{row}");
    }
    room.finish().await;
}

/// §7 (G2): `DRILLDOWN_KEYS` in the DTO file holds `currency` and `channel`, and `accepted_keys` takes them; the
/// view's `branch` scope stays accepted (EX-11 ruling (1) on #1104).
#[test]
fn drilldown_keys_hold_currency_and_channel() {
    let dto = flat_source("src/presentation/dto/v3/admin_transactions.rs");
    let at = dto
        .find("pubconstDRILLDOWN_KEYS:&[&str]=&[")
        .expect("dto/v3/admin_transactions.rs declares `pub const DRILLDOWN_KEYS: &[&str]`");
    let list = &dto[at..dto[at..].find("];").map(|e| at + e).expect("the list ends")];
    for key in ["\"currency\"", "\"channel\""] {
        assert!(list.contains(key), "DRILLDOWN_KEYS holds {key}: {list}");
    }
    let accepted = accepted_keys("admin_transactions");
    for key in [
        "branch", "from", "to", "type", "status", "currency", "channel",
    ] {
        assert!(
            accepted.contains(&key),
            "admin_transactions accepts {key}: {accepted:?}"
        );
    }
}

/// §9 last row: A01's drill-down to this view passes `currency` and `channel` in `filter` and drops nothing.
#[actix_web::test]
async fn a01s_drilldown_passes_currency_and_channel_and_drops_nothing() {
    let room = Room::start().await;
    room.exchange(Ex::default()).await;
    let d = data(
        &overview(
            &room,
            EXECUTIVE,
            &format!("{PERIOD}&currency=USD&channel=Booking"),
        )
        .await,
        "A01 with currency and channel",
    );
    for key in ["buy_volume_thb", "sell_volume_thb"] {
        let dd = &kpi(&d, key)["drilldown"];
        assert_eq!(dd["view"], "admin_transactions", "{key}: {dd}");
        assert_eq!(dd["filter"]["currency"], "USD", "{key}: {dd}");
        assert_eq!(dd["filter"]["channel"], "Booking", "{key}: {dd}");
        assert_eq!(dd["dropped"], json!([]), "{key}: {dd}");
    }
    room.finish().await;
}
