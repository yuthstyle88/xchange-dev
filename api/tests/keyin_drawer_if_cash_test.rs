//! F1 (eb2849a5 on-screen report, #564 issuecomment-6596339013): before the payment method is chosen, a DESK quote
//! checks the drawer for a cash payout (design `docs/design/blueprint/ledger-inventory.md` §5.1.3 item 1a, rows
//! K-8..K-15). The rate step quotes with no `paymentMethod` (`web/src/screens/desk/desk-store.ts:324`), so today a BUY
//! gets no drawer need (`drawer_check.rs:25-26`) and the teller learns of a THB shortfall only when create refuses.
//!
//! Written before the implementation by the ticket author claude/b091f5e0. Do not edit, delete or weaken; add tests in
//! a file of your own. The harness is `keyin_drawer_check_test.rs`'s `Room` (MAIN's open shift with THB 40.00 in the
//! drawer, USD BUY 32.10 / SELL 33.00 published). USD 2 BUY = THB 64.20, 64.25 when settled in cash.
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

/// A DESK quote with **no** `paymentMethod`, as the rate step sends it (`desk-store.ts:324`).
async fn quote_undecided(room: &Room, side: &str, quantity: &str) -> Answer {
    room.call(
        &room.teller,
        Method::POST,
        "/api/quotes",
        Some(json!({
            "branch": "MAIN",
            "customerId": room.customer_id,
            "lines": [{ "shopSide": side, "currency": "USD", "quantity": quantity }]
        })),
    )
    .await
}

