//! Ticket #1071 (EX-3): the executive overview A01, `GET /api/executive/overview`
//! (`docs/design/blueprint/executive-finance.md` §3, §4, §8.1; the ticket's §7–§9, the lane controller's
//! G1–G3 ruling issuecomment-6098404213 and the §7 amendment issuecomment-6096878503).
//!
//! Written before the implementation by the rule-A test-writer claude/dell-claude-5, on the BE lane controller
//! claude/ad05bf8d's order. Do not edit, delete or weaken; add tests in a file of your own.
//!
//! One test per row of the ticket's §9 table, plus the [required] functions of §7 that other tickets call
//! (`Channel`, `accepted_keys`, `ExchangeFacts::branch_day_totals`).
//!
//! **What is pinned, and from where:**
//! - The answer of §8.1 / §7, camelCase: `{changed, versions, data: {period, kpis, branches, alerts,
//!   definitions}}`; `Metric = {key, value, unavailable, drilldown}`; money as decimal strings, THB 2 places; a
//!   drill-down is `{view, filter, dropped}`. The KPI keys are the design's §3 keys.
//! - `MetricUnavailable` on the wire is camelCase (`awaitsLedger`, …), as the design's §3 table writes it.
//! - Refusals by their wire code, as the seam answers them.
//!
//! **Choices of this file**, each marked where used:
//! - `branches` is only ever sent with one branch, so this file does not decide how a list is spelled.
//! - The alerts' `kind` values of EX-3's own alerts are not fixed by the ticket, so this file never asserts them;
//!   it finds an alert by its branch and its drill-down.
//! - `Room::exchange` overwrites every figure of the wide `transactions` row on purpose: per TX-8, the figures come
//!   from COMPLETED lines and payments only, so a reader of the wide columns gets wrong figures and goes red.
//!
//! Needs `TEST_DATABASE_URL` naming a `_test` database; nothing skips. Each test works in a schema of its own.

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
use xchange::application::executive::channel::Channel;
use xchange::application::executive::drilldown::accepted_keys;
use xchange::application::executive::ports::{BranchDayTotals, ExchangeFacts, MethodDayTotals};
use xchange::bootstrap::{self, AppState};
use xchange::infrastructure::executive::wide_transaction_facts::WideTransactionFacts;
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

/// The six KPIs of E-1, in the wireframe's order (design §3, "On A01's first screen").
const KPI_KEYS: [&str; 6] = [
    "buy_volume_thb",
    "sell_volume_thb",
    "realized_margin_thb",
    "repeat_customers",
    "booking_conversion",
    "commission_thb",
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

    /// An approval request of `kind` for `branch`, in `status`.
    async fn approval(&self, kind: &str, branch: &str, status: &str) -> String {
        sqlx::query_scalar(
            "INSERT INTO approval_requests (id, type, branch_id, object_id, object_version_hash, policy_version, \
               proposed_payload, requested_by, requested_at, status, idempotency_key, organization_id) \
             VALUES (gen_random_uuid(), $1, $2, $3, 'h', 'v1', '{}'::jsonb, 'test', now(), $4, $3, $5::uuid) \
             RETURNING id::text",
        )
        .bind(kind)
        .bind(branch)
        .bind(unique("object"))
        .bind(status)
        .bind(org_of(branch))
        .fetch_one(&self.pool)
        .await
        .expect("an approval request")
    }

    async fn version(&self, branch: &str) -> i64 {
        sqlx::query_scalar("SELECT version FROM branch_versions WHERE branch_id = $1")
            .bind(branch)
            .fetch_one(&self.pool)
            .await
            .expect("a branch version")
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
/// A FINANCE of B1 (E-7: branch-scoped), who holds no `OverviewRead`.
const FINANCE: &str = "00000000-0000-0000-0000-0000000000f1|FINANCE|XC-B01";

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

fn branch_row<'a>(data: &'a Value, branch: &str) -> Option<&'a Value> {
    data["branches"]
        .as_array()
        .unwrap_or_else(|| panic!("branches: {data}"))
        .iter()
        .find(|r| r["branchId"] == branch)
}

fn alerts_of(data: &Value, branch: &str) -> Vec<Value> {
    data["alerts"]
        .as_array()
        .unwrap_or_else(|| panic!("alerts: {data}"))
        .iter()
        .filter(|a| a["branchId"] == branch)
        .cloned()
        .collect()
}

