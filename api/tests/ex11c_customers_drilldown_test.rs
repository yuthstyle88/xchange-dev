//! Ticket #1105 (EX-11c): the A01 drill-down to the customer list keeps the period and the "repeat" the executive
//! opened — `GET /api/v3/views/admin_customers` gains `from`, `to` and `repeat` (design
//! `docs/design/blueprint/executive-finance.md` §3 `repeat_customers`, §4.4; the lane controller's G2 ruling on
//! #1071 and its EX-11 rulings on #1104).
//!
//! Written before the implementation by the up-front test-writer claude/07d3765f, on the BE lane controller
//! claude/ad05bf8d's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! **What is pinned** (ticket §7–§9):
//! - `from`/`to` (business dates, `Period::check`): only customers with a COMPLETED LIVE exchange **at the view's
//!   branch** in the period. Each row's statistics (`completed`, `volume`) stay as today, not cut to the period.
//! - `repeat=true`: only customers with a COMPLETED LIVE exchange in the period at the branch **and** an earlier
//!   COMPLETED LIVE exchange anywhere in the organisation (design §3). Two in the period and none before is repeat.
//!   `repeat=true` without `from` is `400 invalidRequest`, field `from`. `repeat=false` adds no condition.
//! - DEMO rows never count. With none of the new keys the list answers as before.
//! - `DRILLDOWN_KEYS` = `q, from, to, repeat` (§7). A01's customers drill-down passes `repeat`, `from` and `to`, and
//!   still drops `currency` and `channel`, which this view does not take.
//!
//! "Earlier" is by `recorded_at` (this file's reading of design §3's "before the current one"); every seeded pair is
//! hours or days apart, so a reading by business date gives the same answers except where a test says otherwise.
//! Readers are the organisation-wide EXECUTIVE. Needs `TEST_DATABASE_URL`; nothing skips.

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

const VIEW: &str = "/api/v3/views/admin_customers";

async fn page(room: &Room, extra: &str) -> Answer {
    get(room, EXECUTIVE, &format!("{VIEW}?branch={B1}{extra}")).await
}

/// The customer ids a 200 page lists.
async fn listed(room: &Room, extra: &str) -> std::collections::BTreeSet<String> {
    let a = page(room, extra).await;
    assert_eq!(a.status, StatusCode::OK, "{extra}: {}", a.text);
    a.body["data"]
        .as_array()
        .unwrap_or_else(|| panic!("{extra}: a page is an array: {}", a.text))
        .iter()
        .map(|r| r["id"].as_str().unwrap_or("").to_owned())
        .collect()
}

fn set(items: &[&str]) -> std::collections::BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// One COMPLETED LIVE exchange of `customer` at `branch` on business date `day`, recorded at `hour`:00 local.
async fn traded(room: &Room, customer: &str, branch: &str, day: &str, hour: u32) {
    room.exchange(Ex {
        branch,
        customer,
        value_date: day,
        recorded_at: bangkok(day, hour, 0),
        ..Ex::default()
    })
    .await;
}

const IN_PERIOD: &str = "&from=2026-10-05&to=2026-10-05";

/// §8: `from`/`to` lists only customers with a completed exchange at B1 in the period; the row's statistics stay
/// all-time at B1, not cut to the period.
#[actix_web::test]
async fn the_period_lists_only_customers_who_traded_here_in_it() {
    let room = Room::start().await;
    traded(&room, "C-IN", B1, "2026-10-05", 10).await;
    traded(&room, "C-IN", B1, "2026-10-01", 10).await; // before the period, same branch
    traded(&room, "C-BEFORE", B1, "2026-10-01", 10).await;
    traded(&room, "C-ELSEWHERE", B2, "2026-10-05", 10).await;
    assert_eq!(listed(&room, IN_PERIOD).await, set(&["C-IN"]));
    let a = page(&room, IN_PERIOD).await;
    let row = &a.body["data"][0];
    assert_eq!(row["id"], "C-IN", "{}", a.text);
    assert_eq!(
        row["completed"], 2,
        "statistics stay all-time at the branch, not cut to the period: {row}"
    );
    room.finish().await;
}

/// §9: a customer whose only exchange is in the period is listed under `from/to`, not under `repeat=true`; one with
/// an exchange before the period **at another branch** and one in it here is listed; one with two in the period and
/// none before is repeat (the second has an earlier one); `repeat=false` adds no condition.
#[actix_web::test]
async fn repeat_needs_an_earlier_completed_exchange_anywhere_in_the_organisation() {
    let room = Room::start().await;
    traded(&room, "C-ONCE", B1, "2026-10-05", 10).await;
    traded(&room, "C-AWAY", B2, "2026-10-01", 10).await;
    traded(&room, "C-AWAY", B1, "2026-10-05", 10).await;
    traded(&room, "C-TWICE", B1, "2026-10-05", 10).await;
    traded(&room, "C-TWICE", B1, "2026-10-05", 14).await;
    assert_eq!(
        listed(&room, IN_PERIOD).await,
        set(&["C-ONCE", "C-AWAY", "C-TWICE"])
    );
    assert_eq!(
        listed(&room, &format!("{IN_PERIOD}&repeat=true")).await,
        set(&["C-AWAY", "C-TWICE"])
    );
    assert_eq!(
        listed(&room, &format!("{IN_PERIOD}&repeat=false")).await,
        set(&["C-ONCE", "C-AWAY", "C-TWICE"]),
        "repeat=false adds no condition"
    );
    room.finish().await;
}

