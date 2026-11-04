//! Backdated keying (`docs/design/backdated-keying.md` §9; ticket #6148):
//! detail view (`views/transaction.rs`) and shared report rows (`transaction_row.rs`, `admin_transactions`)
//! carry the backdated mark.
//!
//! Needs `TEST_DATABASE_URL` (a database ending in `_test`).

mod common;
mod quote_common;
mod tx4_common;

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use actix_web::http::{StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::Value;
use tx4_common::*;
use uuid::Uuid;
use xc_auth::domain::access::Role;
use xc_exchange::domain::exchange::value_objects::payment_method::PaymentMethod;
use xc_exchange::domain::transaction::line::TransactionLine;
use xc_exchange::domain::transaction::payment::TransactionPayment;
use xc_exchange::domain::transaction::repository::TransactionStore;
use xc_exchange::domain::transaction::transaction::{
    ExchangeTransaction, TransactionId, TransactionStatus,
};
use xc_kernel::ids::BranchId;
use xc_kernel::mode::Mode;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money, Rate, Side, UnitSize};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::dto::v3::transaction::BackdateMarkDto;
use xchange::presentation::dto::v3::transaction_detail::TransactionFilter;
use xchange::presentation::http::v3::transaction::TransactionView;
use xchange::presentation::http::v3::view::BranchView;
use xchange::presentation::pages::{Callers, SessionReader};
use xchange::presentation::test_callers;

const LIVE_CUSTOMER: &str = "CU-BK-VIEWS";
const KEYER: &str = "owner-bk";

fn entered() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 14, 0, 0).unwrap()
}

fn paid() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 4, 6, 23, 0).unwrap()
}

fn make_draft(org: Uuid) -> ExchangeTransaction {
    let id = TransactionId(Uuid::new_v4());
    let m = |s: &str| Money::parse(s).unwrap();
    ExchangeTransaction {
        queue_ticket_id: None,
        id,
        organization_id: org,
        branch_id: BranchId::new("MAIN"),
        mode: Mode::Live,
        status: TransactionStatus::Draft,
        document_ref: None,
        quote_id: None,
        booking_id: None,
        shift_id: None,
        submission_id: id.0.to_string(),
        payload_hash: "hash".to_owned(),
        customer_id: LIVE_CUSTOMER.to_owned(),
        customer_name: "Test Customer".to_owned(),
        nationality: "THA".to_owned(),
        staff_id: KEYER.to_owned(),
        booking_no: "WALK-IN".to_owned(),
        notes: String::new(),
        value_date: NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(),
        recorded_at: entered(),
        recorded_by: KEYER.to_owned(),
        completed_at: None,
        cancelled_at: None,
        cancel_reason: None,
        approval_request_id: None,
        passport_file_key: "PASSPORT-MAIN".to_owned(),
        kiosk_request_id: None,
        staff_draft_id: None,
        count_confirmed: false,
        lines: vec![TransactionLine {
            id: Uuid::new_v4(),
            quote_line_id: None,
            shop_side: Side::Buy,
            currency: CurrencyCode::parse("USD").unwrap(),
            quantity: ForeignAmount::from_decimal(Decimal::from_str("500").unwrap()),
            unit_size: UnitSize::ONE,
            rate_snapshot: Rate::parse("32.15").unwrap(),
            reference_rate: None,
            thb_amount: m("16075.00"),
            fee_thb: Money::ZERO,
            settle_thb: m("16075.00"),
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
            amount_thb: m("16075.00"),
            bank_fee_thb: Money::ZERO,
            paid_at: paid(),
        },
        receipts: vec![],
    }
}

async fn org(desk: &Desk) -> Uuid {
    Uuid::parse_str(
        &desk
            .db
            .text("SELECT organization_id::text FROM branches WHERE id = 'MAIN'")
            .await
            .expect("MAIN's organisation"),
    )
    .unwrap()
}

async fn start() -> Desk {
    let desk = Desk::start().await;
    desk.db.customer(LIVE_CUSTOMER, "MAIN").await;
    // Open shift for ordinary transactions
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (organization_id, name, status, created_by) \
         SELECT id, 'test', 'PENDING', 'test' FROM organizations WHERE code = 'XC' \
         RETURNING id",
    )
    .fetch_one(&desk.db.pool)
    .await
    .expect("test user");
    sqlx::query(
        "INSERT INTO shifts (id, branch_id, status, opened_at, opened_by, open_request_id, teller_id) \
         VALUES ('SH-LIVE-VIEWS', 'MAIN', 'OPEN', now(), 'owner-bk', 'REQ-SH-VIEWS', $1) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(user_id)
    .execute(&desk.db.pool)
    .await
    .expect("open shift");
    desk
}

struct HeaderCallers;

impl SessionReader for HeaderCallers {
    fn callers<'a>(&'a self, req: &'a HttpRequest) -> Pin<Box<dyn Future<Output = Callers> + 'a>> {
        let spec = req
            .headers()
            .get("x-test-as")
            .map(|v| v.to_str().expect("an ASCII header").to_owned())
            .unwrap_or_default();
        Box::pin(async move {
            if spec.is_empty() {
                return test_callers::nobody();
            }
            if spec == "owner" {
                return test_callers::callers(Some(test_callers::owner("owner-1")), None);
            }
            let mut parts = spec.splitn(3, ':');
            assert_eq!(parts.next(), Some("staff"), "spec {spec:?}");
            let role = parts.next().expect("a role");
            let roles: &'static [Role] = match role {
                "MANAGER" => &[Role::BranchManager],
                "FRONT" => &[Role::Teller],
                _ => panic!("unexpected role {role}"),
            };
            let branch = parts.next().expect("a branch");
            test_callers::callers(
                None,
                Some(test_callers::staff(
                    &format!("employee-{role}"),
                    roles,
                    BranchId::new(branch),
                )),
            )
        })
    }
}

