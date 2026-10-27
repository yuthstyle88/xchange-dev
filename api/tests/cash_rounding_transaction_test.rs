//! Ticket #5122 (CR-b) — up-front tests for cash settlement rounding through transactions and
//! shift close:
//! - CR8: `POST /api/transactions` refuses when `payment.method.is_cash()` disagrees with the
//!   quote's `cash_settlement` (in both directions) with `400 Bad Request` on field
//!   `paymentMethod`, writing no transaction row.
//! - CR9: a `Cash` quote (BUY USD 1 at 32.10) created, confirmed and completed moves the drawer's
//!   THB by `−32.25`; the stored `TransactionLine` has `settle_thb == 32.25` and
//!   `line.rounding_thb() == 0.15`; the transaction answer's line has `settleThb: "32.25"` and
//!   `roundingThb: "0.15"`.
//! - CR11: after CR9, closing the shift with an exact physical `CLOSE` count verifies with
//!   `difference == 0` and no FINANCE `COUNT_DIFFERENCE` approval request.
//!
//! Written before the implementation by the up-front test writer (gemini/pool-2). Do not edit,
//! delete or weaken; add tests in a file of your own.
//!
//! Design: `docs/design/blueprint/quote-transaction.md` §13; #5122 §3, §4 (CR8, CR9, CR11).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here
//! skips.

mod common;
mod quote_common;

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::{Arc, Once};

use actix_web::http::{Method, StatusCode, header};
use actix_web::{App, HttpRequest, test as atest, web};
use chrono::{TimeZone, Utc};
use pos_infra::transactions::Transactor;
use quote_common::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use uuid::Uuid;
use xc_auth::domain::access::{Actor as User, Role, ScopedRole};
use xc_cash::domain::ledger::posting::{SourceRef, SourceType};
use xc_cash::domain::posting::{CashPlace, CashPostings, PostingRequest};
use xc_cash::infrastructure::ledger_postings::LedgerCashPostings;
use xc_customer::application::customer::service::CustomerService;
use xc_customer::domain::customer::commands::reserve_customer::ReserveCustomer;
use xc_exchange::domain::transaction::repository::TransactionStore;
use xc_exchange::domain::transaction::transaction::TransactionId;
use xc_exchange::infrastructure::transaction::store::PgTransactionStore;
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount, Money};
use xchange::bootstrap::{self, AppState};
use xchange::presentation::pages::{Callers, SessionReader};

const AS: &str = "x-test-as";
const HOST: &str = "xc.test";
const SAME_ORIGIN: &str = "http://xc.test";
const TEST_PII_KEYS: &str = "test-1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

static PII: Once = Once::new();

struct Who {
    id: String,
    spec: String,
}

fn user_of(spec: &str) -> Option<User> {
    let mut parts = spec.splitn(3, '|');
    let (id, role, branch) = (parts.next()?, parts.next()?, parts.next()?);
    let role = Role::parse(role)?;
    let branch = (branch != "*").then(|| BranchId::new(branch));
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    Some(User::new(
        id,
        ORG_1,
        vec![ScopedRole::new(role, branch, from, None)],
        None,
    ))
}

struct HeaderUsers;

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

struct Answer {
    status: StatusCode,
    text: String,
    body: Value,
}

impl Answer {
    fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }

    fn field(&self) -> Option<&str> {
        self.body["error"]["field"].as_str()
    }
}

struct Room {
    db: TestDb,
    teller: Who,
    manager: Who,
    customer_id: String,
    shift_id: String,
    scratch: std::path::PathBuf,
}