/// Percent-encodes a query value.
fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

const PERIOD: &str = "from=2026-10-05&to=2026-10-05";

// ------------------------------------------------------------ §9 row 1: volumes

/// Seeded BUY and SELL rows at two branches, one DEMO: volumes per side and per branch; DEMO excluded. A row that
/// is not COMPLETED is no volume either (design §3: "Completed means `transactions.status = 'COMPLETED'`").
/// The volume is the gross THB of the line, not the settlement: each row has a fee, so the two differ.
#[actix_web::test]
async fn volumes_per_side_and_per_branch_exclude_demo_and_unfinished_rows() {
    let room = Room::start().await;
    for (branch, side, thb, fee) in [
        (B1, "BUY", "1000.00", "10.00"),
        (B1, "SELL", "2000.00", "20.00"),
        (B2, "BUY", "400.00", "4.00"),
    ] {
        room.exchange(Ex {
            branch,
            side,
            thb,
            fee,
            ..Ex::default()
        })
        .await;
    }
    room.exchange(Ex {
        thb: "8000.00",
        live: false,
        ..Ex::default()
    })
    .await;
    room.exchange(Ex {
        side: "SELL",
        thb: "16000.00",
        status: "DRAFT",
        ..Ex::default()
    })
    .await;

    let d = data(&overview(&room, EXECUTIVE, PERIOD).await, "A01");
    assert_eq!(kpi(&d, "buy_volume_thb")["value"], "1400.00", "{d}");
    assert_eq!(kpi(&d, "sell_volume_thb")["value"], "2000.00", "{d}");
    assert_eq!(
        branch_row(&d, B1).expect("B1's row")["volumeThb"],
        "3000.00",
        "B1 = its buy + its sell"
    );
    assert_eq!(branch_row(&d, B2).expect("B2's row")["volumeThb"], "400.00");
    assert!(
        branch_row(&d, B9).is_none(),
        "another organisation's branch is never a row: {d}"
    );
    room.finish().await;
}

// ------------------------------------------------------------ §9 row 2: the business date

/// A transaction at 23:59 and one at 00:01 in a branch at UTC+7: each on its own business date. Both instants fall
/// on the same UTC date, so a filter on the UTC date of the time would put both on 2026-10-05.
#[actix_web::test]
async fn the_business_date_is_the_branchs_own() {
    let room = Room::start().await;
    room.exchange(Ex {
        thb: "100.00",
        value_date: "2026-10-05",
        recorded_at: bangkok("2026-10-05", 23, 59),
        ..Ex::default()
    })
    .await;
    room.exchange(Ex {
        thb: "200.00",
        value_date: "2026-10-06",
        recorded_at: bangkok("2026-10-06", 0, 1),
        ..Ex::default()
    })
    .await;
    let d = data(&overview(&room, EXECUTIVE, PERIOD).await, "2026-10-05");
    assert_eq!(kpi(&d, "buy_volume_thb")["value"], "100.00", "{d}");
    let d = data(
        &overview(&room, EXECUTIVE, "from=2026-10-06&to=2026-10-06").await,
        "2026-10-06",
    );
    assert_eq!(kpi(&d, "buy_volume_thb")["value"], "200.00", "{d}");
    room.finish().await;
}

// ------------------------------------------------------------ §9 row 3: repeat customers

/// A customer with an earlier completed row at another branch is counted once as a repeat customer, however many
/// rows they have in the period. A customer whose only earlier row is not completed, and one with no earlier row,
/// are not repeat customers.
#[actix_web::test]
async fn a_customer_with_an_earlier_completed_row_elsewhere_is_one_repeat_customer() {
    let room = Room::start().await;
    let earlier = bangkok("2026-10-01", 10, 0);
    // C-REPEAT: earlier at B2, twice in the period at B1
    room.exchange(Ex {
        branch: B2,
        customer: "C-REPEAT",
        value_date: "2026-10-01",
        recorded_at: earlier,
        ..Ex::default()
    })
    .await;
    for hour in [9, 11] {
        room.exchange(Ex {
            customer: "C-REPEAT",
            recorded_at: bangkok("2026-10-05", hour, 0),
            ..Ex::default()
        })
        .await;
    }
    // C-DRAFT: its earlier row never completed
    room.exchange(Ex {
        branch: B2,
        customer: "C-DRAFT",
        value_date: "2026-10-01",
        recorded_at: earlier,
        status: "DRAFT",
        ..Ex::default()
    })
    .await;
    room.exchange(Ex {
        customer: "C-DRAFT",
        ..Ex::default()
    })
    .await;
    // C-NEW: first time
    room.exchange(Ex {
        customer: "C-NEW",
        ..Ex::default()
    })
    .await;

    let d = data(&overview(&room, EXECUTIVE, PERIOD).await, "A01");
    assert_eq!(kpi(&d, "repeat_customers")["value"], "1", "{d}");
    room.finish().await;
}

