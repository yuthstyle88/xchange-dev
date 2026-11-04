//! Board currencies BC-4 (`docs/design/board-currencies.md` §3, §4, §9):
//! - `QuoteService::price_in` refuses `Display::Ask` unless the channel is `QuoteChannel::Desk`, with `rateUnavailable`.
//! - `staff_daily_report` lists the branch's `branch_currencies` ∪ `inventory_positions` (`quantity <> 0`) ∪ the codes
//!   counted that day, THB first, by `sort_order`, then by code, keeping 3-place count figures intact.
//!
//! Needs `TEST_DATABASE_URL` naming a database ending in `_test`.

mod common;
mod quote_common;
mod tx4_common;
mod tx5_common;

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::{Arc, Once};

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::Value;
use tx4_common::*;
use tx5_common::lg4_fund;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_cash::application::count::count_service::{CashCountService, SubmitCount};
use xc_cash::domain::count::cash_count::CountKind;
use xc_cash::domain::count::count_line::CountLine;
use xc_cash::domain::posting::CashPlace;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::quote::quote::{Quote, QuoteChannel};
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Side};
use xc_rate::domain::currency::denomination_groups::GroupCode;
use xchange::application::workflows::exchange::quote::{
    QuoteRequest, QuoteRequestLine, QuoteService,
};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const MAIN: &str = "MAIN";
const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SGD: Line = Line::manual("SGD", "1", "24.10", "24.50");

static PII: Once = Once::new();
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

fn ensure_pii() {
    PII.call_once(|| {
        let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
        let _ = xc_customer::install_pii_keys(keys);
    });
}

async fn quote_channel(
    db: &TestDb,
    currency: &str,
    channel: QuoteChannel,
) -> pos_error::AppResult<Quote> {
    let mut tx = db.pool.begin().await.expect("begin");
    let got = QuoteService::new(db.pool.clone())
        .quote_in(
            &mut tx,
            QuoteRequest {
                branch_id: BranchId::new(MAIN),
                customer_id: None,
                channel,
                lines: vec![QuoteRequestLine {
                    currency: CurrencyCode::parse(currency).expect("currency"),
                    shop_side: Side::Buy,
                    quantity: ForeignAmount::from_decimal(Decimal::from_str("100").unwrap()),
                    fee_thb: None,
                    denomination_group: GroupCode::all(),
                }],
                payment_method: Some(PaymentMethod::Cash),
            },
            &teller(),
            whole_second_now(),
        )
        .await;
    if got.is_ok() {
        tx.commit().await.expect("commit");
    } else {
        tx.rollback().await.expect("rollback");
    }
    got
}

struct HeaderUsers;

impl SessionReader for HeaderUsers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let id = req
            .headers()
            .get(AS)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        Box::pin(async move {
            let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
            let user = (!id.is_empty()).then(|| {
                User::new(
                    &id,
                    ORG_1,
                    vec![ScopedRole::new(
                        Role::BranchManager,
                        Some(BranchId::new(MAIN)),
                        from,
                        None,
                    )],
                    None,
                )
                .with_branch(Some(BranchId::new(MAIN)))
            });
            test_callers::callers(None, user)
        })
    }
}

async fn seed_manager(db: &TestDb) -> String {
    let id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, 'manager', 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(ORG_1)
    .fetch_one(&db.pool)
    .await
    .expect("insert manager");
    sqlx::query(
        "INSERT INTO user_roles (user_id, role_code, organization_id, branch_id, valid_from) \
         VALUES ($1::uuid, 'BRANCH_MANAGER', $2::uuid, $3, '2026-01-01T00:00:00Z')",
    )
    .bind(&id)
    .bind(ORG_1)
    .bind(MAIN)
    .execute(&db.pool)
    .await
    .expect("grant manager role");
    id
}