impl Room {
    async fn start() -> Self {
        PII.call_once(|| {
            let keys = Arc::new(xc_kernel::pii::PiiKeys::parse(TEST_PII_KEYS).unwrap());
            let _ = xc_customer::install_pii_keys(keys);
        });
        let db = TestDb::start().await;
        let now_utc = Utc::now();
        let open_hhmm = (now_utc - chrono::Duration::hours(2))
            .format("%H:%M")
            .to_string();
        let close_hhmm = (now_utc + chrono::Duration::hours(2))
            .format("%H:%M")
            .to_string();
        let day_win = json!([{ "open": open_hhmm, "close": close_hhmm }]);
        let hours = json!({
            "mon": day_win, "tue": day_win, "wed": day_win, "thu": day_win,
            "fri": day_win, "sat": day_win, "sun": day_win
        });
        db.exec(&format!(
            "UPDATE branches SET timezone = 'UTC', opening_hours = '{hours}'::jsonb WHERE id = 'MAIN'"
        ))
        .await;
        db.publish("MAIN", &[USD]).await;
        db.exec(
            "INSERT INTO ops_state (branch_id, phase) VALUES ('MAIN', 'OPEN') \
             ON CONFLICT (branch_id) DO UPDATE SET phase = 'OPEN'",
        )
        .await;

        let mut tx = db.pool.begin().await.expect("begin");
        LedgerCashPostings::new()
            .post_in(
                &mut tx,
                &[PostingRequest {
                    from: CashPlace::Bank,
                    to: CashPlace::Safe(BranchId::new("MAIN")),
                    currency: CurrencyCode::THB,
                    quantity: ForeignAmount::from_decimal("1000.00".parse().expect("amount")),
                    source: SourceRef {
                        source_type: SourceType::Opening,
                        source_id: unique("fund-safe-thb"),
                        source_key: "fund".to_owned(),
                    },
                    line: None,
                    reverses: None,
                }],
                None,
            )
            .await
            .expect("fund safe");
        tx.commit().await.expect("commit");

        let teller = Self::seed_person(&db, Role::Teller, Some("MAIN")).await;
        let manager = Self::seed_person(&db, Role::BranchManager, Some("MAIN")).await;

        for (key, sha) in [("PASSPORT-MAIN", "sha-passport"), ("SLIP-MAIN", "sha-slip")] {
            db.exec(&format!(
                "INSERT INTO files (key, branch_id, owner_id, mime, sha256, created_at) \
                 VALUES ('{key}', 'MAIN', '{}', 'image/jpeg', '{sha}', now())",
                teller.id
            ))
            .await;
        }

        let customers = CustomerService::new(BranchTx::new(Transactor::new(db.pool.clone())));
        let customer_id = customers
            .reserve(
                &BranchId::new("MAIN"),
                ReserveCustomer {
                    name: "Somchai Jaidee".to_owned(),
                    passport_no: format!("P{}", &Uuid::new_v4().simple().to_string()[..19])
                        .to_uppercase(),
                    nationality: "THA".to_owned(),
                    birth_date: String::new(),
                    passport_expiry: String::new(),
                    mode: "LIVE".to_owned(),
                },
                whole_second_now(),
            )
            .await
            .expect("reserve LIVE customer")
            .customer_id;

        let scratch = std::env::temp_dir().join(unique("xc-crb"));
        std::fs::create_dir_all(scratch.join("uploads")).expect("create scratch dir");

        let mut room = Self {
            db,
            teller,
            manager,
            customer_id,
            shift_id: String::new(),
            scratch,
        };

        // Open MAIN's shift with THB 20 x 2 = 40.00 through the cash-counts routes.
        let thb20 = room.denomination("THB", "20").await;
        let open = room
            .call(
                &room.teller,
                Method::POST,
                "/api/cash-counts",
                Some(json!({
                    "branch": "MAIN",
                    "kind": "OPEN",
                    "lines": [{ "currency": "THB", "denominationId": thb20, "qty": "2" }]
                })),
            )
            .await;
        assert_eq!(open.status, StatusCode::OK, "{}", open.text);
        let open_id = open.body["id"].as_str().expect("open count id");

        let verified = room
            .call(
                &room.manager,
                Method::POST,
                &format!("/api/cash-counts/{open_id}/verify"),
                Some(json!({ "branch": "MAIN" })),
            )
            .await;
        assert_eq!(verified.status, StatusCode::OK, "{}", verified.text);

        room.shift_id = sqlx::query_scalar(
            "SELECT id::text FROM shifts WHERE teller_id = $1::uuid AND status = 'OPEN'",
        )
        .bind(&room.teller.id)
        .fetch_one(&room.db.pool)
        .await
        .expect("open shift id");

        room
    }

    async fn seed_person(db: &TestDb, role: Role, branch: Option<&str>) -> Who {
        let id = xc_testdb::people::seed_person(
            &mut db.pool.acquire().await.expect("acquire"),
            ORG_1,
            role.as_str(),
            &[xc_testdb::people::RoleRow {
                role_code: role.as_str(),
                branch_id: branch,
                valid_from: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                valid_to: None,
            }],
        )
        .await
        .expect("seed person");
        Who {
            spec: format!("{id}|{}|{}", role.as_str(), branch.unwrap_or("*")),
            id,
        }
    }