// ------------------------------------------------------------ §9 row 4: the channel filter

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
        "INSERT INTO bookings (id, branch_id, request_id, request_hash, name, phone, type, code, amount, rate, \
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

/// `channel=Booking`, `Kiosk`, `WalkIn`, `Referred` over walk-in, booked, kiosk and referred rows plus one referred
/// booking: the matching rows only, the referred booking under both `Booking` and `Referred`; an unknown value →
/// `400 validationFailed`.
#[actix_web::test]
async fn the_channel_filter_keeps_the_matching_rows_only() {
    let room = Room::start().await;
    seed_channels(&room).await;
    for (channel, expected) in [
        // 100 walk-in · 200 booked · 400 kiosk · 800 referred · 1600 referred booking
        ("WalkIn", "100.00"),
        ("Booking", "1800.00"),
        ("Kiosk", "400.00"),
        ("Referred", "2400.00"),
    ] {
        let d = data(
            &overview(&room, EXECUTIVE, &format!("{PERIOD}&channel={channel}")).await,
            channel,
        );
        assert_eq!(
            kpi(&d, "buy_volume_thb")["value"],
            expected,
            "channel={channel}: {d}"
        );
    }
    let a = overview(&room, EXECUTIVE, &format!("{PERIOD}&channel=Phone")).await;
    assert_eq!(
        (a.status, code(&a).as_str()),
        (StatusCode::BAD_REQUEST, "validationFailed"),
        "an unknown channel: {}",
        a.text
    );
    room.finish().await;
}

/// `Channel` is the one place the conditions are written (G1): `parse` takes exactly the wire values, and
/// `condition(alias)` selects the same rows through SQL that EX-11a/b will call.
#[actix_web::test]
async fn channel_parse_and_condition_classify_each_row() {
    for (wire, channel) in [
        ("WalkIn", Channel::WalkIn),
        ("Booking", Channel::Booking),
        ("Kiosk", Channel::Kiosk),
        ("Referred", Channel::Referred),
    ] {
        assert_eq!(Channel::parse(wire), Some(channel), "{wire}");
    }
    for other in ["walkin", "booking", "WALK_IN", "", "Phone"] {
        assert_eq!(Channel::parse(other), None, "{other:?}");
    }

    let room = Room::start().await;
    let rows = seed_channels(&room).await;
    let ids = |rows: &[&(String, Option<String>)]| {
        let mut v: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
        v.sort();
        v
    };
    for (channel, expected) in [
        (Channel::WalkIn, ids(&[&rows.walk_in])),
        (
            Channel::Booking,
            ids(&[&rows.booked, &rows.referred_booking]),
        ),
        (Channel::Kiosk, ids(&[&rows.kiosk])),
        (
            Channel::Referred,
            ids(&[&rows.referred, &rows.referred_booking]),
        ),
    ] {
        let sql = format!(
            "SELECT x.id::text FROM transactions x WHERE {} ORDER BY x.id::text",
            channel.condition("x")
        );
        let got: Vec<String> = sqlx::query_scalar(AssertSqlSafe(sql))
            .fetch_all(&room.pool)
            .await
            .unwrap_or_else(|e| panic!("{channel:?}'s condition runs over alias x: {e}"));
        assert_eq!(got, expected, "{channel:?}");
    }
    room.finish().await;
}

// ------------------------------------------------------------ §9 row 5: drill-down `dropped`