async fn fetch_report(db: &TestDb, manager_id: &str, day: NaiveDate) -> Value {
    let scratch = std::env::temp_dir().join(unique("bc4-rep"));
    std::fs::create_dir_all(&scratch).unwrap();
    let state = AppState::new(
        Arc::new(common::settings(&[(
            "APP_FILES__DIR",
            scratch.to_str().expect("UTF-8"),
        )])),
        db.pool.clone(),
    );
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderUsers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;
    let req = atest::TestRequest::get()
        .uri(&format!(
            "/api/v3/views/staff_daily_report?branch={MAIN}&day={day}"
        ))
        .insert_header((header::HOST, HOST))
        .insert_header((AS, manager_id.to_owned()))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = atest::read_body(resp).await;
    let _ = std::fs::remove_dir_all(&scratch);
    let body: Value = serde_json::from_slice(&bytes).expect("json report");
    body["data"].clone()
}

/// Row 1: Desk on `ASK` with a line is quoted (while non-desk is refused).
#[tokio::test]
async fn bc4_desk_on_ask_with_a_line_is_quoted() {
    ensure_pii();
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD, SGD]).await;

    let desk_quote = quote_channel(&db, "SGD", QuoteChannel::Desk)
        .await
        .expect("Desk on ASK with a published line is quoted");
    assert_eq!(desk_quote.lines[0].currency.as_str(), "SGD");

    let app_quote = quote_channel(&db, "SGD", QuoteChannel::App).await;
    assert_eq!(code(app_quote), "rateUnavailable");

    db.finish().await;
}

/// Row 2: Booking, App or Kiosk on `ASK` is refused with `rateUnavailable`.
#[tokio::test]
async fn bc4_booking_app_or_kiosk_on_ask_is_refused() {
    ensure_pii();
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD, SGD]).await;

    for channel in [
        QuoteChannel::Booking,
        QuoteChannel::App,
        QuoteChannel::Kiosk,
    ] {
        let got = quote_channel(&db, "SGD", channel).await;
        assert_eq!(
            code(got),
            "rateUnavailable",
            "channel {channel:?} on ASK must be refused with rateUnavailable"
        );
    }

    db.finish().await;
}

/// Row 3: any channel on `SHOW` is quoted as today (while `ASK` off-desk is refused).
#[tokio::test]
async fn bc4_any_channel_on_show_is_quoted_as_today() {
    ensure_pii();
    let db = TestDb::start().await;
    db.publish(MAIN, &[USD, SGD]).await;

    for channel in [
        QuoteChannel::Desk,
        QuoteChannel::Booking,
        QuoteChannel::App,
        QuoteChannel::Kiosk,
    ] {
        let got = quote_channel(&db, "USD", channel)
            .await
            .unwrap_or_else(|e| panic!("channel {channel:?} on SHOW must succeed: {e}"));
        assert_eq!(got.lines[0].currency.as_str(), "USD");
    }

    assert_eq!(
        code(quote_channel(&db, "SGD", QuoteChannel::Kiosk).await),
        "rateUnavailable"
    );

    db.finish().await;
}

