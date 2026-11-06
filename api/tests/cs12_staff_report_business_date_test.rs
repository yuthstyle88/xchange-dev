//! Up-front tests for CS-12a (design `docs/design/configurable-shifts.md` row R2): the staff daily report,
//! `GET /api/v3/views/staff_daily_report?branch=&day=`, dates by **business date**, not by the calendar day.
//! Written by the ticket author claude/35271802 — the implementer must not edit, delete or weaken these.
//!
//! - **R2a** sales (`completed`, `buyVolume`, `sellVolume`) are the COMPLETED LIVE exchanges whose `value_date` is
//!   `day`, whenever they were completed;
//! - **R2b** the drawer and the signers come from the verified CLOSE counts of the shifts whose `business_date` is
//!   `day`, whenever they opened;
//! - **R2c** the status counts (counted vs closed shifts) are those of the shifts whose `business_date` is `day`.
//!
//! MAIN keeps the migration's default plan (A 06–14, B 14–22, C 22–06, Asia/Bangkok), so an OPEN count at 02:00
//! opens shift C of the day before (`ShiftPlan::covering`) and that shift's `business_date` is the day before.
//!
//! **How the data is written.** Exchanges go through `tx5_common` (quote, the TX-2 store, confirm, complete). That
//! fixture stores every LIVE draft with a fixed `value_date`, so each sale here is given the `value_date` its shift's
//! business date gives it in production (CS-8) by one SQL `UPDATE` before it completes. Counts go through
//! `CashCountService::{submit, verify}`; the OPEN count's template choice dates the shift. MAIN's v2 shift (the one
//! the exchanges post to) is opened on 2026-01-01, a day no test here reads, so it is in no figure.
//!
//! Needs `TEST_DATABASE_URL`; nothing skips. Each test works in a schema of its own.

mod common;
mod quote_common;
mod shift_plan_common;
mod tx4_common;
mod tx5_common;

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::{Arc, Once};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use pos_infra::transactions::Transactor;
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tx4_common::*;
use tx5_common::*;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_cash::application::count::count_service::{CashCountService, SubmitCount};
use xc_cash::domain::count::cash_count::CountKind;
use xc_cash::domain::count::count_line::CountLine;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::transaction::TransactionId;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::Side;
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const VIEW: &str = "staff_daily_report";
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const MAIN: &str = "MAIN";

static PII: Once = Once::new();
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

// ------------------------------------------------------------ sessions

#[derive(Clone)]
struct Who {
    id: String,
    name: String,
    spec: String,
}

/// `<user id>|<ROLE>|<branch>`.
fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(
        User::new(
            id,
            ORG_1,
            vec![ScopedRole::new(
                role,
                Some(BranchId::new(branch)),
                from,
                None,
            )],
            None,
        )
        .with_branch(Some(BranchId::new(branch))),
    )
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get(AS)
            .map(|v| v.to_str().expect("ASCII").to_owned())
            .unwrap_or_default();
        Box::pin(async move { test_callers::callers(None, user_of(&spec)) })
    }
}

// ------------------------------------------------------------ the branch

/// MAIN with the default plan, custody on, a funded safe, and a v2 shift opened on 2026-01-01 that the exchanges
/// post to.
struct Day {
    c: Counter,
    counts: CashCountService,
}