/// A01's drill-down to `admin_transactions` with `currency` and `channel` set: since EX-11a #1103 `filter` has both
/// keys and `dropped` is empty. The branch row's `dropped` stays `["currency", "channel"]` until #1104 lands.
#[actix_web::test]
async fn a_drilldown_to_admin_transactions_drops_currency_and_channel() {
    let room = Room::start().await;
    room.exchange(Ex::default()).await;
    let d = data(
        &overview(
            &room,
            EXECUTIVE,
            &format!("{PERIOD}&currency=USD&channel=WalkIn"),
        )
        .await,
        "A01 with currency and channel",
    );
    for key in ["buy_volume_thb", "sell_volume_thb"] {
        let dd = &kpi(&d, key)["drilldown"];
        assert_eq!(dd["view"], "admin_transactions", "{key}: {dd}");
        let filter = dd["filter"]
            .as_object()
            .unwrap_or_else(|| panic!("{key}'s filter is an object: {dd}"));
        assert!(
            filter.contains_key("currency") && filter.contains_key("channel"),
            "{key}: the target accepts them since #1103: {dd}"
        );
        assert_eq!(dd["dropped"], json!([]), "{key}");
    }
    let dd = &branch_row(&d, B1).expect("B1's row")["drilldown"];
    assert_eq!(
        dd["dropped"],
        json!(["currency", "channel"]),
        "B1's row: {dd}"
    );
    room.finish().await;
}

/// `accepted_keys` is §4.4's "Passed" column until each EX-11 child swaps its arm (G2).
#[test]
fn accepted_keys_follow_the_passed_column_of_design_4_4() {
    let tx = accepted_keys("admin_transactions");
    for key in ["from", "to", "type"] {
        assert!(
            tx.contains(&key),
            "admin_transactions accepts {key}: {tx:?}"
        );
    }
    for key in ["currency", "channel"] {
        assert!(
            tx.contains(&key),
            "admin_transactions accepts {key} since #1103: {tx:?}"
        );
    }
    let ap = accepted_keys("approval_requests");
    for key in ["branch", "type"] {
        assert!(ap.contains(&key), "approval_requests accepts {key}: {ap:?}");
    }
    assert!(
        !accepted_keys("admin_customers").contains(&"from"),
        "admin_customers takes q only"
    );
}

// ------------------------------------------------------------ §9 rows 6 and 12: the pending-rate alert

/// A `RATE_PUBLICATION` request PENDING at B1: one alert for B1, whose drill-down is `approval_requests` with
/// `branch` and `type = RATE_PUBLICATION`, and an empty `dropped`. A decided request alerts nothing, and another
/// organisation's pending request is not listed.
#[actix_web::test]
async fn a_pending_rate_request_is_one_alert_with_its_drilldown() {
    let room = Room::start().await;
    room.approval("RATE_PUBLICATION", B1, "PENDING").await;
    room.approval("RATE_PUBLICATION", B2, "APPROVED").await;
    room.approval("RATE_PUBLICATION", B9, "PENDING").await;
    let d = data(&overview(&room, EXECUTIVE, PERIOD).await, "A01");

    let b1 = alerts_of(&d, B1);
    assert_eq!(b1.len(), 1, "one alert for B1: {d}");
    let dd = &b1[0]["drilldown"];
    assert_eq!(dd["view"], "approval_requests", "{dd}");
    assert_eq!(dd["filter"]["branch"], B1, "{dd}");
    assert_eq!(dd["filter"]["type"], "RATE_PUBLICATION", "{dd}");
    assert_eq!(dd["dropped"], json!([]), "{dd}");
    assert!(alerts_of(&d, B2).is_empty(), "a decided request: {d}");
    assert!(alerts_of(&d, B9).is_empty(), "another organisation: {d}");
    room.finish().await;
}

// ------------------------------------------------------------ §9 row 7: unavailable metrics