/// Backdated row reads `backdate` in detail and `backdated: true` in admin_transactions;
/// ordinary row reads absent in detail and `backdated: false` in admin_transactions.
#[actix_web::test]
async fn backdated_and_ordinary_views_carry_expected_marks() {
    let desk = start().await;
    let o = org(&desk).await;

    // 1. Insert backdated draft
    let backdated_t = make_draft(o);
    let mut tx = desk.db.pool.begin().await.unwrap();
    desk.store
        .insert_backdated_draft_in(&mut tx, &backdated_t)
        .await
        .expect("insert backdated draft");
    tx.commit().await.unwrap();

    // 2. Insert ordinary draft (with open shift)
    let mut ordinary_t = make_draft(o);
    ordinary_t.id = TransactionId(Uuid::new_v4());
    ordinary_t.submission_id = ordinary_t.id.0.to_string();
    ordinary_t.shift_id = Some("SH-LIVE-VIEWS".to_owned());
    let mut tx = desk.db.pool.begin().await.unwrap();
    desk.store
        .insert_draft_in(&mut tx, &ordinary_t)
        .await
        .expect("insert ordinary draft");
    tx.commit().await.unwrap();

    let branch = BranchId::new("MAIN");
    let actor = owner(KEYER);

    // 3. Check direct detail view (TransactionView)
    let tx_view = TransactionView {
        pool: desk.db.pool.clone(),
    };

    // Backdated detail
    let b_data = tx_view
        .load(
            &branch,
            &actor,
            &TransactionFilter {
                id: Some(backdated_t.id.0.to_string()),
            },
        )
        .await
        .expect("load backdated detail");
    let b_detail = b_data.transaction.expect("backdated detail exists");
    assert_eq!(
        b_detail.backdate,
        Some(BackdateMarkDto {
            entered_at: entered().to_rfc3339(),
            entered_by: KEYER.to_owned(),
        })
    );
    let b_detail_json = serde_json::to_value(&b_detail).expect("serialize b_detail");
    assert_eq!(b_detail_json["backdate"]["enteredBy"], KEYER);

    // Ordinary detail
    let o_data = tx_view
        .load(
            &branch,
            &actor,
            &TransactionFilter {
                id: Some(ordinary_t.id.0.to_string()),
            },
        )
        .await
        .expect("load ordinary detail");
    let o_detail = o_data.transaction.expect("ordinary detail exists");
    assert_eq!(o_detail.backdate, None);
    let o_detail_json = serde_json::to_value(&o_detail).expect("serialize o_detail");
    assert_eq!(o_detail_json.get("backdate"), None);

    // 4. Check HTTP endpoints (views/transaction and views/admin_transactions)
    let state = AppState::new(Arc::new(common::settings(&[])), desk.db.pool.clone());
    let reader: Arc<dyn SessionReader> = Arc::new(HeaderCallers);
    let app = atest::init_service(
        App::new()
            .configure(move |cfg| bootstrap::configure(cfg, state))
            .app_data(web::Data::from(reader)),
    )
    .await;

    // HTTP detail view: backdated
    let req = atest::TestRequest::get()
        .uri(&format!(
            "/api/v3/views/transaction?branch=MAIN&id={}",
            backdated_t.id.0
        ))
        .insert_header((header::HOST, "xc.test"))
        .insert_header(("x-test-as", "staff:MANAGER:MAIN"))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = atest::read_body(resp).await;
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let http_b = &body["data"]["transaction"];
    assert_eq!(http_b["backdate"]["enteredBy"], KEYER);

    // HTTP detail view: ordinary
    let req = atest::TestRequest::get()
        .uri(&format!(
            "/api/v3/views/transaction?branch=MAIN&id={}",
            ordinary_t.id.0
        ))
        .insert_header((header::HOST, "xc.test"))
        .insert_header(("x-test-as", "staff:MANAGER:MAIN"))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = atest::read_body(resp).await;
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let http_o = &body["data"]["transaction"];
    assert_eq!(http_o.get("backdate"), None);

    // HTTP admin_transactions view
    let req = atest::TestRequest::get()
        .uri("/api/v3/views/admin_transactions?branch=MAIN&status=PENDING")
        .insert_header((header::HOST, "xc.test"))
        .insert_header(("x-test-as", "staff:MANAGER:MAIN"))
        .to_request();
    let resp = atest::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = atest::read_body(resp).await;
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let rows = body["data"].as_array().expect("array of rows");

    let b_row = rows
        .iter()
        .find(|r| r["id"] == backdated_t.id.0.to_string())
        .expect("found backdated row in admin_transactions");
    assert_eq!(b_row["backdated"], true);

    let o_row = rows
        .iter()
        .find(|r| r["id"] == ordinary_t.id.0.to_string())
        .expect("found ordinary row in admin_transactions");
    assert_eq!(o_row["backdated"], false);

    desk.db.finish().await;
}