/// MAIN's drawer low level for THB (`cash_drawer_low_levels`, ledger-inventory §5.1.3 item 2).
async fn set_thb_low_level(room: &Room, level: &str) {
    room.db
        .exec(&format!(
            "INSERT INTO cash_drawer_low_levels \
                 (organization_id, branch_id, currency, low_level, updated_by, updated_at) \
             SELECT b.organization_id, b.id, 'THB', {level}, '{}'::uuid, now() FROM branches b WHERE b.id = 'MAIN'",
            room.manager.id
        ))
        .await;
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

/// K-8: a BUY of USD 2 with no method against a drawer of THB 40.00: the quote checks a cash payout (64.25) and says so.
#[actix_web::test]
async fn k8_an_undecided_buy_names_the_cash_payout_shortfall() {
    let room = Room::start().await;
    let q = quote_undecided(&room, "BUY", "2").await;
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
    assert_eq!(q.body["drawerCheckIfCash"], json!(true), "{}", q.text);
    assert_eq!(
        q.body["cashSettlement"],
        json!(false),
        "the quote itself stays non-cash: {}",
        q.text
    );
    room.finish().await;
}

/// K-9: the shortfall uses the cash-rounded settle (64.25), not the quote's own `settleThb` (64.20).
#[actix_web::test]
async fn k9_the_undecided_need_is_the_cash_rounded_settle() {
    let room = Room::start().await;
    let q = quote_undecided(&room, "BUY", "2").await;
    assert_eq!(q.status, StatusCode::OK, "{}", q.text);
    assert_eq!(
        q.body["lines"][0]["settleThb"],
        json!("64.20"),
        "{}",
        q.text
    );
    assert_eq!(
        q.body["drawerShortfall"][0]["shortBy"],
        json!("24.25"),
        "64.25 − 40.00, not 64.20 − 40.00: {}",
        q.text
    );
    room.finish().await;
}

/// K-10: an undecided BUY the drawer can pay (USD 1: 32.25 in cash) that leaves THB 7.75, at or below a level of 10.
#[actix_web::test]
async fn k10_an_undecided_buy_names_the_low_level_it_would_reach() {
    let room = Room::start().await;
    set_thb_low_level(&room, "10").await;
    let q = quote_undecided(&room, "BUY", "1").await;
    assert_eq!(q.status, StatusCode::OK, "{}", q.text);
    assert_eq!(q.body["drawerShortfall"], json!([]), "{}", q.text);
    assert_eq!(q.body["belowLowCashLevel"], json!(["THB"]), "{}", q.text);
    assert_eq!(q.body["drawerCheckIfCash"], json!(true), "{}", q.text);
    room.finish().await;
}

/// K-11: an undecided BUY the drawer pays with room to spare: no advice, but the basis is still "if paid in cash".
#[actix_web::test]
async fn k11_an_undecided_buy_the_drawer_can_pay_has_no_advice() {
    let room = Room::start().await;
    let q = quote_undecided(&room, "BUY", "1").await;
    assert_eq!(q.status, StatusCode::OK, "{}", q.text);
    assert_eq!(q.body["drawerShortfall"], json!([]), "{}", q.text);
    assert_eq!(q.body["drawerShortfallMessage"], Value::Null, "{}", q.text);
    assert_eq!(q.body["belowLowCashLevel"], json!([]), "{}", q.text);
    assert_eq!(q.body["drawerCheckIfCash"], json!(true), "{}", q.text);
    room.finish().await;
}

/// K-12: an undecided SELL short of USD names the USD shortfall as today; it has no BUY line, so the flag is false.
#[actix_web::test]
async fn k12_an_undecided_sell_is_checked_as_today() {
    let room = Room::start().await;
    let q = quote_undecided(&room, "SELL", "1").await;
    assert_eq!(q.status, StatusCode::OK, "{}", q.text);
    assert_eq!(
        q.body["drawerShortfall"],
        json!([{ "currency": "USD", "place": "DRAWER", "shortBy": "1" }]),
        "{}",
        q.text
    );
    assert_eq!(q.body["drawerCheckIfCash"], json!(false), "{}", q.text);
    room.finish().await;
}

/// K-13: a BUY paid in Cash (K1) or by PromptPay (K6) is checked as today, and the flag is false.
#[actix_web::test]
async fn k13_a_chosen_method_keeps_todays_check_and_a_false_flag() {
    let room = Room::start().await;
    let cash = quote(&room, "BUY", "2", "Cash").await;
    assert_eq!(cash.status, StatusCode::OK, "{}", cash.text);
    assert_eq!(
        cash.body["drawerShortfall"],
        json!([{ "currency": "THB", "place": "DRAWER", "shortBy": "24.25" }]),
        "{}",
        cash.text
    );
    assert_eq!(
        cash.body["drawerCheckIfCash"],
        json!(false),
        "{}",
        cash.text
    );

    let promptpay = quote(&room, "BUY", "2", "PromptPay").await;
    assert_eq!(promptpay.status, StatusCode::OK, "{}", promptpay.text);
    assert_eq!(
        promptpay.body["drawerShortfall"],
        json!([]),
        "{}",
        promptpay.text
    );
    assert_eq!(
        promptpay.body["drawerCheckIfCash"],
        json!(false),
        "{}",
        promptpay.text
    );
    room.finish().await;
}

/// K-14: create is unchanged. The undecided quote cannot be created as cash (re-quote, as today); the cash re-quote
/// of the same BUY is refused `409 stockInsufficient` (K3) and nothing is written.
#[actix_web::test]
async fn k14_create_is_unchanged_after_an_undecided_quote() {
    let room = Room::start().await;
    let undecided = quote_undecided(&room, "BUY", "2").await;
    let id = undecided.body["id"].as_str().expect("quote id").to_owned();
    let mismatch = create_cash(&room, &id).await;
    assert_eq!(
        (mismatch.status, mismatch.code()),
        (StatusCode::BAD_REQUEST, "validationFailed"),
        "a non-cash quote is re-quoted for cash, as today: {}",
        mismatch.text
    );

    let cash = quote(&room, "BUY", "2", "Cash").await;
    let cash_id = cash.body["id"].as_str().expect("quote id").to_owned();
    let c = create_cash(&room, &cash_id).await;
    assert_eq!(
        (c.status, c.code()),
        (StatusCode::CONFLICT, "stockInsufficient"),
        "{}",
        c.text
    );
    assert_eq!(
        room.db.count("SELECT count(*) FROM transactions").await,
        0,
        "nothing written"
    );
    room.finish().await;
}

/// K-15: a non-DESK quote with no method (the branch manager, who has no open shift, quoting for the desk counts as
/// "no drawer"): no drawer advice and a false flag.
#[actix_web::test]
async fn k15_no_drawer_means_no_advice_and_a_false_flag() {
    let room = Room::start().await;
    let q = room
        .call(
            &room.manager,
            Method::POST,
            "/api/quotes",
            Some(json!({
                "branch": "MAIN",
                "customerId": room.customer_id,
                "lines": [{ "shopSide": "BUY", "currency": "USD", "quantity": "2" }]
            })),
        )
        .await;
    assert_eq!(q.status, StatusCode::OK, "{}", q.text);
    assert_eq!(q.body["drawerShortfall"], json!([]), "{}", q.text);
    assert_eq!(q.body["belowLowCashLevel"], json!([]), "{}", q.text);
    assert_eq!(q.body["drawerCheckIfCash"], json!(false), "{}", q.text);
    room.finish().await;
}