    async fn denomination(&self, currency: &str, face: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM denominations WHERE currency_code = $1 AND face_value = $2::numeric",
        )
        .bind(currency)
        .bind(face)
        .fetch_one(&self.db.pool)
        .await
        .unwrap_or_else(|e| panic!("denomination {currency} {face}: {e}"))
    }

    async fn call(&self, who: &Who, method: Method, uri: &str, body: Option<Value>) -> Answer {
        let uploads = self.scratch.join("uploads");
        let state = AppState::new(
            Arc::new(common::settings(&[(
                "APP_FILES__DIR",
                uploads.to_str().expect("UTF-8 path"),
            )])),
            self.db.pool.clone(),
        );
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
            .insert_header((AS, who.spec.clone()));
        if let Some(b) = &body {
            req = req
                .insert_header(("x-xchange-request", "1"))
                .insert_header((header::ORIGIN, SAME_ORIGIN))
                .insert_header((header::CONTENT_TYPE, "application/json"))
                .insert_header(("idempotency-key", unique("key")))
                .set_payload(b.to_string());
        }
        let resp = atest::call_service(&app, req.to_request()).await;
        let status = resp.status();
        let bytes = atest::read_body(resp).await;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Answer { status, text, body }
    }

    async fn drawer_balance(&self, currency: CurrencyCode) -> Decimal {
        let mut tx = self.db.pool.begin().await.expect("begin");
        let balances = LedgerCashPostings::new()
            .expected_in(&mut tx, &CashPlace::Drawer(self.shift_id.clone()))
            .await
            .expect("expected_in");
        tx.commit().await.expect("commit");
        balances.get(currency).value()
    }

    async fn finish(self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
        self.db.finish().await;
    }
}

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("a decimal")
}

/// Runs CR9's cash BUY USD 1 at 32.10 through quote, create, confirm and complete, returning the
/// four HTTP answers so callers can inspect both the exchange and the subsequent shift close.
async fn execute_cash_buy_usd_1(room: &Room) -> (Answer, Answer, Answer, Answer) {
    // 1. Quote with Cash: BUY USD 1 at 32.10.
    let quote = room
        .call(
            &room.teller,
            Method::POST,
            "/api/quotes",
            Some(json!({
                "branch": "MAIN",
                "customerId": room.customer_id,
                "paymentMethod": "Cash",
                "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": "1" }]
            })),
        )
        .await;
    assert_eq!(quote.status, StatusCode::OK, "{}", quote.text);
    let quote_id = quote.body["id"].as_str().expect("quote id");

    // 2. Create DRAFT with Cash.
    let created = room
        .call(
            &room.teller,
            Method::POST,
            "/api/transactions",
            Some(json!({
                "branch": "MAIN",
                "quoteId": quote_id,
                "customerId": room.customer_id,
                "passport": { "uploadKey": "PASSPORT-MAIN" },
                "payment": {
                    "method": "Cash",
                    "reference": "",
                    "account": "Cash drawer 1",
                    "slip": null,
                    "bankFeeThb": "0.00",
                    "paidAt": "2026-10-06T09:00:00Z"
                },
                "valueDate": "2026-10-06",
                "notes": "",
                "countConfirmed": true
            })),
        )
        .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.text);
    assert_eq!(created.body["status"], json!("DRAFT"));
    let tx_id = created.body["id"].as_str().expect("tx id").to_owned();

    // 3. Confirm.
    let confirmed = room
        .call(
            &room.teller,
            Method::POST,
            &format!("/api/transactions/{tx_id}/confirm"),
            Some(json!({ "branch": "MAIN" })),
        )
        .await;
    assert_eq!(confirmed.status, StatusCode::OK, "{}", confirmed.text);
    assert_eq!(confirmed.body["status"], json!("CONFIRMED"));

    // 4. Complete.
    let completed = room
        .call(
            &room.teller,
            Method::POST,
            &format!("/api/transactions/{tx_id}/complete"),
            Some(json!({ "branch": "MAIN" })),
        )
        .await;
    assert_eq!(completed.status, StatusCode::OK, "{}", completed.text);
    assert_eq!(completed.body["status"], json!("COMPLETED"));

    (quote, created, confirmed, completed)
}