/// Row 4: report with no counts lists the branch's set, by `sort_order`, THB first.
#[actix_web::test]
async fn bc4_report_with_no_counts_lists_the_set_by_sort_order_thb_first() {
    ensure_pii();
    let db = TestDb::start().await;
    let manager_id = seed_manager(&db).await;

    // Even if THB's sort_order is moved behind USD, THB stays first.
    db.exec(
        "UPDATE branch_currencies SET sort_order = 999 \
         WHERE branch_id = 'MAIN' AND currency_code = 'THB'",
    )
    .await;

    let expected: Vec<String> = sqlx::query_scalar(
        "SELECT trim(currency_code) FROM branch_currencies \
         WHERE branch_id = 'MAIN' \
         GROUP BY trim(currency_code) \
         ORDER BY trim(currency_code) <> 'THB', min(sort_order), trim(currency_code)",
    )
    .fetch_all(&db.pool)
    .await
    .expect("expected set order");
    assert_eq!(expected.len(), 40);
    assert_eq!(expected[0], "THB");

    let report_day = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
    let report = fetch_report(&db, &manager_id, report_day).await;
    let got: Vec<String> = report["currencies"]
        .as_array()
        .expect("currencies array")
        .iter()
        .map(|r| r["currency"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(got, expected);

    db.finish().await;
}

/// Row 5: report with KWD held (`HIDE`) lists KWD with real figures, 3 places intact.
#[actix_web::test]
async fn bc4_report_kwd_held_and_hide_is_listed_with_real_three_place_figures() {
    ensure_pii();
    let db = TestDb::start().await;
    let manager_id = seed_manager(&db).await;

    let display = db
        .text(
            "SELECT display FROM branch_currencies \
             WHERE branch_id = 'MAIN' AND currency_code = 'KWD' AND group_code = 'ALL'",
        )
        .await;
    assert_eq!(display.as_deref(), Some("HIDE"));

    // Fund SAFE(MAIN) with 1.123 KWD (3 decimal places) and count 301 x 0.005 = 1.505 KWD (diff +0.382).
    db.exec(
        "INSERT INTO ops_state (branch_id, phase) VALUES ('MAIN', 'OPEN') \
         ON CONFLICT (branch_id) DO UPDATE SET phase = 'OPEN'",
    )
    .await;
    db.exec(
        &format!(
            "INSERT INTO cash_limits (organization_id, branch_id, count_difference_limit_thb, updated_by, updated_at) \
             VALUES ('{ORG_1}', 'MAIN', 100000, 'test', now())"
        ),
    )
    .await;
    lg4_fund(
        &db.pool,
        CashPlace::Bank,
        CashPlace::Safe(BranchId::new(MAIN)),
        "KWD",
        "1.123",
    )
    .await;

    let kwd_0005: i64 = sqlx::query_scalar(
        "INSERT INTO denominations (organization_id, currency_code, face_value, group_code) \
         VALUES ($1::uuid, 'KWD', 0.005, 'ALL') RETURNING id",
    )
    .bind(ORG_1)
    .fetch_one(&db.pool)
    .await
    .expect("KWD 0.005 note");

    let finance_id: String = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         VALUES ($1::uuid, 'finance', 'ACTIVE', 'test') RETURNING id::text",
    )
    .bind(ORG_1)
    .fetch_one(&db.pool)
    .await
    .expect("insert finance");
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let finance = User::new(
        &finance_id,
        ORG_1,
        vec![ScopedRole::new(
            Role::Finance,
            Some(BranchId::new(MAIN)),
            from,
            None,
        )],
        None,
    );
    let manager = User::new(
        &manager_id,
        ORG_1,
        vec![ScopedRole::new(
            Role::BranchManager,
            Some(BranchId::new(MAIN)),
            from,
            None,
        )],
        None,
    );

    let report_day = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
    let tz: Tz = "Asia/Bangkok".parse().unwrap();
    let noon = tz
        .from_local_datetime(&report_day.and_hms_opt(12, 0, 0).unwrap())
        .single()
        .unwrap()
        .with_timezone(&Utc);

    let counts = CashCountService::pg(db.pool.clone());
    let submitted = counts
        .submit(
            &finance,
            SubmitCount {
                branch: BranchId::new(MAIN),
                shift_id: None,
                kind: CountKind::Safe,
                lines: vec![CountLine::new("KWD", kwd_0005, 301)],
                note: None,
                idempotency_key: unique("kwd-safe"),
            },
            noon,
        )
        .await
        .expect("submit KWD safe count");
    counts
        .verify(&manager, &submitted.id, noon + Duration::seconds(1))
        .await
        .expect("verify KWD safe count");

    let report = fetch_report(&db, &manager_id, report_day).await;
    let kwd = report["currencies"]
        .as_array()
        .expect("currencies array")
        .iter()
        .find(|r| r["currency"] == "KWD")
        .expect("KWD row listed in report");

    assert_eq!(kwd["safeCounted"].as_str(), Some("1.505"));
    assert_eq!(kwd["safeExpected"].as_str(), Some("1.123"));
    assert_eq!(kwd["safeDifference"].as_str(), Some("0.382"));

    db.finish().await;
}
