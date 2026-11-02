//! Key-in check (design `docs/design/blueprint/ledger-inventory.md` §5.1.3 "1. Key-in check"): a DESK quote tells the
//! teller when their drawer cannot pay (`drawerShortfall` and `drawerShortfallMessage`, advisory), and create refuses
//! a cash exchange the drawer cannot pay with §5.1.2's `409 stockInsufficient` body, writing nothing.
//!
//! Root cause pinned here: the v3 create (`CreateExchange`, `api/src/application/workflows/exchange/create.rs:348-397`)
//! checked no stock, so a short BUY passed quote, create and confirm and was refused only at complete
//! (#564 issuecomment-6572802282; re-measured 2026-11-02 for #5971).
//!
//! Written before the implementation by the ticket author claude/b091f5e0. Do not edit, delete or weaken; add tests in
//! a file of your own. The harness is `cash_rounding_transaction_test.rs`'s `Room` (MAIN's open shift with THB 40.00
//! in the drawer, USD BUY 32.10 / SELL 33.00 published, the safe funded with THB 1,000).
//!
//! Needs `TEST_DATABASE_URL` naming an existing database whose name ends in `_test`; nothing here skips.

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
use xc_kernel::branch_tx::BranchTx;
use xc_kernel::ids::BranchId;
use xc_kernel::money::{CurrencyCode, ForeignAmount};
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

const SHORT_THB: &str = "ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: THB ลิ้นชัก ขาด 24.25";

/// A DESK quote for `quantity` USD on `side` (`"BUY"`/`"SELL"`), paid by `method`.
async fn quote(room: &Room, side: &str, quantity: &str, method: &str) -> Answer {
    room.call(
        &room.teller,
        Method::POST,
        "/api/quotes",
        Some(json!({
            "branch": "MAIN",
            "customerId": room.customer_id,
            "paymentMethod": method,
            "lines": [{ "shopSide": side, "currency": "USD", "quantity": quantity }]
        })),
    )
    .await
}

/// `POST /api/transactions` for `quote_id`, paid in cash.
async fn create_cash(room: &Room, quote_id: &str) -> Answer {
    room.call(
        &room.teller,
        Method::POST,
        "/api/transactions",
        Some(json!({
            "branch": "MAIN",
            "quoteId": quote_id,
            "customerId": room.customer_id,
            "passport": { "uploadKey": "PASSPORT-MAIN" },
            "payment": {
                "method": "Cash", "reference": "", "account": "Cash drawer 1", "slip": null,
                "bankFeeThb": "0.00", "paidAt": "2026-10-06T09:00:00Z"
            },
            "valueDate": "2026-10-06",
            "notes": "",
            "countConfirmed": true
        })),
    )
    .await
}

/// THB `amount` from the bank into MAIN's open drawer through the ledger (the top-up §5.1.2 asks for).
async fn top_up_thb(room: &Room, amount: &str) {
    let mut tx = room.db.pool.begin().await.expect("begin");
    LedgerCashPostings::new()
        .post_in(
            &mut tx,
            &[PostingRequest {
                from: CashPlace::Bank,
                to: CashPlace::Drawer(room.shift_id.clone()),
                currency: CurrencyCode::THB,
                quantity: ForeignAmount::from_decimal(d(amount)),
                source: SourceRef {
                    source_type: SourceType::Transfer,
                    source_id: unique("top-up"),
                    source_key: "receive".to_owned(),
                },
                line: None,
                reverses: None,
            }],
            None,
        )
        .await
        .expect("top up the drawer");
    tx.commit().await.expect("commit");
}

/// K1: a cash BUY of USD 2 (THB 64.20, settled in cash at 64.25) against a drawer of THB 40.00: the quote is issued and names the shortfall.
#[actix_web::test]
async fn k1_a_short_cash_buy_quote_names_the_drawer_shortfall() {
    let room = Room::start().await;
    let q = quote(&room, "BUY", "2", "Cash").await;
    assert_eq!(
        q.status,
        StatusCode::OK,
        "advisory: the quote is still issued: {}",
        q.text
    );
    assert_eq!(
        q.body["drawerShortfall"],
        json!([{ "currency": "THB", "place": "DRAWER", "shortBy": "24.25" }]),
        "{}",
        q.text
    );
    assert_eq!(
        q.body["drawerShortfallMessage"],
        json!(SHORT_THB),
        "{}",
        q.text
    );
    room.finish().await;
}