/// Runs CR9's cash BUY USD 1 at 32.10 and asserts every CR9 requirement.
async fn run_cr9_cash_exchange(room: &Room) -> String {
    let thb_before = room.drawer_balance(CurrencyCode::THB).await;
    let usd_before = room
        .drawer_balance(CurrencyCode::parse("USD").unwrap())
        .await;
    assert_eq!(thb_before, d("40.00"));
    assert_eq!(usd_before, d("0"));

    let (quote, created, confirmed, completed) = execute_cash_buy_usd_1(room).await;
    let tx_id = created.body["id"].as_str().expect("tx id").to_owned();

    // Drawer THB moved by -32.25 (from 40.00 to 7.75); USD moved from 0 to 1.
    let thb_after = room.drawer_balance(CurrencyCode::THB).await;
    let usd_after = room
        .drawer_balance(CurrencyCode::parse("USD").unwrap())
        .await;
    assert_eq!(thb_after - thb_before, d("-32.25"));
    assert_eq!(thb_after, d("7.75"));
    assert_eq!(usd_after, d("1"));

    // Stored TransactionLine has settle_thb == 32.25 and rounding_thb() == 0.15.
    let store = PgTransactionStore::new(room.db.pool.clone());
    let mut tx = room.db.pool.begin().await.expect("begin");
    let stored = store
        .find_in(
            &mut tx,
            TransactionId(Uuid::parse_str(&tx_id).expect("tx uuid")),
        )
        .await
        .expect("find_in")
        .expect("stored transaction");
    tx.commit().await.expect("commit");
    assert_eq!(
        stored.lines[0].settle_thb,
        Money::parse("32.25").expect("money")
    );
    assert_eq!(stored.lines[0].rounding_thb(), d("0.15"));

    // Quote and transaction answers carry settleThb "32.25" and roundingThb "0.15".
    assert_eq!(quote.body["cashSettlement"], json!(true));
    assert_eq!(quote.body["lines"][0]["settleThb"], json!("32.25"));
    assert_eq!(quote.body["lines"][0]["roundingThb"], json!("0.15"));

    for ans in [&created, &confirmed, &completed] {
        assert_eq!(ans.body["lines"][0]["settleThb"], json!("32.25"));
        assert_eq!(ans.body["lines"][0]["roundingThb"], json!("0.15"));
        assert_eq!(ans.body["payment"]["amountThb"], json!("32.25"));
    }

    let view = room
        .call(
            &room.teller,
            Method::GET,
            &format!("/api/v3/views/transaction?branch=MAIN&id={tx_id}"),
            None,
        )
        .await;
    assert_eq!(view.status, StatusCode::OK, "{}", view.text);
    assert_eq!(
        view.body["data"]["transaction"]["lines"][0]["settleThb"],
        json!("32.25")
    );
    assert_eq!(
        view.body["data"]["transaction"]["lines"][0]["roundingThb"],
        json!("0.15")
    );

    tx_id
}

// ============================================================ CR8

#[actix_web::test]
async fn cr8_payment_method_must_match_quote_cash_settlement_in_both_directions() {
    let room = Room::start().await;

    // 1. Quote with Cash, then POST /api/transactions with PromptPay -> 400 on field paymentMethod.
    let q_cash = room
        .call(
            &room.teller,
            Method::POST,
            "/api/quotes",
            Some(json!({
                "branch": "MAIN",
                "customerId": room.customer_id,
                "paymentMethod": "Cash",
                "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": "1" }]
            })),
        )
        .await;
    assert_eq!(q_cash.status, StatusCode::OK, "{}", q_cash.text);
    let q_cash_id = q_cash.body["id"].as_str().expect("cash quote id");

    let mismatch_1 = room
        .call(
            &room.teller,
            Method::POST,
            "/api/transactions",
            Some(json!({
                "branch": "MAIN",
                "quoteId": q_cash_id,
                "customerId": room.customer_id,
                "passport": { "uploadKey": "PASSPORT-MAIN" },
                "payment": {
                    "method": "PromptPay",
                    "reference": "PP-REF-001",
                    "account": "SCB-001",
                    "slip": { "uploadKey": "SLIP-MAIN" },
                    "bankFeeThb": "0.00",
                    "paidAt": "2026-10-06T09:00:00Z"
                },
                "valueDate": "2026-10-06",
                "notes": "",
                "countConfirmed": true
            })),
        )
        .await;
    assert_eq!(
        mismatch_1.status,
        StatusCode::BAD_REQUEST,
        "{}",
        mismatch_1.text
    );
    assert_eq!(mismatch_1.code(), "validationFailed");
    assert_eq!(mismatch_1.field(), Some("paymentMethod"));
    assert_eq!(
        room.db.count("SELECT count(*) FROM transactions").await,
        0,
        "Cash quote + PromptPay transaction must write no transaction row"
    );

    // 2. Reverse: quote with PromptPay, then POST /api/transactions with Cash -> 400 on field paymentMethod.
    let q_pp = room
        .call(
            &room.teller,
            Method::POST,
            "/api/quotes",
            Some(json!({
                "branch": "MAIN",
                "customerId": room.customer_id,
                "paymentMethod": "PromptPay",
                "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": "1" }]
            })),
        )
        .await;
    assert_eq!(q_pp.status, StatusCode::OK, "{}", q_pp.text);
    let q_pp_id = q_pp.body["id"].as_str().expect("promptpay quote id");

    let mismatch_2 = room
        .call(
            &room.teller,
            Method::POST,
            "/api/transactions",
            Some(json!({
                "branch": "MAIN",
                "quoteId": q_pp_id,
                "customerId": room.customer_id,
                "passport": { "uploadKey": "PASSPORT-MAIN" },
                "payment": {
                    "method": "Cash",
                    "reference": "",
                    "account": "Cash drawer 1",
                    "slip": null,
                    "bankFeeThb": "0.00",
                    "paidAt": "2026-10-06T09:00:00Z"
                },
                "valueDate": "2026-10-06",
                "notes": "",
                "countConfirmed": true
            })),
        )
        .await;
    assert_eq!(
        mismatch_2.status,
        StatusCode::BAD_REQUEST,
        "{}",
        mismatch_2.text
    );
    assert_eq!(mismatch_2.code(), "validationFailed");
    assert_eq!(mismatch_2.field(), Some("paymentMethod"));
    assert_eq!(
        room.db.count("SELECT count(*) FROM transactions").await,
        0,
        "PromptPay quote + Cash transaction must write no transaction row"
    );

    room.finish().await;
}