/// §8 "completed": an earlier exchange that was never completed (DRAFT) does not make a customer repeat.
#[actix_web::test]
async fn an_earlier_exchange_that_was_not_completed_does_not_make_a_repeat() {
    let room = Room::start().await;
    room.exchange(Ex {
        customer: "C-DRAFT",
        value_date: "2026-10-01",
        recorded_at: bangkok("2026-10-01", 10, 0),
        status: "DRAFT",
        ..Ex::default()
    })
    .await;
    traded(&room, "C-DRAFT", B1, "2026-10-05", 10).await;
    assert_eq!(listed(&room, IN_PERIOD).await, set(&["C-DRAFT"]));
    assert!(
        listed(&room, &format!("{IN_PERIOD}&repeat=true"))
            .await
            .is_empty(),
        "a DRAFT before is not an earlier completed exchange"
    );
    room.finish().await;
}

/// §9: `repeat=true` without `from` is `400 invalidRequest`, field `from`.
#[actix_web::test]
async fn repeat_without_a_period_is_refused() {
    let room = Room::start().await;
    assert_invalid(
        &page(&room, "&repeat=true").await,
        "from",
        "repeat=true alone",
    );
    room.finish().await;
}

/// §9: DEMO rows never count: a DEMO exchange in the period lists nobody, and a DEMO exchange before the period does
/// not make a repeat.
#[actix_web::test]
async fn demo_exchanges_never_count() {
    let room = Room::start().await;
    room.exchange(Ex {
        customer: "C-DEMO-ONLY",
        live: false,
        ..Ex::default()
    })
    .await;
    room.exchange(Ex {
        customer: "C-DEMO-BEFORE",
        live: false,
        value_date: "2026-10-01",
        recorded_at: bangkok("2026-10-01", 10, 0),
        ..Ex::default()
    })
    .await;
    traded(&room, "C-DEMO-BEFORE", B1, "2026-10-05", 10).await;
    assert_eq!(listed(&room, IN_PERIOD).await, set(&["C-DEMO-BEFORE"]));
    assert!(
        listed(&room, &format!("{IN_PERIOD}&repeat=true"))
            .await
            .is_empty(),
        "a DEMO exchange is not an earlier completed one"
    );
    room.finish().await;
}

/// §9: with none of the new keys the list answers as before: every LIVE customer of the organisation, including one
/// who never traded and one who traded only at another branch.
#[actix_web::test]
async fn no_new_key_lists_the_organisations_customers_as_before() {
    let room = Room::start().await;
    traded(&room, "C-HERE", B1, "2026-10-05", 10).await;
    traded(&room, "C-THERE", B2, "2026-10-05", 10).await;
    room.customer(ORG, "C-NEVER").await;
    let all = listed(&room, "").await;
    for c in ["C-HERE", "C-THERE", "C-NEVER"] {
        assert!(all.contains(c), "{c} is listed without a period: {all:?}");
    }
    room.finish().await;
}

/// §7 (G2): `DRILLDOWN_KEYS` is exactly `q, from, to, repeat`, and `accepted_keys` takes them.
#[test]
fn drilldown_keys_are_q_the_period_and_repeat() {
    let dto = flat_source("src/presentation/dto/v3/admin_customers.rs");
    let at = dto
        .find("pubconstDRILLDOWN_KEYS:&[&str]=&[")
        .expect("dto/v3/admin_customers.rs declares `pub const DRILLDOWN_KEYS: &[&str]`");
    let start = at + "pubconstDRILLDOWN_KEYS:&[&str]=&[".len();
    let end = start + dto[start..].find("]").expect("the list ends");
    let mut keys: Vec<&str> = dto[start..end]
        .split(',')
        .map(|k| k.trim_matches('"'))
        .filter(|k| !k.is_empty())
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, vec!["from", "q", "repeat", "to"]);
    let accepted = accepted_keys("admin_customers");
    for key in ["q", "from", "to", "repeat"] {
        assert!(
            accepted.contains(&key),
            "admin_customers accepts {key}: {accepted:?}"
        );
    }
}

/// §9 last row: A01's customers drill-down passes `repeat`, `from` and `to`, and still drops `currency` and `channel`.
#[actix_web::test]
async fn a01s_customers_drilldown_passes_repeat_and_the_period() {
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
    let dd = &kpi(&d, "repeat_customers")["drilldown"];
    assert_eq!(dd["view"], "admin_customers", "{dd}");
    assert_eq!(dd["filter"]["repeat"], "true", "{dd}");
    assert_eq!(dd["filter"]["from"], "2026-10-05", "{dd}");
    assert_eq!(dd["filter"]["to"], "2026-10-05", "{dd}");
    assert_eq!(dd["dropped"], json!(["currency", "channel"]), "{dd}");
    room.finish().await;
}