/// K2: a cash BUY the drawer can pay (USD 1, THB 32.25): no shortfall.
#[actix_web::test]
async fn k2_a_quote_the_drawer_can_pay_has_no_shortfall() {
    let room = Room::start().await;
    let q = quote(&room, "BUY", "1", "Cash").await;
    assert_eq!(q.status, StatusCode::OK, "{}", q.text);
    assert_eq!(q.body["drawerShortfall"], json!([]), "{}", q.text);
    assert_eq!(q.body["drawerShortfallMessage"], Value::Null, "{}", q.text);
    room.finish().await;
}

/// K3: create of the short cash BUY is refused with §5.1.2's body and writes nothing.
#[actix_web::test]
async fn k3_create_of_a_short_cash_buy_is_refused_and_writes_nothing() {
    let room = Room::start().await;
    let q = quote(&room, "BUY", "2", "Cash").await;
    let id = q.body["id"].as_str().expect("quote id").to_owned();

    let c = create_cash(&room, &id).await;
    assert_eq!(
        (c.status, c.code()),
        (StatusCode::CONFLICT, "stockInsufficient"),
        "{}",
        c.text
    );
    assert_eq!(c.body["error"]["message"], json!(SHORT_THB), "{}", c.text);
    assert_eq!(
        room.db.count("SELECT count(*) FROM transactions").await,
        0,
        "nothing written"
    );
    assert_eq!(room.drawer_balance(CurrencyCode::THB).await, d("40.00"));
    room.finish().await;
}

/// K4: after a top-up, the same quote creates.
#[actix_web::test]
async fn k4_after_a_top_up_the_same_quote_creates() {
    let room = Room::start().await;
    let q = quote(&room, "BUY", "2", "Cash").await;
    let id = q.body["id"].as_str().expect("quote id").to_owned();
    let refused = create_cash(&room, &id).await;
    assert_eq!(refused.code(), "stockInsufficient", "{}", refused.text);

    top_up_thb(&room, "100.00").await;
    let c = create_cash(&room, &id).await;
    assert_eq!(c.status, StatusCode::OK, "{}", c.text);
    assert_eq!(c.body["status"], json!("DRAFT"), "{}", c.text);
    room.finish().await;
}

/// K5: a cash SELL of USD the drawer does not hold (USD 0.00): the quote names the USD shortfall; create refuses.
#[actix_web::test]
async fn k5_a_short_cash_sell_names_the_foreign_shortfall_and_create_refuses() {
    let room = Room::start().await;
    let q = quote(&room, "SELL", "1", "Cash").await;
    assert_eq!(q.status, StatusCode::OK, "{}", q.text);
    assert_eq!(
        q.body["drawerShortfall"],
        json!([{ "currency": "USD", "place": "DRAWER", "shortBy": "1" }]),
        "{}",
        q.text
    );
    let id = q.body["id"].as_str().expect("quote id").to_owned();
    let c = create_cash(&room, &id).await;
    assert_eq!(
        (c.status, c.code()),
        (StatusCode::CONFLICT, "stockInsufficient"),
        "{}",
        c.text
    );
    assert_eq!(
        c.body["error"]["message"],
        json!("ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: USD ลิ้นชัก ขาด 1"),
        "{}",
        c.text
    );
    room.finish().await;
}

/// K6: a BUY paid by PromptPay takes no THB from the drawer: no THB shortfall on the quote.
#[actix_web::test]
async fn k6_a_promptpay_buy_takes_no_thb_from_the_drawer() {
    let room = Room::start().await;
    let q = quote(&room, "BUY", "2", "PromptPay").await;
    assert_eq!(q.status, StatusCode::OK, "{}", q.text);
    assert_eq!(q.body["drawerShortfall"], json!([]), "{}", q.text);
    assert_eq!(q.body["drawerShortfallMessage"], Value::Null, "{}", q.text);
    room.finish().await;
}

/// K7: a caller with no open shift (the branch manager) gets no drawer advice: `[]` and `null`.
#[actix_web::test]
async fn k7_a_caller_with_no_open_shift_gets_no_drawer_advice() {
    let room = Room::start().await;
    let q = room
        .call(
            &room.manager,
            Method::POST,
            "/api/quotes",
            Some(json!({
                "branch": "MAIN",
                "customerId": room.customer_id,
                "paymentMethod": "Cash",
                "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": "2" }]
            })),
        )
        .await;
    assert_eq!(q.status, StatusCode::OK, "{}", q.text);
    assert_eq!(q.body["drawerShortfall"], json!([]), "{}", q.text);
    assert_eq!(q.body["drawerShortfallMessage"], Value::Null, "{}", q.text);
    room.finish().await;
}