// ============================================================ CR9

#[actix_web::test]
async fn cr9_cash_quote_create_confirm_and_complete_moves_drawer_by_rounded_settle_and_reports_rounding()
 {
    let room = Room::start().await;
    let _tx_id = run_cr9_cash_exchange(&room).await;
    room.finish().await;
}

// ============================================================ CR11

#[actix_web::test]
async fn cr11_after_cr9_exact_close_count_verifies_with_zero_difference_and_no_finance_request() {
    let room = Room::start().await;
    let _ = execute_cash_buy_usd_1(&room).await;

    // Drawer now holds THB 7.75 (5 + 2 + 0.50 + 0.25) and USD 1 (1).
    let thb5 = room.denomination("THB", "5").await;
    let thb2 = room.denomination("THB", "2").await;
    let thb050 = room.denomination("THB", "0.5").await;
    let thb025 = room.denomination("THB", "0.25").await;
    let usd1 = room.denomination("USD", "1").await;

    let close = room
        .call(
            &room.teller,
            Method::POST,
            "/api/cash-counts",
            Some(json!({
                "branch": "MAIN",
                "kind": "CLOSE",
                "shiftId": room.shift_id,
                "lines": [
                    { "currency": "THB", "denominationId": thb5, "qty": "1" },
                    { "currency": "THB", "denominationId": thb2, "qty": "1" },
                    { "currency": "THB", "denominationId": thb050, "qty": "1" },
                    { "currency": "THB", "denominationId": thb025, "qty": "1" },
                    { "currency": "USD", "denominationId": usd1, "qty": "1" }
                ]
            })),
        )
        .await;
    assert_eq!(close.status, StatusCode::OK, "{}", close.text);
    let close_id = close.body["id"].as_str().expect("close count id");

    let verify = room
        .call(
            &room.manager,
            Method::POST,
            &format!("/api/cash-counts/{close_id}/verify"),
            Some(json!({ "branch": "MAIN" })),
        )
        .await;
    assert_eq!(verify.status, StatusCode::OK, "{}", verify.text);
    assert_eq!(verify.body["status"], json!("VERIFIED"));
    assert_eq!(verify.body["approvalRequestId"], Value::Null);

    let non_zero_diffs = room
        .db
        .count(&format!(
            "SELECT count(*) FROM cash_count_totals \
             WHERE count_id = '{close_id}'::uuid AND difference <> 0"
        ))
        .await;
    assert_eq!(non_zero_diffs, 0, "every currency total has difference = 0");

    let finance_requests = room
        .db
        .count("SELECT count(*) FROM approval_requests WHERE type = 'COUNT_DIFFERENCE'")
        .await;
    assert_eq!(finance_requests, 0, "no FINANCE COUNT_DIFFERENCE request");

    room.finish().await;
}