impl Day {
    async fn start() -> Self {
        PII.call_once(|| {
            let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
            let _ = xc_customer::install_pii_keys(keys);
        });
        let desk = Desk::start().await;
        let pool = desk.db.pool.clone();
        desk.db
            .exec("CREATE TABLE tx5_posted (source_id text NOT NULL)")
            .await;
        desk.db
            .exec("CREATE TABLE tx5_effects (name text NOT NULL, transaction_id uuid NOT NULL)")
            .await;
        let teller_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             SELECT id, 'test', 'PENDING', 'test' FROM organizations WHERE code = 'XC' \
             RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .expect("seed the v2 shift's opener");
        desk.db
            .exec(&format!(
                "INSERT INTO ops_state (branch_id, phase) VALUES ('{MAIN}', 'OPEN') \
                 ON CONFLICT (branch_id) DO UPDATE SET phase = 'OPEN'"
            ))
            .await;
        let codes: Vec<String> = sqlx::query_scalar("SELECT trim(code) FROM currencies ORDER BY 1")
            .fetch_all(&pool)
            .await
            .expect("the currency register");
        for code in &codes {
            lg4_fund(
                &pool,
                xc_cash::domain::posting::CashPlace::Bank,
                xc_cash::domain::posting::CashPlace::Safe(BranchId::new(MAIN)),
                code,
                "1000000000",
            )
            .await;
        }
        desk.db
            .exec(&format!(
                "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, \
                 updated_at) VALUES ('{ORG_1}', '{MAIN}', 100000, 'test', now())"
            ))
            .await;
        // MAIN's v2 shift, written straight into the table in one branch transaction (setup only), opened on a day
        // no report here reads
        let shift = format!("SH-{}", uuid::Uuid::new_v4());
        let (id, request) = (shift.clone(), unique("open-shift").replace('_', "-"));
        let opened_at = Utc.with_ymd_and_hms(2026, 1, 1, 5, 0, 0).unwrap();
        BranchTx::new(Transactor::new(pool.clone()))
            .run(&BranchId::new(MAIN), move |tx| {
                Box::pin(async move {
                    sqlx::query(
                        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, close_note, \
                         open_request_id, teller_id) VALUES ($1, $2, 'OPEN', $3, 'test', '', $4, $5)",
                    )
                    .bind(&id)
                    .bind(MAIN)
                    .bind(opened_at)
                    .bind(&request)
                    .bind(teller_id)
                    .execute(&mut **tx)
                    .await?;
                    Ok::<(), pos_error::AppError>(())
                })
            })
            .await
            .unwrap_or_else(|e| panic!("open MAIN's v2 shift: {e}"));
        let scratch = std::env::temp_dir().join(unique("xc-cs12"));
        std::fs::create_dir_all(&scratch).unwrap();
        let c = Counter {
            files: xchange::infrastructure::files::file_service::FileService::new(
                pool.clone(),
                scratch.join("uploads"),
            ),
            desk,
            shift,
            scratch,
        };
        Self {
            counts: CashCountService::pg(pool),
            c,
        }
    }

    async fn finish(self) {
        self.c.finish().await;
    }

    fn db(&self) -> &TestDb {
        &self.c.desk.db
    }

    async fn person(&self, role: Role) -> Who {
        let name = unique(&format!("{} person", role.as_str()));
        let id: String = sqlx::query_scalar(
            "INSERT INTO users (organization_id, name, status, created_by) \
             VALUES ($1::uuid, $2, 'ACTIVE', 'test') RETURNING id::text",
        )
        .bind(ORG_1)
        .bind(&name)
        .fetch_one(&self.db().pool)
        .await
        .expect("insert a user");
        sqlx::query(
            "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
             VALUES ($1::uuid, $2, $3::uuid, $4, '2026-01-01T00:00:00Z')",
        )
        .bind(&id)
        .bind(role.as_str())
        .bind(ORG_1)
        .bind(MAIN)
        .execute(&self.db().pool)
        .await
        .expect("give the user a role");
        Who {
            spec: format!("{id}|{}|{MAIN}", role.as_str()),
            id,
            name,
        }
    }

    // -------------------------------------------------------- exchanges

    /// A LIVE exchange of `quantity` USD on `side`, completed at `at`, whose `value_date` is `value_date` (the
    /// business date its shift gives it, CS-8). Returns it and its Σ `thb_amount`.
    async fn sale(
        &self,
        side: Side,
        quantity: &str,
        at: DateTime<Utc>,
        value_date: NaiveDate,
    ) -> (TransactionId, Decimal) {
        let start = at - Duration::seconds(2);
        let q = self.c.desk.quote(MAIN, side, quantity, None, start).await;
        let t = self.c.live_of(&q, PaymentMethod::Cash, true, start).await;
        sqlx::query("UPDATE transactions SET value_date = $2 WHERE id = $1")
            .bind(t.id.0)
            .bind(value_date)
            .execute(&self.db().pool)
            .await
            .expect("date the sale");
        self.c
            .complete(&Ledger::new("0.00", None), t.id, at)
            .await
            .unwrap_or_else(|e| panic!("complete: {e}"));
        let thb = self
            .db()
            .text(&format!(
                "SELECT SUM(thb_amount)::text FROM transaction_lines WHERE transaction_id = '{}'",
                t.id.0
            ))
            .await
            .expect("the lines");
        (t.id, Decimal::from_str(&thb).unwrap())
    }

    // -------------------------------------------------------- counts

    async fn submit(
        &self,
        teller: &Who,
        kind: CountKind,
        shift: Option<&str>,
        lines: &[(&str, &str, i32)],
        at: DateTime<Utc>,
    ) -> String {
        let mut count_lines = Vec::new();
        for (currency, face, qty) in lines {
            let denomination: i64 = sqlx::query_scalar(
                "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
            )
            .bind(currency)
            .bind(face)
            .fetch_one(&self.db().pool)
            .await
            .unwrap_or_else(|e| panic!("the {currency} {face} face value: {e}"));
            count_lines.push(CountLine::new(currency, denomination, *qty));
        }
        self.counts
            .submit(
                &user_of(&teller.spec).unwrap(),
                SubmitCount {
                    branch: BranchId::new(MAIN),
                    shift_id: shift.map(str::to_owned),
                    kind,
                    lines: count_lines,
                    note: None,
                    idempotency_key: unique("count"),
                },
                at,
            )
            .await
            .unwrap_or_else(|e| panic!("submit a {kind:?} count: {e}"))
            .id
    }

    async fn verify(&self, manager: &Who, count: &str, at: DateTime<Utc>) {
        self.counts
            .verify(&user_of(&manager.spec).unwrap(), count, at)
            .await
            .unwrap_or_else(|e| panic!("verify count {count}: {e}"));
    }

    /// A new teller's shift, opened by an OPEN count submitted at `at` and verified 1 s later. Returns the teller,
    /// the shift and the shift's `business_date` as stored.
    async fn counted_shift(&self, manager: &Who, at: DateTime<Utc>) -> (Who, String, NaiveDate) {
        let teller = self.person(Role::Teller).await;
        let count = self.submit(&teller, CountKind::Open, None, LINES, at).await;
        self.verify(manager, &count, at + Duration::seconds(1))
            .await;
        let (shift, business_date): (String, NaiveDate) = sqlx::query_as(
            "SELECT id, business_date FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'",
        )
        .bind(&teller.id)
        .fetch_one(&self.db().pool)
        .await
        .expect("the verified OPEN count opened the teller's shift");
        (teller, shift, business_date)
    }

    /// The shift's CLOSE count, submitted at `at` and verified 1 s later. Returns its id.
    async fn close(&self, teller: &Who, manager: &Who, shift: &str, at: DateTime<Utc>) -> String {
        let count = self
            .submit(teller, CountKind::Close, Some(shift), LINES, at)
            .await;
        self.verify(manager, &count, at + Duration::seconds(1))
            .await;
        count
    }

    /// The stored `counted` of `count` for `currency`, as a number.
    async fn counted(&self, count: &str, currency: &str) -> Decimal {
        let t = self
            .db()
            .text(&format!(
                "SELECT SUM(counted)::text FROM cash_count_totals \
                 WHERE count_id = '{count}'::uuid AND currency = '{currency}'"
            ))
            .await
            .expect("the count's total");
        Decimal::from_str(&t).unwrap()
    }

    // -------------------------------------------------------- the view

    /// The report of `day` as `who`, which must be answered 200.
    async fn report(&self, who: &Who, day: NaiveDate) -> Value {
        self.report_query(who, &format!("&day={day}")).await
    }

    /// The report with no `day` (CS-12b), which must be answered 200.
    async fn report_today(&self, who: &Who) -> Value {
        self.report_query(who, "").await
    }

    async fn report_query(&self, who: &Who, day_param: &str) -> Value {
        let state = AppState::new(
            Arc::new(common::settings(&[(
                "APP_FILES__DIR",
                self.c.scratch.join("uploads").to_str().expect("UTF-8"),
            )])),
            self.db().pool.clone(),
        );
        let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
        let app = atest::init_service(
            App::new()
                .configure(move |cfg| bootstrap::configure(cfg, state))
                .app_data(web::Data::from(reader)),
        )
        .await;
        let req = atest::TestRequest::get()
            .uri(&format!("/api/v3/views/{VIEW}?branch={MAIN}{day_param}"))
            .insert_header((header::HOST, HOST))
            .insert_header((AS, who.spec.clone()))
            .to_request();
        let resp = atest::call_service(&app, req).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let body: Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
        assert_eq!(status, StatusCode::OK, "the report ({day_param}): {body}");
        let data = body["data"].clone();
        assert!(data.is_object(), "a report: {body}");
        data
    }
}