/// Realized margin, booking conversion and commission are `unavailable` with `AwaitsLedger`,
/// `AwaitsBookingDesign` and `AwaitsBroker` (D-2): no value poses as a figure.
#[actix_web::test]
async fn the_three_unavailable_metrics_carry_their_reasons() {
    let room = Room::start().await;
    room.exchange(Ex {
        side: "SELL",
        ..Ex::default()
    })
    .await;
    let d = data(&overview(&room, EXECUTIVE, PERIOD).await, "A01");
    let keys: Vec<&str> = d["kpis"]
        .as_array()
        .expect("kpis")
        .iter()
        .map(|m| m["key"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(keys, KPI_KEYS, "the six KPIs of E-1, in order");
    for (key, reason) in [
        ("realized_margin_thb", "awaitsLedger"),
        ("booking_conversion", "awaitsBookingDesign"),
        ("commission_thb", "awaitsBroker"),
    ] {
        let m = kpi(&d, key);
        assert!(m["value"].is_null(), "{key} has no value: {m}");
        assert_eq!(m["unavailable"], reason, "{key}: {m}");
    }
    for key in ["buy_volume_thb", "sell_volume_thb", "repeat_customers"] {
        let m = kpi(&d, key);
        assert!(m["unavailable"].is_null(), "{key} is available: {m}");
        assert!(m["value"].is_string(), "{key} has a value: {m}");
    }
    room.finish().await;
}

// ------------------------------------------------------------ §9 rows 8 and 9: polling

/// The same `versions` sent back: `{changed: false}`, no data. One branch's version moved: only that branch's row,
/// plus the organisation KPIs.
#[actix_web::test]
async fn polling_answers_only_what_changed() {
    let room = Room::start().await;
    room.exchange(Ex::default()).await;
    room.exchange(Ex {
        branch: B2,
        ..Ex::default()
    })
    .await;

    let first = overview(&room, EXECUTIVE, PERIOD).await;
    let d = data(&first, "first read");
    assert_eq!(first.body["changed"], true, "{}", first.text);
    let versions = first.body["versions"].clone();
    let held: BTreeMap<String, i64> =
        serde_json::from_value(versions.clone()).unwrap_or_else(|_| panic!("versions: {versions}"));
    assert_eq!(
        held.keys().cloned().collect::<Vec<_>>(),
        vec![B1.to_owned(), B2.to_owned()],
        "the versions of the branches in scope, and no other organisation's"
    );
    assert_eq!(held[B1], room.version(B1).await);
    assert!(branch_row(&d, B1).is_some() && branch_row(&d, B2).is_some());

    let held_json = serde_json::to_string(&held).expect("json");
    let query = format!("{PERIOD}&versions={}", enc(&held_json));
    let same = overview(&room, EXECUTIVE, &query).await;
    assert_eq!(same.status, StatusCode::OK, "{}", same.text);
    assert_eq!(same.body["changed"], false, "{}", same.text);
    assert!(
        same.body.get("data").is_none_or(Value::is_null),
        "unchanged answers no data: {}",
        same.text
    );

    sqlx::query(
        "UPDATE branch_versions SET version = version + 1, changed_at = now() WHERE branch_id = $1",
    )
    .bind(B1)
    .execute(&room.pool)
    .await
    .expect("move B1");
    let moved = overview(&room, EXECUTIVE, &query).await;
    let d = data(&moved, "after B1 moved");
    assert_eq!(moved.body["changed"], true, "{}", moved.text);
    let rows: Vec<&str> = d["branches"]
        .as_array()
        .expect("branches")
        .iter()
        .map(|r| r["branchId"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(rows, vec![B1], "only the moved branch's row: {d}");
    assert_eq!(
        d["kpis"].as_array().map(Vec::len),
        Some(6),
        "the organisation KPIs, recomputed: {d}"
    );
    assert_eq!(
        kpi(&d, "buy_volume_thb")["value"],
        "200.00",
        "over both branches, not only the moved one"
    );
    assert_eq!(moved.body["versions"][B1], held[B1] + 1, "{}", moved.text);
    room.finish().await;
}

// ------------------------------------------------------------ §9 rows 10 and 11: scope

/// A FINANCE caller, who holds no `OverviewRead`: `outOfScope`, and no data.
#[actix_web::test]
async fn a_finance_caller_is_out_of_scope() {
    let room = Room::start().await;
    room.exchange(Ex::default()).await;
    let a = overview(&room, FINANCE, PERIOD).await;
    assert_eq!(
        (a.status, code(&a).as_str()),
        (StatusCode::FORBIDDEN, "outOfScope"),
        "{}",
        a.text
    );
    assert!(a.body.get("data").is_none_or(Value::is_null), "{}", a.text);
    room.finish().await;
}

/// A branch of another organisation named in `branches`: `outOfScope`, even for an organisation-wide EXECUTIVE.
#[actix_web::test]
async fn a_branch_of_another_organisation_is_out_of_scope() {
    let room = Room::start().await;
    room.exchange(Ex {
        branch: B9,
        ..Ex::default()
    })
    .await;
    let a = overview(&room, EXECUTIVE, &format!("{PERIOD}&branches={B9}")).await;
    assert_eq!(
        (a.status, code(&a).as_str()),
        (StatusCode::FORBIDDEN, "outOfScope"),
        "{}",
        a.text
    );
    // the same caller naming their own branch is answered, so the refusal is the scope's
    let d = data(
        &overview(&room, EXECUTIVE, &format!("{PERIOD}&branches={B1}")).await,
        "B1 only",
    );
    assert!(
        branch_row(&d, B1).is_some() && branch_row(&d, B2).is_none(),
        "{d}"
    );
    room.finish().await;
}

// ------------------------------------------------------------ §9 row 13: the stale-data alert

/// No `branch_alert_settings` row: no stale-data alert, however old the branch's unfinished transaction (E-3). The
/// same pending item at a branch whose threshold it passed does alert, so the absence is the threshold's.
#[actix_web::test]
async fn no_alert_settings_row_means_no_stale_data_alert() {
    let room = Room::start().await;
    let two_hours_ago = Utc::now() - chrono::Duration::hours(2);
    for branch in [B1, B2] {
        room.exchange(Ex {
            branch,
            status: "DRAFT",
            recorded_at: two_hours_ago,
            ..Ex::default()
        })
        .await;
    }
    sqlx::query(
        "INSERT INTO branch_alert_settings (organization_id, branch_id, stale_data_minutes, version, updated_by, updated_at) \
         VALUES ($1::uuid, $2, 30, 1, $3::uuid, now())",
    )
    .bind(ORG)
    .bind(B2)
    .bind(&room.teller[ORG])
    .execute(&room.pool)
    .await
    .expect("B2's stale threshold");
    let d = data(&overview(&room, EXECUTIVE, PERIOD).await, "A01");
    assert!(alerts_of(&d, B1).is_empty(), "B1 has no threshold row: {d}");
    assert_eq!(
        alerts_of(&d, B2).len(),
        1,
        "B2's pending item is older than its 30 minutes: {d}"
    );
    room.finish().await;
}

// ------------------------------------------------------------ §9 row 14: what FN-1 reads

/// `branch_day_totals(B1, d)`: the per-method totals equal the seeded rows. DEMO, unfinished, another day's and
/// another branch's rows are left out; posted reversals are 0 (none exist before RV-1).
#[actix_web::test]
async fn branch_day_totals_are_the_seeded_rows_per_payment_method() {
    let room = Room::start().await;
    for (side, thb, fee, bank_fee, method) in [
        ("BUY", "1000.00", "10.00", "0.00", "Cash"),
        ("SELL", "2000.00", "20.00", "0.00", "Cash"),
        ("SELL", "500.00", "5.00", "3.00", "PromptPay"),
    ] {
        room.exchange(Ex {
            side,
            thb,
            fee,
            bank_fee,
            method,
            ..Ex::default()
        })
        .await;
    }
    room.exchange(Ex {
        thb: "7000.00",
        live: false,
        ..Ex::default()
    })
    .await;
    room.exchange(Ex {
        thb: "7000.00",
        status: "DRAFT",
        ..Ex::default()
    })
    .await;
    room.exchange(Ex {
        thb: "7000.00",
        value_date: "2026-10-06",
        recorded_at: bangkok("2026-10-06", 10, 0),
        ..Ex::default()
    })
    .await;
    room.exchange(Ex {
        branch: B2,
        thb: "7000.00",
        ..Ex::default()
    })
    .await;

    let facts = WideTransactionFacts::new(room.pool.clone());
    let got = facts
        .branch_day_totals(&BranchId::new(B1), date("2026-10-05"))
        .await
        .expect("B1's day");
    let d = |s: &str| Decimal::from_str(s).expect("a decimal");
    assert_eq!(
        got,
        BranchDayTotals {
            by_method: vec![
                MethodDayTotals {
                    method: "Cash".into(),
                    buy_thb: d("1000.00"),
                    sell_thb: d("2000.00"),
                    fees_thb: d("30.00"),
                    bank_fees_thb: d("0.00"),
                },
                MethodDayTotals {
                    method: "PromptPay".into(),
                    buy_thb: d("0.00"),
                    sell_thb: d("500.00"),
                    fees_thb: d("5.00"),
                    bank_fees_thb: d("3.00"),
                },
            ],
            reversals_thb: d("0.00"),
        }
    );
    room.finish().await;
}

// ------------------------------------------------------------ the ruling on #1071: COMPLETED only

/// The lane controller's ruling on #1071 (issuecomment-6107626913): a DRAFT, a CONFIRMED and a CANCELLED transaction
/// are each excluded from the volumes, from repeat customers and from `branch_day_totals`.
///
/// Seeded at B1 on 2026-10-05, all BUY Cash: one COMPLETED row of 100, and per status S:
/// - an S row of 1000 / 2000 / 4000 (excluded from every figure);
/// - customer `C-<S>-EARLIER`, whose earlier row (2026-10-01, B2) is S and whose row in the period is a COMPLETED 1:
///   not a repeat customer, since the earlier row never completed;
/// - customer `C-<S>-NOW`, whose earlier row is COMPLETED and whose row in the period is an S row of 8: not a
///   repeat customer in the period, since its row in the period never completed.
///
/// One customer, `C-OK`, has an earlier COMPLETED row and a COMPLETED 1 in the period: the one repeat customer.
#[actix_web::test]
async fn draft_confirmed_and_cancelled_rows_are_in_no_figure() {
    let room = Room::start().await;
    let earlier = |status| Ex {
        branch: B2,
        thb: "1.00",
        value_date: "2026-10-01",
        recorded_at: bangkok("2026-10-01", 10, 0),
        status,
        ..Ex::default()
    };
    room.exchange(Ex {
        thb: "100.00",
        ..Ex::default()
    })
    .await;
    for (status, thb) in [
        ("DRAFT", "1000.00"),
        ("CONFIRMED", "2000.00"),
        ("CANCELLED", "4000.00"),
    ] {
        room.exchange(Ex {
            thb,
            status,
            ..Ex::default()
        })
        .await;
        let who = format!("C-{status}-EARLIER");
        room.exchange(Ex {
            customer: &who,
            ..earlier(status)
        })
        .await;
        room.exchange(Ex {
            customer: &who,
            thb: "1.00",
            recorded_at: bangkok("2026-10-05", 12, 0),
            ..Ex::default()
        })
        .await;
        let who = format!("C-{status}-NOW");
        room.exchange(Ex {
            customer: &who,
            ..earlier("COMPLETED")
        })
        .await;
        room.exchange(Ex {
            customer: &who,
            thb: "8.00",
            status,
            recorded_at: bangkok("2026-10-05", 12, 0),
            ..Ex::default()
        })
        .await;
    }
    room.exchange(Ex {
        customer: "C-OK",
        ..earlier("COMPLETED")
    })
    .await;
    room.exchange(Ex {
        customer: "C-OK",
        thb: "1.00",
        recorded_at: bangkok("2026-10-05", 12, 0),
        ..Ex::default()
    })
    .await;

    // completed in the period at B1: 100 + three 1s (C-<S>-EARLIER) + 1 (C-OK)
    let d = data(&overview(&room, EXECUTIVE, PERIOD).await, "A01");
    assert_eq!(kpi(&d, "buy_volume_thb")["value"], "104.00", "{d}");
    assert_eq!(kpi(&d, "sell_volume_thb")["value"], "0.00", "{d}");
    assert_eq!(
        branch_row(&d, B1).expect("B1's row")["volumeThb"],
        "104.00",
        "{d}"
    );
    assert_eq!(kpi(&d, "repeat_customers")["value"], "1", "C-OK only: {d}");

    let got = WideTransactionFacts::new(room.pool.clone())
        .branch_day_totals(&BranchId::new(B1), date("2026-10-05"))
        .await
        .expect("B1's day");
    let d = |s: &str| Decimal::from_str(s).expect("a decimal");
    assert_eq!(
        got,
        BranchDayTotals {
            by_method: vec![MethodDayTotals {
                method: "Cash".into(),
                buy_thb: d("104.00"),
                sell_thb: d("0.00"),
                fees_thb: d("0.00"),
                bank_fees_thb: d("0.00"),
            }],
            reversals_thb: d("0.00"),
        }
    );
    room.finish().await;
}