// ------------------------------------------------------------ helpers

/// One OPEN and one CLOSE count's lines: THB 5 × 1000, USD 3 × 100.
const LINES: &[(&str, &str, i32)] = &[("THB", "1000", 5), ("USD", "100", 3)];

fn bangkok() -> Tz {
    "Asia/Bangkok".parse().unwrap()
}

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("a date")
}

/// The instant `h:m` of `day` in Bangkok.
fn at(day: &str, h: u32, m: u32) -> DateTime<Utc> {
    bangkok()
        .from_local_datetime(&date(day).and_hms_opt(h, m, 0).unwrap())
        .single()
        .expect("an unambiguous local time")
        .with_timezone(&Utc)
}

fn number(v: &Value, what: &str) -> Decimal {
    let text = v
        .as_str()
        .unwrap_or_else(|| panic!("{what} is a decimal string: {v}"));
    Decimal::from_str(text).unwrap_or_else(|e| panic!("{what} = {text:?}: {e}"))
}

fn completed(report: &Value) -> u64 {
    report["completed"]
        .as_u64()
        .unwrap_or_else(|| panic!("completed is a JSON number: {report}"))
}

/// The `currencies` row of `code`.
fn row<'a>(report: &'a Value, code: &str) -> &'a Value {
    report["currencies"]
        .as_array()
        .unwrap_or_else(|| panic!("currencies is a list: {report}"))
        .iter()
        .find(|r| r["currency"] == code)
        .unwrap_or_else(|| panic!("a {code} row: {report}"))
}

/// The signers as sorted (name, capacity) pairs.
fn signers(report: &Value) -> Vec<(String, String)> {
    let mut got: Vec<(String, String)> = report["signers"]
        .as_array()
        .unwrap_or_else(|| panic!("signers is a list: {report}"))
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap_or("").to_owned(),
                s["capacity"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    got.sort();
    got
}

fn signed_by(teller: &Who, manager: &Who) -> Vec<(String, String)> {
    let mut want = vec![
        (teller.name.clone(), "counted".to_owned()),
        (manager.name.clone(), "verified".to_owned()),
    ];
    want.sort();
    want
}

// ------------------------------------------------------------ R2a: sales by value_date

/// R2a: a sale completed 5 Nov 02:00 Bangkok whose `value_date` is 4 Nov is on the 4 Nov report and not on the
/// 5 Nov one; a sale completed 4 Nov 01:00 whose `value_date` is 3 Nov is on 3 Nov and not on 4 Nov; a noon sale
/// of 4 Nov dated 4 Nov stays on 4 Nov. Count and both volumes follow `value_date`.
#[actix_web::test]
async fn r2a_sales_are_counted_on_their_value_date_not_on_the_day_they_completed() {
    let d = Day::start().await;
    let reader = d.person(Role::BranchManager).await;
    let (_, noon_buy) = d
        .sale(
            Side::Buy,
            "100",
            at("2026-11-04", 12, 0),
            date("2026-11-04"),
        )
        .await;
    let (_, late_buy) = d
        .sale(Side::Buy, "50", at("2026-11-05", 2, 0), date("2026-11-04"))
        .await;
    let (_, early_sell) = d
        .sale(Side::Sell, "40", at("2026-11-04", 1, 0), date("2026-11-03"))
        .await;

    let nov4 = d.report(&reader, date("2026-11-04")).await;
    assert_eq!(
        completed(&nov4),
        2,
        "4 Nov: the noon buy and the 02:00-of-5-Nov buy: {nov4}"
    );
    assert_eq!(
        number(&nov4["buyVolume"], "buyVolume"),
        noon_buy + late_buy,
        "4 Nov buys are those dated 4 Nov: {nov4}"
    );
    assert_eq!(
        number(&nov4["sellVolume"], "sellVolume"),
        Decimal::ZERO,
        "the 01:00 sell of 4 Nov is dated 3 Nov: {nov4}"
    );

    let nov5 = d.report(&reader, date("2026-11-05")).await;
    assert_eq!(completed(&nov5), 0, "5 Nov has no sale dated 5 Nov: {nov5}");
    assert_eq!(
        number(&nov5["buyVolume"], "buyVolume"),
        Decimal::ZERO,
        "{nov5}"
    );

    let nov3 = d.report(&reader, date("2026-11-03")).await;
    assert_eq!(completed(&nov3), 1, "3 Nov: the sell dated 3 Nov: {nov3}");
    assert_eq!(
        number(&nov3["sellVolume"], "sellVolume"),
        early_sell,
        "{nov3}"
    );
    d.finish().await;
}

// ------------------------------------------------------------ R2b: drawer and signers by business_date

/// R2b, guard: shift C opened 4 Nov 22:00 (business date 4 Nov), its CLOSE count verified 5 Nov 06:30, is in the
/// 4 Nov drawer and signers and not in the 5 Nov ones.
#[actix_web::test]
async fn r2b_shift_c_opened_at_22_is_in_its_business_dates_drawer_and_signers() {
    let d = Day::start().await;
    let reader = d.person(Role::BranchManager).await;
    let manager = d.person(Role::BranchManager).await;
    let (teller, shift, business_date) = d.counted_shift(&manager, at("2026-11-04", 22, 0)).await;
    assert_eq!(
        business_date,
        date("2026-11-04"),
        "the fixture: shift C of 4 Nov"
    );
    let close = d
        .close(&teller, &manager, &shift, at("2026-11-05", 6, 30))
        .await;

    let nov4 = d.report(&reader, date("2026-11-04")).await;
    for code in ["THB", "USD"] {
        assert_eq!(
            number(&row(&nov4, code)["drawerCounted"], "drawerCounted"),
            d.counted(&close, code).await,
            "4 Nov {code} drawer holds the shift's close count: {nov4}"
        );
    }
    assert_eq!(signers(&nov4), signed_by(&teller, &manager), "{nov4}");

    let nov5 = d.report(&reader, date("2026-11-05")).await;
    for code in ["THB", "USD"] {
        assert!(
            row(&nov5, code)["drawerCounted"].is_null(),
            "5 Nov {code} drawer does not hold it: {nov5}"
        );
    }
    assert_eq!(nov5["signers"], json!([]), "{nov5}");
    d.finish().await;
}

/// R2b: shift C opened 5 Nov 02:00 belongs to business date 4 Nov (it is the 22:00–06:00 shift that began 4 Nov),
/// so its CLOSE count, verified 5 Nov 06:30, is in the 4 Nov drawer and signers and not in the 5 Nov ones, though
/// the shift opened on the calendar day 5 Nov.
#[actix_web::test]
async fn r2b_shift_c_opened_after_midnight_is_in_its_business_dates_drawer_and_signers() {
    let d = Day::start().await;
    let reader = d.person(Role::BranchManager).await;
    let manager = d.person(Role::BranchManager).await;
    let (teller, shift, business_date) = d.counted_shift(&manager, at("2026-11-05", 2, 0)).await;
    assert_eq!(
        business_date,
        date("2026-11-04"),
        "the fixture: an OPEN count at 02:00 opens shift C of 4 Nov"
    );
    let close = d
        .close(&teller, &manager, &shift, at("2026-11-05", 6, 30))
        .await;

    let nov4 = d.report(&reader, date("2026-11-04")).await;
    for code in ["THB", "USD"] {
        let got = &row(&nov4, code)["drawerCounted"];
        assert!(
            got.is_string(),
            "4 Nov {code} drawer holds the shift's close count: {nov4}"
        );
        assert_eq!(
            number(got, "drawerCounted"),
            d.counted(&close, code).await,
            "{nov4}"
        );
    }
    assert_eq!(
        signers(&nov4),
        signed_by(&teller, &manager),
        "4 Nov signers are the close count's counter and verifier: {nov4}"
    );

    let nov5 = d.report(&reader, date("2026-11-05")).await;
    for code in ["THB", "USD"] {
        assert!(
            row(&nov5, code)["drawerCounted"].is_null(),
            "5 Nov {code} drawer does not hold a shift of 4 Nov: {nov5}"
        );
    }
    assert_eq!(nov5["signers"], json!([]), "5 Nov has no signer: {nov5}");
    d.finish().await;
}

// ------------------------------------------------------------ R2c: the status by business_date

/// R2c: the status counts the shifts of the business date. On 4 Nov, shift A (opened 4 Nov 10:00) is closed and
/// shift C (opened 5 Nov 02:00, business date 4 Nov) is still open: 4 Nov is preliminary. Once C's CLOSE count is
/// verified (5 Nov 05:30), 4 Nov is closed awaiting reconciliation and 5 Nov, which has no shift, is preliminary.
#[actix_web::test]
async fn r2c_the_status_counts_the_shifts_of_the_business_date() {
    let d = Day::start().await;
    let reader = d.person(Role::BranchManager).await;
    let manager = d.person(Role::BranchManager).await;
    let (a, a_shift, a_date) = d.counted_shift(&manager, at("2026-11-04", 10, 0)).await;
    let (c, c_shift, c_date) = d.counted_shift(&manager, at("2026-11-05", 2, 0)).await;
    assert_eq!(
        (a_date, c_date),
        (date("2026-11-04"), date("2026-11-04")),
        "the fixture: shift A and shift C of 4 Nov"
    );
    d.close(&a, &manager, &a_shift, at("2026-11-04", 13, 30))
        .await;

    let nov4 = d.report(&reader, date("2026-11-04")).await;
    assert_eq!(
        nov4["status"], "preliminary",
        "4 Nov: shift C of 4 Nov is not closed yet: {nov4}"
    );

    d.close(&c, &manager, &c_shift, at("2026-11-05", 5, 30))
        .await;
    let nov4 = d.report(&reader, date("2026-11-04")).await;
    assert_eq!(
        nov4["status"], "closedAwaitingReconciliation",
        "4 Nov: both shifts of 4 Nov are closed: {nov4}"
    );
    let nov5 = d.report(&reader, date("2026-11-05")).await;
    assert_eq!(
        nov5["status"], "preliminary",
        "5 Nov has no shift of its own: {nov5}"
    );
    d.finish().await;
}

/// R4 (CS-12b, re-review of PR 6274 out-of-scope item 2, ruled by the lane controller claude/022a7453): with no `day`,
/// the report opens on the current **business** day, `ShiftPlan::business_date_of(now)`, not the calendar date. Between
/// midnight and the cutover those differ. The view reads the real clock, so the test makes "now is before the cutover"
/// true whatever the time: MAIN takes a timezone where it is before 10:00 locally, and a 24 h plan whose cutover is two
/// hours from now. Before 10:00 both starts fall on the same day (cutover ≤ 11:00, the second start ≤ 23:00), so the
/// cutover, the earliest start (T1), is the first one. The business day is then yesterday's local date. Written by the
/// ticket author claude/35271802 (fixed after gemini/studio-14 found the wrap past midnight on #6305).
#[actix_web::test]
async fn r4_with_no_day_the_report_opens_on_the_current_business_day() {
    let d = Day::start().await;
    let reader = d.person(Role::BranchManager).await;
    let now = Utc::now();
    let zone = [
        "Asia/Bangkok",
        "Europe/London",
        "America/New_York",
        "Pacific/Honolulu",
        "Asia/Tokyo",
    ]
    .into_iter()
    .find(|z| {
        let tz: Tz = z.parse().unwrap();
        now.with_timezone(&tz).hour() < 10
    })
    .expect("the five offsets (0, 7, 9, 14, 19 mod 24) leave no 10-hour gap, so one zone is before 10:00");
    let tz: Tz = zone.parse().unwrap();
    let local = now.with_timezone(&tz);
    let cutover = local.hour() + 2;
    let (a, b) = (
        format!("{cutover:02}:00"),
        format!("{:02}:00", (cutover + 12) % 24),
    );
    shift_plan_common::set_timezone(&d.db().pool, MAIN, zone).await;
    shift_plan_common::set_plan(&d.db().pool, MAIN, &[("X", &a, &b), ("Y", &b, &a)]).await;

    let report = d.report_today(&reader).await;

    let calendar = local.date_naive();
    let business = calendar.pred_opt().unwrap();
    assert_eq!(
        report["day"],
        json!(business.to_string()),
        "R4: at {local} in {zone} with cutover {a}, the business day is {business}, not the calendar {calendar}: {report}"
    );
    d.finish().await;
}
